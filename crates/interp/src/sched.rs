//! The scheduler (`task.sched.1`): the tasks and scopes, which tasks are ready and what the
//! others wait for, how each ended, and the channels.
//!
//! One task runs at a time. The others are queued, first in, first out, and run only when the
//! running one suspends. A task waiting for something becomes ready once it has happened, in the
//! order the waiting tasks began to wait; a cancelled task waiting where cancellation is seen
//! becomes ready too, to unwind.

use std::collections::VecDeque;

use crate::Trap;
use crate::value::Value;

/// How a task ended.
#[derive(Clone, Debug)]
pub(crate) enum Outcome {
    Value(Value),
    Error(Value),
    /// It was cancelled, and unwound (`task.cancel.1`).
    Cancelled,
}

/// Why a scope failed: the first failure of its body or of a task given up (`task.error.1`), or
/// a trap in it, if it is isolated (`task.isolate.1`).
#[derive(Clone, Debug)]
pub(crate) enum Failure {
    Error(Value),
    Trapped(Trap),
}

/// What a suspended task waits for.
#[derive(Clone, Debug)]
pub(crate) enum Wait {
    Join(usize),
    /// Every task of a scope, its body's included, to end.
    Scope(usize),
    /// Room in a channel, or its receiver to go.
    Send(usize),
    /// Its value, the `n`th sent on a rendezvous channel, to be received.
    Taken(usize, u64),
    /// A value in a channel, or its senders to go.
    Recv(usize),
    /// A stop to reach the task (`task.stop.1`).
    Stop,
    /// One of a `select`'s operations to be ready.
    Select(Vec<Op>),
}

/// An operation a `select` arm waits on (`task.select.1`).
#[derive(Clone, Debug)]
pub(crate) enum Op {
    Recv(usize),
    Send(usize),
    Join(usize),
    Stop,
}

struct TaskState {
    /// The scope it belongs to (none for the run's own task).
    scope: Option<usize>,
    outcome: Option<Outcome>,
    /// When it ended, among the tasks that have.
    ended: Option<usize>,
    /// Whether its `Task` has been dropped without a `join`.
    detached: bool,
    joined: bool,
    cancelled: bool,
    /// Whether it is unwinding, during which cancellation is not seen again.
    unwinding: bool,
    waiting: Option<Wait>,
}

struct ScopeState {
    /// The task whose `scope` call it is.
    owner: usize,
    /// Its tasks: its body's first.
    tasks: Vec<usize>,
    isolated: bool,
    failure: Option<Failure>,
    stopped: bool,
}

/// A channel: its values in order, how many it holds, how many senders it has, whether its
/// receiver is alive, and how many values have been sent and received.
struct Channel {
    values: VecDeque<Value>,
    capacity: usize,
    senders: usize,
    receiver: bool,
    sent: u64,
    received: u64,
}

pub(crate) struct Scheduler {
    tasks: Vec<TaskState>,
    scopes: Vec<ScopeState>,
    channels: Vec<Channel>,
    ready: VecDeque<usize>,
    /// The suspended tasks, in the order they began to wait.
    blocked: Vec<usize>,
    ended: usize,
    /// The run's virtual time, in nanoseconds (`task.timer.1`).
    now: i64,
    /// The timers not yet fired: when each fires, in the order made, and its channel.
    timers: Vec<(i64, usize)>,
}

impl Scheduler {
    pub(crate) fn new() -> Self {
        Scheduler {
            tasks: vec![TaskState::new(None)],
            scopes: Vec::new(),
            channels: Vec::new(),
            ready: VecDeque::new(),
            blocked: Vec::new(),
            ended: 0,
            now: 0,
            timers: Vec::new(),
        }
    }

    /// A timer's channel, whose one value arrives `nanos` from now (`task.timer.1`).
    pub(crate) fn timer(&mut self, nanos: i64) -> usize {
        let c = self.channel(1);
        self.timers.push((self.now.saturating_add(nanos.max(0)), c));
        c
    }

    /// Opens a scope for task `owner`'s `scope` call, and gives its index.
    pub(crate) fn open_scope(&mut self, owner: usize, isolated: bool) -> usize {
        self.scopes.push(ScopeState {
            owner,
            tasks: Vec::new(),
            isolated,
            failure: None,
            stopped: false,
        });
        self.scopes.len() - 1
    }

    /// Records task `t`, just started in `scope`. Unless it is the scope's body, which runs at
    /// once, it is ready after those already ready. A task started in a cancelled scope is
    /// cancelled.
    pub(crate) fn spawn(&mut self, t: usize, scope: usize, body: bool) {
        debug_assert_eq!(t, self.tasks.len(), "tasks are numbered as they start");
        let mut task = TaskState::new(Some(scope));
        task.cancelled = self.scope_cancelled(scope);
        self.tasks.push(task);
        self.scopes[scope].tasks.push(t);
        if !body {
            self.ready.push_back(t);
        }
    }

    fn scope_cancelled(&self, s: usize) -> bool {
        self.scopes[s].failure.is_some() || self.tasks[self.scopes[s].owner].cancelled
    }

    /// Records how task `t` ended: a body's error, or that of a task given up, fails its scope.
    pub(crate) fn end(&mut self, t: usize, outcome: Outcome) {
        let scope = self.tasks[t].scope;
        let fails = match &outcome {
            Outcome::Error(e) => {
                let body = scope.is_some_and(|s| self.scopes[s].tasks.first() == Some(&t));
                (body || self.tasks[t].detached).then(|| e.clone())
            }
            _ => None,
        };
        let task = &mut self.tasks[t];
        task.ended = Some(self.ended);
        self.ended += 1;
        task.outcome = Some(if fails.is_some() {
            Outcome::Cancelled
        } else {
            outcome
        });
        if let (Some(e), Some(s)) = (fails, scope) {
            self.fail(s, Failure::Error(e));
        }
        self.wake();
    }

    /// Fails scope `s` with `f`, unless it has failed already, and cancels it.
    pub(crate) fn fail(&mut self, s: usize, f: Failure) {
        if self.scopes[s].failure.is_none() {
            self.scopes[s].failure = Some(f);
            self.cancel(s);
        }
    }

    /// Cancels scope `s`: its tasks, and the scopes they have opened.
    fn cancel(&mut self, s: usize) {
        let tasks = self.scopes[s].tasks.clone();
        for &t in &tasks {
            if self.tasks[t].ended.is_none() {
                self.tasks[t].cancelled = true;
            }
        }
        for inner in 0..self.scopes.len() {
            if tasks.contains(&self.scopes[inner].owner) && !self.scope_done(inner) {
                self.cancel(inner);
            }
        }
        self.wake();
    }

    /// The isolated scope a trap in task `t` ends, if any: its own scope's, or that of the task
    /// whose scope call that is, and so on out.
    pub(crate) fn isolating(&self, t: usize) -> Option<usize> {
        let mut scope = self.tasks[t].scope;
        while let Some(s) = scope {
            if self.scopes[s].isolated {
                return Some(s);
            }
            scope = self.tasks[self.scopes[s].owner].scope;
        }
        None
    }

    pub(crate) fn cancelled(&self, t: usize) -> bool {
        self.tasks[t].cancelled && !self.tasks[t].unwinding
    }

    pub(crate) fn unwinding(&self, t: usize) -> bool {
        self.tasks[t].unwinding
    }

    pub(crate) fn start_unwinding(&mut self, t: usize) {
        self.tasks[t].unwinding = true;
    }

    /// Records that task `t`'s `Task` has been dropped: an error it ended with fails its scope.
    pub(crate) fn detach(&mut self, t: usize) {
        self.tasks[t].detached = true;
        if let Some(Outcome::Error(e)) = &self.tasks[t].outcome {
            let e = e.clone();
            self.tasks[t].outcome = Some(Outcome::Cancelled);
            if let Some(s) = self.tasks[t].scope {
                self.fail(s, Failure::Error(e));
            }
        }
    }

    /// Task `t`'s outcome, taken by its `join`, once it has ended.
    pub(crate) fn take(&mut self, t: usize) -> Option<Outcome> {
        let task = &mut self.tasks[t];
        task.ended?;
        task.joined = true;
        Some(std::mem::replace(
            task.outcome.as_mut()?,
            Outcome::Cancelled,
        ))
    }

    /// Whether every task of scope `s` has ended.
    pub(crate) fn scope_done(&self, s: usize) -> bool {
        self.scopes[s]
            .tasks
            .iter()
            .all(|&t| self.tasks[t].ended.is_some())
    }

    /// How scope `s` ends, once its tasks have: its failure, or its body's value.
    pub(crate) fn scope_result(&mut self, s: usize) -> Result<Value, Failure> {
        if let Some(f) = self.scopes[s].failure.take() {
            return Err(f);
        }
        let body = self.scopes[s].tasks[0];
        match self.tasks[body].outcome.take() {
            Some(Outcome::Value(v)) => Ok(v),
            Some(Outcome::Error(e)) => Err(Failure::Error(e)),
            _ => Ok(Value::Unit),
        }
    }

    /// Asks scope `s` to stop (`task.stop.1`).
    pub(crate) fn stop(&mut self, s: usize) {
        self.scopes[s].stopped = true;
        self.wake();
    }

    /// Whether a stop has reached task `t`: its scope, or one around it, has been asked to stop,
    /// and the scopes its scope's tasks have opened inside it have all ended.
    pub(crate) fn stop_reached(&self, t: usize) -> bool {
        let Some(own) = self.tasks[t].scope else {
            return false;
        };
        let mut stopped = false;
        let mut scope = Some(own);
        while let Some(s) = scope {
            stopped |= self.scopes[s].stopped;
            scope = self.tasks[self.scopes[s].owner].scope;
        }
        let tasks = &self.scopes[own].tasks;
        stopped
            && (0..self.scopes.len())
                .all(|s| !tasks.contains(&self.scopes[s].owner) || self.scope_done(s))
    }

    /// A channel holding up to `capacity` values, with one sender and its receiver.
    pub(crate) fn channel(&mut self, capacity: usize) -> usize {
        self.channels.push(Channel {
            values: VecDeque::new(),
            capacity,
            senders: 1,
            receiver: true,
            sent: 0,
            received: 0,
        });
        self.channels.len() - 1
    }

    pub(crate) fn clone_sender(&mut self, c: usize) {
        self.channels[c].senders += 1;
    }

    pub(crate) fn drop_sender(&mut self, c: usize) {
        self.channels[c].senders -= 1;
        self.wake();
    }

    /// The receiver is gone: what the channel holds is given back, for its destructors to run
    /// (`task.channel.2`).
    pub(crate) fn drop_receiver(&mut self, c: usize) -> Vec<Value> {
        let ch = &mut self.channels[c];
        ch.receiver = false;
        let left: Vec<Value> = ch.values.drain(..).collect();
        self.wake();
        left
    }

    pub(crate) fn receiver_alive(&self, c: usize) -> bool {
        self.channels[c].receiver
    }

    /// Whether `send` on channel `c` can go ahead: it has room, or its receiver is gone.
    pub(crate) fn can_send(&self, c: usize) -> bool {
        let ch = &self.channels[c];
        !ch.receiver || ch.values.len() < ch.capacity.max(1)
    }

    /// Puts `v` in channel `c`, which has room: the number it is among those sent, and whether
    /// the sender waits for it to be received (a rendezvous).
    pub(crate) fn push(&mut self, c: usize, v: Value) -> (u64, bool) {
        let ch = &mut self.channels[c];
        ch.values.push_back(v);
        ch.sent += 1;
        let r = (ch.sent, ch.capacity == 0);
        self.wake();
        r
    }

    /// Whether the `n`th value sent on channel `c` has been received, or cannot be.
    pub(crate) fn taken(&self, c: usize, n: u64) -> bool {
        let ch = &self.channels[c];
        ch.received >= n || !ch.receiver
    }

    /// Whether `recv` on channel `c` can go ahead: it holds a value, or has no senders.
    pub(crate) fn can_recv(&self, c: usize) -> bool {
        let ch = &self.channels[c];
        !ch.values.is_empty() || ch.senders == 0
    }

    /// The next value of channel `c`, which can be received: `None` if it has no senders left.
    pub(crate) fn pop(&mut self, c: usize) -> Option<Value> {
        let ch = &mut self.channels[c];
        let v = ch.values.pop_front()?;
        ch.received += 1;
        self.wake();
        Some(v)
    }

    /// Whether `op` can go ahead for task `t`.
    pub(crate) fn op_ready(&self, t: usize, op: &Op) -> bool {
        match op {
            Op::Recv(c) => self.can_recv(*c),
            Op::Send(c) => self.can_send(*c),
            Op::Join(j) => self.tasks[*j].ended.is_some(),
            Op::Stop => self.stop_reached(t),
        }
    }

    /// Suspends task `t` until `w` happens.
    pub(crate) fn block(&mut self, t: usize, w: Wait) {
        self.tasks[t].waiting = Some(w);
        self.blocked.push(t);
    }

    /// Queues task `t` again, after those already ready (`yield_now`).
    pub(crate) fn requeue(&mut self, t: usize) {
        self.ready.push_back(t);
    }

    /// The next task to run, if any is ready. With none, time moves to the earliest timer,
    /// which fires (`task.timer.1`), until a task is ready or no timer is left.
    pub(crate) fn next(&mut self) -> Option<usize> {
        while self.ready.is_empty() && !self.timers.is_empty() {
            let (i, _) = self
                .timers
                .iter()
                .enumerate()
                .min_by_key(|(_, (at, _))| *at)
                .expect("a timer");
            let (at, c) = self.timers.remove(i);
            self.now = at;
            if self.channels[c].receiver {
                self.push(c, Value::Unit);
            }
            self.drop_sender(c);
        }
        self.ready.pop_front()
    }

    /// Makes ready, in the order they began to wait, the suspended tasks whose wait is over, or
    /// which are cancelled where cancellation is seen (anywhere but a scope's end).
    fn wake(&mut self) {
        let mut still = Vec::new();
        for t in std::mem::take(&mut self.blocked) {
            let done = match self.tasks[t].waiting.as_ref().expect("a waiting task") {
                Wait::Scope(s) => self.scope_done(*s),
                w => {
                    self.cancelled(t)
                        || match w {
                            Wait::Join(j) => self.tasks[*j].ended.is_some(),
                            Wait::Send(c) => self.can_send(*c),
                            Wait::Taken(c, n) => self.taken(*c, *n),
                            Wait::Recv(c) => self.can_recv(*c),
                            Wait::Stop => self.stop_reached(t),
                            Wait::Select(ops) => ops.iter().any(|o| self.op_ready(t, o)),
                            Wait::Scope(_) => unreachable!("handled above"),
                        }
                }
            };
            if done {
                self.tasks[t].waiting = None;
                self.ready.push_back(t);
            } else {
                still.push(t);
            }
        }
        self.blocked = still;
    }
}

impl TaskState {
    fn new(scope: Option<usize>) -> Self {
        TaskState {
            scope,
            outcome: None,
            ended: None,
            detached: false,
            joined: false,
            cancelled: false,
            unwinding: false,
            waiting: None,
        }
    }
}

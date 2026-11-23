//! The scheduler (`task.sched.1`), as `moddil-interp`'s, with values as their bytes: the tasks
//! and scopes, which tasks are ready and what the others wait for, how each ended, and the
//! channels.
//!
//! One task runs at a time. The others are queued, first in, first out, and run only when the
//! running one suspends. A task waiting for something becomes ready once it has happened, in the
//! order the waiting tasks began to wait; a cancelled task waiting where cancellation is seen
//! becomes ready too, to unwind. Keeping the interpreter's order makes a native run's order the
//! interpreters'.

use std::collections::VecDeque;

use crate::Trapped;
use crate::baton::DropFn;

/// Values to drop, each with its glue.
pub(crate) type Dropped = Vec<(DropFn, Vec<u8>)>;

/// How a task ended.
#[derive(Clone, Debug)]
pub(crate) enum Outcome {
    Value(Vec<u8>),
    Error(Vec<u8>),
    /// It was cancelled, and unwound (`task.cancel.1`).
    Cancelled,
}

/// Why a scope failed: the first failure of its body or of a task given up, or a trap in it, if
/// it is isolated.
#[derive(Clone, Debug)]
pub(crate) enum Failure {
    Error(Vec<u8>),
    Trapped(Trapped),
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
    cancelled: bool,
    /// Whether it is unwinding, during which cancellation is not seen again.
    unwinding: bool,
    waiting: Option<Wait>,
    /// What drops its value, if nobody takes it.
    drop: Option<DropFn>,
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
    values: VecDeque<Vec<u8>>,
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
}

impl Scheduler {
    pub(crate) fn new() -> Self {
        Scheduler {
            tasks: vec![TaskState::new(None, None)],
            scopes: Vec::new(),
            channels: Vec::new(),
            ready: VecDeque::new(),
            blocked: Vec::new(),
            ended: 0,
        }
    }

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

    /// Records a task, just started in `scope`, and gives its number. Unless it is the scope's
    /// body, which runs at once, it is ready after those already ready. A task started in a
    /// cancelled scope is cancelled.
    pub(crate) fn spawn(&mut self, scope: usize, body: bool, drop: Option<DropFn>) -> usize {
        let t = self.tasks.len();
        let mut task = TaskState::new(Some(scope), drop);
        task.cancelled = self.scope_cancelled(scope);
        self.tasks.push(task);
        self.scopes[scope].tasks.push(t);
        if !body {
            self.ready.push_back(t);
        }
        t
    }

    fn scope_cancelled(&self, s: usize) -> bool {
        self.scopes[s].failure.is_some() || self.tasks[self.scopes[s].owner].cancelled
    }

    /// Records how task `t` ended: a body's error, or that of a task given up, fails its scope.
    /// What is to be dropped (a value nobody will take) is given back.
    pub(crate) fn end(&mut self, t: usize, outcome: Outcome) -> Vec<(DropFn, Vec<u8>)> {
        let scope = self.tasks[t].scope;
        let fails = match &outcome {
            Outcome::Error(e) => {
                let body = scope.is_some_and(|s| self.scopes[s].tasks.first() == Some(&t));
                (body || self.tasks[t].detached).then(|| e.clone())
            }
            _ => None,
        };
        let mut dropped = Vec::new();
        let task = &mut self.tasks[t];
        task.ended = Some(self.ended);
        self.ended += 1;
        task.outcome = Some(if fails.is_some() {
            Outcome::Cancelled
        } else {
            outcome
        });
        // A detached task's value: nobody will take it.
        if task.detached
            && let (Some(Outcome::Value(v)), Some(d)) = (&task.outcome, task.drop)
        {
            dropped.push((d, v.clone()));
            task.outcome = Some(Outcome::Cancelled);
        }
        if let (Some(e), Some(s)) = (fails, scope) {
            self.fail(s, Failure::Error(e));
        }
        self.wake();
        dropped
    }

    pub(crate) fn fail(&mut self, s: usize, f: Failure) {
        if self.scopes[s].failure.is_none() {
            self.scopes[s].failure = Some(f);
            self.cancel(s);
        }
    }

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

    /// The isolated scope a trap in task `t` ends, if any.
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

    /// Task `t`'s `Task` has been dropped: an error it ended with fails its scope, and a value
    /// it ended with is given back to be dropped.
    pub(crate) fn detach(&mut self, t: usize) -> Option<(DropFn, Vec<u8>)> {
        self.tasks[t].detached = true;
        match self.tasks[t].outcome.clone() {
            Some(Outcome::Error(e)) => {
                self.tasks[t].outcome = Some(Outcome::Cancelled);
                if let Some(s) = self.tasks[t].scope {
                    self.fail(s, Failure::Error(e));
                }
                None
            }
            Some(Outcome::Value(v)) => {
                self.tasks[t].outcome = Some(Outcome::Cancelled);
                self.tasks[t].drop.map(|d| (d, v))
            }
            _ => None,
        }
    }

    /// Task `t`'s outcome, taken by its `join`, once it has ended.
    pub(crate) fn take(&mut self, t: usize) -> Option<Outcome> {
        let task = &mut self.tasks[t];
        task.ended?;
        Some(std::mem::replace(
            task.outcome.as_mut()?,
            Outcome::Cancelled,
        ))
    }

    pub(crate) fn scope_done(&self, s: usize) -> bool {
        self.scopes[s]
            .tasks
            .iter()
            .all(|&t| self.tasks[t].ended.is_some())
    }

    /// How scope `s` ends, once its tasks have: its failure, or its body's value. The values of
    /// its other tasks that nobody took are given back to be dropped.
    pub(crate) fn scope_result(&mut self, s: usize) -> (Result<Option<Vec<u8>>, Failure>, Dropped) {
        let mut dropped = Vec::new();
        for &t in &self.scopes[s].tasks[1..] {
            if let (Some(Outcome::Value(v)), Some(d)) = (&self.tasks[t].outcome, self.tasks[t].drop)
            {
                dropped.push((d, v.clone()));
            }
        }
        for &t in &self.scopes[s].tasks.clone()[1..] {
            if matches!(self.tasks[t].outcome, Some(Outcome::Value(_))) {
                self.tasks[t].outcome = Some(Outcome::Cancelled);
            }
        }
        if let Some(f) = self.scopes[s].failure.take() {
            return (Err(f), dropped);
        }
        let body = self.scopes[s].tasks[0];
        let r = match self.tasks[body].outcome.take() {
            Some(Outcome::Value(v)) => Ok(Some(v)),
            Some(Outcome::Error(e)) => Err(Failure::Error(e)),
            _ => Ok(None),
        };
        (r, dropped)
    }

    pub(crate) fn stop(&mut self, s: usize) {
        self.scopes[s].stopped = true;
        self.wake();
    }

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

    /// The receiver is gone: what the channel holds is given back, to be dropped.
    pub(crate) fn drop_receiver(&mut self, c: usize) -> Vec<Vec<u8>> {
        let ch = &mut self.channels[c];
        ch.receiver = false;
        let left: Vec<Vec<u8>> = ch.values.drain(..).collect();
        self.wake();
        left
    }

    pub(crate) fn receiver_alive(&self, c: usize) -> bool {
        self.channels[c].receiver
    }

    pub(crate) fn can_send(&self, c: usize) -> bool {
        let ch = &self.channels[c];
        !ch.receiver || ch.values.len() < ch.capacity.max(1)
    }

    pub(crate) fn push(&mut self, c: usize, v: Vec<u8>) -> (u64, bool) {
        let ch = &mut self.channels[c];
        ch.values.push_back(v);
        ch.sent += 1;
        let r = (ch.sent, ch.capacity == 0);
        self.wake();
        r
    }

    pub(crate) fn taken(&self, c: usize, n: u64) -> bool {
        let ch = &self.channels[c];
        ch.received >= n || !ch.receiver
    }

    pub(crate) fn can_recv(&self, c: usize) -> bool {
        let ch = &self.channels[c];
        !ch.values.is_empty() || ch.senders == 0
    }

    pub(crate) fn pop(&mut self, c: usize) -> Option<Vec<u8>> {
        let ch = &mut self.channels[c];
        let v = ch.values.pop_front()?;
        ch.received += 1;
        self.wake();
        Some(v)
    }

    pub(crate) fn op_ready(&self, t: usize, op: &Op) -> bool {
        match op {
            Op::Recv(c) => self.can_recv(*c),
            Op::Send(c) => self.can_send(*c),
            Op::Join(j) => self.tasks[*j].ended.is_some(),
            Op::Stop => self.stop_reached(t),
        }
    }

    pub(crate) fn block(&mut self, t: usize, w: Wait) {
        self.tasks[t].waiting = Some(w);
        self.blocked.push(t);
    }

    pub(crate) fn requeue(&mut self, t: usize) {
        self.ready.push_back(t);
    }

    pub(crate) fn next(&mut self) -> Option<usize> {
        self.ready.pop_front()
    }

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
    fn new(scope: Option<usize>, drop: Option<DropFn>) -> Self {
        TaskState {
            scope,
            outcome: None,
            ended: None,
            detached: false,
            cancelled: false,
            unwinding: false,
            waiting: None,
            drop,
        }
    }
}

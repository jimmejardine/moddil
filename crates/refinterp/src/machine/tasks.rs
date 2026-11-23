//! Tasks (`task.scope.1`): scopes, tasks, cancellation, isolation, channels and stop,
//! independently of the MIR interpreter's scheduler.
//!
//! Each task runs on a thread of its own, with a machine of its own and its own parse of the
//! program (a syntax node is not shared between threads), and the memory shared
//! ([`crate::memory`]). One runs at a time: the running task hands the turn on, first in, first
//! out, only where it suspends (`task.sched.1`), and waits for it to come back. A cancelled task
//! unwinds from its next suspension point, as an error would, running its `defer`s and drops
//! (`task.cancel.1`).

use std::collections::VecDeque;
use std::sync::Arc;

use moddil_syntax::TextRange;

use crate::memory::{Memory, Shared};
use crate::program::Program;
use crate::value::{Head, Value};
use crate::{Eval, Flow, ModSpec, SourceSpec, Trap, TrapClass};

use super::Machine;

/// How a task ended.
#[derive(Clone)]
enum Outcome {
    Value(Value),
    Error(Value),
    Cancelled,
}

/// Why a scope failed.
#[derive(Clone)]
enum Failure {
    Error(Value),
    Trapped(Trap),
}

/// What a suspended task waits for.
#[derive(Clone)]
enum Wait {
    Join(usize),
    Scope(usize),
    Send(usize),
    Taken(usize, u64),
    Recv(usize),
    Stop,
    Select(Vec<Op>),
}

/// An operation a `select` arm waits on (`task.select.1`).
#[derive(Clone)]
pub(super) enum Op {
    Recv(usize),
    Send(usize),
    Join(usize),
    Stop,
}

struct TaskState {
    scope: Option<usize>,
    outcome: Option<Outcome>,
    ended: Option<usize>,
    detached: bool,
    cancelled: bool,
    unwinding: bool,
    waiting: Option<Wait>,
}

struct ScopeState {
    owner: usize,
    tasks: Vec<usize>,
    isolated: bool,
    failure: Option<Failure>,
    stopped: bool,
}

struct Channel {
    values: VecDeque<Value>,
    capacity: usize,
    senders: usize,
    receiver: bool,
    sent: u64,
    received: u64,
}

/// The tasks, scopes and channels of a test, which every task's thread shares.
pub(crate) struct Tasks {
    states: Vec<TaskState>,
    scopes: Vec<ScopeState>,
    channels: Vec<Channel>,
    ready: VecDeque<usize>,
    blocked: Vec<usize>,
    ended: usize,
    /// The task whose turn it is.
    running: usize,
    /// The trap that ended the test, once one has, whichever task it was in: every task stops.
    trapped: Option<(Trap, bool)>,
    /// What the tasks other than the test's observed.
    observed: Vec<(u32, TextRange, String)>,
    threads: Vec<std::thread::JoinHandle<()>>,
    /// Whether a task's thread panicked (a bug in the interpreter): every task stops.
    panicked: bool,
}

impl TaskState {
    fn new(scope: Option<usize>) -> Self {
        TaskState {
            scope,
            outcome: None,
            ended: None,
            detached: false,
            cancelled: false,
            unwinding: false,
            waiting: None,
        }
    }
}

impl Tasks {
    pub(crate) fn new() -> Self {
        Tasks {
            states: vec![TaskState::new(None)],
            scopes: Vec::new(),
            channels: Vec::new(),
            ready: VecDeque::new(),
            blocked: Vec::new(),
            ended: 0,
            running: 0,
            trapped: None,
            observed: Vec::new(),
            threads: Vec::new(),
            panicked: false,
        }
    }

    /// Records task `t`, just started in `scope`: ready after those already ready, unless it is
    /// the scope's body, which runs at once. A task started in a cancelled scope is cancelled.
    fn spawn(&mut self, t: usize, scope: usize, body: bool) {
        let mut task = TaskState::new(Some(scope));
        task.cancelled =
            self.scopes[scope].failure.is_some() || self.states[self.scopes[scope].owner].cancelled;
        self.states.push(task);
        self.scopes[scope].tasks.push(t);
        if !body {
            self.ready.push_back(t);
        }
    }

    /// Records how task `t` ended: a body's error, or that of a task given up, fails its scope.
    fn end(&mut self, t: usize, outcome: Outcome) {
        let scope = self.states[t].scope;
        let fails = match &outcome {
            Outcome::Error(e) => {
                let body = scope.is_some_and(|s| self.scopes[s].tasks.first() == Some(&t));
                (body || self.states[t].detached).then(|| e.clone())
            }
            _ => None,
        };
        let task = &mut self.states[t];
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
    fn fail(&mut self, s: usize, f: Failure) {
        if self.scopes[s].failure.is_none() {
            self.scopes[s].failure = Some(f);
            self.cancel(s);
        }
    }

    /// Cancels scope `s`: its tasks, and the scopes they have opened.
    fn cancel(&mut self, s: usize) {
        let tasks = self.scopes[s].tasks.clone();
        for &t in &tasks {
            if self.states[t].ended.is_none() {
                self.states[t].cancelled = true;
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
    fn isolating(&self, t: usize) -> Option<usize> {
        let mut scope = self.states[t].scope;
        while let Some(s) = scope {
            if self.scopes[s].isolated {
                return Some(s);
            }
            scope = self.states[self.scopes[s].owner].scope;
        }
        None
    }

    fn cancelled(&self, t: usize) -> bool {
        self.states[t].cancelled && !self.states[t].unwinding
    }

    /// Records that task `t`'s `Task` has been dropped: an error it ended with fails its scope.
    fn detach(&mut self, t: usize) {
        self.states[t].detached = true;
        if let Some(Outcome::Error(e)) = &self.states[t].outcome {
            let e = e.clone();
            self.states[t].outcome = Some(Outcome::Cancelled);
            if let Some(s) = self.states[t].scope {
                self.fail(s, Failure::Error(e));
            }
        }
    }

    fn take(&mut self, t: usize) -> Option<Outcome> {
        let task = &mut self.states[t];
        task.ended?;
        Some(std::mem::replace(
            task.outcome.as_mut()?,
            Outcome::Cancelled,
        ))
    }

    fn scope_done(&self, s: usize) -> bool {
        self.scopes[s]
            .tasks
            .iter()
            .all(|&t| self.states[t].ended.is_some())
    }

    fn scope_result(&mut self, s: usize) -> Result<Value, Failure> {
        if let Some(f) = self.scopes[s].failure.take() {
            return Err(f);
        }
        let body = self.scopes[s].tasks[0];
        match self.states[body].outcome.take() {
            Some(Outcome::Value(v)) => Ok(v),
            Some(Outcome::Error(e)) => Err(Failure::Error(e)),
            _ => Ok(Value::Unit),
        }
    }

    /// Whether a stop has reached task `t` (`task.stop.1`).
    fn stop_reached(&self, t: usize) -> bool {
        let Some(own) = self.states[t].scope else {
            return false;
        };
        let mut stopped = false;
        let mut scope = Some(own);
        while let Some(s) = scope {
            stopped |= self.scopes[s].stopped;
            scope = self.states[self.scopes[s].owner].scope;
        }
        let tasks = &self.scopes[own].tasks;
        stopped
            && (0..self.scopes.len())
                .all(|s| !tasks.contains(&self.scopes[s].owner) || self.scope_done(s))
    }

    fn can_send(&self, c: usize) -> bool {
        let ch = &self.channels[c];
        !ch.receiver || ch.values.len() < ch.capacity.max(1)
    }

    fn taken(&self, c: usize, n: u64) -> bool {
        let ch = &self.channels[c];
        ch.received >= n || !ch.receiver
    }

    fn can_recv(&self, c: usize) -> bool {
        let ch = &self.channels[c];
        !ch.values.is_empty() || ch.senders == 0
    }

    fn push(&mut self, c: usize, v: Value) -> (u64, bool) {
        let ch = &mut self.channels[c];
        ch.values.push_back(v);
        ch.sent += 1;
        let r = (ch.sent, ch.capacity == 0);
        self.wake();
        r
    }

    fn pop(&mut self, c: usize) -> Option<Value> {
        let ch = &mut self.channels[c];
        let v = ch.values.pop_front()?;
        ch.received += 1;
        self.wake();
        Some(v)
    }

    fn op_ready(&self, t: usize, op: &Op) -> bool {
        match op {
            Op::Recv(c) => self.can_recv(*c),
            Op::Send(c) => self.can_send(*c),
            Op::Join(j) => self.states[*j].ended.is_some(),
            Op::Stop => self.stop_reached(t),
        }
    }

    fn block(&mut self, t: usize, w: Wait) {
        self.states[t].waiting = Some(w);
        self.blocked.push(t);
    }

    /// Makes ready, in the order they began to wait, the suspended tasks whose wait is over, or
    /// which are cancelled where cancellation is seen (anywhere but a scope's end).
    fn wake(&mut self) {
        let mut still = Vec::new();
        for t in std::mem::take(&mut self.blocked) {
            let done = match self.states[t].waiting.as_ref().expect("a waiting task") {
                Wait::Scope(s) => self.scope_done(*s),
                w => {
                    self.cancelled(t)
                        || match w {
                            Wait::Join(j) => self.states[*j].ended.is_some(),
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
                self.states[t].waiting = None;
                self.ready.push_back(t);
            } else {
                still.push(t);
            }
        }
        self.blocked = still;
    }
}

/// The program's sources, which each task parses again.
pub(crate) type Sources = Arc<(Vec<SourceSpec>, Vec<ModSpec>)>;

/// How an operation went: done, with a result or an error; waiting; or to be unwound.
enum Attempt {
    Done(Result<Value, Value>),
    Wait(Wait),
    Unwind,
}

impl Machine<'_> {
    /// The scheduler's operations of the prelude: `name` (of a value of kind `head`) on `args`,
    /// if it is one of them.
    pub(super) fn task_intrinsic(
        &mut self,
        name: &str,
        head: &Head,
        args: &[Value],
        range: TextRange,
    ) -> Option<Eval> {
        // A destructor is called with no kind for `Self`: its value's is it.
        let owner = match (head, name) {
            (Head::Adt(k), _) => k.clone(),
            (_, "drop") => match self.target(args[0].clone()) {
                Value::Adt { ty, .. } => ty,
                _ => String::new(),
            },
            _ => String::new(),
        };
        let owner = owner.as_str();
        let id = |m: &Self, v: &Value| id_of(&m.target(v.clone()));
        Some(match (owner, name) {
            ("", "__scope_open") => {
                let me = self.memory.task();
                let isolated = args[0] == Value::Bool(true);
                let s = self.with_tasks(|ts| {
                    ts.scopes.push(ScopeState {
                        owner: me,
                        tasks: Vec::new(),
                        isolated,
                        failure: None,
                        stopped: false,
                    });
                    ts.scopes.len() - 1
                });
                Ok(Value::Int(s as i64))
            }
            ("", "__scope_run") => {
                let s = id(self, &args[0]);
                self.scope_run(s, args[1].clone(), args[0].clone(), range)
            }
            ("Scope", "spawn") => {
                let s = id(self, &args[0]);
                let t = self.start_task(s, args[1].clone(), None, range);
                Ok(handle("Task", t))
            }
            ("Scope", "stop") => {
                let s = id(self, &args[0]);
                self.with_tasks(|ts| {
                    ts.scopes[s].stopped = true;
                    ts.wake();
                });
                Ok(Value::Unit)
            }
            ("", "channel") => {
                let Value::Int(n) = args[0] else {
                    unreachable!("an int")
                };
                if n < 0 {
                    return Some(Err(crate::trap(TrapClass::CapacityExceeded, range)));
                }
                let c = self.with_tasks(|ts| {
                    ts.channels.push(Channel {
                        values: VecDeque::new(),
                        capacity: n as usize,
                        senders: 1,
                        receiver: true,
                        sent: 0,
                        received: 0,
                    });
                    ts.channels.len() - 1
                });
                Ok(Value::Tuple(vec![
                    handle("Sender", c),
                    handle("Receiver", c),
                ]))
            }
            ("Sender", "clone") => {
                let c = id(self, &args[0]);
                self.with_tasks(|ts| ts.channels[c].senders += 1);
                Ok(handle("Sender", c))
            }
            ("Task", "drop") => {
                let t = id(self, &args[0]);
                self.with_tasks(|ts| ts.detach(t));
                Ok(Value::Unit)
            }
            ("Sender", "drop") => {
                let c = id(self, &args[0]);
                self.with_tasks(|ts| {
                    ts.channels[c].senders -= 1;
                    ts.wake();
                });
                Ok(Value::Unit)
            }
            // What the channel holds is dropped, with its destructors (`task.channel.2`).
            ("Receiver", "drop") => {
                let c = id(self, &args[0]);
                let left: Vec<Value> = self.with_tasks(|ts| {
                    ts.channels[c].receiver = false;
                    let left = ts.channels[c].values.drain(..).collect();
                    ts.wake();
                    left
                });
                for v in left {
                    if let Err(flow) = self.drop_value(v, true, range) {
                        return Some(Err(flow));
                    }
                }
                Ok(Value::Unit)
            }
            ("", "yield_now") => {
                let me = self.memory.task();
                if self.with_tasks(|ts| ts.cancelled(me)) {
                    return Some(Err(Flow::Unwind));
                }
                self.suspend(range, |ts, me| ts.ready.push_back(me))
                    .and_then(|()| self.check_cancelled())
                    .map(|()| Value::Unit)
            }
            ("Task", "join") => {
                let t = id(self, &args[0]);
                self.run_op(range, |ts, _| match ts.take(t) {
                    Some(Outcome::Value(v)) => Attempt::Done(Ok(v)),
                    Some(Outcome::Error(e)) => Attempt::Done(Err(e)),
                    // A cancelled task's scope is cancelled, and so is the task joining it.
                    Some(Outcome::Cancelled) => Attempt::Unwind,
                    None => Attempt::Wait(Wait::Join(t)),
                })
            }
            ("Sender", "send") => {
                let c = id(self, &args[0]);
                self.send(c, args[1].clone(), range)
            }
            ("Receiver", "recv") => {
                let c = id(self, &args[0]);
                self.run_op(range, |ts, _| {
                    if !ts.can_recv(c) {
                        return Attempt::Wait(Wait::Recv(c));
                    }
                    Attempt::Done(Ok(option(ts.pop(c))))
                })
            }
            ("", "stop_requested") => self.run_op(range, |ts, me| {
                if ts.stop_reached(me) {
                    Attempt::Done(Ok(Value::Unit))
                } else {
                    Attempt::Wait(Wait::Stop)
                }
            }),
            _ => return None,
        })
    }

    /// Runs `f` on the shared tasks.
    fn with_tasks<R>(&self, f: impl FnOnce(&mut Tasks) -> R) -> R {
        let shared = self.memory.shared();
        let mut s = shared.0.lock().expect("the memory's lock");
        f(&mut s.tasks)
    }

    /// Unwinds if the running task has been cancelled (and is not unwinding already).
    fn check_cancelled(&self) -> Eval<()> {
        let me = self.memory.task();
        if self.with_tasks(|ts| ts.cancelled(me)) {
            Err(Flow::Unwind)
        } else {
            Ok(())
        }
    }

    /// Runs an operation: `try_op` says whether it is done, or must wait (it is then tried
    /// again once the task resumes), or the task, cancelled, unwinds.
    fn run_op(
        &mut self,
        range: TextRange,
        mut try_op: impl FnMut(&mut Tasks, usize) -> Attempt,
    ) -> Eval {
        let me = self.memory.task();
        loop {
            let attempt = self.with_tasks(|ts| {
                if ts.cancelled(me) {
                    Attempt::Unwind
                } else {
                    try_op(ts, me)
                }
            });
            match attempt {
                Attempt::Done(Ok(v)) => return Ok(v),
                Attempt::Done(Err(e)) => return Err(Flow::Fail(e, range)),
                Attempt::Unwind => return Err(Flow::Unwind),
                Attempt::Wait(w) => self.suspend(range, |ts, me| ts.block(me, w))?,
            }
        }
    }

    /// `send`: room in the channel, then the value in it; on a rendezvous channel, until it is
    /// received.
    fn send(&mut self, c: usize, v: Value, range: TextRange) -> Eval {
        let closed = || Value::Adt {
            ty: "Closed".to_string(),
            variant: "Closed".to_string(),
            fields: Vec::new(),
        };
        let mut value = Some(v);
        let mut sent = None;
        self.run_op(range, |ts, _| {
            if let Some(n) = sent {
                if !ts.taken(c, n) {
                    return Attempt::Wait(Wait::Taken(c, n));
                }
                return if ts.channels[c].receiver {
                    Attempt::Done(Ok(Value::Unit))
                } else {
                    Attempt::Done(Err(closed()))
                };
            }
            if !ts.channels[c].receiver {
                return Attempt::Done(Err(closed()));
            }
            if !ts.can_send(c) {
                return Attempt::Wait(Wait::Send(c));
            }
            match ts.push(c, value.take().expect("a value to send")) {
                (n, true) => {
                    sent = Some(n);
                    Attempt::Wait(Wait::Taken(c, n))
                }
                _ => Attempt::Done(Ok(Value::Unit)),
            }
        })
    }

    /// A scope's run: its body as its first task, at once, then the scope's end, once every
    /// task has ended: the body's value, or the scope's failure.
    fn scope_run(&mut self, s: usize, body: Value, handle: Value, range: TextRange) -> Eval {
        let me = self.memory.task();
        let t = self.start_task(s, body, Some(handle), range);
        let shared = self.memory.shared();
        {
            let (lock, turn) = &*shared;
            let mut st = lock.lock().expect("the memory's lock");
            st.tasks.block(me, Wait::Scope(s));
            st.tasks.running = t;
            turn.notify_all();
            while st.tasks.running != me && st.tasks.trapped.is_none() && !st.tasks.panicked {
                st = turn.wait(st).expect("the memory's lock");
            }
            assert!(!st.tasks.panicked, "a task's thread panicked");
            if let Some((t, placed)) = st.tasks.trapped {
                return Err(Flow::Trap(t, placed));
            }
        }
        let result = self.with_tasks(|ts| ts.scope_result(s));
        self.check_cancelled()?;
        match result {
            Ok(v) => Ok(v),
            Err(Failure::Error(e)) => Err(Flow::Fail(e, range)),
            Err(Failure::Trapped(t)) => Err(Flow::Fail(self.trapped(&t), range)),
        }
    }

    /// The prelude's `Trapped` for trap `t`: its kind, and where it is as `file:line:column`.
    fn trapped(&self, t: &Trap) -> Value {
        let file = t.file as usize;
        let at = self
            .program
            .texts
            .get(file)
            .map(|text| {
                let (line, col) = moddil_syntax::line_col(text, t.range.start);
                let path = self.program.paths.get(file).map_or("", String::as_str);
                format!("{path}:{line}:{col}")
            })
            .unwrap_or_default();
        Value::Adt {
            ty: "Trapped".to_string(),
            variant: "Trapped".to_string(),
            fields: vec![
                crate::value::make_string(&t.class.to_string()),
                crate::value::make_string(&at),
            ],
        }
    }

    /// Suspends the running task at the operation at `range`: `park` records why (or queues it
    /// again), the next ready task gets the turn, and this one waits for it to come back. A trap
    /// in another task ends it; with no task ready, every task waits: the test is deadlocked.
    fn suspend(&mut self, range: TextRange, park: impl FnOnce(&mut Tasks, usize)) -> Eval<()> {
        let shared = self.memory.shared();
        let me = self.memory.task();
        let (lock, turn) = &*shared;
        let mut s = lock.lock().expect("the memory's lock");
        park(&mut s.tasks, me);
        let Some(next) = s.tasks.ready.pop_front() else {
            return Err(crate::trap(TrapClass::Deadlock, range));
        };
        s.tasks.running = next;
        turn.notify_all();
        while s.tasks.running != me && s.tasks.trapped.is_none() && !s.tasks.panicked {
            s = turn.wait(s).expect("the memory's lock");
        }
        assert!(!s.tasks.panicked, "a task's thread panicked");
        match s.tasks.trapped {
            Some((t, placed)) => Err(Flow::Trap(t, placed)),
            None => Ok(()),
        }
    }

    /// Starts a task running closure `f` in `scope` (the scope's body if it is given the
    /// scope's `handle`), on a thread of its own; its index.
    fn start_task(
        &mut self,
        scope: usize,
        f: Value,
        handle: Option<Value>,
        range: TextRange,
    ) -> usize {
        let t = self.memory.add_task();
        let body = handle.is_some();
        self.with_tasks(|ts| ts.spawn(t, scope, body));
        let shared = self.memory.shared();
        let sources = self.program.sources();
        let args: Vec<Value> = handle.into_iter().collect();
        let thread = std::thread::Builder::new()
            .stack_size(1 << 30)
            .spawn(move || run_task(&shared, t, &sources, f, args, range))
            .expect("a thread for the task");
        self.with_tasks(|ts| ts.threads.push(thread));
        t
    }

    /// A trap leaving a block of the running task's code: in an isolated scope, and not while
    /// unwinding, it fails the scope with `Trapped`, cancelling it, and the task unwinds
    /// (`task.isolate.1`).
    pub(super) fn isolate(&mut self, flow: Flow) -> Flow {
        let Flow::Trap(mut t, placed) = flow else {
            return flow;
        };
        if !self.memory.user() {
            return Flow::Trap(t, placed);
        }
        if !placed {
            t.file = self.memory.file().unwrap_or(0);
        }
        let me = self.memory.task();
        let isolated = self.with_tasks(|ts| {
            if ts.states[me].unwinding {
                return false;
            }
            let Some(s) = ts.isolating(me) else {
                return false;
            };
            ts.fail(s, Failure::Trapped(t));
            ts.states[me].unwinding = true;
            true
        });
        if isolated {
            Flow::Unwind
        } else {
            Flow::Trap(t, placed)
        }
    }

    /// Records that the running task is unwinding, so its cleanup sees no cancellation.
    pub(super) fn unwinding(&mut self) {
        let me = self.memory.task();
        self.with_tasks(|ts| ts.states[me].unwinding = true);
    }

    /// At the end of the test: what its other tasks observed, once their threads have ended.
    pub(super) fn end_tasks(&mut self) {
        let (threads, observed) = self.with_tasks(|ts| {
            (
                std::mem::take(&mut ts.threads),
                std::mem::take(&mut ts.observed),
            )
        });
        for t in threads {
            let _ = t.join();
        }
        self.observed.extend(observed);
    }
}

/// A handle of the prelude's for index `i`: a `Task`, `Sender` or `Receiver`.
fn handle(name: &str, i: usize) -> Value {
    Value::Adt {
        ty: name.to_string(),
        variant: name.to_string(),
        fields: vec![Value::Int(i as i64)],
    }
}

fn option(v: Option<Value>) -> Value {
    match v {
        Some(v) => Value::Adt {
            ty: "Option".to_string(),
            variant: "Some".to_string(),
            fields: vec![v],
        },
        None => Value::Adt {
            ty: "Option".to_string(),
            variant: "None".to_string(),
            fields: Vec::new(),
        },
    }
}

/// The index a scope's, task's or channel's handle holds.
fn id_of(v: &Value) -> usize {
    match v {
        Value::Int(i) => *i as usize,
        Value::Adt { fields, .. } => match fields[0] {
            Value::Int(i) => i as usize,
            _ => unreachable!("an index"),
        },
        _ => unreachable!("a handle"),
    }
}

/// Task `t`'s thread: once it has the turn, it runs closure `f` with `args`, records how it
/// ended, and hands the turn on.
fn run_task(
    shared: &Shared,
    t: usize,
    sources: &Sources,
    f: Value,
    args: Vec<Value>,
    range: TextRange,
) {
    let program = Program::new(&sources.0, &sources.1);
    let (lock, turn) = &**shared;
    {
        let mut s = lock.lock().expect("the memory's lock");
        while s.tasks.running != t && s.tasks.trapped.is_none() && !s.tasks.panicked {
            s = turn.wait(s).expect("the memory's lock");
        }
        if s.tasks.trapped.is_some() || s.tasks.panicked {
            return;
        }
    }
    let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut m = Machine::new(&program, Memory::of_task(shared.clone(), t));
        let Value::Closure(c) = f else {
            unreachable!("a task's closure")
        };
        // The task's first frame is where its closure was written.
        m.memory
            .push_frame(std::collections::HashMap::new(), c.file, c.module, false);
        let result = m.call_closure(&c, args, range);
        (result, std::mem::take(&mut m.observed))
    }));
    // A bug in the interpreter: the other tasks are woken, and find the lock poisoned.
    let Ok((result, mut observed)) = run else {
        let mut s = lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        s.tasks.panicked = true;
        turn.notify_all();
        return;
    };
    let mut s = lock.lock().expect("the memory's lock");
    s.tasks.observed.append(&mut observed);
    match result {
        Ok(v) => s.tasks.end(t, Outcome::Value(v)),
        Err(Flow::Fail(e, _)) => s.tasks.end(t, Outcome::Error(e)),
        Err(Flow::Unwind) => s.tasks.end(t, Outcome::Cancelled),
        Err(Flow::Trap(trap, placed)) => {
            s.tasks.trapped.get_or_insert((trap, placed));
            turn.notify_all();
            return;
        }
        Err(_) => unreachable!("a closure's body leaves only by its value, an error or a trap"),
    }
    if let Some(next) = s.tasks.ready.pop_front() {
        s.tasks.running = next;
    } else {
        let d = Trap {
            class: TrapClass::Deadlock,
            file: 0,
            range,
        };
        s.tasks.trapped.get_or_insert((d, true));
    }
    turn.notify_all();
}

impl Machine<'_> {
    /// Selectable function `f`'s receive, by range, and the channel it receives from, read
    /// from its arguments `values`: its body starts with `place.recv()`, `place` a parameter
    /// or a field of one (`task.select.4`).
    fn selectable_receiver(
        &self,
        f: &crate::program::FnDef,
        values: &[Value],
    ) -> (TextRange, Value) {
        use moddil_syntax::SyntaxKind as K;
        use moddil_syntax::ast::{self, AstNode};
        let block = f.node.body().expect("a body");
        let mut e = block
            .syntax()
            .children()
            .find_map(ast::Expr::cast)
            .expect("a checked selectable body");
        while !(e.syntax().kind() == K::METHOD_CALL_EXPR
            && ast::member_name(e.syntax()).is_some_and(|n| n.text() == "recv"))
        {
            e = e
                .sub_exprs()
                .into_iter()
                .next()
                .expect("a checked selectable body");
        }
        let at = e.syntax().text_range();
        // The place: a parameter, then fields.
        let mut fields = Vec::new();
        let mut p = e.sub_exprs()[0].clone();
        loop {
            match p.syntax().kind() {
                K::FIELD_EXPR => {
                    fields.push(
                        ast::member_name(p.syntax())
                            .expect("a field")
                            .text()
                            .to_string(),
                    );
                    p = p.sub_exprs()[0].clone();
                }
                K::PAREN_EXPR => p = p.sub_exprs()[0].clone(),
                _ => break,
            }
        }
        let name = p.syntax().text().trim().to_string();
        let mut names = Vec::new();
        if f.self_kind.is_some() {
            names.push("self".to_string());
        }
        names.extend(f.node.params().into_iter().flat_map(|l| {
            l.params()
                .filter_map(|q| q.pat().map(|x| x.syntax().text().trim().to_string()))
                .collect::<Vec<_>>()
        }));
        let k = names.iter().position(|n| *n == name).expect("a parameter");
        let mut v = self.target(values[k].clone());
        for field in fields.iter().rev() {
            let i = self.field_index(&v, field);
            v = match v {
                Value::Adt { fields, .. } => self.target(fields[i].clone()),
                other => other,
            };
        }
        (at, v)
    }

    /// `select { p = op => body, … }` (`task.select.1`): each arm's operation's handle and
    /// value to send are evaluated first, in order; then the first arm whose operation is ready
    /// does it (or, with none ready, the task waits for one to be), its pattern binds the
    /// result, and its body gives the value. An error of an operation marked with `?` passes
    /// on.
    pub(super) fn select_expr(&mut self, node: &moddil_syntax::SyntaxNode) -> Eval {
        use moddil_syntax::SyntaxKind as K;
        use moddil_syntax::ast::{self, AstNode};
        let range = node.text_range();
        let list = node.child(K::MATCH_ARM_LIST).expect("arms");
        let mut ops = Vec::new();
        let mut payloads: Vec<Option<Value>> = Vec::new();
        let mut arms = Vec::new();
        // For an arm on a `#[selectable]` function: the function, its arguments, and its receive.
        let mut rests: Vec<Option<(crate::program::FnDef, Vec<Value>, TextRange)>> = Vec::new();
        for arm in list.children().filter(|c| c.kind() == K::SELECT_ARM) {
            let mut rest = None;
            let exprs: Vec<ast::Expr> = arm.children().filter_map(ast::Expr::cast).collect();
            let pat = arm.children().find_map(ast::Pat::cast).expect("a pattern");
            let op = exprs[0].clone();
            let call = if op.syntax().kind() == K::PROPAGATE_EXPR {
                op.sub_exprs()[0].clone()
            } else {
                op
            };
            let mut payload = None;
            let op = if call.syntax().kind() == K::METHOD_CALL_EXPR {
                let receiver = call.sub_exprs()[0].clone();
                let name = ast::member_name(call.syntax())
                    .expect("a method name")
                    .text()
                    .to_string();
                let v = match self.place_of(&receiver)? {
                    Some((p, _)) => self.memory.load(&p),
                    None => self.eval(&receiver)?,
                };
                let recv_arg = v.clone();
                let v = self.target(v);
                let prelude = matches!(&v, Value::Adt { ty, .. }
                    if ["Receiver", "Sender", "Task"].contains(&ty.as_str()));
                if !prelude {
                    // A `#[selectable]` function: it waits on the channel its body receives
                    // from first (`task.select.4`).
                    let f = self
                        .program
                        .methods(&v.head(), &name)
                        .into_iter()
                        .find_map(|m| match m {
                            crate::program::Method::Code(f, _) => Some(f),
                            _ => None,
                        })
                        .expect("a checked selectable method");
                    let this = match (self.place_of(&receiver)?, recv_arg) {
                        (_, r @ Value::Ref(..)) => r,
                        (Some((p, _)), _) => Value::Ref(p, false),
                        (None, other) => other,
                    };
                    let mut values = vec![this];
                    for a in call
                        .syntax()
                        .child(K::ARG_LIST)
                        .and_then(ast::ArgList::cast)
                        .into_iter()
                        .flat_map(|l| l.args().filter_map(|a| a.expr()).collect::<Vec<_>>())
                    {
                        values.push(self.eval(&a)?);
                    }
                    let (at, chan) = self.selectable_receiver(&f, &values);
                    rest = Some((f, values, at));
                    Op::Recv(id_of(&chan))
                } else {
                    let h = id_of(&v);
                    match name.as_str() {
                        "recv" => Op::Recv(h),
                        "send" => {
                            let arg = call
                                .syntax()
                                .child(K::ARG_LIST)
                                .and_then(ast::ArgList::cast)
                                .and_then(|l| l.args().next())
                                .and_then(|a| a.expr())
                                .expect("a value to send");
                            payload = Some(self.eval(&arg)?);
                            Op::Send(h)
                        }
                        _ => Op::Join(h),
                    }
                }
            } else if crate::machine::segments(
                call.sub_exprs()
                    .first()
                    .map_or(call.syntax(), |c| c.syntax()),
            ) == ["stop_requested"]
            {
                Op::Stop
            } else {
                // A selectable free function (`task.select.4`).
                let path = crate::machine::segments(call.sub_exprs()[0].syntax());
                let f = match self.program.resolve(self.memory.module(), &path) {
                    Some((crate::program::Def::Item(k), _)) => self.program.fns[&k].clone(),
                    _ => unreachable!("a checked selectable function"),
                };
                let mut values = Vec::new();
                for a in call
                    .syntax()
                    .child(K::ARG_LIST)
                    .and_then(ast::ArgList::cast)
                    .into_iter()
                    .flat_map(|l| l.args().filter_map(|a| a.expr()).collect::<Vec<_>>())
                {
                    values.push(self.eval(&a)?);
                }
                let (at, chan) = self.selectable_receiver(&f, &values);
                rest = Some((f, values, at));
                Op::Recv(id_of(&chan))
            };
            ops.push(op);
            payloads.push(payload);
            rests.push(rest);
            arms.push((pat, exprs[1].clone()));
        }
        let closed = Value::Adt {
            ty: "Closed".to_string(),
            variant: "Closed".to_string(),
            fields: Vec::new(),
        };
        let chosen = self.run_op(range, |ts, me| {
            let Some(i) = ops.iter().position(|o| ts.op_ready(me, o)) else {
                return Attempt::Wait(Wait::Select(ops.clone()));
            };
            let outcome = match ops[i] {
                Op::Recv(c) => Ok(option(ts.pop(c))),
                Op::Send(c) => {
                    let v = payloads[i].take().expect("a value to send");
                    if ts.channels[c].receiver {
                        ts.push(c, v);
                        Ok(Value::Unit)
                    } else {
                        Err(closed.clone())
                    }
                }
                Op::Join(t) => match ts.take(t) {
                    Some(Outcome::Value(v)) => Ok(v),
                    Some(Outcome::Error(e)) => Err(e),
                    _ => return Attempt::Unwind,
                },
                Op::Stop => Ok(Value::Unit),
            };
            Attempt::Done(outcome.map(|v| Value::Tuple(vec![Value::Int(i as i64), v])))
        });
        // The values the arms that lost would have sent are dropped.
        for v in payloads.into_iter().flatten() {
            self.drop_value(v, true, range)?;
        }
        let Value::Tuple(mut parts) = chosen? else {
            unreachable!("the chosen arm")
        };
        let v = parts.pop().expect("a result");
        let Value::Int(i) = parts[0] else {
            unreachable!("an index")
        };
        let (pat, body) = arms[i as usize].clone();
        // A selectable function's rest, with what was received.
        let v = match rests[i as usize].take() {
            Some((f, values, at)) => {
                self.received = Some((at, v));
                let r = self.invoke(&f, values, std::collections::HashMap::new(), range);
                self.received = None;
                r?
            }
            None => v,
        };
        let mark = self.memory.mark();
        let slot = self.memory.temp(v);
        let mut binds = Vec::new();
        let matched = self.matches(&pat, &super::patterns::Src::Place(slot, false), &mut binds);
        debug_assert!(matched, "a `select` arm's pattern always matches");
        self.bind(binds);
        let result = self.eval(&body);
        self.exit(mark, result, range)
    }
}

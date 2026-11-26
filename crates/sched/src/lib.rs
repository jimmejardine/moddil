//! The scheduler (`task.sched.1`) the runtimes written in Rust share: the MIR interpreter's,
//! and `moddil-rt`'s, natively and in WebAssembly. It holds the tasks and scopes, which tasks are
//! ready and what the others wait for, how each ended, the channels, the timers, and the OS's
//! events. One scheduler for both is what makes a native run's order the interpreter's.
//!
//! One task runs at a time. The others are queued, first in, first out, and run only when the
//! running one suspends. A task waiting for something becomes ready once it has happened, in the
//! order the waiting tasks began to wait; a cancelled task waiting where cancellation is seen
//! becomes ready too, to unwind.
//!
//! What it is generic over, each runtime gives:
//! - `V`, a value a task gives or a channel carries: the interpreter's `Value`, or the runtime's
//!   bytes ([`Item`]);
//! - `T`, a trap, which fails an isolated scope (`task.isolate.1`);
//! - `D`, what drops a value nobody will take: the runtime's drop glue. A task given none has
//!   nothing given back;
//! - `H`, the OS of a run of `main` ([`Host`]).

use std::collections::{HashMap, VecDeque};
use std::time::Duration;

/// A value of a task or a channel.
pub trait Item: Clone {
    /// `()`, which a timer's channel delivers.
    fn unit() -> Self;
    /// An `int`, as the OS's channels deliver: a connection, a count of bytes, a signal.
    fn int(i: i64) -> Self;
}

/// A value as its bytes, as native code keeps it: `()` has none, and an `int` is its 8
/// little-endian bytes.
impl Item for Vec<u8> {
    fn unit() -> Self {
        Vec::new()
    }

    fn int(i: i64) -> Self {
        i.to_le_bytes().to_vec()
    }
}

/// What the OS brought (`os.1`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// Listener `.0` accepted connection `.1`.
    Accepted(usize, usize),
    /// Connection `.0` received `.1` bytes.
    Data(usize, usize),
    /// Connection `.0` ended.
    Ended(usize),
    /// A signal arrived.
    Signal(i64),
}

/// The OS of a run of `main`, whose events feed channels, and its clock.
pub trait Host {
    /// Reports listener's or connection's `id` events.
    fn watch(&mut self, id: usize);
    /// Reports the signals.
    fn watch_signals(&mut self);
    /// Whether anything watched may still bring an event.
    fn waiting(&self) -> bool;
    /// The events that have arrived, waiting up to `wait` for one (for ever with none).
    fn poll(&mut self, wait: Option<Duration>) -> Vec<Event>;
    /// The time since the run began, in nanoseconds: the wall clock's.
    fn elapsed(&self) -> i64;
}

/// How a task ended.
#[derive(Clone, Debug)]
pub enum Outcome<V> {
    /// Its value.
    Value(V),
    /// Its error.
    Error(V),
    /// It was cancelled, and unwound (`task.cancel.1`).
    Cancelled,
}

/// Why a scope failed: the first failure of its body or of a task given up (`task.error.1`), or
/// a trap in it, if it is isolated (`task.isolate.1`).
#[derive(Clone, Debug)]
pub enum Failure<V, T> {
    /// An error.
    Error(V),
    /// A trap.
    Trapped(T),
}

/// What a suspended task waits for.
#[derive(Clone, Debug)]
pub enum Wait {
    /// A task to end.
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
    /// An `unpark` of the place with this key (a sync cell's word, `task.park.1`).
    Park(u64),
}

/// An operation a `select` arm waits on (`task.select.1`).
#[derive(Clone, Debug)]
pub enum Op {
    /// A `recv` on a channel.
    Recv(usize),
    /// A `send` on a channel.
    Send(usize),
    /// A `join` of a task.
    Join(usize),
    /// `stop_requested()`.
    Stop,
}

/// What an OS channel brings (`os.1`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OsSource {
    /// A listener's connections.
    Incoming,
    /// A connection's arrivals.
    Arrivals,
    /// The signals.
    Signals,
}

struct TaskState<V, D> {
    /// The scope it belongs to (none for the run's own task).
    scope: Option<usize>,
    outcome: Option<Outcome<V>>,
    /// When it ended, among the tasks that have.
    ended: Option<usize>,
    /// Whether its `Task` has been dropped without a `join`.
    detached: bool,
    cancelled: bool,
    /// Whether it is unwinding, during which cancellation is not seen again.
    unwinding: bool,
    waiting: Option<Wait>,
    /// What drops its value, if nobody takes it.
    drop: Option<D>,
}

struct ScopeState<V, T> {
    /// The task whose `scope` call it is.
    owner: usize,
    /// Its tasks: its body's first.
    tasks: Vec<usize>,
    isolated: bool,
    failure: Option<Failure<V, T>>,
    stopped: bool,
}

/// A channel: its values in order, how many it holds, how many senders it has, whether its
/// receiver is alive, and how many values have been sent and received.
struct Channel<V> {
    values: VecDeque<V>,
    capacity: usize,
    senders: usize,
    receiver: bool,
    sent: u64,
    received: u64,
}

/// The scheduler of one run.
pub struct Scheduler<V, T, D, H> {
    tasks: Vec<TaskState<V, D>>,
    scopes: Vec<ScopeState<V, T>>,
    channels: Vec<Channel<V>>,
    ready: VecDeque<usize>,
    /// The suspended tasks, in the order they began to wait.
    blocked: Vec<usize>,
    ended: usize,
    /// The run's virtual time, in nanoseconds (`task.timer.1`).
    now: i64,
    /// The timers not yet fired: when each fires, in the order made, and its channel.
    timers: Vec<(i64, usize)>,
    /// For a run of `main`: the OS, whose events feed channels, and whose clock the run's time
    /// then is (`os.1`).
    os: Option<H>,
    /// The channels the OS's events go to: each listener's connections, each connection's
    /// arrivals, and the signals.
    incoming: HashMap<usize, usize>,
    arrivals: HashMap<usize, usize>,
    signals: Option<usize>,
    /// The seed's generator, which picks the next ready task (`task.runtime.1`); with seed 0,
    /// none, and the first ready runs.
    seed: u64,
}

/// Values given back to be dropped, each with what drops it.
pub type Dropped<D, V> = Vec<(D, V)>;

/// How a scope ended ([`Scheduler::scope_result`]): its body's value or its failure, and the
/// values nobody took, to be dropped.
pub type ScopeEnd<V, T, D> = (Result<Option<V>, Failure<V, T>>, Dropped<D, V>);

impl<V: Item, T: Clone, D: Copy, H: Host> Default for Scheduler<V, T, D, H> {
    fn default() -> Self {
        Self::new()
    }
}

impl<V: Item, T: Clone, D: Copy, H: Host> Scheduler<V, T, D, H> {
    /// A scheduler with the run's own task, and no OS.
    pub fn new() -> Self {
        Scheduler {
            tasks: vec![TaskState::new(None, None)],
            scopes: Vec::new(),
            channels: Vec::new(),
            ready: VecDeque::new(),
            blocked: Vec::new(),
            ended: 0,
            now: 0,
            timers: Vec::new(),
            os: None,
            incoming: HashMap::new(),
            arrivals: HashMap::new(),
            signals: None,
            seed: 0,
        }
    }

    /// For a run of `main`: the OS (`os.1`).
    pub fn set_os(&mut self, os: H) {
        self.os = Some(os);
    }

    /// The OS of a run of `main`, if this is one.
    pub fn os(&mut self) -> Option<&mut H> {
        self.os.as_mut()
    }

    /// A timer's channel, whose one value arrives `nanos` from now (`task.timer.1`).
    pub fn timer(&mut self, nanos: i64) -> usize {
        let c = self.channel(1);
        self.timers
            .push((self.clock().saturating_add(nanos.max(0)), c));
        c
    }

    /// A channel the OS feeds: its events for `id` (a listener's connections, a connection's
    /// arrivals) or the signals. Outside a run of `main` nothing feeds it, so it has no sender,
    /// but for the signals, which never come.
    pub fn os_channel(&mut self, kind: OsSource, id: usize) -> usize {
        let c = self.channel(1024);
        match (kind, self.os.as_mut()) {
            (OsSource::Incoming, Some(os)) => {
                os.watch(id);
                self.incoming.insert(id, c);
            }
            (OsSource::Arrivals, Some(os)) => {
                os.watch(id);
                self.arrivals.insert(id, c);
            }
            (OsSource::Signals, Some(os)) => {
                os.watch_signals();
                self.signals = Some(c);
            }
            (OsSource::Signals, None) => {}
            (_, None) => self.channels[c].senders = 0,
        }
        c
    }

    /// The time since the run began: the wall clock's for a run of `main`, else virtual.
    fn clock(&self) -> i64 {
        match &self.os {
            Some(os) => os.elapsed(),
            None => self.now,
        }
    }

    /// Puts what the OS brought into the channels waiting for it.
    fn deliver_os(&mut self, events: Vec<Event>) {
        for e in events {
            match e {
                Event::Accepted(l, conn) => {
                    if let Some(&c) = self.incoming.get(&l) {
                        self.push(c, V::int(conn as i64));
                    }
                }
                Event::Data(conn, n) => {
                    if let Some(&c) = self.arrivals.get(&conn) {
                        self.push(c, V::int(n as i64));
                    }
                }
                Event::Ended(conn) => {
                    if let Some(c) = self.arrivals.remove(&conn) {
                        self.drop_sender(c);
                    }
                }
                Event::Signal(s) => {
                    if let Some(c) = self.signals {
                        self.push(c, V::int(s));
                    }
                }
            }
        }
    }

    /// Fires timer `i`.
    fn fire(&mut self, i: usize) {
        let (at, c) = self.timers.remove(i);
        self.now = at;
        if self.channels[c].receiver {
            self.push(c, V::unit());
        }
        self.drop_sender(c);
    }

    /// The earliest timer: its index and when it fires.
    fn earliest(&self) -> Option<(usize, i64)> {
        self.timers
            .iter()
            .enumerate()
            .min_by_key(|(_, (at, _))| *at)
            .map(|(i, (at, _))| (i, *at))
    }

    /// Opens a scope for task `owner`'s `scope` call, and gives its index.
    pub fn open_scope(&mut self, owner: usize, isolated: bool) -> usize {
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
    /// cancelled scope is cancelled. `drop` drops its value if nobody takes it.
    pub fn spawn(&mut self, scope: usize, body: bool, drop: Option<D>) -> usize {
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
    pub fn end(&mut self, t: usize, outcome: Outcome<V>) -> Dropped<D, V> {
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

    /// Fails scope `s` with `f`, unless it has failed already, and cancels it.
    pub fn fail(&mut self, s: usize, f: Failure<V, T>) {
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
    pub fn isolating(&self, t: usize) -> Option<usize> {
        let mut scope = self.tasks[t].scope;
        while let Some(s) = scope {
            if self.scopes[s].isolated {
                return Some(s);
            }
            scope = self.tasks[self.scopes[s].owner].scope;
        }
        None
    }

    /// Whether task `t` is cancelled and not yet unwinding.
    pub fn cancelled(&self, t: usize) -> bool {
        self.tasks[t].cancelled && !self.tasks[t].unwinding
    }

    /// Whether task `t` is unwinding.
    pub fn unwinding(&self, t: usize) -> bool {
        self.tasks[t].unwinding
    }

    /// Task `t` starts unwinding.
    pub fn start_unwinding(&mut self, t: usize) {
        self.tasks[t].unwinding = true;
    }

    /// Task `t`'s `Task` has been dropped: an error it ended with fails its scope, and a value
    /// it ended with is given back to be dropped.
    pub fn detach(&mut self, t: usize) -> Option<(D, V)> {
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
    pub fn take(&mut self, t: usize) -> Option<Outcome<V>> {
        let task = &mut self.tasks[t];
        task.ended?;
        Some(std::mem::replace(
            task.outcome.as_mut()?,
            Outcome::Cancelled,
        ))
    }

    /// Whether every task of scope `s` has ended.
    pub fn scope_done(&self, s: usize) -> bool {
        self.scopes[s]
            .tasks
            .iter()
            .all(|&t| self.tasks[t].ended.is_some())
    }

    /// How scope `s` ends, once its tasks have: its failure, or its body's value (`None` for no
    /// value). The values of its other tasks that nobody took are given back to be dropped.
    pub fn scope_result(&mut self, s: usize) -> ScopeEnd<V, T, D> {
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

    /// Asks scope `s` to stop (`task.stop.1`).
    pub fn stop(&mut self, s: usize) {
        self.scopes[s].stopped = true;
        self.wake();
    }

    /// Whether a stop has reached task `t`: its scope, or one around it, has been asked to stop,
    /// and the scopes its scope's tasks have opened inside it have all ended.
    pub fn stop_reached(&self, t: usize) -> bool {
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
    pub fn channel(&mut self, capacity: usize) -> usize {
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

    /// Channel `c` has another sender.
    pub fn clone_sender(&mut self, c: usize) {
        self.channels[c].senders += 1;
    }

    /// A sender of channel `c` is gone.
    pub fn drop_sender(&mut self, c: usize) {
        self.channels[c].senders -= 1;
        self.wake();
    }

    /// The receiver is gone: what the channel holds is given back, for its destructors to run
    /// (`task.channel.2`).
    pub fn drop_receiver(&mut self, c: usize) -> Vec<V> {
        let ch = &mut self.channels[c];
        ch.receiver = false;
        let left: Vec<V> = ch.values.drain(..).collect();
        self.wake();
        left
    }

    /// Whether channel `c`'s receiver is alive.
    pub fn receiver_alive(&self, c: usize) -> bool {
        self.channels[c].receiver
    }

    /// Whether `send` on channel `c` can go ahead: it has room, or its receiver is gone.
    pub fn can_send(&self, c: usize) -> bool {
        let ch = &self.channels[c];
        !ch.receiver || ch.values.len() < ch.capacity.max(1)
    }

    /// Puts `v` in channel `c`, which has room: the number it is among those sent, and whether
    /// the sender waits for it to be received (a rendezvous).
    pub fn push(&mut self, c: usize, v: V) -> (u64, bool) {
        let ch = &mut self.channels[c];
        ch.values.push_back(v);
        ch.sent += 1;
        let r = (ch.sent, ch.capacity == 0);
        self.wake();
        r
    }

    /// Whether the `n`th value sent on channel `c` has been received, or cannot be.
    pub fn taken(&self, c: usize, n: u64) -> bool {
        let ch = &self.channels[c];
        ch.received >= n || !ch.receiver
    }

    /// Whether `recv` on channel `c` can go ahead: it holds a value, or has no senders.
    pub fn can_recv(&self, c: usize) -> bool {
        let ch = &self.channels[c];
        !ch.values.is_empty() || ch.senders == 0
    }

    /// The next value of channel `c`, which can be received: `None` if it has no senders left.
    pub fn pop(&mut self, c: usize) -> Option<V> {
        let ch = &mut self.channels[c];
        let v = ch.values.pop_front()?;
        ch.received += 1;
        self.wake();
        Some(v)
    }

    /// Whether `op` can go ahead for task `t`.
    pub fn op_ready(&self, t: usize, op: &Op) -> bool {
        match op {
            Op::Recv(c) => self.can_recv(*c),
            Op::Send(c) => self.can_send(*c),
            Op::Join(j) => self.tasks[*j].ended.is_some(),
            Op::Stop => self.stop_reached(t),
        }
    }

    /// Suspends task `t` until `w` happens.
    pub fn block(&mut self, t: usize, w: Wait) {
        self.tasks[t].waiting = Some(w);
        self.blocked.push(t);
    }

    /// Makes ready up to `n` of the tasks parked on `key`, in the order they parked
    /// (`task.park.1`): how many.
    pub fn unpark(&mut self, key: u64, n: usize) -> usize {
        let mut woken = 0;
        let tasks = &mut self.tasks;
        let ready = &mut self.ready;
        self.blocked.retain(|&t| {
            let parked = matches!(tasks[t].waiting, Some(Wait::Park(k)) if k == key);
            if parked && woken < n {
                woken += 1;
                tasks[t].waiting = None;
                ready.push_back(t);
                false
            } else {
                true
            }
        });
        woken
    }

    /// Queues task `t` again, after those already ready (`yield_now`).
    pub fn requeue(&mut self, t: usize) {
        self.ready.push_back(t);
    }

    /// The next task to run, if any is ready. With none, time moves to the earliest timer,
    /// which fires (`task.timer.1`), until a task is ready or no timer is left; in a run of
    /// `main`, the OS's events and the wall clock's timers come in, waiting for either.
    pub fn next_ready(&mut self) -> Option<usize> {
        if self.os.is_some() {
            while self.ready.is_empty() {
                let now = self.clock();
                if let Some((i, at)) = self.earliest()
                    && at <= now
                {
                    self.fire(i);
                    continue;
                }
                let until = self.earliest().map(|(_, at)| at);
                let os = self.os.as_mut().expect("the OS");
                if until.is_none() && !os.waiting() {
                    break;
                }
                let wait = until.map(|at| Duration::from_nanos((at - now).max(0) as u64));
                let events = os.poll(wait);
                self.deliver_os(events);
            }
        } else {
            while self.ready.is_empty()
                && let Some((i, _)) = self.earliest()
            {
                self.fire(i);
            }
        }
        if self.seed == 0 || self.ready.len() < 2 {
            return self.ready.pop_front();
        }
        let i = (split_mix(&mut self.seed) % self.ready.len() as u64) as usize;
        self.ready.remove(i)
    }

    /// Runs the tasks from now on in the order seed `seed` picks (`task.runtime.1`): with 0,
    /// first in, first out.
    pub fn set_seed(&mut self, seed: u64) {
        self.seed = seed;
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
                            Wait::Park(_) => false,
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

impl<V, D> TaskState<V, D> {
    fn new(scope: Option<usize>, drop: Option<D>) -> Self {
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

/// The next number of the SplitMix64 generator whose state is `state` (Steele, Lea and Flood,
/// 2014): what a seed's order of tasks is made from (`task.runtime.1`). The state never becomes
/// 0, so a seeded scheduler stays seeded.
pub fn split_mix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    if *state == 0 {
        *state = 0x9e37_79b9_7f4a_7c15;
    }
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

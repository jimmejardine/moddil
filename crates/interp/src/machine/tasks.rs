//! The machine's side of tasks (`task.scope.1`): the prelude's scopes, tasks and channels, run
//! by the [`Scheduler`](crate::sched::Scheduler); suspending and resuming tasks; and unwinding
//! one that is cancelled or traps in an isolated scope (`task.cancel.1`, `task.isolate.1`).

use moddil_mir::TrapClass;
use moddil_sema::{Container, Item, Ty};
use moddil_syntax::TextRange;

use super::Machine;
use crate::calls::tuple;
use crate::sched::{Failure, Op, Outcome, Wait};
use crate::stack::{Frame, Ret};
use crate::value::{Pointer, Value};
use crate::{Stop, Trap};

/// The operation a suspended task is in, which it tries again when it resumes.
pub(super) enum Pending {
    Join(usize),
    /// `yield_now`, once the task has yielded.
    Yielded,
    /// A scope's end.
    Scope(usize),
    /// A `send` of a value of the type, on a channel.
    Send(usize, Value, Ty),
    /// A rendezvous `send`, waiting for the `n`th value to be received.
    Taken(usize, u64),
    /// A `select` whose arm `index` sent on a rendezvous channel, waiting for the `n`th value to
    /// be received (`task.select.1`).
    SelectTaken {
        c: usize,
        n: u64,
        op: SelectOp,
        index: usize,
    },
    Recv(usize),
    Stop,
    /// A `select`: its arms' operations.
    Select(Vec<SelectOp>),
    /// `__park` on the word at `at` (its key) if it holds `expected`; once parked, until an
    /// `unpark` (`task.park.1`).
    Park {
        at: Value,
        key: u64,
        expected: i64,
        parked: bool,
    },
}

/// A `select` arm's operation (`task.select.1`): what it waits on, the value it sends, and
/// where its result, its error and whether it failed go.
#[derive(Clone)]
pub(super) struct SelectOp {
    op: Op,
    payload: Option<Pointer>,
    result: Pointer,
    error: Pointer,
    failed: Pointer,
}

/// How an operation went: done, with a result or an error; waiting; or to be unwound.
enum Attempt {
    Done(Result<Value, Value>),
    Wait(Pending, Wait),
    Unwind,
    /// Unwinding, started already.
    Unwound,
}

impl Machine<'_> {
    /// The scheduler's operation that function `f` of the prelude is, if it is one.
    pub(super) fn task_native(&self, f: u32) -> Option<&'static str> {
        let items = self.dispatch.items();
        let sig = &items.fns[f as usize];
        if !sig.intrinsic {
            return None;
        }
        // The `core` mod's `gpu` module (`gpu.1`): no GPU here.
        if items.modules[sig.module as usize].path == ["gpu"] {
            return Some(match sig.name.as_str() {
                "source" => "gpu_source",
                "bytes" => "gpu_bytes",
                "dispatch" => "gpu_dispatch",
                "frame" => "gpu_frame",
                "show" => "gpu_show",
                _ => return None,
            });
        }
        // The `os` module and the tasks, scopes and channels: as native code names them.
        moddil_mir::runtime_op(items, f)
    }

    /// A destructor the scheduler runs: of a `Task` (given up without a `join`), a `Sender` or
    /// a `Receiver`.
    pub(super) fn native_drop(&mut self, f: u32, type_args: &[Ty], v: &Value) {
        let items = self.dispatch.items();
        let sig = &items.fns[f as usize];
        let Container::Impl(k) = sig.container else {
            return;
        };
        let Ty::Adt(a, ..) = &items.impls[k as usize].self_ty else {
            return;
        };
        let id = id_of(v);
        match items.adts[*a as usize].name.as_str() {
            // The interpreter drops values itself, so the scheduler gives none back.
            "Task" => {
                self.sched.detach(id);
            }
            "Sender" => self.sched.drop_sender(id),
            // What the channel holds is dropped, its destructors first (`task.channel.2`).
            "Receiver" => {
                let left = self.sched.drop_receiver(id);
                if let Some(t) = type_args.first() {
                    let t = self.concrete(t);
                    let calls: Vec<_> = left
                        .into_iter()
                        .flat_map(|x| self.dispatch.drop_glue(x, &t, true))
                        .collect();
                    self.stack.drops_first(calls);
                }
            }
            _ => {}
        }
    }

    /// A call of one of the scheduler's operations, `name`, with type arguments `args`, on
    /// `values`; its result goes where `ret` says, now or once the task resumes.
    pub(super) fn task_call(
        &mut self,
        name: &str,
        args: &[Ty],
        values: &[Value],
        ret: Ret,
        range: TextRange,
    ) -> Option<Stop> {
        let me = self.stack.current();
        let pending = match name {
            "task_id" => return self.deliver(ret, Ok(Value::Int(me as i64))),
            "install_runtime" => {
                self.sched.set_seed(values[0].as_int() as u64);
                return self.deliver(ret, Ok(Value::Unit));
            }
            "executor_of_task" => {
                let e = self.sched.executor(me);
                return self.deliver(ret, Ok(Value::Int(e)));
            }
            "executor_new" => {
                let w = |i: usize| values[i].as_int();
                let e = self.sched.new_executor(crate::sched::Executor {
                    kind: w(0),
                    cores: (w(1), w(2)),
                    node: w(3),
                    priority: w(4),
                    busy_poll: values[5] == Value::Bool(true),
                    threads: w(6),
                });
                return self.deliver(ret, Ok(Value::Int(e)));
            }
            "__scope_open" => {
                let on = values[1].as_int();
                let s = self
                    .sched
                    .open_scope(me, values[0] == Value::Bool(true), on);
                return self.deliver(ret, Ok(Value::Int(s as i64)));
            }
            // The body runs at once, as the scope's first task (`task.scope.1`).
            "__scope_run" => {
                let s = id_of(&self.stack.deref(&values[0]));
                let t = self.start_task(&args[0], &values[1], vec![values[0].clone()]);
                let n = self.sched.spawn(s, true, None);
                debug_assert_eq!(n, t, "tasks are numbered as they start");
                self.sched.place(t, -1, me);
                self.sched.block(me, Wait::Scope(s));
                self.pending.insert(me, (Pending::Scope(s), ret));
                self.stack.switch(t);
                return None;
            }
            "spawn" => {
                let s = id_of(&self.stack.deref(&values[0]));
                let t = self.start_task(&args[0], &values[1], Vec::new());
                let n = self.sched.spawn(s, false, None);
                debug_assert_eq!(n, t, "tasks are numbered as they start");
                let on = match &values[2] {
                    Value::Adt { fields, .. } => fields[0].as_int(),
                    other => unreachable!("an executor: {other:?}"),
                };
                self.sched.place(t, on, me);
                let task = self.natives.handle("Task", t);
                return self.deliver(ret, Ok(task));
            }
            "stop" => {
                self.sched.stop(id_of(&self.stack.deref(&values[0])));
                return self.deliver(ret, Ok(Value::Unit));
            }
            "channel" => {
                let n = values[0].as_int();
                if n < 0 {
                    let t = self.trap(TrapClass::CapacityExceeded, Some(range), Vec::new());
                    return Some(Stop::Trap(t));
                }
                let c = self.sched.channel(n as usize);
                let pair = Value::Tuple(vec![
                    self.natives.handle("Sender", c),
                    self.natives.handle("Receiver", c),
                ]);
                return self.deliver(ret, Ok(pair));
            }
            name if name.starts_with("gpu_") => {
                let v = self.gpu_call(name, values);
                return self.deliver(ret, Ok(v));
            }
            name if name.starts_with("os_") => {
                let v = self.os_call(name, values);
                return self.deliver(ret, Ok(v));
            }
            "timer" => {
                let c = self.sched.timer(values[0].as_int());
                let rx = self.natives.handle("Receiver", c);
                return self.deliver(ret, Ok(rx));
            }
            "clone" => {
                let c = id_of(&self.stack.deref(&values[0]));
                self.sched.clone_sender(c);
                let sender = self.natives.handle("Sender", c);
                return self.deliver(ret, Ok(sender));
            }
            "join" => Pending::Join(id_of(&values[0])),
            "yield_now" => {
                if self.sched.cancelled(me) {
                    return self.unwind_here();
                }
                self.sched.requeue(me);
                self.pending.insert(me, (Pending::Yielded, ret));
                return self.switch_next();
            }
            "send" => {
                let t = args.first().map_or(Ty::Unit, |t| self.concrete(t));
                Pending::Send(id_of(&self.stack.deref(&values[0])), values[1].clone(), t)
            }
            "recv" => Pending::Recv(id_of(&self.stack.deref(&values[0]))),
            "stop_requested" => Pending::Stop,
            "park" => Pending::Park {
                at: values[0].clone(),
                key: park_key(&values[0]),
                expected: values[1].as_int(),
                parked: false,
            },
            "unpark" => {
                let n = usize::try_from(values[1].as_int()).unwrap_or(0);
                let woken = self.sched.unpark(park_key(&values[0]), n);
                return self.deliver(ret, Ok(Value::Int(woken as i64)));
            }
            _ => {
                // An arm on a `#[selectable]` function waits for the runtimes (`task.select.3`).
                let Value::Tuple(arms) = &values[0] else {
                    unreachable!("a `select`'s arms")
                };
                if arms
                    .iter()
                    .any(|a| matches!(a, Value::Tuple(p) if p[0].as_int() == 4))
                {
                    let t = self.trap(TrapClass::Unimplemented, Some(range), Vec::new());
                    return Some(Stop::Trap(t));
                }
                self.select_pending(values)
            }
        };
        self.attempt(pending, ret)
    }

    /// A call of the `core` mod's `os` module, `name` (`os.1`): through the run's OS, which only
    /// a run of `main` has; under a test there is none.
    fn os_call(&mut self, name: &str, values: &[Value]) -> Value {
        use crate::sched::OsSource;
        let int = |v: &Value| v.as_int();
        let id = |v: &Value| usize::try_from(v.as_int()).unwrap_or(usize::MAX);
        let byte = |b: u8| Value::Num(moddil_sema::Num::U8, u128::from(b));
        let bytes = |v: &Value| -> Vec<u8> {
            match v {
                Value::Array(xs) | Value::Buf(_, xs) => xs
                    .iter()
                    .map(|x| match x {
                        Value::Num(_, b) => *b as u8,
                        _ => 0,
                    })
                    .collect(),
                _ => Vec::new(),
            }
        };
        match name {
            "os_arg_count" => {
                let n = self.sched.os().map_or(0, |os| os.args().len());
                Value::Int(n as i64)
            }
            "os_arg_len" => {
                let i = id(&values[0]);
                let n = self
                    .sched
                    .os()
                    .and_then(|os| os.args().get(i).map(String::len))
                    .unwrap_or(0);
                Value::Int(n as i64)
            }
            "os_arg_copy" => {
                let i = id(&values[0]);
                let arg = self
                    .sched
                    .os()
                    .and_then(|os| os.args().get(i).cloned())
                    .unwrap_or_default();
                if let Value::Ref(p, _) = &values[1] {
                    self.store_bytes(p, arg.bytes().map(byte).collect());
                }
                Value::Unit
            }
            "os_write_stderr" => {
                if let Value::Str(s) = self.stack.deref(&values[0]) {
                    moddil_os::write_stderr(&s);
                }
                Value::Unit
            }
            // Under a test, a number from a fixed sequence, so a run replays.
            "os_random" => match self.sched.os() {
                Some(_) => Value::Int(moddil_os::random().max(1)),
                None => Value::Int(4),
            },
            // Under a test, the epoch, so a run replays.
            "os_wall_clock" => match self.sched.os() {
                Some(_) => Value::Int(moddil_os::wall_clock()),
                None => Value::Int(0),
            },
            "os_listen" => {
                let port = u16::try_from(int(&values[0])).unwrap_or(0);
                let h = self
                    .sched
                    .os()
                    .and_then(|os| os.listen(port).ok())
                    .map_or(-1, |h| h as i64);
                Value::Int(h)
            }
            "os_connect" => {
                let host = match self.stack.deref(&values[0]) {
                    Value::Str(s) => s.to_string(),
                    _ => String::new(),
                };
                let port = u16::try_from(int(&values[1])).unwrap_or(0);
                let h = self
                    .sched
                    .os()
                    .and_then(|os| os.connect(&host, port).ok())
                    .map_or(-1, |h| h as i64);
                Value::Int(h)
            }
            "os_incoming" | "os_arrivals" | "os_signals" => {
                let kind = match name {
                    "os_incoming" => OsSource::Incoming,
                    "os_arrivals" => OsSource::Arrivals,
                    _ => OsSource::Signals,
                };
                let which = values.first().map_or(0, id);
                let c = self.sched.os_channel(kind, which);
                self.natives.handle("Receiver", c)
            }
            "os_take" => {
                let conn = id(&values[0]);
                let Value::Ref(p, _) = &values[1] else {
                    return Value::Int(0);
                };
                let mut buf = bytes(&self.stack.load(p));
                let n = self.sched.os().map_or(0, |os| os.take(conn, &mut buf));
                self.store_bytes(p, buf.into_iter().map(byte).collect());
                Value::Int(n as i64)
            }
            "os_send" => {
                let conn = id(&values[0]);
                let data = match &values[1] {
                    Value::Ref(p, _) => bytes(&self.stack.load(p)),
                    _ => Vec::new(),
                };
                let ok = self
                    .sched
                    .os()
                    .is_some_and(|os| os.write(conn, &data).is_ok());
                Value::Bool(ok)
            }
            "os_close" => {
                let h = id(&values[0]);
                if let Some(os) = self.sched.os() {
                    os.close(h);
                }
                Value::Unit
            }
            _ => unreachable!("an `os` function: {name}"),
        }
    }

    /// A call of the `core` mod's `gpu` module, `name` (`gpu.1`): the interpreter has no GPU, so
    /// a kernel has no WGSL, values no bytes, nothing is dispatched, and a frame asks to stop.
    fn gpu_call(&mut self, name: &str, values: &[Value]) -> Value {
        // A slot for the empty slice the calls give, as statics are kept.
        const EMPTY: u32 = u32::MAX - 1;
        match name {
            "gpu_source" => Value::Str("".into()),
            "gpu_bytes" => {
                if self.stack.static_ref(EMPTY).is_none() {
                    self.stack.set_static(EMPTY, Value::Array(Vec::new()));
                }
                self.stack.static_ref(EMPTY).expect("just set")
            }
            "gpu_dispatch" => Value::Bool(false),
            "gpu_show" => Value::Unit,
            "gpu_frame" => {
                if let Value::Ref(p, _) = &values[0] {
                    let words = (0..8).map(|i| Value::Int(i64::from(i == 7))).collect();
                    self.stack.store(p, Value::Array(words));
                }
                Value::Unit
            }
            _ => unreachable!("a `gpu` function: {name}"),
        }
    }

    /// Stores `bytes` where the slice `p` points: as the buffer or array that is there.
    fn store_bytes(&mut self, p: &Pointer, bytes: Vec<Value>) {
        let v = match self.stack.load(p) {
            Value::Buf(cap, _) => Value::Buf(cap, bytes),
            _ => Value::Array(bytes),
        };
        self.stack.store(p, v);
    }

    /// A task running closure `f` (of type `ty`) with `args`, in a stack of its own.
    fn start_task(&mut self, ty: &Ty, f: &Value, args: Vec<Value>) -> usize {
        let Ty::Closure(c) = self.concrete(ty) else {
            unreachable!("a task's closure")
        };
        let Value::Closure(captures) = f.clone() else {
            unreachable!("a closure value")
        };
        let code = self.body(Item::closure(c.owner, c.expr));
        let frame = Frame::new(
            code,
            vec![tuple(captures), tuple(args)],
            c.args.to_vec(),
            Ret::Task,
        );
        self.stack.add_task(frame)
    }

    /// Tries operation `p` for the running task: it is done, the task waits, or (cancelled) it
    /// unwinds.
    fn attempt(&mut self, p: Pending, ret: Ret) -> Option<Stop> {
        match self.try_op(p) {
            Attempt::Done(outcome) => self.deliver(ret, outcome),
            Attempt::Wait(p, w) => {
                let me = self.stack.current();
                self.sched.block(me, w);
                self.pending.insert(me, (p, ret));
                self.switch_next()
            }
            Attempt::Unwind => self.unwind_here(),
            Attempt::Unwound => None,
        }
    }

    fn try_op(&mut self, p: Pending) -> Attempt {
        let me = self.stack.current();
        // A scope's end waits for its tasks however it is cancelled. A value a cancelled
        // `send` was to send is dropped (`task.cancel.1`).
        if self.sched.cancelled(me) && !matches!(p, Pending::Scope(_)) {
            if let Pending::Send(_, v, t) = p {
                let calls = self.dispatch.drop_glue(v, &t, true);
                self.unwind_here();
                self.stack.drops_before_cleanup(calls);
                return Attempt::Unwound;
            }
            return Attempt::Unwind;
        }
        match p {
            Pending::Join(t) => match self.sched.take(t) {
                Some(Outcome::Value(v)) => Attempt::Done(Ok(v)),
                Some(Outcome::Error(e)) => Attempt::Done(Err(e)),
                // A cancelled task's scope is cancelled, and so is the task joining it.
                Some(Outcome::Cancelled) => Attempt::Unwind,
                None => Attempt::Wait(Pending::Join(t), Wait::Join(t)),
            },
            Pending::Yielded => Attempt::Done(Ok(Value::Unit)),
            Pending::Park {
                at,
                key,
                expected,
                parked,
            } => {
                if parked || self.stack.deref(&at).as_int() != expected {
                    return Attempt::Done(Ok(Value::Unit));
                }
                let p = Pending::Park {
                    at,
                    key,
                    expected,
                    parked: true,
                };
                Attempt::Wait(p, Wait::Park(key))
            }
            Pending::Scope(s) => {
                if !self.sched.scope_done(s) {
                    return Attempt::Wait(Pending::Scope(s), Wait::Scope(s));
                }
                let (result, _) = self.sched.scope_result(s);
                if self.sched.cancelled(me) {
                    return Attempt::Unwind;
                }
                match result {
                    Ok(v) => Attempt::Done(Ok(v.unwrap_or(Value::Unit))),
                    Err(Failure::Error(e)) => Attempt::Done(Err(e)),
                    Err(Failure::Trapped(t)) => Attempt::Done(Err(self.trapped(&t))),
                }
            }
            Pending::Send(c, v, t) => {
                if !self.sched.receiver_alive(c) {
                    return Attempt::Done(Err(self.natives.unit_struct("Closed")));
                }
                if !self.sched.can_send(c) {
                    return Attempt::Wait(Pending::Send(c, v, t), Wait::Send(c));
                }
                match self.sched.push(c, v) {
                    (n, true) => Attempt::Wait(Pending::Taken(c, n), Wait::Taken(c, n)),
                    _ => Attempt::Done(Ok(Value::Unit)),
                }
            }
            Pending::Taken(c, n) => {
                if !self.sched.taken(c, n) {
                    return Attempt::Wait(Pending::Taken(c, n), Wait::Taken(c, n));
                }
                if self.sched.receiver_alive(c) {
                    Attempt::Done(Ok(Value::Unit))
                } else {
                    Attempt::Done(Err(self.natives.unit_struct("Closed")))
                }
            }
            Pending::Recv(c) => {
                if !self.sched.can_recv(c) {
                    return Attempt::Wait(Pending::Recv(c), Wait::Recv(c));
                }
                let v = self.sched.pop(c);
                Attempt::Done(Ok(self.natives.option_of(v)))
            }
            Pending::Stop => {
                if self.sched.stop_reached(me) {
                    Attempt::Done(Ok(Value::Unit))
                } else {
                    Attempt::Wait(Pending::Stop, Wait::Stop)
                }
            }
            Pending::Select(ops) => self.try_select(ops),
            Pending::SelectTaken { c, n, op, index } => {
                if !self.sched.taken(c, n) {
                    let w = Wait::Taken(c, n);
                    return Attempt::Wait(Pending::SelectTaken { c, n, op, index }, w);
                }
                if !self.sched.receiver_alive(c) {
                    self.stack
                        .store(&op.error, self.natives.unit_struct("Closed"));
                    self.stack.store(&op.failed, Value::Bool(true));
                }
                Attempt::Done(Ok(Value::Int(index as i64)))
            }
        }
    }

    /// Gives the running task's top frame the result of the call it is at.
    pub(super) fn deliver(&mut self, ret: Ret, outcome: Result<Value, Value>) -> Option<Stop> {
        match (ret, outcome) {
            (Ret::To(dest, target, _), Ok(v)) => {
                self.stack.store(&dest, v);
                self.stack.goto(target);
            }
            (Ret::To(_, _, Some((err, target))), Err(e)) => {
                self.stack.store(&err, e);
                self.stack.goto(target);
            }
            _ => unreachable!("a checked program handles every error"),
        }
        None
    }

    /// Runs the next ready task, trying again the operation it was suspended in. With none
    /// ready, the run is deadlocked: every task waits.
    pub(super) fn switch_next(&mut self) -> Option<Stop> {
        let Some(t) = self.sched.next_ready() else {
            let trap = self.trap(TrapClass::Deadlock, None, Vec::new());
            return Some(Stop::Trap(trap));
        };
        self.stack.switch(t);
        match self.pending.remove(&t) {
            Some((p, ret)) => self.attempt(p, ret),
            None => None,
        }
    }

    /// Starts unwinding the running task from the call it is at.
    fn unwind_here(&mut self) -> Option<Stop> {
        let me = self.stack.current();
        self.sched.start_unwinding(me);
        self.stack.unwind_top(true);
        None
    }

    /// A trap: in an isolated scope (and not while unwinding), it fails the scope with
    /// `Trapped`, cancelling it, and the task unwinds from where it trapped (at its terminator,
    /// or at the statement it has just run); otherwise it ends the run.
    pub(super) fn isolate(&mut self, t: Trap, at_terminator: bool) -> Option<Stop> {
        let me = self.stack.current();
        if self.sched.unwinding(me) || self.stack.in_drop() {
            return Some(Stop::Trap(t));
        }
        let Some(s) = self.sched.isolating(me) else {
            return Some(Stop::Trap(t));
        };
        self.sched.fail(s, Failure::Trapped(t));
        self.sched.start_unwinding(me);
        self.stack.unwind_top(at_terminator);
        None
    }

    /// The end of a frame's cleanup: its caller goes on unwinding from the call; a task's
    /// closure has unwound, and the task ends, cancelled.
    pub(super) fn resume(&mut self) -> Option<Stop> {
        match self.stack.pop() {
            Ret::To(..) => {
                self.stack.unwind_top(true);
                None
            }
            Ret::Task => {
                self.sched.end(self.stack.current(), Outcome::Cancelled);
                self.switch_next()
            }
            Ret::Drop | Ret::Top | Ret::Callback => unreachable!("only a task unwinds"),
        }
    }

    /// The prelude's `Trapped` for trap `t`: its kind, and where it is as `file:line:column`.
    fn trapped(&self, t: &Trap) -> Value {
        let files = self.program.files(self.db);
        let at = files.get(t.file as usize).map_or_else(String::new, |f| {
            let (line, col) = moddil_syntax::line_col(f.source.text(self.db), t.range.start);
            format!("{}:{line}:{col}", f.source.path(self.db))
        });
        self.natives.trapped(&t.class.to_string(), &at)
    }

    /// The pending `select` for a call of `__select`, from its arms' descriptions: each a
    /// tuple of the operation's kind, its handle's borrow, the borrow of the value to send, and
    /// the borrows its result, error and failure go to.
    fn select_pending(&mut self, values: &[Value]) -> Pending {
        let Value::Tuple(arms) = &values[0] else {
            unreachable!("a `select`'s arms")
        };
        let ptr = |v: &Value| match v {
            Value::Ref(p, _) => p.clone(),
            other => unreachable!("a borrow: {other:?}"),
        };
        let ops = arms
            .iter()
            .map(|arm| {
                let Value::Tuple(parts) = arm else {
                    unreachable!("an arm's description")
                };
                let handle = || id_of(&self.through(&parts[1]));
                let op = match parts[0].as_int() {
                    0 => Op::Recv(handle()),
                    1 => Op::Send(handle()),
                    2 => Op::Join(handle()),
                    _ => Op::Stop,
                };
                SelectOp {
                    op,
                    payload: matches!(parts[2], Value::Ref(..)).then(|| ptr(&parts[2])),
                    result: ptr(&parts[3]),
                    error: ptr(&parts[4]),
                    failed: ptr(&parts[5]),
                }
            })
            .collect();
        Pending::Select(ops)
    }

    /// What `v` is once every borrow is followed.
    fn through(&self, v: &Value) -> Value {
        let mut v = v.clone();
        while let Value::Ref(p, _) = &v {
            v = self.stack.load(p);
        }
        v
    }

    /// A `select`: the first arm, in order, whose operation is ready does it, and its index is
    /// the result; with none ready, the task waits for any to be.
    fn try_select(&mut self, ops: Vec<SelectOp>) -> Attempt {
        let me = self.stack.current();
        let Some(i) = ops.iter().position(|o| self.sched.op_ready(me, &o.op)) else {
            let waits = ops.iter().map(|o| o.op.clone()).collect();
            return Attempt::Wait(Pending::Select(ops), Wait::Select(waits));
        };
        let o = &ops[i];
        let outcome = match o.op {
            Op::Recv(c) => Ok(self.natives.option_of(self.sched.pop(c))),
            Op::Send(c) => {
                let p = o.payload.clone().expect("a value to send");
                let v = self.stack.load(&p);
                self.stack.store(&p, Value::Empty);
                if self.sched.receiver_alive(c) {
                    self.stack.store(&o.result, Value::Unit);
                    // On a rendezvous channel the arm is done once the value is received.
                    if let (n, true) = self.sched.push(c, v) {
                        let (op, index) = (o.clone(), i);
                        let w = Wait::Taken(c, n);
                        return Attempt::Wait(Pending::SelectTaken { c, n, op, index }, w);
                    }
                    Ok(Value::Unit)
                } else {
                    Err(self.natives.unit_struct("Closed"))
                }
            }
            Op::Join(t) => match self.sched.take(t) {
                Some(Outcome::Value(v)) => Ok(v),
                Some(Outcome::Error(e)) => Err(e),
                _ => return Attempt::Unwind,
            },
            Op::Stop => Ok(Value::Unit),
        };
        match outcome {
            Ok(v) => self.stack.store(&o.result, v),
            Err(e) => {
                self.stack.store(&o.error, e);
                self.stack.store(&o.failed, Value::Bool(true));
            }
        }
        Attempt::Done(Ok(Value::Int(i as i64)))
    }
}

/// The index a scope's, task's or channel's handle holds.
fn id_of(v: &Value) -> usize {
    match v {
        Value::Int(i) => *i as usize,
        Value::Adt { fields, .. } => fields[0].as_int() as usize,
        other => unreachable!("a handle: {other:?}"),
    }
}

/// The key a sync cell's word at `p` parks tasks under (`task.park.1`): its place's hash.
fn park_key(p: &Value) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::hash::DefaultHasher::new();
    match p {
        Value::Ref(at, _) => at.hash(&mut h),
        Value::Foreign(a) => return *a,
        other => unreachable!("a pointer: {other:?}"),
    }
    h.finish()
}

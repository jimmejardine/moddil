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
    Send(usize, Value),
    /// A rendezvous `send`, waiting for the `n`th value to be received.
    Taken(usize, u64),
    Recv(usize),
    Stop,
    /// A `select`: its arms' operations.
    Select(Vec<SelectOp>),
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
}

impl Machine<'_> {
    /// The scheduler's operation that function `f` of the prelude is, if it is one.
    pub(super) fn task_native(&self, f: u32) -> Option<&'static str> {
        let items = self.dispatch.items();
        let sig = &items.fns[f as usize];
        if !sig.intrinsic {
            return None;
        }
        let owner = match sig.container {
            Container::Impl(k) => match &items.impls[k as usize].self_ty {
                Ty::Adt(a, ..) => items.adts[*a as usize].name.as_str(),
                _ => return None,
            },
            _ => "",
        };
        Some(match (owner, sig.name.as_str()) {
            ("", "yield_now") => "yield_now",
            ("", "stop_requested") => "stop_requested",
            ("", "__scope_open") => "__scope_open",
            ("", "__scope_run") => "__scope_run",
            ("", "channel") => "channel",
            ("", "__select") => "__select",
            ("Scope", "spawn") => "spawn",
            ("Scope", "stop") => "stop",
            ("Task", "join") => "join",
            ("Sender", "send") => "send",
            ("Sender", "clone") => "clone",
            ("Receiver", "recv") => "recv",
            _ => return None,
        })
    }

    /// A destructor the scheduler runs: of a `Task` (given up without a `join`), a `Sender` or
    /// a `Receiver`.
    pub(super) fn native_drop(&mut self, f: u32, v: &Value) {
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
            "Task" => self.sched.detach(id),
            "Sender" => self.sched.drop_sender(id),
            "Receiver" => self.sched.drop_receiver(id),
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
            "__scope_open" => {
                let s = self.sched.open_scope(me, values[0] == Value::Bool(true));
                return self.deliver(ret, Ok(Value::Int(s as i64)));
            }
            // The body runs at once, as the scope's first task (`task.scope.1`).
            "__scope_run" => {
                let s = id_of(&self.stack.deref(&values[0]));
                let t = self.start_task(&args[0], &values[1], vec![values[0].clone()]);
                self.sched.spawn(t, s, true);
                self.sched.block(me, Wait::Scope(s));
                self.pending.insert(me, (Pending::Scope(s), ret));
                self.stack.switch(t);
                return None;
            }
            "spawn" => {
                let s = id_of(&self.stack.deref(&values[0]));
                let t = self.start_task(&args[0], &values[1], Vec::new());
                self.sched.spawn(t, s, false);
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
            "send" => Pending::Send(id_of(&self.stack.deref(&values[0])), values[1].clone()),
            "recv" => Pending::Recv(id_of(&self.stack.deref(&values[0]))),
            "stop_requested" => Pending::Stop,
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
        }
    }

    fn try_op(&mut self, p: Pending) -> Attempt {
        let me = self.stack.current();
        // A scope's end waits for its tasks however it is cancelled.
        if self.sched.cancelled(me) && !matches!(p, Pending::Scope(_)) {
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
            Pending::Scope(s) => {
                if !self.sched.scope_done(s) {
                    return Attempt::Wait(Pending::Scope(s), Wait::Scope(s));
                }
                let result = self.sched.scope_result(s);
                if self.sched.cancelled(me) {
                    return Attempt::Unwind;
                }
                match result {
                    Ok(v) => Attempt::Done(Ok(v)),
                    Err(Failure::Error(e)) => Attempt::Done(Err(e)),
                    Err(Failure::Trapped(t)) => Attempt::Done(Err(self.trapped(&t))),
                }
            }
            Pending::Send(c, v) => {
                if !self.sched.receiver_alive(c) {
                    return Attempt::Done(Err(self.natives.unit_struct("Closed")));
                }
                if !self.sched.can_send(c) {
                    return Attempt::Wait(Pending::Send(c, v), Wait::Send(c));
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
        let Some(t) = self.sched.next() else {
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
            Ret::Drop | Ret::Top => unreachable!("only a task unwinds"),
        }
    }

    /// The prelude's `Trapped` for trap `t`: its kind, and where it is as `line:column`.
    fn trapped(&self, t: &Trap) -> Value {
        let files = self.program.files(self.db);
        let at = files.get(t.file as usize).map_or_else(String::new, |f| {
            let (line, col) = moddil_syntax::line_col(f.source.text(self.db), t.range.start);
            format!("{line}:{col}")
        });
        self.natives.trapped(t.class.to_string(), at)
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
                    self.sched.push(c, v);
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

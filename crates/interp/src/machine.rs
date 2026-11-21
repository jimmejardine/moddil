//! The interpreter's loop: it runs the top frame's statements and terminators, using the frame
//! stack for memory, the dispatcher for types and calls, and the natives for built-in functions.

use moddil_mir::{
    AggregateKind, Body, Local, Place, Projection, Rvalue, Statement, Terminator, TrapClass, mir,
};
use moddil_sema::{Callee, Db, Item, Owner, Program, Ty};
use moddil_syntax::TextRange;

use crate::calls::{CallPlan, plan, tuple};
use crate::dispatch::Dispatch;
use crate::natives::Natives;
use crate::ops;
use crate::sched::{Outcome, Scheduler, Wait};
use crate::stack::{Frame, PendingDrop, Ret, Stack};
use crate::value::{Pointer, Value};
use crate::{MAX_DEPTH, Observer, Stop, Trap};

/// The interpreter's state while it runs one function or test.
struct Machine<'a> {
    db: &'a dyn Db,
    program: Program,
    stack: Stack<'a>,
    dispatch: Dispatch<'a>,
    natives: Natives<'a>,
    /// For compile-time evaluation: the steps it may still take.
    fuel: Option<u64>,
    /// The tasks and scopes (`task.sched.1`).
    sched: Scheduler,
    /// For each suspended task: the call it is suspended in, and where its result goes.
    pending: std::collections::HashMap<usize, (Pending, Ret)>,
}

/// The call a suspended task is in.
#[derive(Clone, Copy)]
enum Pending {
    Join(usize),
    Yield,
    /// A scope's end: after its body (`__scope_close`, which can fail), or after the body failed
    /// (`__scope_wait`).
    Scope(usize, bool),
}

/// The MIR of a function, test or closure of a checked program.
fn body<'db>(db: &'db dyn Db, owner: Owner<'db>) -> &'db Body {
    mir(db, owner)
        .as_ref()
        .expect("the interpreter only runs files without errors")
}

/// Runs `item` of `program` with `args`, until it returns, fails or traps.
pub(crate) fn run(
    db: &dyn Db,
    program: Program,
    item: Item,
    args: &[Value],
    observer: Observer<'_>,
    fuel: Option<u64>,
) -> Result<Value, Stop> {
    let items = moddil_sema::item_tree(db, program);
    let first = Frame::new(
        body(db, Owner::new(db, program, item)),
        args.to_vec(),
        Vec::new(),
        Ret::Top,
    );
    let mut m = Machine {
        db,
        program,
        stack: Stack::new(first),
        dispatch: Dispatch::new(items),
        natives: Natives::new(items),
        fuel,
        sched: Scheduler::new(),
        pending: std::collections::HashMap::new(),
    };
    loop {
        if let Some(f) = &mut m.fuel {
            if *f == 0 {
                let trap = m.trap(TrapClass::OutOfFuel, None, Vec::new());
                return Err(Stop::Trap(trap));
            }
            *f -= 1;
        }
        if let Some(done) = m.step(observer) {
            return done;
        }
    }
}

impl<'a> Machine<'a> {
    /// The MIR of `item` of the program.
    fn body(&self, item: Item) -> &'a Body {
        body(self.db, Owner::new(self.db, self.program, item))
    }

    /// Runs one statement, terminator or destructor call; the result once the run has ended.
    fn step(&mut self, observer: Observer<'_>) -> Option<Result<Value, Stop>> {
        // Running the destructors of a `Drop` terminator, one call at a time.
        match self.stack.pending_drop() {
            PendingDrop::None => {}
            PendingDrop::Finished => return None,
            PendingDrop::Call((f, type_args, v)) => {
                if self.stack.depth() >= MAX_DEPTH {
                    let trap = self.trap(TrapClass::StackOverflow, None, Vec::new());
                    return Some(Err(Stop::Trap(trap)));
                }
                let code = self.body(Item::Fn(f));
                self.stack
                    .push(Frame::new(code, vec![v], type_args, Ret::Drop));
                return None;
            }
        }
        match self.stack.next_statement() {
            Some(s) => self.statement(s, observer).map(Err),
            None => self.terminator(self.stack.terminator()),
        }
    }

    fn statement(&mut self, s: &Statement, observer: Observer<'_>) -> Option<Stop> {
        match s {
            Statement::Assign(place, r) => {
                let v = self.rvalue(r);
                let ptr = self.stack.resolve(place);
                self.stack.store(&ptr, v);
            }
            Statement::Assert {
                cond,
                expected,
                trap,
                range,
                parts,
            } => {
                if self.stack.operand(cond) != Value::Bool(*expected) {
                    let parts = parts
                        .iter()
                        .map(|p| {
                            let evaluated = *self.stack.local(p.evaluated) == Value::Bool(true);
                            let value = evaluated
                                .then(|| self.stack.shown(self.stack.local(p.value).clone()));
                            (p.range, value)
                        })
                        .collect();
                    return Some(Stop::Trap(self.trap(*trap, Some(*range), parts)));
                }
            }
            Statement::Observe { range, value } => {
                let v = self.stack.shown(self.stack.operand(value));
                observer(self.stack.body().src.file().unwrap_or(0), *range, v);
            }
            Statement::Read(_) | Statement::StorageDead(_) => {}
        }
        None
    }

    fn terminator(&mut self, t: &'a Terminator) -> Option<Result<Value, Stop>> {
        match t {
            Terminator::Goto(to) => self.stack.goto(to.index()),
            Terminator::SwitchBool { cond, then, else_ } => {
                let to = if self.stack.operand(cond) == Value::Bool(true) {
                    then
                } else {
                    else_
                };
                self.stack.goto(to.index());
            }
            Terminator::Call {
                callee,
                args,
                dest,
                target,
                on_error,
                range,
                ..
            } => {
                if self.stack.depth() >= MAX_DEPTH {
                    let trap = self.trap(TrapClass::StackOverflow, Some(*range), Vec::new());
                    return Some(Err(Stop::Trap(trap)));
                }
                let values: Vec<Value> = args.iter().map(|a| self.stack.take(a)).collect();
                let dest = self.stack.resolve(dest);
                let on_error = on_error
                    .as_ref()
                    .map(|(p, b)| (self.stack.resolve(p), b.index()));
                let ret = Ret::To(dest.clone(), target.index(), on_error);
                return self
                    .call(callee, values, &dest, target.index(), ret, *range)
                    .map(Err);
            }
            Terminator::Drop {
                place,
                target,
                glue_only,
            } => {
                let ptr = self.stack.resolve(place);
                let v = self.stack.load(&ptr);
                self.stack.store(&ptr, Value::Empty);
                let ty = self.concrete(&self.stack.body().place_ty(self.dispatch.items(), place));
                let calls = self.dispatch.drop_glue(v, &ty, !glue_only);
                self.stack.start_drops(calls, target.index());
            }
            Terminator::Return => {
                let value = self.stack.local(Local::RETURN).clone();
                if self.stack.at_root() {
                    // Show borrows while what they point to still exists.
                    return Some(Ok(self.stack.shown(value)));
                }
                match self.stack.pop() {
                    Ret::To(dest, target, _) => {
                        self.stack.store(&dest, value);
                        self.stack.goto(target);
                    }
                    Ret::Task => {
                        self.sched.end(self.stack.current(), Ok(value));
                        return self.switch_next().map(Err);
                    }
                    Ret::Drop | Ret::Top => {}
                }
            }
            Terminator::Fail(e) => {
                let error = self.stack.operand(e);
                if self.stack.at_root() {
                    return Some(Err(Stop::Failed {
                        file: self.stack.body().src.file().unwrap_or(0),
                        range: self.stack.terminator_span(),
                        error: self.stack.shown(error),
                    }));
                }
                match self.stack.pop() {
                    Ret::To(_, _, Some((err, target))) => {
                        self.stack.store(&err, error);
                        self.stack.goto(target);
                    }
                    Ret::Task => {
                        self.sched.end(self.stack.current(), Err(error));
                        return self.switch_next().map(Err);
                    }
                    _ => unreachable!("a checked program handles every error"),
                }
            }
            Terminator::Trap(class, reason) => {
                let mut t = self.trap(*class, None, Vec::new());
                t.reason = Some(reason.clone());
                return Some(Err(Stop::Trap(t)));
            }
            Terminator::Unreachable => unreachable!("MIR reached an unreachable block"),
        }
        None
    }

    /// Calls `callee` with `values`: pushes the frame it runs in, or, for a built-in function or
    /// a copy, stores the result at `dest` and continues at `target`.
    fn call(
        &mut self,
        callee: &Callee,
        values: Vec<Value>,
        dest: &Pointer,
        target: usize,
        ret: Ret,
        range: TextRange,
    ) -> Option<Stop> {
        // The prelude's tasks and scopes (`task.scope.1`), which the scheduler runs.
        if let Callee::Fn { func, args } = callee {
            let sig = &self.dispatch.items().fns[*func as usize];
            if sig.intrinsic
                && matches!(
                    sig.name.as_str(),
                    "spawn"
                        | "join"
                        | "yield_now"
                        | "__scope_open"
                        | "__scope_wait"
                        | "__scope_close"
                )
            {
                let name = sig.name.clone();
                return self.task_call(&name, args, &values, ret);
            }
        }
        let result = match plan(callee, values, &self.stack, &mut self.dispatch) {
            CallPlan::Closure {
                item,
                args,
                type_args,
            } => {
                let code = self.body(item);
                self.stack.push(Frame::new(code, args, type_args, ret));
                return None;
            }
            CallPlan::Fn((f, type_args), values) => {
                let code = self.body(Item::Fn(f));
                self.stack.push(Frame::new(code, values, type_args, ret));
                return None;
            }
            CallPlan::Intrinsic(f, values) => {
                match self.natives.call(f, &values, &mut self.stack) {
                    Ok(v) => v,
                    Err(class) => {
                        return Some(Stop::Trap(self.trap(class, Some(range), Vec::new())));
                    }
                }
            }
            CallPlan::Copy(v) => v,
        };
        self.stack.store(dest, result);
        self.stack.goto(target);
        None
    }

    /// A call of the prelude's tasks and scopes: `name`, with type arguments `args`, on
    /// `values`; its result goes where `ret` says, now or once the task resumes.
    fn task_call(&mut self, name: &str, args: &[Ty], values: &[Value], ret: Ret) -> Option<Stop> {
        let me = self.stack.current();
        let id = |v: &Value| match v {
            Value::Int(i) => *i as usize,
            Value::Adt { fields, .. } => fields[0].as_int() as usize,
            other => unreachable!("a task or scope: {other:?}"),
        };
        match name {
            "__scope_open" => {
                let s = self.sched.open_scope();
                self.deliver(ret, Ok(Value::Int(s as i64)));
            }
            // A task starts queued, running its closure in a stack of its own (`task.spawn.1`).
            "spawn" => {
                let scope = id(&self.stack.deref(&values[0]));
                let Ty::Closure(c) = self.concrete(&args[0]) else {
                    unreachable!("a task's closure")
                };
                let Value::Closure(captures) = values[1].clone() else {
                    unreachable!("a closure value")
                };
                let code = self.body(Item::closure(c.owner, c.expr));
                let frame = Frame::new(
                    code,
                    vec![tuple(captures), Value::Unit],
                    c.args.to_vec(),
                    Ret::Task,
                );
                let t = self.stack.add_task(frame);
                self.sched.spawn(t, scope);
                let task = self.natives.task(t);
                self.deliver(ret, Ok(task));
            }
            "join" => {
                let t = id(&values[0]);
                match self.sched.take(t) {
                    Some(outcome) => self.deliver(ret, outcome),
                    None => {
                        self.sched.block(me, Wait::Join(t));
                        self.pending.insert(me, (Pending::Join(t), ret));
                        return self.switch_next();
                    }
                }
            }
            "yield_now" => {
                self.sched.requeue(me);
                self.pending.insert(me, (Pending::Yield, ret));
                return self.switch_next();
            }
            _ => {
                let s = id(&values[0]);
                let close = name == "__scope_close";
                if self.sched.scope_done(s) {
                    let outcome = self.scope_outcome(s, close);
                    self.deliver(ret, outcome);
                } else {
                    self.sched.block(me, Wait::Scope(s));
                    self.pending.insert(me, (Pending::Scope(s, close), ret));
                    return self.switch_next();
                }
            }
        }
        None
    }

    /// How a scope's end goes once its tasks have: after its body, it fails with the error of a
    /// task never joined (`task.error.1`).
    fn scope_outcome(&mut self, s: usize, close: bool) -> Outcome {
        match self.sched.scope_error(s) {
            Some(e) if close => Err(e),
            _ => Ok(Value::Unit),
        }
    }

    /// Runs the next ready task, finishing the call it was suspended in.
    fn switch_next(&mut self) -> Option<Stop> {
        let t = self
            .sched
            .next()
            .expect("a task is ready while the run has not ended");
        self.stack.switch(t);
        if let Some((p, ret)) = self.pending.remove(&t) {
            let outcome = match p {
                Pending::Join(j) => self.sched.take(j).expect("an ended task"),
                Pending::Yield => Ok(Value::Unit),
                Pending::Scope(s, close) => self.scope_outcome(s, close),
            };
            self.deliver(ret, outcome);
        }
        None
    }

    /// Gives the running task's top frame the result of the call it is at.
    fn deliver(&mut self, ret: Ret, outcome: Outcome) {
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
    }

    /// `ty` as the top frame runs it.
    fn concrete(&self, ty: &Ty) -> Ty {
        self.dispatch.concrete(ty, self.stack.type_args())
    }

    fn rvalue(&mut self, r: &Rvalue) -> Value {
        match r {
            // A const's value, computed once (it was checked when compiling).
            Rvalue::StaticRef(f) => {
                if let Some(r) = self.stack.static_ref(*f) {
                    return r;
                }
                let v = run(
                    self.db,
                    self.program,
                    Item::Fn(*f),
                    &[],
                    &mut |_, _, _| {},
                    None,
                )
                .unwrap_or_else(|_| unreachable!("a const evaluated when compiling"));
                self.stack.set_static(*f, v);
                self.stack.static_ref(*f).expect("just set")
            }
            Rvalue::Use(a) => self.stack.take(a),
            Rvalue::Unary(op, a) => ops::unary(*op, self.stack.operand(a)),
            Rvalue::NegOverflows(a) => Value::Bool(ops::neg_overflows(&self.stack.operand(a))),
            Rvalue::Overflows(op, a, b) => Value::Bool(ops::overflows(
                *op,
                &self.stack.operand(a),
                &self.stack.operand(b),
            )),
            Rvalue::Binary(op, a, b) => {
                ops::binary(*op, self.stack.operand(a), self.stack.operand(b))
            }
            Rvalue::Aggregate(kind, ops) => {
                let fields: Vec<Value> = ops.iter().map(|o| self.stack.take(o)).collect();
                match kind {
                    AggregateKind::Tuple => Value::Tuple(fields),
                    AggregateKind::Array => Value::Array(fields),
                    AggregateKind::Adt { adt, variant, .. } => Value::Adt {
                        adt: *adt,
                        variant: *variant,
                        fields,
                    },
                    AggregateKind::Closure(_) => Value::Closure(fields),
                }
            }
            Rvalue::ErrorIs(p, a) => {
                Value::Bool(matches!(self.stack.read(p), Value::Adt { adt, .. } if adt == *a))
            }
            Rvalue::Discriminant(p) => match self.stack.read(p) {
                Value::Adt { variant, .. } => Value::Int(i64::from(variant)),
                other => unreachable!("the discriminant of {other:?}"),
            },
            Rvalue::Ref(m, p) => {
                let ptr = self.stack.resolve(p);
                // A borrow of what a `dyn` borrow points to is one too.
                let (Some(Projection::Deref), Some(base)) = (
                    p.projection.last(),
                    p.projection.len().checked_sub(1).map(|n| Place {
                        local: p.local,
                        projection: p.projection[..n].to_vec(),
                    }),
                ) else {
                    return Value::Ref(ptr, *m);
                };
                match self.stack.read(&base) {
                    Value::Dyn(_, _, t) => Value::Dyn(ptr, *m, t),
                    _ => Value::Ref(ptr, *m),
                }
            }
            Rvalue::Repeat(a, n) => {
                let v = self.stack.take(a);
                Value::Array(vec![v; self.dispatch.length(n, self.stack.type_args())])
            }
            Rvalue::Length(n) => Value::Int(self.dispatch.length(n, self.stack.type_args()) as i64),
            Rvalue::InBounds(i, p) => {
                let i = self.stack.operand(i).as_int();
                match self.stack.read(p) {
                    Value::Array(xs) | Value::Buf(_, xs) => {
                        Value::Bool(i >= 0 && (i as usize) < xs.len())
                    }
                    other => unreachable!("indexing {other:?}"),
                }
            }
            // An array borrow is a slice borrow as it is.
            Rvalue::Unsize(a, from) if matches!(self.concrete(from), Ty::Array(..)) => {
                self.stack.take(a)
            }
            Rvalue::Unsize(a, Ty::String) => match self.stack.take(a) {
                Value::Ref(ptr, _) => match self.stack.load(&ptr) {
                    Value::String(s) => Value::Str(s.as_str().into()),
                    other => unreachable!("a `String`: {other:?}"),
                },
                other => unreachable!("a borrow of a `String`: {other:?}"),
            },
            Rvalue::Unsize(a, from) => {
                let from = self.concrete(from);
                match self.stack.take(a) {
                    Value::Ref(ptr, m) => Value::Dyn(ptr, m, from),
                    v @ Value::Dyn(..) => v,
                    other => unreachable!("a `dyn` borrow of {other:?}"),
                }
            }
        }
    }

    /// A trap of `class` at `range` (or at the current terminator). A trap in a
    /// `#[track_caller]` function, or in the prelude's or derived code, is reported where the
    /// nearest caller that is neither called it.
    fn trap(
        &self,
        class: TrapClass,
        range: Option<TextRange>,
        parts: Vec<(TextRange, Option<Value>)>,
    ) -> Trap {
        let (file, range, here) = self
            .stack
            .trap_site_from(range, class == TrapClass::Precondition);
        Trap {
            class,
            file,
            range,
            parts: if here { parts } else { Vec::new() },
            reason: None,
        }
    }
}

//! The interpreter's loop: it runs the top frame's statements and terminators, using the frame
//! stack for memory, the dispatcher for types and calls, and the natives for built-in functions.

use moddil_mir::{
    AggregateKind, Body, Local, Place, Projection, Rvalue, Statement, Terminator, TrapClass, mir,
};
use moddil_sema::{Callee, Db, Item, Owner, Program, Ty};
use moddil_syntax::TextRange;

use crate::calls::{CallPlan, plan};
use crate::dispatch::Dispatch;
use crate::natives::Natives;
use crate::ops;
use crate::sched::{Outcome, Scheduler};
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
    pending: std::collections::HashMap<usize, (tasks::Pending, Ret)>,
}

mod ffi;
mod tasks;

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
    os: Option<Vec<String>>,
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
        sched: match os {
            Some(args) => Scheduler::new().with_os(args),
            None => Scheduler::new(),
        },
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
                if self.dispatch.items().fns[f as usize].intrinsic {
                    self.native_drop(f, &type_args, &v);
                    return None;
                }
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
            Some(s) => match self.statement(s, observer) {
                Some(Stop::Trap(t)) => self.isolate(t, false).map(Err),
                other => other.map(Err),
            },
            None => match self.terminator(self.stack.terminator()) {
                Some(Err(Stop::Trap(t))) => self.isolate(t, true).map(Err),
                other => other,
            },
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
                            let value =
                                evaluated.then(|| self.stack.shown(self.stack.read(&p.value)));
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
                        self.sched.end(self.stack.current(), Outcome::Value(value));
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
                        self.sched.end(self.stack.current(), Outcome::Error(error));
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
            Terminator::Resume => return self.resume().map(Err),
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
        // The prelude's tasks, scopes and channels (`task.scope.1`), which the scheduler runs.
        if let Callee::Fn { func, args } = callee
            && let Some(name) = self.task_native(*func)
        {
            return self.task_call(name, args, &values, ret, range);
        }
        let result = match plan(callee, values, &self.stack, &mut self.dispatch) {
            CallPlan::Closure {
                item,
                args,
                type_args,
                back,
            } => {
                let code = self.body(item);
                let mut frame = Frame::new(code, args, type_args, ret);
                frame.write_back = back;
                self.stack.push(frame);
                return None;
            }
            CallPlan::Fn((f, _), values) if self.dispatch.items().fns[f as usize].foreign => {
                // A C function (`ffi.extern.1`).
                match self.c_call(f, &values) {
                    Ok(v) => v,
                    Err(why) => {
                        eprintln!("moddil: calling C: {why}");
                        let t = self.trap(TrapClass::Unimplemented, Some(range), Vec::new());
                        return Some(Stop::Trap(t));
                    }
                }
            }
            CallPlan::Fn((f, type_args), values) => {
                // A stub mod's function without a body (`mod.stub.1`).
                if !self.dispatch.items().fns[f as usize].has_body {
                    let t = self.trap(TrapClass::Unimplemented, Some(range), Vec::new());
                    return Some(Stop::Trap(t));
                }
                let code = self.body(Item::Fn(f));
                self.stack.push(Frame::new(code, values, type_args, ret));
                return None;
            }
            CallPlan::Intrinsic(f, values, type_args) => {
                if let Some(name) = self.task_native(f) {
                    return self.task_call(name, &[], &values, ret, range);
                }
                match self.natives.call(f, &values, &type_args, &mut self.stack) {
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
            Rvalue::Len(p) => match self.stack.read(p) {
                Value::Array(xs) | Value::Buf(_, xs) => Value::Int(xs.len() as i64),
                other => unreachable!("the length of {other:?}"),
            },
            Rvalue::SpanOk(a, b, p) => {
                let (a, b) = (
                    self.stack.operand(a).as_int(),
                    self.stack.operand(b).as_int(),
                );
                let n = match self.stack.read(p) {
                    Value::Array(xs) | Value::Buf(_, xs) => xs.len() as i64,
                    other => unreachable!("slicing {other:?}"),
                };
                Value::Bool(0 <= a && a <= b && b <= n)
            }
            Rvalue::InBounds(i, p) => {
                let i = self.stack.operand(i).as_int();
                match self.stack.read(p) {
                    Value::Array(xs) | Value::Buf(_, xs) => {
                        Value::Bool(i >= 0 && (i as usize) < xs.len())
                    }
                    other => unreachable!("indexing {other:?}"),
                }
            }
            // A number as a wider type (`num.widen.1`): exact, so `checked_to` gives it.
            Rvalue::Widen(a, to) => {
                let v = self.stack.operand(a);
                crate::num::convert("checked_to", &v, to).expect("a widening is exact")
            }
            // An array borrow is a slice borrow as it is.
            Rvalue::Unsize(a, from) if matches!(self.concrete(from), Ty::Array(..)) => {
                self.stack.take(a)
            }
            // `Box<T>` as `Box<dyn Trait>`: its value keeps its type (`type.box.dyn.1`).
            Rvalue::Unsize(a, from) if matches!(self.concrete(from), Ty::Adt(b, ..) if Some(b) == self.dispatch.items().lang("Box")) =>
            {
                let Ty::Adt(_, _, args) = self.concrete(from) else {
                    unreachable!()
                };
                let inner = args[0].clone();
                match self.stack.take(a) {
                    Value::Adt {
                        adt,
                        variant,
                        mut fields,
                    } => {
                        if let Some(Value::Buf(_, xs)) = fields.first_mut() {
                            for x in xs.iter_mut() {
                                if !matches!(x, Value::Owned(..)) {
                                    let v = std::mem::replace(x, Value::Empty);
                                    *x = Value::Owned(Box::new((v, inner.clone())));
                                }
                            }
                        }
                        Value::Adt {
                            adt,
                            variant,
                            fields,
                        }
                    }
                    other => unreachable!("a box: {other:?}"),
                }
            }
            Rvalue::Unsize(a, from) => {
                let from = self.concrete(from);
                match self.stack.take(a) {
                    Value::Ref(ptr, m) => Value::Dyn(ptr, m, Box::new(from)),
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

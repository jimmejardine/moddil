//! The MIR interpreter: runs analysed or runtime MIR for compile-time evaluation, tests before
//! any backend, the Miri-style checker and the playground.
//!
//! It runs calls on its own stack of frames, not the host's, so deep recursion in a Moddil
//! program is a [`TrapClass::StackOverflow`] trap and never crashes the interpreter. A borrow is
//! a pointer to a frame's local and a path of fields in it; borrows cannot outlive what they
//! point to (the borrow checker sees to that), so the frame is always there. See this crate's
//! `README.md`.
//!
//! Generic code runs once for every type: each frame carries its type arguments, and a call of a
//! trait's method is resolved from them, through a **witness table** built the first time a type
//! is asked for that trait (`types.md`: Generics). Moving a value out of a place leaves the place
//! empty, which is the drop flag: dropping an empty place does nothing.

use std::collections::HashMap;

use moddil_mir::{
    AggregateKind, BinOp, Body, Const, Local, Operand, Place, Projection, Rvalue, Statement,
    Terminator, TrapClass, UnOp, mir,
};
use moddil_sema::{Callee, Db, Item, ItemTree, Owner, Program, Shape, Solver, Src, Ty};
use moddil_syntax::TextRange;

/// A value.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Value {
    /// An `int`.
    Int(i64),
    /// A `bool`.
    Bool(bool),
    /// `()`.
    Unit,
    /// A `char`.
    Char(char),
    /// A `&str`: the text it points to. A shared borrow keeps what it points to from changing,
    /// so the text is held, not where it is.
    Str(std::sync::Arc<str>),
    /// A `String`.
    String(String),
    /// A tuple of one or more elements.
    Tuple(Vec<Value>),
    /// A struct (variant 0) or an enum's variant, with its fields in order.
    Adt {
        /// The struct or enum, by its index in the file's item tree.
        adt: u32,
        /// The variant.
        variant: u32,
        /// The fields' values.
        fields: Vec<Value>,
    },
    /// A borrow, while the program runs: where it points, and whether it is `&mut`.
    Ref(Pointer, bool),
    /// A `dyn` borrow (`&dyn Time`): where it points, whether it is `&mut`, and the type of what
    /// it points to, which chooses the methods called through it (its **vtable**).
    Dyn(Pointer, bool, Ty),
    /// A closure: what it captures, in order. Its type says which body it runs.
    Closure(Vec<Value>),
    /// A borrow as reported outside the program (an observed value, a result): what it pointed
    /// to, and whether it was `&mut`.
    Borrowed(Box<Value>, bool),
    /// Nothing: a place not assigned yet, or moved out of.
    Empty,
}

/// Where a borrow points: a local of a frame, and a path of field indices into it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Pointer {
    frame: usize,
    local: u32,
    path: Vec<u32>,
}

impl Value {
    /// The value as written in Moddil source and in `// =>` expectations: `3`, `-1`, `true`,
    /// `()`, `(1, true)`, `Point { x: 1, y: 2 }`, `Shape::Circle(3)`, `&3`.
    pub fn render(&self, items: &ItemTree) -> String {
        match self {
            Value::Int(v) => v.to_string(),
            Value::Bool(v) => v.to_string(),
            Value::Unit => "()".to_string(),
            Value::Char(c) => format!("{c:?}"),
            Value::Str(s) => format!("{:?}", &**s),
            Value::String(s) => format!("{s:?}"),
            Value::Tuple(xs) if xs.len() == 1 => format!("({},)", xs[0].render(items)),
            Value::Tuple(xs) => {
                let xs: Vec<String> = xs.iter().map(|x| x.render(items)).collect();
                format!("({})", xs.join(", "))
            }
            Value::Adt {
                adt,
                variant,
                fields,
            } => {
                let path = items.adts[*adt as usize].path(*variant);
                let def = items.variant(*adt, *variant);
                match def.shape {
                    Shape::Unit => path,
                    Shape::Tuple => {
                        let xs: Vec<String> = fields.iter().map(|x| x.render(items)).collect();
                        format!("{path}({})", xs.join(", "))
                    }
                    Shape::Named => {
                        let xs: Vec<String> = def
                            .fields
                            .iter()
                            .zip(fields)
                            .map(|(f, x)| format!("{}: {}", f.name, x.render(items)))
                            .collect();
                        format!("{path} {{ {} }}", xs.join(", "))
                    }
                }
            }
            Value::Borrowed(v, true) => format!("&mut {}", v.render(items)),
            Value::Borrowed(v, false) => format!("&{}", v.render(items)),
            Value::Ref(..) | Value::Dyn(..) => "&…".to_string(),
            Value::Closure(_) => "{closure}".to_string(),
            Value::Empty => "<moved>".to_string(),
        }
    }
}

/// A trap: what kind, and the source of the check that failed (or of the call, for a stack
/// overflow).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trap {
    /// What kind of trap.
    pub class: TrapClass,
    /// The file it is in, by its index in the program.
    pub file: u32,
    /// Where.
    pub range: TextRange,
    /// For a failed `assert`: each part of the condition and its value, or `None` if it was not
    /// evaluated.
    pub parts: Vec<(TextRange, Option<Value>)>,
    /// For a failed `?!`: its reason.
    pub reason: Option<String>,
}

/// Why running a function or test stopped without a result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stop {
    /// A trap.
    Trap(Trap),
    /// An error escaped it (a test's, or a function asked to run that can fail): where it left
    /// the function or test, and the error.
    Failed {
        /// The file the error left from.
        file: u32,
        /// Where the error left.
        range: TextRange,
        /// The error.
        error: Value,
    },
}

/// How deeply calls may nest before a [`TrapClass::StackOverflow`] trap.
pub const MAX_DEPTH: usize = 10_000;

/// Reports the values of observed expressions (those followed by `// =>`): their file, range and
/// value.
pub type Observer<'a> = &'a mut dyn FnMut(u32, TextRange, Value);

/// Calls function `index` of `program` with `args`.
///
/// # Panics
///
/// If the file has errors (check `moddil_mir::has_errors` first), or `args` does not match the
/// function's parameters.
///
/// # Errors
///
/// The trap, if the program traps, or the error, if one escapes.
pub fn call_fn(
    db: &dyn Db,
    program: Program,
    index: u32,
    args: &[Value],
    observer: Observer<'_>,
) -> Result<Value, Stop> {
    run(db, Owner::new(db, program, Item::Fn(index)), args, observer)
}

/// Runs test `index` of `program`.
///
/// # Panics
///
/// If the file has errors.
///
/// # Errors
///
/// The trap, if the test traps, or the error, if one escapes it (which fails it).
pub fn run_test(
    db: &dyn Db,
    program: Program,
    index: u32,
    observer: Observer<'_>,
) -> Result<(), Stop> {
    run(
        db,
        Owner::new(db, program, Item::Test(index)),
        &[],
        observer,
    )
    .map(|_| ())
}

/// Where a call's result goes.
enum Ret {
    /// Out of the interpreter: the function or test it was asked to run.
    Top,
    /// To a place of the caller, which then continues at a block; and, for a function that can
    /// fail, where its error goes and where the caller continues then.
    To(Pointer, usize, Option<(Pointer, usize)>),
    /// Nowhere: the call is a destructor, run while the caller drops a value.
    Drop,
}

/// A function to run, with its type arguments.
type Target = (u32, Vec<Ty>);

/// A destructor call waiting to run: the function, its type arguments, and the value.
type DropCall = (u32, Vec<Ty>, Value);

struct Frame<'a> {
    body: &'a Body,
    /// The type arguments it runs with (none for a function that is not generic).
    type_args: Vec<Ty>,
    locals: Vec<Value>,
    block: usize,
    stmt: usize,
    ret: Ret,
    /// While a `Drop` terminator runs: the destructors still to call, and where to go after.
    drops: Vec<DropCall>,
    after_drops: Option<usize>,
}

impl<'a> Frame<'a> {
    fn new(body: &'a Body, args: Vec<Value>, type_args: Vec<Ty>, ret: Ret) -> Self {
        assert_eq!(args.len(), body.arg_count, "argument count");
        let mut locals = vec![Value::Empty; body.locals.len()];
        for (slot, a) in locals[1..].iter_mut().zip(args) {
            *slot = a;
        }
        Frame {
            body,
            type_args,
            locals,
            block: 0,
            stmt: 0,
            ret,
            drops: Vec::new(),
            after_drops: None,
        }
    }
}

/// The interpreter's state: the stack of frames, and the witness tables built so far.
struct Machine<'a> {
    stack: Vec<Frame<'a>>,
    items: &'a ItemTree,
    /// For a trait's method and the type arguments of a call of it: the function that runs
    /// and its type arguments.
    witnesses: HashMap<(u32, Vec<Ty>), Option<Target>>,
}

impl Machine<'_> {
    fn top(&self) -> usize {
        self.stack.len() - 1
    }

    /// Where `place` of the top frame is, following borrows.
    fn resolve(&self, place: &Place) -> Pointer {
        let mut ptr = Pointer {
            frame: self.top(),
            local: place.local.0,
            path: Vec::new(),
        };
        for p in &place.projection {
            match p {
                Projection::Field(i) => ptr.path.push(*i),
                Projection::Downcast(_) => {}
                Projection::Deref => match self.load(&ptr) {
                    Value::Ref(target, _) | Value::Dyn(target, _, _) => ptr = target,
                    other => unreachable!("dereferencing {other:?}"),
                },
            }
        }
        ptr
    }

    fn load(&self, ptr: &Pointer) -> Value {
        let mut v = &self.stack[ptr.frame].locals[ptr.local as usize];
        for &i in &ptr.path {
            v = match v {
                Value::Tuple(xs) | Value::Adt { fields: xs, .. } => &xs[i as usize],
                other => unreachable!("a field of {other:?}"),
            };
        }
        v.clone()
    }

    fn store(&mut self, ptr: &Pointer, value: Value) {
        let mut v = &mut self.stack[ptr.frame].locals[ptr.local as usize];
        for &i in &ptr.path {
            v = match v {
                Value::Tuple(xs) | Value::Adt { fields: xs, .. } => &mut xs[i as usize],
                other => unreachable!("a field of {other:?}"),
            };
        }
        *v = value;
    }

    fn read(&self, place: &Place) -> Value {
        self.load(&self.resolve(place))
    }

    fn operand(&self, o: &Operand) -> Value {
        match o {
            Operand::Copy(p) | Operand::Move(p) => self.read(p),
            Operand::Const(Const::Int(v)) => Value::Int(*v),
            Operand::Const(Const::Bool(v)) => Value::Bool(*v),
            Operand::Const(Const::Unit) => Value::Unit,
            Operand::Const(Const::Char(c)) => Value::Char(*c),
            Operand::Const(Const::Str(i)) => Value::Str(
                self.stack[self.top()].body.strs[*i as usize]
                    .as_str()
                    .into(),
            ),
        }
    }

    /// Like [`Machine::operand`], but a moved value leaves its place empty.
    fn take(&mut self, o: &Operand) -> Value {
        let v = self.operand(o);
        if let Operand::Move(p) = o
            && !p.is_indirect()
        {
            let ptr = self.resolve(p);
            self.store(&ptr, Value::Empty);
        }
        v
    }

    /// `ty` as the top frame runs it: its type parameters replaced, associated types resolved.
    fn concrete(&self, ty: &Ty) -> Ty {
        let args = &self.stack[self.top()].type_args;
        Solver::new(self.items, &[]).deep(&ty.subst(args))
    }

    /// The function a call runs and its type arguments: for a trait's method, the impl's method
    /// (or the trait's default) for the type it is called for. `None` for the built-in `Clone`
    /// of a borrow or tuple, which copies.
    fn target(&mut self, callee: &Callee) -> Option<(u32, Vec<Ty>)> {
        let args: Vec<Ty> = callee.args().iter().map(|t| self.concrete(t)).collect();
        match callee {
            Callee::Fn { func, .. } => Some((*func, args)),
            Callee::Trait { func, .. } => {
                let key = (*func, args.clone());
                if let Some(w) = self.witnesses.get(&key) {
                    return w.clone();
                }
                let items = self.items;
                let sig = &items.fns[*func as usize];
                let t = items.trait_of(*func).expect("a trait's method");
                let def = &items.traits[t as usize];
                let n = def.generics.params.len();
                let trait_ref = moddil_sema::TraitRef {
                    trait_: t,
                    name: def.name.as_str().into(),
                    args: args[1..n].to_vec().into(),
                };
                if let Ty::Anon(_) = &args[0] {
                    let found = Some((*func, args.clone()));
                    self.witnesses.insert(key, found.clone());
                    return found;
                }
                let found = Solver::new(items, &[])
                    .impl_method(*func, &args[0], &trait_ref)
                    .map(|(f, mut parent)| {
                        parent.extend(args[sig.generics.parent as usize..].iter().cloned());
                        (f, parent)
                    });
                self.witnesses.insert(key, found.clone());
                found
            }
        }
    }

    /// The destructors to run, in order, to drop `v` of type `ty`: its own (if `own` and its type
    /// has one; the destructor then drops the fields), or else its fields', last first.
    fn drop_glue(&self, v: Value, ty: &Ty, own: bool, out: &mut Vec<DropCall>) {
        match (v, ty) {
            (v @ Value::Adt { .. }, Ty::Adt(a, ..)) if own && self.items.has_drop_impl(*a) => {
                let drop = self.items.lang_ref("Drop").expect("the prelude's `Drop`");
                let decl = self.items.traits[drop.trait_ as usize].methods[0];
                let (f, args) = Solver::new(self.items, &[])
                    .impl_method(decl, ty, &drop)
                    .expect("a destructor");
                out.push((f, args, v));
            }
            (
                Value::Adt {
                    variant, fields, ..
                },
                t @ Ty::Adt(..),
            ) => {
                let types = self.items.field_types(t, variant);
                for (f, ft) in fields.into_iter().zip(types).rev() {
                    self.drop_glue(f, &ft, true, out);
                }
            }
            (Value::Tuple(xs), Ty::Tuple(ts)) => {
                for (x, t) in xs.into_iter().zip(ts.iter()).rev() {
                    self.drop_glue(x, t, true, out);
                }
            }
            (Value::Closure(xs), Ty::Closure(c)) => {
                for (x, t) in xs.into_iter().zip(c.captures.iter()).rev() {
                    self.drop_glue(x, t, true, out);
                }
            }
            (Value::Tuple(xs), Ty::Anon(a)) => {
                for (x, (_, t)) in xs.into_iter().zip(a.methods.iter()).rev() {
                    self.drop_glue(x, t, true, out);
                }
            }
            _ => {}
        }
    }

    /// `v` with every borrow replaced by what it points to, for reporting outside the program.
    fn shown(&self, v: Value) -> Value {
        match v {
            Value::Ref(ptr, m) | Value::Dyn(ptr, m, _) => {
                Value::Borrowed(Box::new(self.shown(self.load(&ptr))), m)
            }
            Value::Tuple(xs) => Value::Tuple(xs.into_iter().map(|x| self.shown(x)).collect()),
            Value::Adt {
                adt,
                variant,
                fields,
            } => Value::Adt {
                adt,
                variant,
                fields: fields.into_iter().map(|x| self.shown(x)).collect(),
            },
            v => v,
        }
    }

    fn rvalue(&mut self, r: &Rvalue) -> Value {
        match r {
            Rvalue::Use(a) => self.take(a),
            Rvalue::Unary(op, a) => match (op, self.operand(a)) {
                (UnOp::Neg, Value::Int(v)) => Value::Int(v.wrapping_neg()),
                (UnOp::Not, Value::Int(v)) => Value::Int(!v),
                (UnOp::Not, Value::Bool(v)) => Value::Bool(!v),
                (op, v) => unreachable!("{op:?} on {v:?}"),
            },
            Rvalue::NegOverflows(a) => Value::Bool(int(&self.operand(a)) == i64::MIN),
            Rvalue::Overflows(op, a, b) => {
                let (a, b) = (int(&self.operand(a)), int(&self.operand(b)));
                Value::Bool(match op {
                    BinOp::Add => a.checked_add(b).is_none(),
                    BinOp::Sub => a.checked_sub(b).is_none(),
                    BinOp::Mul => a.checked_mul(b).is_none(),
                    BinOp::Div => a.checked_div(b).is_none(),
                    BinOp::Shl => !(0..64).contains(&b) || (a << b) >> b != a,
                    BinOp::Shr => !(0..64).contains(&b),
                    _ => false,
                })
            }
            Rvalue::Binary(op, a, b) => binary(*op, self.operand(a), self.operand(b)),
            Rvalue::Aggregate(kind, ops) => {
                let fields: Vec<Value> = ops.iter().map(|o| self.take(o)).collect();
                match kind {
                    AggregateKind::Tuple => Value::Tuple(fields),
                    AggregateKind::Adt { adt, variant, .. } => Value::Adt {
                        adt: *adt,
                        variant: *variant,
                        fields,
                    },
                    AggregateKind::Closure(_) => Value::Closure(fields),
                }
            }
            Rvalue::ErrorIs(p, a) => {
                Value::Bool(matches!(self.read(p), Value::Adt { adt, .. } if adt == *a))
            }
            Rvalue::Discriminant(p) => match self.read(p) {
                Value::Adt { variant, .. } => Value::Int(i64::from(variant)),
                other => unreachable!("the discriminant of {other:?}"),
            },
            Rvalue::Ref(m, p) => {
                let ptr = self.resolve(p);
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
                match self.read(&base) {
                    Value::Dyn(_, _, t) => Value::Dyn(ptr, *m, t),
                    _ => Value::Ref(ptr, *m),
                }
            }
            Rvalue::Unsize(a, Ty::String) => match self.take(a) {
                Value::Ref(ptr, _) => match self.load(&ptr) {
                    Value::String(s) => Value::Str(s.as_str().into()),
                    other => unreachable!("a `String`: {other:?}"),
                },
                other => unreachable!("a borrow of a `String`: {other:?}"),
            },
            Rvalue::Unsize(a, from) => match self.take(a) {
                Value::Ref(ptr, m) => Value::Dyn(ptr, m, self.concrete(from)),
                v @ Value::Dyn(..) => v,
                other => unreachable!("a `dyn` borrow of {other:?}"),
            },
        }
    }
}

fn body<'db>(db: &'db dyn Db, owner: Owner<'db>) -> &'db Body {
    mir(db, owner)
        .as_ref()
        .expect("the interpreter only runs files without errors")
}

fn run<'db>(
    db: &'db dyn Db,
    owner: Owner<'db>,
    args: &[Value],
    observer: Observer<'_>,
) -> Result<Value, Stop> {
    let file = owner.program(db);
    let mut m = Machine {
        stack: vec![Frame::new(
            body(db, owner),
            args.to_vec(),
            Vec::new(),
            Ret::Top,
        )],
        items: moddil_sema::item_tree(db, file),
        witnesses: HashMap::new(),
    };
    loop {
        let depth = m.stack.len();
        let top = m.top();
        // Running the destructors of a `Drop` terminator, one call at a time.
        if let Some(after) = m.stack[top].after_drops {
            if m.stack[top].drops.is_empty() {
                m.stack[top].after_drops = None;
                m.stack[top].block = after;
                m.stack[top].stmt = 0;
            } else {
                let (f, type_args, v) = m.stack[top].drops.remove(0);
                if depth >= MAX_DEPTH {
                    return Err(Stop::Trap(m.trap(
                        TrapClass::StackOverflow,
                        None,
                        Vec::new(),
                    )));
                }
                let callee = body(db, Owner::new(db, file, Item::Fn(f)));
                m.stack
                    .push(Frame::new(callee, vec![v], type_args, Ret::Drop));
            }
            continue;
        }
        let frame = &m.stack[top];
        let code = frame.body;
        let block = &code.blocks[frame.block];
        if let Some(s) = block.statements.get(frame.stmt) {
            m.stack[top].stmt += 1;
            match s {
                Statement::Assign(place, r) => {
                    let v = m.rvalue(r);
                    let ptr = m.resolve(place);
                    m.store(&ptr, v);
                }
                Statement::Assert {
                    cond,
                    expected,
                    trap,
                    range,
                    parts,
                } => {
                    if m.operand(cond) != Value::Bool(*expected) {
                        let locals = &m.stack[top].locals;
                        let parts = parts
                            .iter()
                            .map(|p| {
                                let evaluated = locals[p.evaluated.index()] == Value::Bool(true);
                                let value =
                                    evaluated.then(|| m.shown(locals[p.value.index()].clone()));
                                (p.range, value)
                            })
                            .collect();
                        return Err(Stop::Trap(m.trap(*trap, Some(*range), parts)));
                    }
                }
                Statement::Observe { range, value } => {
                    let v = m.operand(value);
                    let v = m.shown(v);
                    observer(code.src.file().unwrap_or(0), *range, v);
                }
                Statement::Read(_) | Statement::StorageDead(_) => {}
            }
            continue;
        }
        match &block.terminator {
            Terminator::Goto(t) => {
                m.stack[top].block = t.index();
                m.stack[top].stmt = 0;
            }
            Terminator::SwitchBool { cond, then, else_ } => {
                let to = if m.operand(cond) == Value::Bool(true) {
                    then
                } else {
                    else_
                };
                m.stack[top].block = to.index();
                m.stack[top].stmt = 0;
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
                if depth >= MAX_DEPTH {
                    return Err(Stop::Trap(m.trap(
                        TrapClass::StackOverflow,
                        Some(*range),
                        Vec::new(),
                    )));
                }
                let (callee, args, dest, target) =
                    (callee.clone(), args.clone(), dest.clone(), *target);
                let mut values: Vec<Value> = args.iter().map(|a| m.take(a)).collect();
                // A method called through a `dyn` borrow is the one of the type it points to.
                let callee = match callee {
                    Callee::Trait {
                        func,
                        args: mut targs,
                    } if matches!(m.concrete(&targs[0]), Ty::Dyn(_)) => {
                        if let Some(Value::Dyn(ptr, mutable, t)) = values.first().cloned() {
                            values[0] = Value::Ref(ptr, mutable);
                            targs[0] = t;
                        }
                        Callee::Trait { func, args: targs }
                    }
                    c => c,
                };
                let dest = m.resolve(&dest);
                let on_error = on_error.clone().map(|(p, b)| (m.resolve(&p), b.index()));
                // An anonymous impl's method is a closure of its own: its value is the method's,
                // the arguments but `self` its arguments. One it leaves out is the trait's default.
                let mut anon = None;
                if let Callee::Trait { func, args: targs } = &callee
                    && let Ty::Anon(a) = m.concrete(&targs[0])
                    && let Some(k) = a.methods.iter().position(|(f, _)| f == func)
                {
                    let this = match values[0].clone() {
                        Value::Ref(ptr, _) => m.load(&ptr),
                        v => v,
                    };
                    let Value::Tuple(methods) = this else {
                        unreachable!("an anonymous impl's value")
                    };
                    let rest = values.split_off(1);
                    let tuple = if rest.is_empty() {
                        Value::Unit
                    } else {
                        Value::Tuple(rest)
                    };
                    values = vec![methods[k].clone(), tuple];
                    anon = Some(a.methods[k].1.clone());
                }
                let closure_ty = match (&callee, &anon) {
                    (_, Some(t)) => Some(t.clone()),
                    (Callee::Trait { args: targs, .. }, None) => match m.concrete(&targs[0]) {
                        t @ Ty::Closure(_) => Some(t),
                        _ => None,
                    },
                    _ => None,
                };
                if let Some(Ty::Closure(c)) = closure_ty {
                    let captures = match values[0].clone() {
                        Value::Ref(ptr, _) => m.load(&ptr),
                        v => v,
                    };
                    let Value::Closure(captures) = captures else {
                        unreachable!("a closure's value")
                    };
                    let env = if captures.is_empty() {
                        Value::Unit
                    } else {
                        Value::Tuple(captures)
                    };
                    let item = Item::closure(c.owner, c.expr);
                    let code = body(db, Owner::new(db, file, item));
                    m.stack.push(Frame::new(
                        code,
                        vec![env, values[1].clone()],
                        c.args.to_vec(),
                        Ret::To(dest, target.index(), on_error),
                    ));
                    continue;
                }
                match m.target(&callee) {
                    Some((f, _)) if m.items.fns[f as usize].intrinsic => {
                        let v = m.intrinsic(f, &values);
                        m.store(&dest, v);
                        m.stack[top].block = target.index();
                        m.stack[top].stmt = 0;
                    }
                    Some((f, type_args)) => {
                        let code = body(db, Owner::new(db, file, Item::Fn(f)));
                        m.stack.push(Frame::new(
                            code,
                            values,
                            type_args,
                            Ret::To(dest, target.index(), on_error),
                        ));
                    }
                    None => {
                        // The built-in `Clone` of a borrow or a tuple: a copy.
                        let copy = match &values[0] {
                            Value::Ref(ptr, _) => m.load(ptr),
                            v => v.clone(),
                        };
                        m.store(&dest, copy);
                        m.stack[top].block = target.index();
                        m.stack[top].stmt = 0;
                    }
                }
            }
            Terminator::Drop {
                place,
                target,
                glue_only,
            } => {
                let (place, target, glue_only) = (place.clone(), *target, *glue_only);
                let ptr = m.resolve(&place);
                let v = m.load(&ptr);
                m.store(&ptr, Value::Empty);
                let ty = m.concrete(&code.place_ty(m.items, &place));
                let mut calls = Vec::new();
                m.drop_glue(v, &ty, !glue_only, &mut calls);
                m.stack[top].drops = calls;
                m.stack[top].after_drops = Some(target.index());
            }
            Terminator::Return => {
                let value = m.stack[top].locals[Local::RETURN.index()].clone();
                if let Ret::Top = m.stack[top].ret {
                    // Show borrows while what they point to still exists.
                    return Ok(m.shown(value));
                }
                let done = m.stack.pop().expect("a frame");
                match done.ret {
                    Ret::To(dest, target, _) => {
                        m.store(&dest, value);
                        let caller = m.stack.last_mut().expect("a caller");
                        caller.block = target;
                        caller.stmt = 0;
                    }
                    Ret::Drop | Ret::Top => {}
                }
            }
            Terminator::Fail(e) => {
                let error = m.operand(e);
                if let Ret::Top = m.stack[top].ret {
                    let range = code.blocks[m.stack[top].block].terminator_span;
                    return Err(Stop::Failed {
                        file: code.src.file().unwrap_or(0),
                        range,
                        error: m.shown(error),
                    });
                }
                let done = m.stack.pop().expect("a frame");
                match done.ret {
                    Ret::To(_, _, Some((err, target))) => {
                        m.store(&err, error);
                        let caller = m.stack.last_mut().expect("a caller");
                        caller.block = target;
                        caller.stmt = 0;
                    }
                    _ => unreachable!("a checked program handles every error"),
                }
            }
            Terminator::Trap(class, reason) => {
                let mut t = m.trap(*class, None, Vec::new());
                t.reason = Some(reason.clone());
                return Err(Stop::Trap(t));
            }
            Terminator::Unreachable => unreachable!("MIR reached an unreachable block"),
        }
    }
}

impl Machine<'_> {
    /// A trap of `class` at `range` (or at the current terminator). A trap in the prelude's or
    /// derived code is reported where the file's code called it.
    fn trap(
        &self,
        class: TrapClass,
        range: Option<TextRange>,
        parts: Vec<(TextRange, Option<Value>)>,
    ) -> Trap {
        let top = &self.stack[self.top()];
        // A trap in a `#[track_caller]` function, or in the prelude's or derived code, is reported
        // where the nearest caller that is neither called it.
        let tracked = |f: &Frame<'_>| f.body.track_caller || !f.body.src.is_file();
        if !tracked(top)
            && let Src::File(file) = top.body.src
        {
            let range = range.unwrap_or(top.body.blocks[top.block].terminator_span);
            return Trap {
                class,
                file,
                range,
                parts,
                reason: None,
            };
        }
        let caller = self.stack.iter().rev().find(|f| !tracked(f)).unwrap_or(top);
        Trap {
            class,
            file: caller.body.src.file().unwrap_or(0),
            range: caller.body.blocks[caller.block].terminator_span,
            parts: Vec::new(),
            reason: None,
        }
    }
}

fn int(v: &Value) -> i64 {
    match v {
        Value::Int(i) => *i,
        other => unreachable!("expected an int, found {other:?}"),
    }
}

fn binary(op: BinOp, a: Value, b: Value) -> Value {
    use Value::{Bool, Char, Int, Str};
    match (op, a, b) {
        (BinOp::Eq, a, b) => Bool(a == b),
        (BinOp::Ne, a, b) => Bool(a != b),
        (BinOp::Lt, Char(a), Char(b)) => Bool(a < b),
        (BinOp::Le, Char(a), Char(b)) => Bool(a <= b),
        (BinOp::Gt, Char(a), Char(b)) => Bool(a > b),
        (BinOp::Ge, Char(a), Char(b)) => Bool(a >= b),
        (BinOp::Lt, Str(a), Str(b)) => Bool(a < b),
        (BinOp::Le, Str(a), Str(b)) => Bool(a <= b),
        (BinOp::Gt, Str(a), Str(b)) => Bool(a > b),
        (BinOp::Ge, Str(a), Str(b)) => Bool(a >= b),
        (BinOp::Lt, Int(a), Int(b)) => Bool(a < b),
        (BinOp::Le, Int(a), Int(b)) => Bool(a <= b),
        (BinOp::Gt, Int(a), Int(b)) => Bool(a > b),
        (BinOp::Ge, Int(a), Int(b)) => Bool(a >= b),
        (BinOp::Add, Int(a), Int(b)) => Int(a.wrapping_add(b)),
        (BinOp::Sub, Int(a), Int(b)) => Int(a.wrapping_sub(b)),
        (BinOp::Mul, Int(a), Int(b)) => Int(a.wrapping_mul(b)),
        (BinOp::Div, Int(a), Int(b)) => Int(a.wrapping_div(b)),
        (BinOp::Rem, Int(a), Int(b)) => Int(a.wrapping_rem(b)),
        (BinOp::BitAnd, Int(a), Int(b)) => Int(a & b),
        (BinOp::BitOr, Int(a), Int(b)) => Int(a | b),
        (BinOp::BitXor, Int(a), Int(b)) => Int(a ^ b),
        (BinOp::Shl, Int(a), Int(b)) => Int(a.wrapping_shl(b as u32)),
        (BinOp::Shr, Int(a), Int(b)) => Int(a.wrapping_shr(b as u32)),
        (BinOp::BitAnd, Bool(a), Bool(b)) => Bool(a & b),
        (BinOp::BitOr, Bool(a), Bool(b)) => Bool(a | b),
        (BinOp::BitXor, Bool(a), Bool(b)) => Bool(a ^ b),
        (op, a, b) => unreachable!("{op:?} on {a:?} and {b:?}"),
    }
}

impl Machine<'_> {
    /// Runs the prelude's built-in function `f` (`#[intrinsic]`) on `args`.
    fn intrinsic(&mut self, f: u32, args: &[Value]) -> Value {
        let sig = &self.items.fns[f as usize];
        let self_ty = match sig.container {
            moddil_sema::Container::Impl(k) => self.items.impls[k as usize].self_ty.clone(),
            _ => Ty::Unit,
        };
        let name = sig.name.clone();
        // An argument as a value: a borrow is read, a `&str` is its text.
        let arg = |m: &Self, i: usize| match &args[i] {
            Value::Ref(p, _) => m.load(p),
            v => v.clone(),
        };
        let text = |m: &Self, i: usize| -> String {
            match arg(m, i) {
                Value::Str(s) => s.to_string(),
                Value::String(s) => s,
                other => unreachable!("text: {other:?}"),
            }
        };
        let ch = |m: &Self, i: usize| match arg(m, i) {
            Value::Char(c) => c,
            other => unreachable!("a `char`: {other:?}"),
        };
        // Changes the `String` the first argument borrows.
        let edit = |m: &mut Self, f: &mut dyn FnMut(&mut String) -> Value| {
            let Value::Ref(p, _) = &args[0] else {
                unreachable!("a `&mut String`")
            };
            let Value::String(mut s) = m.load(p) else {
                unreachable!("a `String`")
            };
            let v = f(&mut s);
            m.store(p, Value::String(s));
            v
        };
        match (&self_ty, name.as_str()) {
            (Ty::Int | Ty::Bool | Ty::Char, "fmt") => {
                let s = match arg(self, 0) {
                    Value::Int(v) => v.to_string(),
                    Value::Bool(v) => v.to_string(),
                    Value::Char(c) => c.to_string(),
                    other => unreachable!("`Display` of {other:?}"),
                };
                let Value::Ref(p, _) = &args[1] else {
                    unreachable!("a `&mut String`")
                };
                let Value::String(mut out) = self.load(p) else {
                    unreachable!("a `String`")
                };
                out.push_str(&s);
                self.store(p, Value::String(out));
                Value::Unit
            }
            (Ty::Char, "code") => Value::Int(i64::from(u32::from(ch(self, 0)))),
            (Ty::Char, "from_code") => {
                let Value::Int(n) = arg(self, 0) else {
                    unreachable!("an `int`")
                };
                let c = u32::try_from(n).ok().and_then(char::from_u32);
                self.option(c.map(Value::Char))
            }
            (Ty::Char, "is_alphabetic") => Value::Bool(ch(self, 0).is_alphabetic()),
            (Ty::Char, "is_numeric") => Value::Bool(ch(self, 0).is_numeric()),
            (Ty::Char, "is_whitespace") => Value::Bool(ch(self, 0).is_whitespace()),
            (Ty::Char, "is_uppercase") => Value::Bool(ch(self, 0).is_uppercase()),
            (Ty::Char, "is_lowercase") => Value::Bool(ch(self, 0).is_lowercase()),
            (Ty::Char, "len_utf8") => Value::Int(ch(self, 0).len_utf8() as i64),
            (Ty::Str, "len") => Value::Int(text(self, 0).len() as i64),
            (Ty::Str, "starts_with") => Value::Bool(text(self, 0).starts_with(&text(self, 1))),
            (Ty::Str, "ends_with") => Value::Bool(text(self, 0).ends_with(&text(self, 1))),
            (Ty::Str, "contains") => Value::Bool(text(self, 0).contains(&text(self, 1))),
            (Ty::Str, "trim") => Value::Str(text(self, 0).trim().into()),
            (Ty::Str, "to_uppercase") => Value::String(text(self, 0).to_uppercase()),
            (Ty::Str, "to_lowercase") => Value::String(text(self, 0).to_lowercase()),
            (Ty::Str, "eq") => Value::Bool(text(self, 0) == text(self, 1)),
            (Ty::Str, "cmp") => {
                let o = text(self, 0).cmp(&text(self, 1));
                self.ordering(o)
            }
            (Ty::String, "new") => Value::String(String::new()),
            (Ty::String, "as_str") => Value::Str(text(self, 0).into()),
            (Ty::String, "clone") => Value::String(text(self, 0)),
            (Ty::String, "push_str") => {
                let s = text(self, 1);
                edit(self, &mut |out| {
                    out.push_str(&s);
                    Value::Unit
                })
            }
            (Ty::String, "push") => {
                let c = ch(self, 1);
                edit(self, &mut |out| {
                    out.push(c);
                    Value::Unit
                })
            }
            (Ty::String, "clear") => edit(self, &mut |out| {
                out.clear();
                Value::Unit
            }),
            (Ty::String, "pop") => {
                let c = edit(self, &mut |out| match out.pop() {
                    Some(c) => Value::Char(c),
                    None => Value::Empty,
                });
                self.option((c != Value::Empty).then_some(c))
            }
            (t, n) => unreachable!("no built-in `{n}` for `{t}`"),
        }
    }

    /// A value of the prelude's `Option`.
    fn option(&self, v: Option<Value>) -> Value {
        let adt = self.prelude_adt("Option");
        let name = if v.is_some() { "Some" } else { "None" };
        Value::Adt {
            adt,
            variant: self.variant(adt, name),
            fields: v.into_iter().collect(),
        }
    }

    /// A value of the prelude's `Ordering`.
    fn ordering(&self, o: std::cmp::Ordering) -> Value {
        let adt = self.prelude_adt("Ordering");
        let name = match o {
            std::cmp::Ordering::Less => "Less",
            std::cmp::Ordering::Equal => "Equal",
            std::cmp::Ordering::Greater => "Greater",
        };
        Value::Adt {
            adt,
            variant: self.variant(adt, name),
            fields: Vec::new(),
        }
    }

    fn prelude_adt(&self, name: &str) -> u32 {
        self.items
            .adts
            .iter()
            .position(|a| a.src == Src::Prelude && a.name == name)
            .expect("a prelude type") as u32
    }

    fn variant(&self, adt: u32, name: &str) -> u32 {
        self.items.adts[adt as usize]
            .variants
            .iter()
            .position(|v| v.name == name)
            .expect("a variant") as u32
    }
}

/// This crate's version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests;

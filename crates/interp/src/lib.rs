//! The MIR interpreter: runs analysed or runtime MIR for compile-time evaluation, tests before
//! any backend, the Miri-style checker and the playground.
//!
//! It runs calls on its own stack of frames, not the host's, so deep recursion in a Moddil
//! program is a [`TrapClass::StackOverflow`] trap and never crashes the interpreter. A borrow is
//! a pointer to a frame's local and a path of fields in it; borrows cannot outlive what they
//! point to (the borrow checker sees to that), so the frame is always there. See this crate's
//! `README.md`.

use moddil_mir::{
    AggregateKind, BinOp, Body, Const, Local, Operand, Place, Projection, Rvalue, Statement,
    Terminator, TrapClass, UnOp, mir,
};
use moddil_sema::{Db, Item, ItemTree, Owner, Shape, SourceFile};
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
    /// A borrow as reported outside the program (an observed value, a result): what it pointed
    /// to, and whether it was `&mut`.
    Borrowed(Box<Value>, bool),
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
            Value::Ref(..) => "&…".to_string(),
        }
    }
}

/// A trap: what kind, and the source of the check that failed (or of the call, for a stack
/// overflow).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trap {
    /// What kind of trap.
    pub class: TrapClass,
    /// Where.
    pub range: TextRange,
    /// For a failed `assert`: each part of the condition and its value, or `None` if it was not
    /// evaluated.
    pub parts: Vec<(TextRange, Option<Value>)>,
}

/// How deeply calls may nest before a [`TrapClass::StackOverflow`] trap.
pub const MAX_DEPTH: usize = 10_000;

/// Reports the values of observed expressions (those followed by `// =>`): their range and value.
pub type Observer<'a> = &'a mut dyn FnMut(TextRange, Value);

/// Calls function `index` of `file` with `args`.
///
/// # Panics
///
/// If the file has errors (check `moddil_mir::has_errors` first), or `args` does not match the
/// function's parameters.
///
/// # Errors
///
/// The trap, if the program traps.
pub fn call_fn(
    db: &dyn Db,
    file: SourceFile,
    index: u32,
    args: &[Value],
    observer: Observer<'_>,
) -> Result<Value, Trap> {
    run(db, Owner::new(db, file, Item::Fn(index)), args, observer)
}

/// Runs test `index` of `file`.
///
/// # Panics
///
/// If the file has errors.
///
/// # Errors
///
/// The trap, if the test traps.
pub fn run_test(
    db: &dyn Db,
    file: SourceFile,
    index: u32,
    observer: Observer<'_>,
) -> Result<(), Trap> {
    run(db, Owner::new(db, file, Item::Test(index)), &[], observer).map(|_| ())
}

struct Frame<'a> {
    body: &'a Body,
    locals: Vec<Value>,
    block: usize,
    stmt: usize,
    /// Where the caller wants the result, and where it continues.
    ret: Option<(Pointer, usize)>,
}

impl<'a> Frame<'a> {
    fn new(body: &'a Body, args: Vec<Value>, ret: Option<(Pointer, usize)>) -> Self {
        assert_eq!(args.len(), body.arg_count, "argument count");
        let mut locals = vec![Value::Unit; body.locals.len()];
        for (slot, a) in locals[1..].iter_mut().zip(args) {
            *slot = a;
        }
        Frame {
            body,
            locals,
            block: 0,
            stmt: 0,
            ret,
        }
    }
}

/// The interpreter's state: the stack of frames.
struct Machine<'a> {
    stack: Vec<Frame<'a>>,
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
                    Value::Ref(target, _) => ptr = target,
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
        }
    }

    /// `v` with every borrow replaced by what it points to, for reporting outside the program.
    fn shown(&self, v: Value) -> Value {
        match v {
            Value::Ref(ptr, m) => Value::Borrowed(Box::new(self.shown(self.load(&ptr))), m),
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

    fn rvalue(&self, r: &Rvalue) -> Value {
        match r {
            Rvalue::Use(a) => self.operand(a),
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
                let fields: Vec<Value> = ops.iter().map(|o| self.operand(o)).collect();
                match kind {
                    AggregateKind::Tuple => Value::Tuple(fields),
                    AggregateKind::Adt { adt, variant, .. } => Value::Adt {
                        adt: *adt,
                        variant: *variant,
                        fields,
                    },
                }
            }
            Rvalue::Discriminant(p) => match self.read(p) {
                Value::Adt { variant, .. } => Value::Int(i64::from(variant)),
                other => unreachable!("the discriminant of {other:?}"),
            },
            Rvalue::Ref(m, p) => Value::Ref(self.resolve(p), *m),
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
) -> Result<Value, Trap> {
    let file = owner.file(db);
    let mut m = Machine {
        stack: vec![Frame::new(body(db, owner), args.to_vec(), None)],
    };
    loop {
        let depth = m.stack.len();
        let top = m.top();
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
                        return Err(Trap {
                            class: *trap,
                            range: *range,
                            parts,
                        });
                    }
                }
                Statement::Observe { range, value } => {
                    let v = m.shown(m.operand(value));
                    observer(*range, v);
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
                func,
                args,
                dest,
                target,
                range,
            } => {
                if depth >= MAX_DEPTH {
                    return Err(Trap {
                        class: TrapClass::StackOverflow,
                        range: *range,
                        parts: Vec::new(),
                    });
                }
                let values: Vec<Value> = args.iter().map(|a| m.operand(a)).collect();
                let dest = m.resolve(dest);
                let callee = body(db, Owner::new(db, file, Item::Fn(*func)));
                m.stack
                    .push(Frame::new(callee, values, Some((dest, target.index()))));
            }
            Terminator::Return => {
                let value = m.stack[top].locals[Local::RETURN.index()].clone();
                if m.stack[top].ret.is_none() {
                    // Show borrows while what they point to still exists.
                    return Ok(m.shown(value));
                }
                let done = m.stack.pop().expect("a frame");
                let (dest, target) = done.ret.expect("a caller");
                m.store(&dest, value);
                let caller = m.stack.last_mut().expect("a caller");
                caller.block = target;
                caller.stmt = 0;
            }
            Terminator::Unreachable => unreachable!("MIR reached an unreachable block"),
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
    use Value::{Bool, Int};
    match (op, a, b) {
        (BinOp::Eq, a, b) => Bool(a == b),
        (BinOp::Ne, a, b) => Bool(a != b),
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

/// This crate's version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests;

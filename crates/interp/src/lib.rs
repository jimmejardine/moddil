//! The MIR interpreter: runs analysed or runtime MIR for compile-time evaluation, tests before
//! any backend, the Miri-style checker and the playground.
//!
//! It runs calls on its own stack of frames, not the host's, so deep recursion in a Moddil
//! program is a [`TrapClass::StackOverflow`] trap and never crashes the interpreter. See this
//! crate's `README.md`.

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
}

impl Value {
    /// The value as written in Moddil source and in `// =>` expectations: `3`, `-1`, `true`,
    /// `()`, `(1, true)`, `Point { x: 1, y: 2 }`, `Shape::Circle(3)`.
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
/// If the file has errors (check `moddil_sema::has_errors` first), or `args` does not match the
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
    ret: Option<(Place, usize)>,
}

impl<'a> Frame<'a> {
    fn new(body: &'a Body, args: Vec<Value>, ret: Option<(Place, usize)>) -> Self {
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

    fn read(&self, place: &Place) -> Value {
        let mut v = &self.locals[place.local.index()];
        for p in &place.projection {
            if let Projection::Field(i) = p {
                v = match v {
                    Value::Tuple(xs) | Value::Adt { fields: xs, .. } => &xs[*i as usize],
                    other => unreachable!("a field of {other:?}"),
                };
            }
        }
        v.clone()
    }

    fn write(&mut self, place: &Place, value: Value) {
        let mut v = &mut self.locals[place.local.index()];
        for p in &place.projection {
            if let Projection::Field(i) = p {
                v = match v {
                    Value::Tuple(xs) | Value::Adt { fields: xs, .. } => &mut xs[*i as usize],
                    other => unreachable!("a field of {other:?}"),
                };
            }
        }
        *v = value;
    }

    fn operand(&self, o: &Operand) -> Value {
        match o {
            Operand::Copy(p) => self.read(p),
            Operand::Const(Const::Int(v)) => Value::Int(*v),
            Operand::Const(Const::Bool(v)) => Value::Bool(*v),
            Operand::Const(Const::Unit) => Value::Unit,
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
    let mut stack = vec![Frame::new(body(db, owner), args.to_vec(), None)];
    loop {
        let depth = stack.len();
        let frame = stack.last_mut().expect("a frame");
        let block = &frame.body.blocks[frame.block];
        if let Some(s) = block.statements.get(frame.stmt) {
            frame.stmt += 1;
            match s {
                Statement::Assign(place, r) => {
                    let v = rvalue(frame, r);
                    frame.write(place, v);
                }
                Statement::Assert {
                    cond,
                    expected,
                    trap,
                    range,
                    parts,
                } => {
                    if frame.operand(cond) != Value::Bool(*expected) {
                        let parts = parts
                            .iter()
                            .map(|p| {
                                let evaluated =
                                    frame.locals[p.evaluated.index()] == Value::Bool(true);
                                let value =
                                    evaluated.then(|| frame.locals[p.value.index()].clone());
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
                Statement::Observe { range, value } => observer(*range, frame.operand(value)),
            }
            continue;
        }
        match &block.terminator {
            Terminator::Goto(t) => {
                frame.block = t.index();
                frame.stmt = 0;
            }
            Terminator::SwitchBool { cond, then, else_ } => {
                let to = if frame.operand(cond) == Value::Bool(true) {
                    then
                } else {
                    else_
                };
                frame.block = to.index();
                frame.stmt = 0;
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
                let values: Vec<Value> = args.iter().map(|a| frame.operand(a)).collect();
                let callee = body(db, Owner::new(db, file, Item::Fn(*func)));
                let next = Frame::new(callee, values, Some((dest.clone(), target.index())));
                stack.push(next);
            }
            Terminator::Return => {
                let mut done = stack.pop().expect("a frame");
                let value = std::mem::replace(&mut done.locals[Local::RETURN.index()], Value::Unit);
                let Some((dest, target)) = done.ret else {
                    return Ok(value);
                };
                let caller = stack.last_mut().expect("a caller");
                caller.write(&dest, value);
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

fn rvalue(frame: &Frame<'_>, r: &Rvalue) -> Value {
    match r {
        Rvalue::Use(a) => frame.operand(a),
        Rvalue::Unary(op, a) => match (op, frame.operand(a)) {
            (UnOp::Neg, Value::Int(v)) => Value::Int(v.wrapping_neg()),
            (UnOp::Not, Value::Int(v)) => Value::Int(!v),
            (UnOp::Not, Value::Bool(v)) => Value::Bool(!v),
            (op, v) => unreachable!("{op:?} on {v:?}"),
        },
        Rvalue::NegOverflows(a) => Value::Bool(int(&frame.operand(a)) == i64::MIN),
        Rvalue::Overflows(op, a, b) => {
            let (a, b) = (int(&frame.operand(a)), int(&frame.operand(b)));
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
        Rvalue::Binary(op, a, b) => binary(*op, frame.operand(a), frame.operand(b)),
        Rvalue::Aggregate(kind, ops) => {
            let fields: Vec<Value> = ops.iter().map(|o| frame.operand(o)).collect();
            match kind {
                AggregateKind::Tuple => Value::Tuple(fields),
                AggregateKind::Adt { adt, variant, .. } => Value::Adt {
                    adt: *adt,
                    variant: *variant,
                    fields,
                },
            }
        }
        Rvalue::Discriminant(p) => match frame.read(p) {
            Value::Adt { variant, .. } => Value::Int(i64::from(variant)),
            other => unreachable!("the discriminant of {other:?}"),
        },
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

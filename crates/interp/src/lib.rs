//! The MIR interpreter: runs analysed or runtime MIR for compile-time evaluation, tests before
//! any backend, the Miri-style checker and the playground.
//!
//! It runs calls on its own stack of frames, not the host's, so deep recursion in a Moddil
//! program is a [`TrapClass::StackOverflow`] trap and never crashes the interpreter. See this
//! crate's `README.md`.

use std::fmt;

use moddil_mir::{
    BinOp, Body, Const, Local, Operand, Rvalue, Statement, Terminator, TrapClass, UnOp, mir,
};
use moddil_sema::{Db, Item, Owner, SourceFile};
use moddil_syntax::TextRange;

/// A value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Value {
    /// An `int`.
    Int(i64),
    /// A `bool`.
    Bool(bool),
    /// `()`.
    Unit,
}

/// As written in Moddil source and in `// =>` expectations: `3`, `-1`, `true`, `()`.
impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Int(v) => write!(f, "{v}"),
            Value::Bool(v) => write!(f, "{v}"),
            Value::Unit => write!(f, "()"),
        }
    }
}

/// A trap: what kind, and the source of the check that failed (or of the call, for a stack
/// overflow).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Trap {
    /// What kind of trap.
    pub class: TrapClass,
    /// Where.
    pub range: TextRange,
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
    ret: Option<(Local, usize)>,
}

impl<'a> Frame<'a> {
    fn new(body: &'a Body, args: &[Value], ret: Option<(Local, usize)>) -> Self {
        assert_eq!(args.len(), body.arg_count, "argument count");
        let mut locals = vec![Value::Unit; body.locals.len()];
        locals[1..=args.len()].copy_from_slice(args);
        Frame {
            body,
            locals,
            block: 0,
            stmt: 0,
            ret,
        }
    }

    fn operand(&self, o: Operand) -> Value {
        match o {
            Operand::Copy(l) => self.locals[l.index()],
            Operand::Const(Const::Int(v)) => Value::Int(v),
            Operand::Const(Const::Bool(v)) => Value::Bool(v),
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
    let mut stack = vec![Frame::new(body(db, owner), args, None)];
    loop {
        let depth = stack.len();
        let frame = stack.last_mut().expect("a frame");
        let block = &frame.body.blocks[frame.block];
        if let Some(s) = block.statements.get(frame.stmt) {
            frame.stmt += 1;
            match s {
                Statement::Assign(l, r) => {
                    let v = rvalue(frame, r);
                    frame.locals[l.index()] = v;
                }
                Statement::Assert {
                    cond,
                    expected,
                    trap,
                    range,
                } => {
                    if frame.operand(*cond) != Value::Bool(*expected) {
                        return Err(Trap {
                            class: *trap,
                            range: *range,
                        });
                    }
                }
                Statement::Observe { range, value } => observer(*range, frame.operand(*value)),
            }
            continue;
        }
        match &block.terminator {
            Terminator::Goto(t) => {
                frame.block = t.index();
                frame.stmt = 0;
            }
            Terminator::SwitchBool { cond, then, else_ } => {
                let to = if frame.operand(*cond) == Value::Bool(true) {
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
                    });
                }
                let values: Vec<Value> = args.iter().map(|a| frame.operand(*a)).collect();
                let callee = body(db, Owner::new(db, file, Item::Fn(*func)));
                let next = Frame::new(callee, &values, Some((*dest, target.index())));
                stack.push(next);
            }
            Terminator::Return => {
                let done = stack.pop().expect("a frame");
                let value = done.locals[Local::RETURN.index()];
                let Some((dest, target)) = done.ret else {
                    return Ok(value);
                };
                let caller = stack.last_mut().expect("a caller");
                caller.locals[dest.index()] = value;
                caller.block = target;
                caller.stmt = 0;
            }
            Terminator::Unreachable => unreachable!("MIR reached an unreachable block"),
        }
    }
}

fn int(v: Value) -> i64 {
    match v {
        Value::Int(i) => i,
        other => unreachable!("expected an int, found {other}"),
    }
}

fn rvalue(frame: &Frame<'_>, r: &Rvalue) -> Value {
    match *r {
        Rvalue::Use(a) => frame.operand(a),
        Rvalue::Unary(op, a) => match (op, frame.operand(a)) {
            (UnOp::Neg, Value::Int(v)) => Value::Int(v.wrapping_neg()),
            (UnOp::Not, Value::Int(v)) => Value::Int(!v),
            (UnOp::Not, Value::Bool(v)) => Value::Bool(!v),
            (op, v) => unreachable!("{op:?} on {v}"),
        },
        Rvalue::NegOverflows(a) => Value::Bool(int(frame.operand(a)) == i64::MIN),
        Rvalue::Overflows(op, a, b) => {
            let (a, b) = (int(frame.operand(a)), int(frame.operand(b)));
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
        Rvalue::Binary(op, a, b) => binary(op, frame.operand(a), frame.operand(b)),
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
        (op, a, b) => unreachable!("{op:?} on {a} and {b}"),
    }
}

/// This crate's version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests;

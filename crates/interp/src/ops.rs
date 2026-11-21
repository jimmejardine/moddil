//! The operators on values: pure, with no access to memory.

use moddil_mir::{BinOp, UnOp};

use crate::value::Value;

/// `op` applied to `v`. Negation wraps; the MIR checks for overflow first.
pub(crate) fn unary(op: UnOp, v: Value) -> Value {
    match (op, v) {
        (UnOp::Neg, Value::Int(v)) => Value::Int(v.wrapping_neg()),
        (UnOp::Not, Value::Int(v)) => Value::Int(!v),
        (UnOp::Not, Value::Bool(v)) => Value::Bool(!v),
        (op, v) => unreachable!("{op:?} on {v:?}"),
    }
}

/// Whether negating `a` overflows.
pub(crate) fn neg_overflows(a: &Value) -> bool {
    a.as_int() == i64::MIN
}

/// Whether `a op b` overflows, or shifts by an amount out of range.
pub(crate) fn overflows(op: BinOp, a: &Value, b: &Value) -> bool {
    let (a, b) = (a.as_int(), b.as_int());
    match op {
        BinOp::Add => a.checked_add(b).is_none(),
        BinOp::Sub => a.checked_sub(b).is_none(),
        BinOp::Mul => a.checked_mul(b).is_none(),
        BinOp::Div => a.checked_div(b).is_none(),
        BinOp::Shl => !(0..64).contains(&b) || (a << b) >> b != a,
        BinOp::Shr => !(0..64).contains(&b),
        _ => false,
    }
}

/// `a op b`. Arithmetic wraps; the MIR checks for overflow first.
pub(crate) fn binary(op: BinOp, a: Value, b: Value) -> Value {
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

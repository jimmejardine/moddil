//! Arithmetic and comparisons on values, with the traps they can raise.

use moddil_syntax::{SyntaxKind as K, TextRange};

use crate::value::Value;
use crate::{Eval, TrapClass, trap};

pub(super) type Comparison = fn(&Value, &Value) -> bool;

/// Comparisons of `int`s, `bool`s (`false` before `true`) and `()`.
pub(super) fn compare(k: K) -> Option<Comparison> {
    fn ints(a: &Value, b: &Value) -> (i64, i64) {
        match (a, b) {
            (Value::Int(a), Value::Int(b)) => (*a, *b),
            (Value::Bool(a), Value::Bool(b)) => (i64::from(*a), i64::from(*b)),
            _ => unreachable!("ordering compares integers or booleans"),
        }
    }
    Some(match k {
        K::EQ_EQ => |a, b| a == b,
        K::BANG_EQ => |a, b| a != b,
        K::LT => |a, b| {
            let (a, b) = ints(a, b);
            a < b
        },
        K::LT_EQ => |a, b| {
            let (a, b) = ints(a, b);
            a <= b
        },
        K::GT => |a, b| {
            let (a, b) = ints(a, b);
            a > b
        },
        K::GT_EQ => |a, b| {
            let (a, b) = ints(a, b);
            a >= b
        },
        _ => return None,
    })
}

/// The operator of a compound assignment: `+` for `+=`.
pub(super) fn compound(k: K) -> K {
    match k {
        K::PLUS_EQ => K::PLUS,
        K::MINUS_EQ => K::MINUS,
        K::STAR_EQ => K::STAR,
        K::SLASH_EQ => K::SLASH,
        K::PERCENT_EQ => K::PERCENT,
        K::AMP_EQ => K::AMP,
        K::PIPE_EQ => K::PIPE,
        K::CARET_EQ => K::CARET,
        K::LT_LT_EQ => K::LT_LT,
        K::GT_GT_EQ => K::GT_GT,
        K::PLUS_PERCENT_EQ => K::PLUS_PERCENT,
        K::MINUS_PERCENT_EQ => K::MINUS_PERCENT,
        K::STAR_PERCENT_EQ => K::STAR_PERCENT,
        _ => unreachable!("a compound assignment"),
    }
}

/// `a op b` for the arithmetic and bitwise operators, trapping as the reference says
/// (`expr.arith.overflow.1`, `expr.arith.div.1`, `expr.bit.shift.1`).
pub(super) fn arith(op: K, a: Value, b: Value, range: TextRange) -> Eval {
    let overflow = || trap(TrapClass::Overflow, range);
    match (a, b) {
        (Value::Int(a), Value::Int(b)) => {
            let v = match op {
                K::PLUS => a.checked_add(b).ok_or_else(overflow)?,
                K::MINUS => a.checked_sub(b).ok_or_else(overflow)?,
                K::STAR => a.checked_mul(b).ok_or_else(overflow)?,
                K::SLASH | K::PERCENT if b == 0 => {
                    return Err(trap(TrapClass::DivisionByZero, range));
                }
                K::SLASH => a.checked_div(b).ok_or_else(overflow)?,
                // `int::MIN % -1` is 0: the exact answer fits.
                K::PERCENT => a.wrapping_rem(b),
                K::PLUS_PERCENT => a.wrapping_add(b),
                K::MINUS_PERCENT => a.wrapping_sub(b),
                K::STAR_PERCENT => a.wrapping_mul(b),
                K::AMP => a & b,
                K::PIPE => a | b,
                K::CARET => a ^ b,
                K::LT_LT => {
                    if !(0..64).contains(&b) || (a << b) >> b != a {
                        return Err(overflow());
                    }
                    a << b
                }
                K::GT_GT => {
                    if !(0..64).contains(&b) {
                        return Err(overflow());
                    }
                    a >> b
                }
                _ => unreachable!("an arithmetic operator"),
            };
            Ok(Value::Int(v))
        }
        (Value::Bool(a), Value::Bool(b)) => Ok(Value::Bool(match op {
            K::AMP => a & b,
            K::PIPE => a | b,
            K::CARET => a ^ b,
            _ => unreachable!("a checked operator on `bool`"),
        })),
        _ => unreachable!("checked operands"),
    }
}

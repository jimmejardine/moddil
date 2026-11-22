//! The sized integers and the floats (`num.types.1`): a value is its type and its bits, and the
//! operators work at the type's width. Integers wrap here; the MIR checks for overflow first.
//! Floats follow IEEE 754, and their functions come from the `libm` crate, so every platform
//! gives the same bits.

use moddil_mir::{BinOp, UnOp};
use moddil_sema::Num;

use crate::value::Value;

/// The bits that hold a value of `n`.
fn mask(n: Num) -> u128 {
    if n.bits() == 128 {
        u128::MAX
    } else {
        (1u128 << n.bits()) - 1
    }
}

/// An integer of type `n` with bits `bits`, as an `i128` (exact but for `u128` past `i128::MAX`).
pub(crate) fn signed(n: Num, bits: u128) -> i128 {
    if n.is_signed() {
        let shift = 128 - n.bits();
        ((bits as i128) << shift) >> shift
    } else {
        bits as i128
    }
}

/// The value of type `n` for the integer `v`, wrapped to its width.
pub(crate) fn int(n: Num, v: i128) -> Value {
    Value::Num(n, (v as u128) & mask(n))
}

/// The `f64` a float value holds (an `f32` widened exactly).
pub(crate) fn float(n: Num, bits: u128) -> f64 {
    match n {
        Num::F32 => f64::from(f32::from_bits(bits as u32)),
        _ => f64::from_bits(bits as u64),
    }
}

/// The value of float type `n` for `v`, rounded to `f32` if it is one.
pub(crate) fn from_float(n: Num, v: f64) -> Value {
    match n {
        Num::F32 => Value::Num(n, u128::from((v as f32).to_bits())),
        _ => Value::Num(n, u128::from(v.to_bits())),
    }
}

/// `op` on an `f32`, computed in `f32` (not in `f64` and rounded: the result is the same for
/// `+ - * /`, but this says what is meant).
fn f32_op(op: BinOp, a: f32, b: f32) -> Value {
    let r = match op {
        BinOp::Add => a + b,
        BinOp::Sub => a - b,
        BinOp::Mul => a * b,
        BinOp::Div => a / b,
        BinOp::Rem => a % b,
        BinOp::Lt => return Value::Bool(a < b),
        BinOp::Le => return Value::Bool(a <= b),
        BinOp::Gt => return Value::Bool(a > b),
        BinOp::Ge => return Value::Bool(a >= b),
        BinOp::Eq => return Value::Bool(a == b),
        BinOp::Ne => return Value::Bool(a != b),
        op => unreachable!("{op:?} on floats"),
    };
    Value::Num(Num::F32, u128::from(r.to_bits()))
}

fn f64_op(op: BinOp, a: f64, b: f64) -> Value {
    let r = match op {
        BinOp::Add => a + b,
        BinOp::Sub => a - b,
        BinOp::Mul => a * b,
        BinOp::Div => a / b,
        BinOp::Rem => a % b,
        BinOp::Lt => return Value::Bool(a < b),
        BinOp::Le => return Value::Bool(a <= b),
        BinOp::Gt => return Value::Bool(a > b),
        BinOp::Ge => return Value::Bool(a >= b),
        BinOp::Eq => return Value::Bool(a == b),
        BinOp::Ne => return Value::Bool(a != b),
        op => unreachable!("{op:?} on floats"),
    };
    Value::Num(Num::F64, u128::from(r.to_bits()))
}

/// `a op b` on two values of type `n`.
pub(crate) fn binary(op: BinOp, n: Num, a: u128, b: u128) -> Value {
    match n {
        Num::F32 => return f32_op(op, f32::from_bits(a as u32), f32::from_bits(b as u32)),
        Num::F64 => return f64_op(op, f64::from_bits(a as u64), f64::from_bits(b as u64)),
        _ => {}
    }
    // Comparisons, by the type's order.
    let cmp = if n.is_signed() {
        signed(n, a).cmp(&signed(n, b))
    } else {
        a.cmp(&b)
    };
    use std::cmp::Ordering::{Equal, Greater, Less};
    match op {
        BinOp::Eq => return Value::Bool(cmp == Equal),
        BinOp::Ne => return Value::Bool(cmp != Equal),
        BinOp::Lt => return Value::Bool(cmp == Less),
        BinOp::Le => return Value::Bool(cmp != Greater),
        BinOp::Gt => return Value::Bool(cmp == Greater),
        BinOp::Ge => return Value::Bool(cmp != Less),
        _ => {}
    }
    let m = mask(n);
    let r = if n.is_signed() {
        let (x, y) = (signed(n, a), signed(n, b));
        (match op {
            BinOp::Add => x.wrapping_add(y),
            BinOp::Sub => x.wrapping_sub(y),
            BinOp::Mul => x.wrapping_mul(y),
            BinOp::Div => x.wrapping_div(y),
            BinOp::Rem => x.wrapping_rem(y),
            BinOp::BitAnd => x & y,
            BinOp::BitOr => x | y,
            BinOp::BitXor => x ^ y,
            BinOp::Shl => x.wrapping_shl(y as u32),
            BinOp::Shr => x.wrapping_shr(y as u32),
            op => unreachable!("{op:?} on integers"),
        }) as u128
    } else {
        match op {
            BinOp::Add => a.wrapping_add(b),
            BinOp::Sub => a.wrapping_sub(b),
            BinOp::Mul => a.wrapping_mul(b),
            BinOp::Div => a / b,
            BinOp::Rem => a % b,
            BinOp::BitAnd => a & b,
            BinOp::BitOr => a | b,
            BinOp::BitXor => a ^ b,
            BinOp::Shl => a.wrapping_shl(b as u32),
            BinOp::Shr => a.wrapping_shr(b as u32),
            op => unreachable!("{op:?} on integers"),
        }
    };
    Value::Num(n, r & m)
}

/// Whether `a op b` overflows type `n`, or shifts by an amount out of its width.
pub(crate) fn overflows(op: BinOp, n: Num, a: u128, b: u128) -> bool {
    if n.is_float() {
        return false;
    }
    let width = n.bits();
    match op {
        BinOp::Shl | BinOp::Shr => {
            let shift = if n.is_signed() {
                signed(n, b)
            } else {
                b as i128
            };
            if !(0..i128::from(width)).contains(&shift) {
                return true;
            }
            if op == BinOp::Shr {
                return false;
            }
            // `<<` overflows when bits (or the sign) are lost.
            let back = binary(
                BinOp::Shr,
                n,
                match binary(BinOp::Shl, n, a, b) {
                    Value::Num(_, r) => r,
                    _ => unreachable!("an integer"),
                },
                b,
            );
            back != Value::Num(n, a)
        }
        _ if n == Num::U128 => match op {
            BinOp::Add => a.checked_add(b).is_none(),
            BinOp::Sub => a.checked_sub(b).is_none(),
            BinOp::Mul => a.checked_mul(b).is_none(),
            _ => false,
        },
        _ if n == Num::I128 => {
            let (x, y) = (a as i128, b as i128);
            match op {
                BinOp::Add => x.checked_add(y).is_none(),
                BinOp::Sub => x.checked_sub(y).is_none(),
                BinOp::Mul => x.checked_mul(y).is_none(),
                BinOp::Div => x.checked_div(y).is_none(),
                _ => false,
            }
        }
        _ => {
            // At most 64 bits wide: exact in an `i128`, then checked against the range.
            let (x, y) = (signed(n, a), signed(n, b));
            let r = match op {
                BinOp::Add => x + y,
                BinOp::Sub => x - y,
                BinOp::Mul => x * y,
                BinOp::Div if y != 0 => x / y,
                _ => return false,
            };
            let (lo, hi) = n.int_range();
            r < lo || r > hi
        }
    }
}

/// `op` on a value of type `n`.
pub(crate) fn unary(op: UnOp, n: Num, a: u128) -> Value {
    match (op, n) {
        (UnOp::Neg, Num::F32) => Value::Num(n, u128::from((-f32::from_bits(a as u32)).to_bits())),
        (UnOp::Neg, Num::F64) => Value::Num(n, u128::from((-f64::from_bits(a as u64)).to_bits())),
        (UnOp::Neg, _) => int(n, signed(n, a).wrapping_neg()),
        (UnOp::Not, _) => Value::Num(n, !a & mask(n)),
    }
}

/// Whether negating `a` of type `n` overflows: it is the least value of a signed type.
pub(crate) fn neg_overflows(n: Num, a: u128) -> bool {
    n.is_signed() && signed(n, a) == n.int_range().0
}

/// How a value of type `n` is written: `3`, `-1`, `1.5`, `inf`, `NaN`.
pub(crate) fn text(n: Num, bits: u128) -> String {
    match n {
        Num::F32 => format!("{:?}", f32::from_bits(bits as u32)),
        Num::F64 => format!("{:?}", f64::from_bits(bits as u64)),
        n if n.is_signed() => signed(n, bits).to_string(),
        _ => bits.to_string(),
    }
}

/// A float function of the prelude (`num.float.fn.1`), on `x` and maybe `y` of float type `n`.
pub(crate) fn float_fn(n: Num, name: &str, x: f64, y: f64) -> Option<Value> {
    let f32 = n == Num::F32;
    let one = |a: fn(f64) -> f64, b: fn(f32) -> f32| {
        if f32 {
            from_float(n, f64::from(b(x as f32)))
        } else {
            from_float(n, a(x))
        }
    };
    let two = |a: fn(f64, f64) -> f64, b: fn(f32, f32) -> f32| {
        if f32 {
            from_float(n, f64::from(b(x as f32, y as f32)))
        } else {
            from_float(n, a(x, y))
        }
    };
    Some(match name {
        "sqrt" => one(libm::sqrt, libm::sqrtf),
        "sin" => one(libm::sin, libm::sinf),
        "cos" => one(libm::cos, libm::cosf),
        "tan" => one(libm::tan, libm::tanf),
        "asin" => one(libm::asin, libm::asinf),
        "acos" => one(libm::acos, libm::acosf),
        "atan" => one(libm::atan, libm::atanf),
        "exp" => one(libm::exp, libm::expf),
        "ln" => one(libm::log, libm::logf),
        "log2" => one(libm::log2, libm::log2f),
        "log10" => one(libm::log10, libm::log10f),
        "abs" => one(libm::fabs, libm::fabsf),
        "floor" => one(libm::floor, libm::floorf),
        "ceil" => one(libm::ceil, libm::ceilf),
        "trunc" => one(libm::trunc, libm::truncf),
        "atan2" => two(libm::atan2, libm::atan2f),
        "powf" => two(libm::pow, libm::powf),
        "min" => two(libm::fmin, libm::fminf),
        "max" => two(libm::fmax, libm::fmaxf),
        "is_nan" => Value::Bool(x.is_nan()),
        "is_finite" => Value::Bool(x.is_finite()),
        _ => return None,
    })
}

/// `x.checked_to::<T>()` (as `Some` or `None`), `truncate`, `wrap` and `saturate`
/// (`num.convert.1`), from the number `x` to type `to`: [`moddil_mir::convert`], which native
/// code uses too.
pub(crate) fn convert(how: &str, x: &Value, to: &moddil_sema::Ty) -> Option<Value> {
    use moddil_sema::Ty;
    let (from, bits) = match x {
        Value::Int(i) => (None, *i as u64 as u128),
        Value::Num(n, b) => (Some(*n), *b),
        _ => return None,
    };
    let to_num = match to {
        Ty::Int => None,
        Ty::Num(n) => Some(*n),
        _ => return None,
    };
    let out = moddil_mir::convert(how, from, bits, to_num)?;
    Some(match to_num {
        None => Value::Int(out as u64 as i64),
        Some(n) => Value::Num(n, out),
    })
}

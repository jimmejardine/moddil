//! The sized integers and the floats (`num.types.1`), as the reference describes them: a value
//! is its type and its bits. Integer arithmetic is computed exactly and then checked against the
//! type's range (rather than wrapped and checked, as the compiler does); floats follow IEEE 754,
//! with their functions from the `libm` crate.
//!
//! The reference interpreter infers no types, so a literal's type comes from what it meets
//! (`num.lit.1`): an integer literal is an `int` and a fractional one an exact text until it is
//! stored where a type is written, or meets a number of a sized type in an operator, which a
//! checked program allows only if the literal has that type.

use std::cmp::Ordering;

use moddil_syntax::TextRange;

use crate::value::Value;
use crate::{Eval, TrapClass, trap};

/// A sized integer or float type (`int` is [`Value::Int`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Kind {
    I8,
    I16,
    I32,
    I128,
    U8,
    U16,
    U32,
    U64,
    U128,
    F32,
    F64,
}

impl Kind {
    /// The type a name stands for (`i64` is `int`, so not here).
    pub(crate) fn named(name: &str) -> Option<Kind> {
        Some(match name {
            "i8" => Kind::I8,
            "i16" => Kind::I16,
            "i32" => Kind::I32,
            "i128" => Kind::I128,
            "u8" => Kind::U8,
            "u16" => Kind::U16,
            "u32" => Kind::U32,
            "u64" => Kind::U64,
            "u128" => Kind::U128,
            "f32" => Kind::F32,
            "f64" => Kind::F64,
            _ => return None,
        })
    }

    pub(crate) fn is_float(self) -> bool {
        matches!(self, Kind::F32 | Kind::F64)
    }

    fn signed(self) -> bool {
        matches!(self, Kind::I8 | Kind::I16 | Kind::I32 | Kind::I128)
    }

    fn width(self) -> u32 {
        match self {
            Kind::I8 | Kind::U8 => 8,
            Kind::I16 | Kind::U16 => 16,
            Kind::I32 | Kind::U32 | Kind::F32 => 32,
            Kind::U64 | Kind::F64 => 64,
            Kind::I128 | Kind::U128 => 128,
        }
    }

    /// The least and greatest values of an integer type, as far as `i128` reaches (`u128`'s
    /// greatest is checked apart).
    fn range(self) -> (i128, i128) {
        let w = self.width();
        if self.signed() {
            if w == 128 {
                (i128::MIN, i128::MAX)
            } else {
                (-(1i128 << (w - 1)), (1i128 << (w - 1)) - 1)
            }
        } else if w >= 127 {
            (0, i128::MAX)
        } else {
            (0, (1i128 << w) - 1)
        }
    }
}

/// A number, exactly: an integer (with `u128`'s upper half apart), or a float's value.
#[derive(Clone, Copy, Debug)]
enum Exact {
    Int(i128),
    Huge(u128),
    Float(f64),
}

fn mask(k: Kind) -> u128 {
    if k.width() == 128 {
        u128::MAX
    } else {
        (1u128 << k.width()) - 1
    }
}

/// The number a value of kind `k` with bits `bits` is.
fn exact(k: Kind, bits: u128) -> Exact {
    match k {
        Kind::F32 => Exact::Float(f64::from(f32::from_bits(bits as u32))),
        Kind::F64 => Exact::Float(f64::from_bits(bits as u64)),
        Kind::U128 if bits > i128::MAX as u128 => Exact::Huge(bits),
        k if k.signed() => {
            let sign = 1u128 << (k.width() - 1);
            let v = if bits & sign != 0 {
                (bits | !mask(k)) as i128
            } else {
                bits as i128
            };
            Exact::Int(v)
        }
        _ => Exact::Int(bits as i128),
    }
}

/// The value of integer kind `k` for `v`, if `v` is in its range.
fn int_value(k: Kind, v: i128) -> Option<Value> {
    let (lo, hi) = k.range();
    (lo <= v && v <= hi).then(|| Value::Num(k, (v as u128) & mask(k)))
}

/// The value of float kind `k` nearest `v`.
fn float_value(k: Kind, v: f64) -> Value {
    match k {
        Kind::F32 => Value::Num(k, u128::from((v as f32).to_bits())),
        _ => Value::Num(k, u128::from(v.to_bits())),
    }
}

/// A fractional literal's text as a value of float kind `k`, rounded once (`num.lit.1`).
pub(crate) fn parse_float(k: Kind, text: &str) -> Value {
    match k {
        Kind::F32 => Value::Num(
            k,
            u128::from(text.parse::<f32>().unwrap_or(f32::NAN).to_bits()),
        ),
        _ => Value::Num(
            Kind::F64,
            u128::from(text.parse::<f64>().unwrap_or(f64::NAN).to_bits()),
        ),
    }
}

/// `v` as a value of kind `k`, if it is a literal not yet of a type (an `int` or a fractional
/// literal's text) or already one; anything else is as it is.
pub(crate) fn as_kind(v: Value, k: Kind) -> Value {
    match v {
        Value::BigLit(i) if k.is_float() => float_value(k, i as f64),
        Value::BigLit(i) => int_value(k, i).unwrap_or(Value::BigLit(i)),
        Value::HugeLit(u) if k.is_float() => float_value(k, u as f64),
        Value::HugeLit(u) if k == Kind::U128 => Value::Num(k, u),
        Value::Int(i) if k.is_float() => float_value(k, i as f64),
        Value::Int(i) => int_value(k, i128::from(i)).unwrap_or(Value::Int(i)),
        Value::FloatLit(t) => parse_float(k, &t),
        v => v,
    }
}

/// A literal's value with the type nothing else gave it: a fractional literal is an `f64`.
pub(crate) fn settle(v: Value) -> Value {
    match v {
        Value::FloatLit(t) => parse_float(Kind::F64, &t),
        Value::BigLit(i) => int_value(Kind::I128, i).expect("an `i128`"),
        Value::HugeLit(u) => Value::Num(Kind::U128, u),
        v => v,
    }
}

/// `v` stored where type `ty` is written: a literal takes the type if it is a number type
/// (`num.lit.1`).
pub(crate) fn typed(v: Value, ty: Option<&moddil_syntax::ast::Type>) -> Value {
    use moddil_syntax::ast::AstNode;
    let Some(t) = ty else {
        return v;
    };
    // An array of numbers: each element (`[0; 23]` as a `[u8; 23]`).
    if let moddil_syntax::ast::Type::Array(_) = t
        && let Value::Array(xs) = &v
        && let Some(elem) = t
            .syntax()
            .children()
            .find_map(moddil_syntax::ast::Type::cast)
    {
        return Value::Array(xs.iter().cloned().map(|x| typed(x, Some(&elem))).collect());
    }
    let text = t.syntax().text().to_string();
    let text = text.trim();
    match (Kind::named(text), &v) {
        // A number of a narrower type widens (`num.widen.1`): exactly.
        (Some(k), Value::Num(j, _)) if *j != k => convert("checked_to", &v, Some(k)).unwrap_or(v),
        (Some(k), _) => as_kind(v, k),
        (None, Value::Num(..)) if text == "int" || text == "i64" => {
            convert("checked_to", &v, None).unwrap_or(v)
        }
        (None, _) => v,
    }
}

/// The two operands of a binary operator with one kind: a literal takes its partner's.
fn agree(a: Value, b: Value) -> (Value, Value) {
    match (&a, &b) {
        (Value::Num(k, _), _) => {
            let k = *k;
            (a, as_kind(b, k))
        }
        (_, Value::Num(k, _)) => {
            let k = *k;
            (as_kind(a, k), b)
        }
        // A fractional literal makes an integer literal a float too.
        (Value::FloatLit(_), _) | (_, Value::FloatLit(_)) => {
            (as_kind(a, Kind::F64), as_kind(b, Kind::F64))
        }
        _ => (a, b),
    }
}

/// Whether `a op b` involves a sized integer or a float.
pub(crate) fn is_num(a: &Value, b: &Value) -> bool {
    let num = |v: &Value| {
        matches!(
            v,
            Value::Num(..) | Value::FloatLit(_) | Value::BigLit(_) | Value::HugeLit(_)
        )
    };
    num(a) || num(b)
}

/// `a op b` (`+ - * / %`, the bitwise operators, the shifts, and the wrapping `+% -% *%`) on
/// numbers of one kind, trapping as the reference says.
pub(crate) fn arith(op: &str, a: Value, b: Value, range: TextRange) -> Eval {
    let (a, b) = agree(a, b);
    let (Value::Num(k, x), Value::Num(_, y)) = (a, b) else {
        unreachable!("two numbers of one type")
    };
    let overflow = || trap(TrapClass::Overflow, range);
    if k.is_float() {
        let (Exact::Float(x), Exact::Float(y)) = (exact(k, x), exact(k, y)) else {
            unreachable!("floats")
        };
        // In the type's own precision.
        let r = if k == Kind::F32 {
            let (x, y) = (x as f32, y as f32);
            f64::from(match op {
                "+" => x + y,
                "-" => x - y,
                "*" => x * y,
                "/" => x / y,
                "%" => x % y,
                _ => unreachable!("a float operator"),
            })
        } else {
            match op {
                "+" => x + y,
                "-" => x - y,
                "*" => x * y,
                "/" => x / y,
                "%" => x % y,
                _ => unreachable!("a float operator"),
            }
        };
        return Ok(float_value(k, r));
    }
    // Shifts: by an amount below the width; `<<` must keep every bit (and the sign).
    if op == "<<" || op == ">>" {
        let s = match exact(k, y) {
            Exact::Int(s) => s,
            Exact::Huge(_) => return Err(overflow()),
            Exact::Float(_) => unreachable!("an integer"),
        };
        if !(0..i128::from(k.width())).contains(&s) {
            return Err(overflow());
        }
        let s = s as u32;
        return match (op, exact(k, x)) {
            (">>", Exact::Int(v)) => Ok(Value::Num(k, ((v >> s) as u128) & mask(k))),
            (">>", Exact::Huge(v)) => Ok(Value::Num(k, v >> s)),
            _ => {
                let r = (x << s) & mask(k);
                let back = match exact(k, r) {
                    Exact::Int(v) => (v >> s) as u128 & mask(k),
                    Exact::Huge(v) => v >> s,
                    Exact::Float(_) => unreachable!("an integer"),
                };
                if back != x {
                    return Err(overflow());
                }
                Ok(Value::Num(k, r))
            }
        };
    }
    match op {
        "&" => return Ok(Value::Num(k, x & y)),
        "|" => return Ok(Value::Num(k, x | y)),
        "^" => return Ok(Value::Num(k, x ^ y)),
        "+%" => return Ok(Value::Num(k, x.wrapping_add(y) & mask(k))),
        "-%" => return Ok(Value::Num(k, x.wrapping_sub(y) & mask(k))),
        "*%" => return Ok(Value::Num(k, x.wrapping_mul(y) & mask(k))),
        _ => {}
    }
    if (op == "/" || op == "%") && y == 0 {
        return Err(trap(TrapClass::DivisionByZero, range));
    }
    // `u128` past `i128::MAX`: its own checked arithmetic.
    if k == Kind::U128 {
        let r = match op {
            "+" => x.checked_add(y),
            "-" => x.checked_sub(y),
            "*" => x.checked_mul(y),
            "/" => Some(x / y),
            "%" => Some(x % y),
            _ => unreachable!("an arithmetic operator"),
        };
        return r.map(|r| Value::Num(k, r)).ok_or_else(overflow);
    }
    let (Exact::Int(x), Exact::Int(y)) = (exact(k, x), exact(k, y)) else {
        unreachable!("integers")
    };
    let r = match op {
        "+" => x.checked_add(y),
        "-" => x.checked_sub(y),
        "*" => x.checked_mul(y),
        "/" => x.checked_div(y),
        "%" => Some(x.wrapping_rem(y)),
        _ => unreachable!("an arithmetic operator"),
    };
    r.and_then(|r| int_value(k, r)).ok_or_else(overflow)
}

/// `-a` or `!a` on a number of a sized type.
pub(crate) fn unary(neg: bool, a: Value, range: TextRange) -> Eval {
    let a = settle(a);
    let Value::Num(k, x) = a else {
        unreachable!("a number")
    };
    if !neg {
        return Ok(Value::Num(k, !x & mask(k)));
    }
    match exact(k, x) {
        Exact::Float(f) => Ok(float_value(k, -f)),
        Exact::Int(v) if k.signed() => {
            int_value(k, -v).ok_or_else(|| trap(TrapClass::Overflow, range))
        }
        _ => unreachable!("negation of a signed integer or a float"),
    }
}

/// How `a` and `b` (numbers, one of a sized type or a literal) compare; `None` if a float is
/// NaN.
pub(crate) fn compare(a: &Value, b: &Value) -> Option<Ordering> {
    let (a, b) = agree(a.clone(), b.clone());
    let (Value::Num(k, x), Value::Num(_, y)) = (a, b) else {
        unreachable!("two numbers of one type")
    };
    match (exact(k, x), exact(k, y)) {
        (Exact::Float(x), Exact::Float(y)) => x.partial_cmp(&y),
        (Exact::Huge(x), Exact::Huge(y)) => Some(x.cmp(&y)),
        (Exact::Huge(_), _) => Some(Ordering::Greater),
        (_, Exact::Huge(_)) => Some(Ordering::Less),
        (Exact::Int(x), Exact::Int(y)) => Some(x.cmp(&y)),
        _ => unreachable!("one kind"),
    }
}

/// How a number of kind `k` is written: `3`, `-1`, `1.5`, `inf`, `NaN`.
pub(crate) fn text(k: Kind, bits: u128) -> String {
    match (k, exact(k, bits)) {
        (Kind::F32, _) => format!("{:?}", f32::from_bits(bits as u32)),
        (_, Exact::Float(f)) => format!("{f:?}"),
        (_, Exact::Int(v)) => v.to_string(),
        (_, Exact::Huge(v)) => v.to_string(),
    }
}

/// The float function `name` (`num.float.fn.1`) of `x` (and `y`) of kind `k`.
pub(crate) fn float_fn(k: Kind, name: &str, x: &Value, y: Option<&Value>) -> Option<Value> {
    let f = |v: &Value| match as_kind(v.clone(), k) {
        Value::Num(k, b) => match exact(k, b) {
            Exact::Float(f) => f,
            _ => 0.0,
        },
        _ => 0.0,
    };
    let (a, b) = (f(x), y.map_or(0.0, f));
    let single = k == Kind::F32;
    macro_rules! one {
        ($d:path, $s:path) => {
            if single {
                f64::from($s(a as f32))
            } else {
                $d(a)
            }
        };
    }
    macro_rules! two {
        ($d:path, $s:path) => {
            if single {
                f64::from($s(a as f32, b as f32))
            } else {
                $d(a, b)
            }
        };
    }
    let r = match name {
        "sqrt" => one!(libm::sqrt, libm::sqrtf),
        "sin" => one!(libm::sin, libm::sinf),
        "cos" => one!(libm::cos, libm::cosf),
        "tan" => one!(libm::tan, libm::tanf),
        "asin" => one!(libm::asin, libm::asinf),
        "acos" => one!(libm::acos, libm::acosf),
        "atan" => one!(libm::atan, libm::atanf),
        "exp" => one!(libm::exp, libm::expf),
        "ln" => one!(libm::log, libm::logf),
        "log2" => one!(libm::log2, libm::log2f),
        "log10" => one!(libm::log10, libm::log10f),
        "abs" => one!(libm::fabs, libm::fabsf),
        "floor" => one!(libm::floor, libm::floorf),
        "ceil" => one!(libm::ceil, libm::ceilf),
        "trunc" => one!(libm::trunc, libm::truncf),
        "atan2" => two!(libm::atan2, libm::atan2f),
        "powf" => two!(libm::pow, libm::powf),
        "min" => two!(libm::fmin, libm::fminf),
        "max" => two!(libm::fmax, libm::fmaxf),
        "is_nan" => return Some(Value::Bool(a.is_nan())),
        "is_finite" => return Some(Value::Bool(a.is_finite())),
        _ => return None,
    };
    Some(float_value(k, r))
}

/// The number `v` is, exactly.
fn exact_of(v: &Value) -> Exact {
    match settle(v.clone()) {
        Value::Int(i) => Exact::Int(i128::from(i)),
        Value::Num(k, b) => exact(k, b),
        other => unreachable!("a number: {other:?}"),
    }
}

/// `x.checked_to::<T>()`, `truncate`, `wrap` and `saturate` (`num.convert.1`), to `to`:
/// `None` for `int`, or a kind. `checked_to` gives `None` when `x` is not a `to` exactly.
pub(crate) fn convert(how: &str, x: &Value, to: Option<Kind>) -> Option<Value> {
    let ex = exact_of(x);
    let to_int = |v: i128| match to {
        None => i64::try_from(v).ok().map(Value::Int),
        Some(k) if k.is_float() => {
            let f = v as f64;
            let back = if k == Kind::F32 {
                f64::from(f as f32)
            } else {
                f
            };
            (back == v as f64 && back as i128 == v).then(|| float_value(k, f))
        }
        Some(k) => int_value(k, v),
    };
    match how {
        "checked_to" => match ex {
            Exact::Int(v) => to_int(v),
            Exact::Huge(v) => match to {
                Some(Kind::U128) => Some(Value::Num(Kind::U128, v)),
                Some(k) if k.is_float() => {
                    let f = v as f64;
                    (f as u128 == v).then(|| float_value(k, f))
                }
                _ => None,
            },
            Exact::Float(f) => match to {
                Some(k) if k.is_float() => {
                    let back = if k == Kind::F32 {
                        f64::from(f as f32)
                    } else {
                        f
                    };
                    (back == f || f.is_nan()).then(|| float_value(k, f))
                }
                _ if f.is_finite() && f.fract() == 0.0 && (f as i128) as f64 == f => {
                    to_int(f as i128)
                }
                _ => None,
            },
        },
        "truncate" | "wrap" => {
            let bits = match settle(x.clone()) {
                Value::Int(i) => i as u128,
                Value::Num(_, b) => b,
                _ => 0,
            };
            Some(match to {
                None => Value::Int(bits as i64),
                Some(k) => Value::Num(k, bits & mask(k)),
            })
        }
        _ => {
            let v = match ex {
                Exact::Int(v) => v,
                Exact::Huge(_) => i128::MAX,
                Exact::Float(_) => return None,
            };
            Some(match to {
                None => Value::Int(v.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64),
                Some(k) => {
                    let (lo, hi) = k.range();
                    int_value(k, v.clamp(lo, hi)).expect("in range")
                }
            })
        }
    }
}

/// A literal's exact value, for folding: an integer, or `mantissa × 10^exponent`.
#[derive(Clone, Copy)]
enum Folded {
    Int(i128),
    Dec(i128, i32),
}

/// The exact value of `e` if it is a literal, a parenthesised one, or arithmetic on literals
/// that folds (`num.lit.1`).
fn fold_of(e: &moddil_syntax::ast::Expr) -> Option<Folded> {
    use moddil_syntax::SyntaxKind as K;
    use moddil_syntax::ast::{self, AstNode};
    match e {
        ast::Expr::Literal(_) => {
            let tok = e.syntax().child_tokens().find(|t| !t.kind().is_trivia())?;
            match tok.kind() {
                K::INTEGER => Some(Folded::Int(ast::int_value(tok.text())?)),
                K::FLOAT => {
                    let t: String = tok.text().chars().filter(|&c| c != '_').collect();
                    let (digits, exp) = match t.find(['e', 'E']) {
                        Some(i) => (t[..i].to_string(), t[i + 1..].parse::<i32>().ok()?),
                        None => (t.clone(), 0),
                    };
                    let (int, frac) = digits.split_once('.').unwrap_or((&digits, ""));
                    let m: i128 = format!("{int}{frac}").parse().ok()?;
                    Some(Folded::Dec(m, exp - i32::try_from(frac.len()).ok()?))
                }
                _ => None,
            }
        }
        ast::Expr::Paren(_) => fold_of(e.sub_exprs().first()?),
        ast::Expr::Prefix(_) => {
            let neg = e.syntax().child_tokens().any(|t| t.kind() == K::MINUS);
            let inner = e.sub_exprs();
            match (neg, inner.first()) {
                (true, Some(x @ ast::Expr::Literal(_))) => match fold_of(x)? {
                    Folded::Int(v) => Some(Folded::Int(v.checked_neg()?)),
                    Folded::Dec(m, k) => Some(Folded::Dec(m.checked_neg()?, k)),
                },
                _ => None,
            }
        }
        ast::Expr::Bin(_) => {
            let op = e.syntax().child_tokens().find(|t| !t.kind().is_trivia())?;
            let (l, r) = e.operands();
            let (a, b) = (fold_of(&l?)?, fold_of(&r?)?);
            match (a, b) {
                (Folded::Int(x), Folded::Int(y)) => Some(Folded::Int(match op.kind() {
                    K::PLUS => x.checked_add(y)?,
                    K::MINUS => x.checked_sub(y)?,
                    K::STAR => x.checked_mul(y)?,
                    K::SLASH => x.checked_div(y)?,
                    K::PERCENT => x.checked_rem(y)?,
                    K::AMP => x & y,
                    K::PIPE => x | y,
                    K::CARET => x ^ y,
                    // A shift is checked against its type's width: left to the type.
                    _ => return None,
                })),
                (a, b) => {
                    let parts = |f: Folded| match f {
                        Folded::Int(v) => (v, 0),
                        Folded::Dec(m, k) => (m, k),
                    };
                    let ((m1, e1), (m2, e2)) = (parts(a), parts(b));
                    match op.kind() {
                        K::PLUS | K::MINUS => {
                            let k = e1.min(e2);
                            let up = |m: i128, by: i32| {
                                m.checked_mul(10i128.checked_pow(by.unsigned_abs())?)
                            };
                            let (x, y) = (up(m1, e1 - k)?, up(m2, e2 - k)?);
                            let m = if op.kind() == K::PLUS {
                                x.checked_add(y)?
                            } else {
                                x.checked_sub(y)?
                            };
                            Some(Folded::Dec(m, k))
                        }
                        K::STAR => Some(Folded::Dec(m1.checked_mul(m2)?, e1.checked_add(e2)?)),
                        _ => None,
                    }
                }
            }
        }
        _ => None,
    }
}

/// The value of binary expression `e` if it is arithmetic on literals that folds: an exact
/// literal not yet of a type.
pub(crate) fn folded(e: &moddil_syntax::ast::Expr) -> Option<Value> {
    match fold_of(e)? {
        Folded::Int(v) => Some(i64::try_from(v).map_or(Value::BigLit(v), Value::Int)),
        Folded::Dec(m, k) => Some(Value::FloatLit(format!("{m}e{k}").into())),
    }
}

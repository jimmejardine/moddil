//! Arithmetic on literals is exact (`num.lit.1`): an operator whose operands are literals, or
//! such operations, is folded into one literal before it gets a type, so `200 + 100` is the
//! literal `300` (and an error as a `u8`), and `0.1 * 3.0` is the exact `0.3`.
//!
//! Integers are folded in `i128`; fractional literals as exact decimals, a mantissa times a
//! power of ten, under `+`, `-` and `*` (a quotient need not be a finite decimal, so `/` on them
//! is left to the type), and so are shifts, whose amount is checked against the type's width.
//! Anything that would overflow or divide by zero is left unfolded, for the type's own rules to
//! report.

use crate::hir::{BinaryOp, ExprKind};

/// A literal's exact value.
#[derive(Clone, Copy, Debug)]
pub(super) enum Exact {
    Int(i128),
    /// `mantissa × 10^exponent`.
    Decimal(i128, i32),
}

impl Exact {
    /// The value of literal `kind`, if it is a number literal.
    pub(super) fn of(kind: &ExprKind) -> Option<Exact> {
        match kind {
            ExprKind::Int(v) => Some(Exact::Int(*v)),
            ExprKind::Float(text) => decimal(text),
            _ => None,
        }
    }

    /// The literal it is: an integer, or a fractional literal written exactly.
    pub(super) fn literal(self) -> ExprKind {
        match self {
            Exact::Int(v) => ExprKind::Int(v),
            Exact::Decimal(m, e) => ExprKind::Float(format!("{m}e{e}")),
        }
    }
}

/// `text` (`1.25`, `2e-3`, `-0.5`) as an exact decimal, if its digits fit.
fn decimal(text: &str) -> Option<Exact> {
    let (digits, exp) = match text.find(['e', 'E']) {
        Some(i) => (&text[..i], text[i + 1..].parse::<i32>().ok()?),
        None => (text, 0),
    };
    let (int, frac) = digits.split_once('.').unwrap_or((digits, ""));
    let m: i128 = format!("{int}{frac}").parse().ok()?;
    Some(Exact::Decimal(m, exp - i32::try_from(frac.len()).ok()?))
}

/// `m × 10^k`, if it fits.
fn scale(m: i128, k: u32) -> Option<i128> {
    m.checked_mul(10i128.checked_pow(k)?)
}

/// `a op b`, exactly, if it can be folded.
pub(super) fn fold(op: BinaryOp, a: Exact, b: Exact) -> Option<Exact> {
    match (a, b) {
        (Exact::Int(x), Exact::Int(y)) => Some(Exact::Int(match op {
            BinaryOp::Add => x.checked_add(y)?,
            BinaryOp::Sub => x.checked_sub(y)?,
            BinaryOp::Mul => x.checked_mul(y)?,
            BinaryOp::Div => x.checked_div(y)?,
            BinaryOp::Rem => x.checked_rem(y)?,
            BinaryOp::BitAnd => x & y,
            BinaryOp::BitOr => x | y,
            BinaryOp::BitXor => x ^ y,
            // A shift's amount is checked against its type's width, and `+%`, `-%`, `*%` wrap
            // at it: left to the type.
            _ => return None,
        })),
        (a, b) => {
            let dec = |e: Exact| match e {
                Exact::Int(v) => (v, 0),
                Exact::Decimal(m, e) => (m, e),
            };
            let ((m1, e1), (m2, e2)) = (dec(a), dec(b));
            match op {
                BinaryOp::Add | BinaryOp::Sub => {
                    let e = e1.min(e2);
                    let x = scale(m1, (e1 - e).unsigned_abs())?;
                    let y = scale(m2, (e2 - e).unsigned_abs())?;
                    let m = if op == BinaryOp::Add {
                        x.checked_add(y)?
                    } else {
                        x.checked_sub(y)?
                    };
                    Some(Exact::Decimal(m, e))
                }
                BinaryOp::Mul => Some(Exact::Decimal(m1.checked_mul(m2)?, e1.checked_add(e2)?)),
                _ => None,
            }
        }
    }
}

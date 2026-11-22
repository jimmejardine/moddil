//! The number conversions (`num.convert.1`), on bits: `x.checked_to::<T>()`, `truncate`, `wrap`
//! and `saturate`. Both interpreters' and native code's, so that they agree.
//!
//! A type is `None` for `int` (an `i64`, its bits in the low 64) or `Some` number type.

use moddil_sema::Num;

/// The bits that hold a value of `n`.
fn mask(n: Num) -> u128 {
    if n.bits() == 128 {
        u128::MAX
    } else {
        (1u128 << n.bits()) - 1
    }
}

/// An integer of type `n` with bits `bits`, as an `i128` (exact but for `u128` past `i128::MAX`).
fn signed(n: Num, bits: u128) -> i128 {
    if n.is_signed() {
        let shift = 128 - n.bits();
        ((bits as i128) << shift) >> shift
    } else {
        bits as i128
    }
}

/// The bits of type `n` for the integer `v`, wrapped to its width.
fn int(n: Num, v: i128) -> u128 {
    (v as u128) & mask(n)
}

/// The `f64` a float of type `n` holds (an `f32` widened exactly).
fn float(n: Num, bits: u128) -> f64 {
    match n {
        Num::F32 => f64::from(f32::from_bits(bits as u32)),
        _ => f64::from_bits(bits as u64),
    }
}

/// The bits of float type `n` for `v`, rounded to `f32` if it is one.
fn from_float(n: Num, v: f64) -> u128 {
    match n {
        Num::F32 => u128::from((v as f32).to_bits()),
        _ => u128::from(v.to_bits()),
    }
}

/// The number a value is, exactly: an integer, or a float (a NaN or an infinity has none).
enum Exact {
    Int(i128),
    /// A `u128` past `i128::MAX`.
    Big(u128),
    Float(f64),
}

fn exact(from: Option<Num>, bits: u128) -> Exact {
    match from {
        None => Exact::Int(i128::from(bits as u64 as i64)),
        Some(n) if n.is_float() => Exact::Float(float(n, bits)),
        Some(Num::U128) if bits > i128::MAX as u128 => Exact::Big(bits),
        Some(n) => Exact::Int(signed(n, bits)),
    }
}

/// The bits of type `to` for the integer `v`, if it holds it exactly.
fn int_to(v: i128, to: Option<Num>) -> Option<u128> {
    match to {
        None => i64::try_from(v).ok().map(|i| u128::from(i as u64)),
        Some(n) if n.is_float() => {
            let f = v as f64;
            // Exact if it converts back to the same integer.
            let back = if n == Num::F32 {
                f64::from(f as f32)
            } else {
                f
            };
            (back.is_finite() && back as i128 == v && back == v as f64).then(|| from_float(n, f))
        }
        Some(n) => {
            let (lo, hi) = n.int_range();
            (v >= lo && v <= hi).then(|| int(n, v))
        }
    }
}

/// The conversion `how` (`checked_to`, `truncate`, `wrap` or `saturate`) of the number of type
/// `from` with bits `bits` to type `to`: its bits, or `None` where `checked_to` gives `None`
/// (and `saturate` of a float, which the checker rules out).
pub fn convert(how: &str, from: Option<Num>, bits: u128, to: Option<Num>) -> Option<u128> {
    let bits = match from {
        None => u128::from(bits as u64),
        Some(_) => bits,
    };
    match how {
        "checked_to" => match exact(from, bits) {
            Exact::Int(v) => int_to(v, to),
            Exact::Big(v) => match to {
                Some(Num::U128) => Some(v),
                Some(n) if n.is_float() => {
                    let f = v as f64;
                    (f as u128 == v).then(|| from_float(n, f))
                }
                _ => None,
            },
            Exact::Float(f) => match to {
                Some(n) if n.is_float() => {
                    let back = if n == Num::F32 {
                        f64::from(f as f32)
                    } else {
                        f
                    };
                    (back == f || f.is_nan()).then(|| from_float(n, f))
                }
                _ => {
                    (f.is_finite() && f.fract() == 0.0).then_some(())?;
                    int_to(f as i128, to).filter(|_| (f as i128) as f64 == f)
                }
            },
        },
        // The low bits, or the bits read again: both keep the bits the target's width holds.
        "truncate" | "wrap" => Some(match to {
            None => u128::from(bits as u64),
            Some(n) => bits & mask(n),
        }),
        _ => {
            let v = match exact(from, bits) {
                Exact::Int(v) => v,
                Exact::Big(_) => i128::MAX,
                Exact::Float(_) => return None,
            };
            Some(match to {
                None => {
                    u128::from(v.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64 as u64)
                }
                Some(n) => {
                    let (lo, hi) = n.int_range();
                    int(n, v.clamp(lo, hi))
                }
            })
        }
    }
}

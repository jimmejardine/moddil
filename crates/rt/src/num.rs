//! The number conversions (`num.convert.1`) and how numbers are written, on bits: both
//! interpreters' (through `moddil_mir::convert` and `moddil_mir::num_text`) and native code's.
//!
//! A number type is a code: 0 for `int` (an `i64`, its bits in the low 64), then [`NUMS`]' from
//! 1.

/// Float function `name` of the prelude (`num.float.fn.1`, one of [`crate::FLOAT_FNS`]) on
/// `f64`s, by `libm`, so every platform and runner gives the same bits; `y` is the second
/// argument of those that take two. `None` for another name.
pub fn float_f64(name: &str, x: f64, y: f64) -> Option<f64> {
    Some(match name {
        "sqrt" => libm::sqrt(x),
        "sin" => libm::sin(x),
        "cos" => libm::cos(x),
        "tan" => libm::tan(x),
        "asin" => libm::asin(x),
        "acos" => libm::acos(x),
        "atan" => libm::atan(x),
        "exp" => libm::exp(x),
        "ln" => libm::log(x),
        "log2" => libm::log2(x),
        "log10" => libm::log10(x),
        "abs" => libm::fabs(x),
        "floor" => libm::floor(x),
        "ceil" => libm::ceil(x),
        "trunc" => libm::trunc(x),
        "atan2" => libm::atan2(x, y),
        "powf" => libm::pow(x, y),
        "min" => libm::fmin(x, y),
        "max" => libm::fmax(x, y),
        _ => return None,
    })
}

/// [`float_f64`] on `f32`s, by `libm`'s `f32` functions.
pub fn float_f32(name: &str, x: f32, y: f32) -> Option<f32> {
    Some(match name {
        "sqrt" => libm::sqrtf(x),
        "sin" => libm::sinf(x),
        "cos" => libm::cosf(x),
        "tan" => libm::tanf(x),
        "asin" => libm::asinf(x),
        "acos" => libm::acosf(x),
        "atan" => libm::atanf(x),
        "exp" => libm::expf(x),
        "ln" => libm::logf(x),
        "log2" => libm::log2f(x),
        "log10" => libm::log10f(x),
        "abs" => libm::fabsf(x),
        "floor" => libm::floorf(x),
        "ceil" => libm::ceilf(x),
        "trunc" => libm::truncf(x),
        "atan2" => libm::atan2f(x, y),
        "powf" => libm::powf(x, y),
        "min" => libm::fminf(x, y),
        "max" => libm::fmaxf(x, y),
        _ => return None,
    })
}

/// The sized number types, by their codes from 1: their bits, and whether each is signed or a
/// float.
pub const NUMS: [(&str, u32, bool, bool); 11] = [
    ("i8", 8, true, false),
    ("i16", 16, true, false),
    ("i32", 32, true, false),
    ("i128", 128, true, false),
    ("u8", 8, false, false),
    ("u16", 16, false, false),
    ("u32", 32, false, false),
    ("u64", 64, false, false),
    ("u128", 128, false, false),
    ("f32", 32, true, true),
    ("f64", 64, true, true),
];

/// The conversions, by number: `checked_to`, `truncate`, `wrap`, `saturate`.
pub const CONVERSIONS: [&str; 4] = ["checked_to", "truncate", "wrap", "saturate"];

/// A sized number type, by its code (`None` for `int`).
#[derive(Clone, Copy, PartialEq, Eq)]
struct N(u32);

impl N {
    fn of(code: u32) -> Option<N> {
        (1..=NUMS.len() as u32).contains(&code).then_some(N(code))
    }
    fn bits(self) -> u32 {
        NUMS[self.0 as usize - 1].1
    }
    fn is_signed(self) -> bool {
        NUMS[self.0 as usize - 1].2
    }
    fn is_float(self) -> bool {
        NUMS[self.0 as usize - 1].3
    }
    fn is_f32(self) -> bool {
        self.is_float() && self.bits() == 32
    }
    fn is_u128(self) -> bool {
        !self.is_signed() && self.bits() == 128
    }
    /// The least and greatest values of an integer type.
    fn int_range(self) -> (i128, i128) {
        let b = self.bits();
        if self.is_signed() {
            if b == 128 {
                (i128::MIN, i128::MAX)
            } else {
                (-(1i128 << (b - 1)), (1i128 << (b - 1)) - 1)
            }
        } else if b == 128 {
            (0, i128::MAX)
        } else {
            (0, (1i128 << b) - 1)
        }
    }
}

/// The bits that hold a value of `n`.
fn mask(n: N) -> u128 {
    if n.bits() == 128 {
        u128::MAX
    } else {
        (1u128 << n.bits()) - 1
    }
}

/// An integer of type `n` with bits `bits`, as an `i128` (exact but for `u128` past `i128::MAX`).
fn signed(n: N, bits: u128) -> i128 {
    if n.is_signed() {
        let shift = 128 - n.bits();
        ((bits as i128) << shift) >> shift
    } else {
        bits as i128
    }
}

/// The bits of type `n` for the integer `v`, wrapped to its width.
fn int(n: N, v: i128) -> u128 {
    (v as u128) & mask(n)
}

/// The `f64` a float of type `n` holds (an `f32` widened exactly).
fn float(n: N, bits: u128) -> f64 {
    if n.is_f32() {
        f64::from(f32::from_bits(bits as u32))
    } else {
        f64::from_bits(bits as u64)
    }
}

/// The bits of float type `n` for `v`, rounded to `f32` if it is one.
fn from_float(n: N, v: f64) -> u128 {
    if n.is_f32() {
        u128::from((v as f32).to_bits())
    } else {
        u128::from(v.to_bits())
    }
}

/// The number a value is, exactly: an integer, or a float (a NaN or an infinity has none).
enum Exact {
    Int(i128),
    /// A `u128` past `i128::MAX`.
    Big(u128),
    Float(f64),
}

fn exact(from: Option<N>, bits: u128) -> Exact {
    match from {
        None => Exact::Int(i128::from(bits as u64 as i64)),
        Some(n) if n.is_float() => Exact::Float(float(n, bits)),
        Some(n) if n.is_u128() && bits > i128::MAX as u128 => Exact::Big(bits),
        Some(n) => Exact::Int(signed(n, bits)),
    }
}

/// The bits of type `to` for the integer `v`, if it holds it exactly.
fn int_to(v: i128, to: Option<N>) -> Option<u128> {
    match to {
        None => i64::try_from(v).ok().map(|i| u128::from(i as u64)),
        Some(n) if n.is_float() => {
            let f = v as f64;
            // Exact if it converts back to the same integer.
            let back = if n.is_f32() { f64::from(f as f32) } else { f };
            (back.is_finite() && back as i128 == v && back == v as f64).then(|| from_float(n, f))
        }
        Some(n) => {
            let (lo, hi) = n.int_range();
            (v >= lo && v <= hi).then(|| int(n, v))
        }
    }
}

/// The conversion `how` (`checked_to`, `truncate`, `wrap` or `saturate`) of the number of type
/// code `from` with bits `bits` to type code `to`: its bits, or `None` where `checked_to` gives
/// `None` (and `saturate` of a float, which the checker rules out).
pub fn convert(how: &str, from: u32, bits: u128, to: u32) -> Option<u128> {
    let (from, to) = (N::of(from), N::of(to));
    let bits = match from {
        None => u128::from(bits as u64),
        Some(_) => bits,
    };
    match how {
        "checked_to" => match exact(from, bits) {
            Exact::Int(v) => int_to(v, to),
            Exact::Big(v) => match to {
                Some(n) if n.is_u128() => Some(v),
                Some(n) if n.is_float() => {
                    let f = v as f64;
                    (f as u128 == v).then(|| from_float(n, f))
                }
                _ => None,
            },
            Exact::Float(f) => match to {
                Some(n) if n.is_float() => {
                    let back = if n.is_f32() { f64::from(f as f32) } else { f };
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

/// How a number of type code `code` with bits `bits` is written.
pub fn text(code: u32, bits: u128) -> String {
    match N::of(code) {
        None => (bits as u64 as i64).to_string(),
        Some(n) if n.is_float() => {
            if n.is_f32() {
                format!("{:?}", f32::from_bits(bits as u32))
            } else {
                format!("{:?}", f64::from_bits(bits as u64))
            }
        }
        Some(n) if n.is_signed() => signed(n, bits).to_string(),
        Some(_) => bits.to_string(),
    }
}

/// A float's text with `digits` digits after the point, rounded to the nearest
/// (`expr.fmt.spec.3`): an `f32` if `f32` says so, else an `f64`, from its bits.
pub fn fixed_text(f32: bool, bits: u64, digits: usize) -> String {
    if f32 {
        format!("{:.*}", digits, f32::from_bits(bits as u32))
    } else {
        format!("{:.*}", digits, f64::from_bits(bits))
    }
}

/// The code of the number type named `name` (`int`, `i8`, …).
pub fn code(name: &str) -> Option<u32> {
    if name == "int" {
        return Some(0);
    }
    NUMS.iter()
        .position(|(n, ..)| *n == name)
        .map(|k| k as u32 + 1)
}

/// An `i128` or `u128` operation native code leaves to the runtime, as Cranelift cannot compile
/// it for every target: `op` 0 is signed `/`, 1 unsigned `/`, 2 signed `%`, 3 unsigned `%`, 4
/// whether the signed product overflows (1 if so), 5 whether the unsigned one does. The operands
/// come as their low and high words. A division by zero, or of the least `i128` by -1, is checked
/// before; here it gives 0.
pub fn wide(op: u32, alo: u64, ahi: u64, blo: u64, bhi: u64) -> u128 {
    let a = u128::from(alo) | (u128::from(ahi) << 64);
    let b = u128::from(blo) | (u128::from(bhi) << 64);
    let (sa, sb) = (a as i128, b as i128);
    match op {
        0 if sb != 0 => sa.wrapping_div(sb) as u128,
        1 if b != 0 => a / b,
        2 if sb != 0 => sa.wrapping_rem(sb) as u128,
        3 if b != 0 => a % b,
        4 => u128::from(sa.checked_mul(sb).is_none()),
        5 => u128::from(a.checked_mul(b).is_none()),
        _ => 0,
    }
}

#[cfg(test)]
mod wide_tests {
    use super::wide;

    fn run(op: u32, a: u128, b: u128) -> u128 {
        wide(op, a as u64, (a >> 64) as u64, b as u64, (b >> 64) as u64)
    }

    #[test]
    fn wide_operations() {
        let a = (-81_000_000_000_000_000_000_000_000_000_000_000_007i128) as u128;
        let b = 9_000_000_000_000_000_000u128;
        assert_eq!(run(0, a, b) as i128, -9_000_000_000_000_000_000);
        assert_eq!(run(2, a, b) as i128, -7);
        assert_eq!(run(1, u128::MAX, 3), u128::MAX / 3);
        assert_eq!(run(3, u128::MAX, 7), u128::MAX % 7);
        assert_eq!(run(4, i128::MAX as u128, 2), 1);
        assert_eq!(run(4, b, b), 0);
        assert_eq!(run(5, u128::MAX, 2), 1);
        assert_eq!(run(5, 1 << 64, 3), 0);
    }
}

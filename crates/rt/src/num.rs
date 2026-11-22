//! The number conversions (`num.convert.1`) and how numbers are written, on bits: both
//! interpreters' (through `moddil_mir::convert` and `moddil_mir::num_text`) and native code's.
//!
//! A number type is a code: 0 for `int` (an `i64`, its bits in the low 64), then [`NUMS`]' from
//! 1.

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

/// The code of the number type named `name` (`int`, `i8`, …).
pub fn code(name: &str) -> Option<u32> {
    if name == "int" {
        return Some(0);
    }
    NUMS.iter()
        .position(|(n, ..)| *n == name)
        .map(|k| k as u32 + 1)
}

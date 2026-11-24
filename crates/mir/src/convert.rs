//! The number conversions (`num.convert.1`) and how numbers are written: `moddil-rt`'s, which
//! native code calls too, so that the interpreters and compiled code agree.
//!
//! A type is `None` for `int` (an `i64`, its bits in the low 64) or `Some` number type.

use moddil_sema::Num;

/// The code `moddil-rt` knows a number type by: 0 for `int`, then its `NUMS`' from 1.
pub fn num_code(n: Option<Num>) -> u32 {
    let name = match n {
        None => "int",
        Some(Num::I8) => "i8",
        Some(Num::I16) => "i16",
        Some(Num::I32) => "i32",
        Some(Num::I128) => "i128",
        Some(Num::U8) => "u8",
        Some(Num::U16) => "u16",
        Some(Num::U32) => "u32",
        Some(Num::U64) => "u64",
        Some(Num::U128) => "u128",
        Some(Num::F32) => "f32",
        Some(Num::F64) => "f64",
    };
    moddil_rt::num::code(name).expect("a number type the runtime knows")
}

/// The conversion `how` (`checked_to`, `truncate`, `wrap` or `saturate`) of the number of type
/// `from` with bits `bits` to type `to`: its bits, or `None` where `checked_to` gives `None`
/// (and `saturate` of a float, which the checker rules out).
pub fn convert(how: &str, from: Option<Num>, bits: u128, to: Option<Num>) -> Option<u128> {
    moddil_rt::num::convert(how, num_code(from), bits, num_code(to))
}

/// How a number constant of type `n` with bits `bits` is written.
pub fn num_text(n: Num, bits: u128) -> String {
    moddil_rt::num::text(num_code(Some(n)), bits)
}

/// Float `n`'s text, from its bits, with `digits` digits after the point (`expr.fmt.spec.3`).
pub fn fixed_text(n: Num, bits: u128, digits: usize) -> String {
    moddil_rt::num::fixed_text(n == Num::F32, bits as u64, digits)
}

//! What native code needs that is easier written in Rust and shared with the interpreters: the
//! number conversions ([`crate::num`]), the text of a value, the log, and the text of a
//! `Trapped`.
//!
//! A number crosses as a type code ([`crate::num`]) and its bits, in two words.

#![allow(unsafe_code)]

use crate::Trapped;

/// Conversion `how` (by its number in [`crate::num::CONVERSIONS`]) of the number of type `from`
/// with bits `lo`, `hi` to type `to`: 1 and its bits at `out` (two words), or 0 if there is
/// none.
#[allow(unsafe_code)]
#[unsafe(export_name = "moddil_rt_convert")]
pub extern "C" fn convert(how: u32, from: u32, lo: u64, hi: u64, to: u32, out: *mut u64) -> u8 {
    let bits = u128::from(lo) | (u128::from(hi) << 64);
    let how = crate::num::CONVERSIONS[how as usize];
    match crate::num::convert(how, from, bits, to) {
        Some(v) => {
            put_words(out, v);
            1
        }
        None => 0,
    }
}

/// Writes `v` as two words at `out`, where the generated code gives room for them.
fn put_words(out: *mut u64, v: u128) {
    // SAFETY: the generated code passes two words of room.
    unsafe {
        *out = v as u64;
        *out.add(1) = (v >> 64) as u64;
    }
}

/// Keeps `s` until the run ends: its address, and its length at `len`.
fn keep(s: String, len: *mut u64) -> *const u8 {
    // SAFETY: the generated code passes a word to write the length to.
    unsafe { *len = s.len() as u64 };
    crate::keep_text(s.into())
}

/// The text the code at `p` has, `n` bytes of UTF-8.
fn text_at<'a>(p: *const u8, n: u64) -> &'a str {
    if n == 0 {
        return "";
    }
    // SAFETY: the generated code passes a `&str`'s address and length.
    unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(p, n as usize)) }
}

/// What a text function gives: `text` of a number (by its code), of a `bool` (code 100) or of a
/// `char` (code 101), in `lo`; `quoted_text` of a `char` (code 102); or, of the text at `p` (`n`
/// bytes), `quoted_text` (103), `upper_text` (104), `lower_text` (105) or `trim` (106); or
/// `fixed_text` of an `f32` (107) or an `f64` (108) in `lo`, with `hi` digits. Its address, and
/// its length at `len`.
#[allow(unsafe_code)]
#[unsafe(export_name = "moddil_rt_text")]
pub extern "C" fn text(
    code: u32,
    lo: u64,
    hi: u64,
    p: *const u8,
    n: u64,
    len: *mut u64,
) -> *const u8 {
    let bits = u128::from(lo) | (u128::from(hi) << 64);
    let s = match code {
        100 => (lo & 1 == 1).to_string(),
        101 => char::from_u32(lo as u32).map_or_else(String::new, String::from),
        102 => char::from_u32(lo as u32).map_or_else(String::new, |c| format!("{c:?}")),
        103 => format!("{:?}", text_at(p, n)),
        104 => text_at(p, n).to_uppercase(),
        105 => text_at(p, n).to_lowercase(),
        106 => text_at(p, n).trim().to_string(),
        107 | 108 => crate::num::fixed_text(code == 107, lo, hi as usize),
        k => crate::num::text(k, bits),
    };
    keep(s, len)
}

/// The log's levels, by number.
pub const LEVELS: [&str; 4] = ["debug", "info", "warn", "error"];

/// Writes an event of the ambient log (`obs.log.1`) to standard error, as the interpreters do:
/// its level and the text at `p` (`n` bytes).
#[allow(unsafe_code)]
#[unsafe(export_name = "moddil_rt_log")]
pub extern "C" fn log(level: u32, p: *const u8, n: u64) {
    eprintln!("[{}] {}", LEVELS[level as usize], text_at(p, n));
}

/// What gives the text of a `Trapped`, for the trap: its class (`which` 0) or where it was, as
/// `file:line:column` (1). The embedder installs it: the compiler, or a built program's `main`.
pub type TrapText = fn(&Trapped, u32) -> String;

/// The text of the `Trapped` the last isolated scope failed with: its class (`which` 0) or where
/// it was (1), by the resolver the run has.
#[allow(unsafe_code)]
#[unsafe(export_name = "moddil_rt_trapped")]
pub extern "C" fn trapped(which: u32, len: *mut u64) -> *const u8 {
    let text = match (crate::run::with(|s| s.trap_text), crate::last_trapped()) {
        (Some(f), Some(t)) => f(&t, which),
        _ => String::new(),
    };
    keep(text, len)
}

//! Functions native code calls in the host, for what is easier written in Rust and shared with
//! the interpreters: the number conversions (`moddil_mir::convert`), the text of a value, and
//! the log.
//!
//! A number crosses as a type code ([`num_code`]) and its bits, in two words.

#![allow(unsafe_code)]

use moddil_sema::Num;

/// The number types by code: `int` is 0, these from 1.
const NUMS: [Num; 11] = [
    Num::I8,
    Num::I16,
    Num::I32,
    Num::I128,
    Num::U8,
    Num::U16,
    Num::U32,
    Num::U64,
    Num::U128,
    Num::F32,
    Num::F64,
];

/// The code of `int` (`None`) or a number type.
pub(crate) fn num_code(n: Option<Num>) -> u32 {
    match n {
        None => 0,
        Some(n) => 1 + NUMS.iter().position(|m| *m == n).expect("a number type") as u32,
    }
}

fn num_of(code: u32) -> Option<Num> {
    code.checked_sub(1).map(|k| NUMS[k as usize])
}

/// The conversions, by number: `checked_to`, `truncate`, `wrap`, `saturate`.
pub(crate) const CONVERSIONS: [&str; 4] = ["checked_to", "truncate", "wrap", "saturate"];

/// Conversion `how` of the number of type `from` with bits `lo`, `hi` to type `to`: 1 and its
/// bits at `out` (two words), or 0 if there is none.
pub(crate) extern "C" fn convert(
    how: u32,
    from: u32,
    lo: u64,
    hi: u64,
    to: u32,
    out: *mut u64,
) -> u8 {
    let bits = u128::from(lo) | (u128::from(hi) << 64);
    let how = CONVERSIONS[how as usize];
    match moddil_mir::convert(how, num_of(from), bits, num_of(to)) {
        Some(v) => {
            // SAFETY: the generated code passes two words of room.
            unsafe {
                *out = v as u64;
                *out.add(1) = (v >> 64) as u64;
            }
            1
        }
        None => 0,
    }
}

/// What a text function gives: `text` of a number (by its code), of a `bool` (code 100) or of a
/// `char` (code 101), in `lo`; or `quoted_text` of a `char` (code 102), or of the text at `p`
/// (`n` bytes, code 103). Its address, and its length at `len`.
pub(crate) extern "C" fn text(
    code: u32,
    lo: u64,
    hi: u64,
    p: *const u8,
    n: u64,
    len: *mut u64,
) -> *const u8 {
    let bits = u128::from(lo) | (u128::from(hi) << 64);
    let s = match code {
        0 => (lo as i64).to_string(),
        100 => (lo & 1 == 1).to_string(),
        101 => char::from_u32(lo as u32).map_or_else(String::new, String::from),
        102 => char::from_u32(lo as u32).map_or_else(String::new, |c| format!("{c:?}")),
        103..=106 => {
            // SAFETY: the generated code passes a `&str`'s address and length.
            let t =
                unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(p, n as usize)) };
            match code {
                103 => format!("{t:?}"),
                104 => t.to_uppercase(),
                105 => t.to_lowercase(),
                _ => t.trim().to_string(),
            }
        }
        k => num_of(k).map_or_else(String::new, |n| moddil_mir::num_text(n, bits)),
    };
    keep(s, len)
}

/// Keeps `s` until the run ends: its address, and its length at `len`.
fn keep(s: String, len: *mut u64) -> *const u8 {
    // SAFETY: the generated code passes a word to write the length to.
    unsafe { *len = s.len() as u64 };
    moddil_rt::keep_text(s.into())
}

/// What the host's functions need of a run: its sites, and its files' paths and text.
pub(crate) struct HostData<'a> {
    pub(crate) sites: &'a [crate::lower::Site],
    pub(crate) files: &'a [(String, String)],
}

/// The text of the `Trapped` an isolated scope failed with: its class (`which` 0) or where it
/// was, as `file:line:column` (1), the trap reported where the interpreters report it.
pub(crate) extern "C" fn trapped(which: u32, len: *mut u64) -> *const u8 {
    let host = moddil_rt::host() as *const HostData<'_>;
    let t = moddil_rt::last_trapped();
    // SAFETY: the run's host data lives while its code runs.
    let text = match (unsafe { host.as_ref() }, t) {
        (Some(h), Some(t)) => match crate::jit::reported(h.sites, &t) {
            crate::Native::Trap(class, file, range) => {
                if which == 0 {
                    class.to_string()
                } else {
                    let (path, text) = &h.files[file as usize];
                    let (line, col) = moddil_syntax::line_col(text, range.start);
                    format!("{path}:{line}:{col}")
                }
            }
            _ => String::new(),
        },
        _ => String::new(),
    };
    keep(text, len)
}

/// The log's levels, by number.
pub(crate) const LEVELS: [&str; 4] = ["debug", "info", "warn", "error"];

/// Writes an event of the ambient log (`obs.log.1`) to standard error, as the interpreters do:
/// its level and the text at `p` (`n` bytes).
pub(crate) extern "C" fn log(level: u32, p: *const u8, n: u64) {
    // SAFETY: the generated code passes a `&str`'s address and length.
    let msg = unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(p, n as usize)) };
    eprintln!("[{}] {msg}", LEVELS[level as usize]);
}

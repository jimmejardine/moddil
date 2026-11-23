//! `moddil-rt-wasm`: [`moddil_rt`] built for `wasm32-unknown-unknown`, the runtime a Moddil WASM
//! module calls into.
//!
//! `moddil-wasm`'s build script builds this crate as a module that imports its memory, so it
//! shares the program's: the program calls `moddil-rt`'s functions (exported as `moddil_rt_*`,
//! as native code links them) with its own addresses. Its allocator takes memory only by growing
//! it, above the program's data and stack. Besides those, it exports what the host needs to
//! run entries and read what they recorded, start tasks and give the text of a `Trapped`, and
//! the `i128` arithmetic WASM has no instructions for.

#![allow(unsafe_code)]

use std::cell::Cell;

use moddil_rt::sites::Tables;

thread_local! {
    /// The program's site tables, once the host has said where they are.
    static TABLES: Cell<Option<Tables>> = const { Cell::new(None) };
}

/// The text of a `Trapped`, from the program's site tables.
fn trapped(t: &moddil_rt::Trapped, which: u32) -> String {
    TABLES
        .with(Cell::get)
        .map_or_else(String::new, |tables| tables.trapped(t, which))
}

/// Says where the program's site tables are: their count, their sites and their strings.
///
/// # Safety
///
/// The addresses are the program's tables', which live as long as the module.
#[unsafe(export_name = "moddil_wasm_sites")]
pub unsafe extern "C" fn sites(count: *const u64, sites: *const u32, strings: *const u8) {
    // SAFETY: as the caller says.
    let count = unsafe { *count };
    TABLES.with(|c| {
        c.set(Some(Tables {
            count,
            sites,
            strings,
        }));
    });
}

/// Takes the trap the run recorded, and writes its class and where it is reported
/// (`file:line:column`), a line apart, at `out` (as much as `cap` bytes): how many bytes that
/// is. For a built program, which has no compiler to ask.
///
/// # Safety
///
/// `out` has room for `cap` bytes.
#[unsafe(export_name = "moddil_wasm_trap_text")]
pub unsafe extern "C" fn trap_text(out: *mut u8, cap: u32) -> u32 {
    let text = match (moddil_rt::take().trap, TABLES.with(Cell::get)) {
        (Some(t), Some(tables)) => {
            let (class, at) = tables.trap_text(&t);
            format!(
                "{class}
{at}"
            )
        }
        _ => String::new(),
    };
    let n = text.len().min(cap as usize);
    // SAFETY: as the caller says.
    unsafe { std::ptr::copy_nonoverlapping(text.as_ptr(), out, n) };
    n as u32
}

/// Starts a new run: what the last one recorded is forgotten.
#[unsafe(export_name = "moddil_wasm_begin")]
pub extern "C" fn begin() {
    moddil_rt::begin_run();
    moddil_rt::set_trap_text(trapped);
}

/// A run of `main`: it has the OS, the host's (`os.1`).
#[unsafe(export_name = "moddil_wasm_os")]
pub extern "C" fn os() {
    moddil_rt::set_os(Vec::new());
}

/// Task `t`'s call: the host starts it the first time the task gets the turn, as a promising
/// call ([`moddil_rt::task`]).
#[cfg(target_arch = "wasm32")]
#[unsafe(export_name = "moddil_wasm_task")]
pub extern "C" fn task(t: u32) {
    moddil_rt::task(t as usize);
}

/// How many allocations are not yet freed.
#[unsafe(export_name = "moddil_wasm_live")]
pub extern "C" fn live() -> i64 {
    moddil_rt::live()
}

/// Takes what the run recorded, into the `cap` words at `out`: whether it trapped, the trap's
/// site, whether its code is tracked, whether an error left it, the last `fail`'s site, and
/// then how many call sites were on the stack and each one, innermost last (as many as fit).
/// How many words that is.
///
/// # Safety
///
/// `out` has room for `cap` words.
#[unsafe(export_name = "moddil_wasm_record")]
pub unsafe extern "C" fn record(out: *mut u32, cap: u32) -> u32 {
    let r = moddil_rt::take();
    let mut words = vec![0u32; 6];
    if let Some(t) = &r.trap {
        words[0] = 1;
        words[1] = t.site;
        words[2] = u32::from(t.tracked);
        words[5] = t.stack.len() as u32;
        words.extend(&t.stack);
    }
    if let Some(f) = r.failed {
        words[3] = 1;
        words[4] = f;
    }
    let n = words.len().min(cap as usize);
    // SAFETY: as the caller says.
    unsafe { std::ptr::copy_nonoverlapping(words.as_ptr(), out, n) };
    n as u32
}

/// `i128` operation `op` on `a` and `b` (each as its low and high words): its result at `out`
/// (two words), and, for the overflow checks, whether it overflowed.
///
/// The operations: 0 `*`, 1 signed `/`, 2 unsigned `/`, 3 signed `%`, 4 unsigned `%`, 5 `<<`,
/// 6 unsigned `>>`, 7 signed `>>`, 8 rotate left (each shift by `b` modulo 128), 9 whether the
/// signed product overflows, 10 whether the unsigned one does.
///
/// # Safety
///
/// `out` has room for two words.
#[unsafe(export_name = "moddil_wasm_i128")]
pub unsafe extern "C" fn i128_op(
    op: u32,
    alo: u64,
    ahi: u64,
    blo: u64,
    bhi: u64,
    out: *mut u64,
) -> u32 {
    let a = u128::from(alo) | (u128::from(ahi) << 64);
    let b = u128::from(blo) | (u128::from(bhi) << 64);
    let (sa, sb) = (a as i128, b as i128);
    let k = (b % 128) as u32;
    let (v, flag) = match op {
        0 => (a.wrapping_mul(b), false),
        1 if sb != 0 => (sa.wrapping_div(sb) as u128, false),
        2 if b != 0 => (a / b, false),
        3 if sb != 0 => (sa.wrapping_rem(sb) as u128, false),
        4 if b != 0 => (a % b, false),
        5 => (a << k, false),
        6 => (a >> k, false),
        7 => ((sa >> k) as u128, false),
        8 => (a.rotate_left(k), false),
        9 => {
            let (v, o) = sa.overflowing_mul(sb);
            (v as u128, o)
        }
        10 => a.overflowing_mul(b),
        // Division by zero is checked before it is done.
        _ => (0, false),
    };
    // SAFETY: as the caller says.
    unsafe {
        *out = v as u64;
        *out.add(1) = (v >> 64) as u64;
    }
    u32::from(flag)
}

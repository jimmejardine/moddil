//! `moddil-rt`: the runtime that native Moddil code calls into.
//!
//! JIT-compiled code is given the addresses of these functions.
//!
//! - A failed check calls [`trap`] (or, in `#[track_caller]` and the prelude's code,
//!   [`trap_tracked`]) with its site, then returns a "trapped" status that each caller passes on
//!   (the status-return ABI, `specs/compiler.md`).
//! - A test's `// =>` expression calls [`observe`] with its value's address, which hands it to
//!   the observer the host installed ([`set_observer`]) while the value is still there to read.
//! - Each call goes through [`enter`] and [`leave`], which keep the stack of call sites: so
//!   recursion without end traps as the interpreters' does, rather than overflowing the native
//!   stack, and a trap in tracked code can be reported where untracked code called it.
//! - Text and float functions the code needs and Cranelift has no instruction for.
//!
//! - Tasks, scopes and channels ([`run`]): each task is a thread, and one runs at a time.
//!
//! What they record is kept per run, shared by its threads, until the host [`take`]s it.

mod baton;
pub mod host;
pub mod num;
mod os;
mod run;
mod sched;
pub mod sites;

#[cfg(target_arch = "wasm32")]
pub use baton::task;
pub use baton::{DropFn, TaskFn};
pub use run::{
    TaskSpec, begin_run, channel, clone_sender, detach, drop_receiver, drop_sender, end_run, host,
    join, join_now, keep_text, last_trapped, park, recv, recv_now, scope_open, scope_run, select,
    send, send_now, set_host, set_os, set_trap_text, spawn, stop, stop_requested, task_id, timer,
    unpark, yield_now,
};
pub use run::{
    callback_trapped, export_begin, export_trapped, gpu_dispatch, gpu_frame, gpu_show, os_arg_copy,
    os_arg_count, os_arg_len, os_channel, os_close, os_connect, os_listen, os_random, os_send,
    os_take, os_wall_clock, os_write_stderr, trapped,
};

/// How many calls deep a run may go, the frame it starts in included: as the MIR interpreter's
/// limit (`moddil_interp::MAX_DEPTH`).
pub const MAX_DEPTH: u32 = 10_000;

/// A trap: its site, whether the code it happened in is tracked (so it is reported where
/// untracked code called it), and the call sites on the stack then, innermost last.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trapped {
    /// The site of the check that failed (or of the call too deep).
    pub site: u32,
    /// Whether the code it is in is tracked.
    pub tracked: bool,
    /// The call sites on the stack, innermost last.
    pub stack: Vec<u32>,
}

impl Trapped {
    /// The site to report the trap at, as the interpreters do: its own, or, in tracked code
    /// (`#[track_caller]`, the prelude's or derived code), the innermost call untracked code
    /// made, where `untracked_call` says which sites those are.
    pub fn reported_at(&self, untracked_call: impl Fn(u32) -> bool) -> u32 {
        if !self.tracked {
            return self.site;
        }
        self.stack
            .iter()
            .rev()
            .copied()
            .find(|&s| untracked_call(s))
            .unwrap_or(self.site)
    }
}

/// What a run recorded: the trap, if one happened.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Record {
    /// The trap, if one happened.
    pub trap: Option<Trapped>,
    /// The site of the last error raised (`fail`), which is where one that leaves the run was.
    pub failed: Option<u32>,
}

/// What the host gives to see observed values: a function and the context it is called with,
/// with each observation's site and the address of its value.
pub type Observer = (fn(*mut (), u32, *const u8), *mut ());

/// Installs (or, with `None`, removes) the run's observer.
pub fn set_observer(o: Option<Observer>) {
    run::with(|s| s.observer = o);
}

/// Records that the check at `site` failed: the status the code returns, 1 (the run ends) or 3
/// (an isolated scope takes the trap, and the task unwinds).
#[allow(unsafe_code)]
#[unsafe(export_name = "moddil_rt_trap")]
pub extern "C" fn trap(site: u32) -> u8 {
    run::record_trap(site, false)
}

/// As [`trap`], in tracked code: `#[track_caller]`, the prelude's or derived code.
#[allow(unsafe_code)]
#[unsafe(export_name = "moddil_rt_trap_tracked")]
pub extern "C" fn trap_tracked(site: u32) -> u8 {
    run::record_trap(site, true)
}

/// Records that the `fail` at `site` raised an error: the code then returns a failed status.
#[allow(unsafe_code)]
#[unsafe(export_name = "moddil_rt_fail_at")]
pub extern "C" fn fail_at(site: u32) {
    run::with(|s| s.failed = Some(site));
}

/// Hands the value at `addr` of the observed expression at `site` to the observer, if there is
/// one.
#[allow(unsafe_code)]
#[unsafe(export_name = "moddil_rt_observe")]
pub extern "C" fn observe(site: u32, addr: *const u8) {
    if let Some((f, cx)) = run::with(|s| s.observer) {
        f(cx, site, addr);
    }
}

/// Before the call at `site`: 0 if it may go a frame deeper; if it may not, the status of the
/// stack overflow recorded as a trap at `site` (1, or 3 in an isolated scope). `tracked` says
/// whether the calling code is.
#[allow(unsafe_code)]
#[unsafe(export_name = "moddil_rt_enter")]
pub extern "C" fn enter(site: u32, tracked: u8) -> u8 {
    let full = run::STACK.with(|r| r.borrow().len() as u32 + 1 >= MAX_DEPTH);
    if full {
        return run::record_trap(site, tracked != 0);
    }
    run::STACK.with(|r| r.borrow_mut().push(site));
    0
}

/// After a call returns, however it returns.
#[allow(unsafe_code)]
#[unsafe(export_name = "moddil_rt_leave")]
pub extern "C" fn leave() {
    run::STACK.with(|r| {
        r.borrow_mut().pop();
    });
}

/// `a % b` on `f64`s, as IEEE 754 and the interpreters compute it.
#[allow(unsafe_code)]
#[unsafe(export_name = "moddil_rt_fmod")]
pub extern "C" fn fmod(a: f64, b: f64) -> f64 {
    libm::fmod(a, b)
}

/// The low word of an `i128` operation native code leaves to the runtime ([`num::wide`]).
#[allow(unsafe_code)]
#[unsafe(export_name = "moddil_rt_i128_lo")]
pub extern "C" fn i128_lo(op: u32, alo: u64, ahi: u64, blo: u64, bhi: u64) -> u64 {
    num::wide(op, alo, ahi, blo, bhi) as u64
}

/// Its high word.
#[allow(unsafe_code)]
#[unsafe(export_name = "moddil_rt_i128_hi")]
pub extern "C" fn i128_hi(op: u32, alo: u64, ahi: u64, blo: u64, bhi: u64) -> u64 {
    (num::wide(op, alo, ahi, blo, bhi) >> 64) as u64
}

/// `a % b` on `f32`s.
#[allow(unsafe_code)]
#[unsafe(export_name = "moddil_rt_fmodf")]
pub extern "C" fn fmodf(a: f32, b: f32) -> f32 {
    libm::fmodf(a, b)
}

/// The float functions, by number ([`FLOAT_FNS`]), on `f64`s.
#[allow(unsafe_code)]
#[unsafe(export_name = "moddil_rt_float_fn")]
pub extern "C" fn float_fn(which: u32, x: f64, y: f64) -> f64 {
    let name = FLOAT_FNS.get(which as usize).copied().unwrap_or("");
    num::float_f64(name, x, y).unwrap_or(f64::NAN)
}

/// The same on `f32`s, by `libm`'s `f32` functions, as the interpreters compute them.
#[allow(unsafe_code)]
#[unsafe(export_name = "moddil_rt_float_fn_f32")]
pub extern "C" fn float_fn_f32(which: u32, x: f32, y: f32) -> f32 {
    let name = FLOAT_FNS.get(which as usize).copied().unwrap_or("");
    num::float_f32(name, x, y).unwrap_or(f32::NAN)
}

/// The float functions [`float_fn`] computes, by number.
pub const FLOAT_FNS: &[&str] = &[
    "sqrt", "sin", "cos", "tan", "asin", "acos", "atan", "exp", "ln", "log2", "log10", "abs",
    "floor", "ceil", "trunc", "atan2", "powf", "min", "max",
];

/// The text at `ptr` (`len` bytes of UTF-8) as a `&str`.
///
/// The code only passes text it holds a borrow of.
fn text<'a>(ptr: *const u8, len: u64) -> &'a str {
    // The generated code passes the pointer and length of a `&str` it holds: live, UTF-8.
    #[allow(unsafe_code)]
    // SAFETY: as above.
    unsafe {
        std::str::from_utf8_unchecked(std::slice::from_raw_parts(ptr, len as usize))
    }
}

/// The character that starts at byte `at` of the text, or `0x11_0000` if none does.
#[allow(unsafe_code)]
#[unsafe(export_name = "moddil_rt_char_at_byte")]
pub extern "C" fn char_at_byte(ptr: *const u8, len: u64, at: i64) -> u32 {
    let s = text(ptr, len);
    usize::try_from(at)
        .ok()
        .and_then(|at| s.get(at..))
        .and_then(|rest| rest.chars().next())
        .map_or(0x11_0000, u32::from)
}

/// Whether bytes `a` to `b` of the text are text: in range, in order, on character boundaries.
#[allow(unsafe_code)]
#[unsafe(export_name = "moddil_rt_is_between")]
pub extern "C" fn is_between(ptr: *const u8, len: u64, a: i64, b: i64) -> u8 {
    let s = text(ptr, len);
    let ok = usize::try_from(a)
        .ok()
        .zip(usize::try_from(b).ok())
        .is_some_and(|(a, b)| s.get(a..b).is_some());
    u8::from(ok)
}

/// How two texts compare, byte by byte: -1, 0 or 1.
#[allow(unsafe_code)]
#[unsafe(export_name = "moddil_rt_str_cmp")]
pub extern "C" fn str_cmp(p: *const u8, n: u64, q: *const u8, m: u64) -> i8 {
    match text(p, n).cmp(text(q, m)) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

/// A `char`'s class, by number: alphabetic, numeric, white space, upper and lower case.
#[allow(unsafe_code)]
#[unsafe(export_name = "moddil_rt_char_is")]
pub extern "C" fn char_is(which: u32, c: u32) -> u8 {
    let Some(c) = char::from_u32(c) else {
        return 0;
    };
    u8::from(match which {
        0 => c.is_alphabetic(),
        1 => c.is_numeric(),
        2 => c.is_whitespace(),
        3 => c.is_uppercase(),
        _ => c.is_lowercase(),
    })
}

/// How many allocations made with [`alloc`] are not yet freed, on this thread, not counting
/// those of `const` items' values (which live as long as the program).
pub fn live() -> i64 {
    run::with(|s| s.live)
}

/// Before a `const` item's value is computed.
#[allow(unsafe_code)]
#[unsafe(export_name = "moddil_rt_static_begin")]
pub extern "C" fn static_begin() {
    run::with(|s| s.static_from = s.live);
}

/// After it is computed: what it allocated lives as long as the program, so it is not counted.
#[allow(unsafe_code)]
#[unsafe(export_name = "moddil_rt_static_end")]
pub extern "C" fn static_end() {
    run::with(|s| s.live = s.static_from);
}

fn layout(size: u64, align: u64) -> std::alloc::Layout {
    std::alloc::Layout::from_size_align(size as usize, align.max(1) as usize)
        .expect("a layout the backend computed")
}

/// `size` bytes aligned to `align`: a `Buf`'s memory. Nothing (a dangling, aligned address) for
/// no bytes.
#[allow(unsafe_code)]
#[unsafe(export_name = "moddil_rt_alloc")]
pub extern "C" fn alloc(size: u64, align: u64) -> *mut u8 {
    if size == 0 {
        return std::ptr::without_provenance_mut(align.max(1) as usize);
    }
    run::with(|s| s.live += 1);
    let l = layout(size, align);
    #[allow(unsafe_code)]
    // SAFETY: `l` has a size other than zero.
    let p = unsafe { std::alloc::alloc(l) };
    if p.is_null() {
        std::alloc::handle_alloc_error(l);
    }
    p
}

/// Frees memory [`alloc`] gave, of `size` bytes aligned to `align`.
///
/// # Safety
///
/// `ptr` is what [`alloc`] (or [`realloc`]) gave for that size and alignment, not yet freed.
#[allow(unsafe_code)]
#[unsafe(export_name = "moddil_rt_free")]
pub unsafe extern "C" fn free(ptr: *mut u8, size: u64, align: u64) {
    if size == 0 {
        return;
    }
    run::with(|s| s.live -= 1);
    // SAFETY: as the caller says.
    unsafe {
        std::alloc::dealloc(ptr, layout(size, align));
    }
}

/// Moves memory [`alloc`] gave (`old` bytes) to memory of `new` bytes, keeping what fits.
///
/// # Safety
///
/// As [`free`]'s, for `ptr` of `old` bytes.
#[allow(unsafe_code)]
#[unsafe(export_name = "moddil_rt_realloc")]
pub unsafe extern "C" fn realloc(ptr: *mut u8, old: u64, new: u64, align: u64) -> *mut u8 {
    let q = alloc(new, align);
    // SAFETY: `ptr` holds `old` bytes and `q` room for `new`; they do not overlap. Then `ptr`
    // is freed as the caller says it may be.
    unsafe {
        std::ptr::copy_nonoverlapping(ptr, q, old.min(new) as usize);
        free(ptr, old, align);
    }
    q
}

/// Copies `n` bytes from `src` to `dst`, which may overlap.
///
/// # Safety
///
/// `src` has `n` readable bytes and `dst` `n` writable ones.
#[allow(unsafe_code)]
#[unsafe(export_name = "moddil_rt_memmove")]
pub unsafe extern "C" fn memmove(dst: *mut u8, src: *const u8, n: u64) {
    // SAFETY: as the caller says.
    unsafe {
        std::ptr::copy(src, dst, n as usize);
    }
}

/// Every function native code calls, by the name it is exported as: what a JIT binds, and what
/// a built program links against.
pub fn symbols() -> Vec<(&'static str, *const u8)> {
    vec![
        ("moddil_rt_trap", trap as *const u8),
        // A Moddil function given C as a function pointer starts and traps as an export does
        // (`type.fnptr.1`).
        ("moddil_rt_export_begin", export_begin as *const u8),
        ("moddil_rt_export_trapped", export_trapped as *const u8),
        ("moddil_rt_callback_trapped", callback_trapped as *const u8),
        ("moddil_trapped", trapped as *const u8),
        ("moddil_rt_trap_tracked", trap_tracked as *const u8),
        ("moddil_rt_fail_at", fail_at as *const u8),
        ("moddil_rt_observe", observe as *const u8),
        ("moddil_rt_enter", enter as *const u8),
        ("moddil_rt_leave", leave as *const u8),
        ("moddil_rt_fmod", fmod as *const u8),
        ("moddil_rt_fmodf", fmodf as *const u8),
        ("moddil_rt_i128_lo", i128_lo as *const u8),
        ("moddil_rt_i128_hi", i128_hi as *const u8),
        ("moddil_rt_float_fn", float_fn as *const u8),
        ("moddil_rt_float_fn_f32", float_fn_f32 as *const u8),
        ("moddil_rt_char_at_byte", char_at_byte as *const u8),
        ("moddil_rt_is_between", is_between as *const u8),
        ("moddil_rt_str_cmp", str_cmp as *const u8),
        ("moddil_rt_char_is", char_is as *const u8),
        ("moddil_rt_alloc", alloc as *const u8),
        ("moddil_rt_free", free as *const u8),
        ("moddil_rt_realloc", realloc as *const u8),
        ("moddil_rt_memmove", memmove as *const u8),
        ("moddil_rt_static_begin", static_begin as *const u8),
        ("moddil_rt_static_end", static_end as *const u8),
        ("moddil_rt_convert", host::convert as *const u8),
        ("moddil_rt_text", host::text as *const u8),
        ("moddil_rt_log", host::log as *const u8),
        ("moddil_rt_trapped", host::trapped as *const u8),
        ("moddil_rt_scope_open", scope_open as *const u8),
        ("moddil_rt_spawn", spawn as *const u8),
        ("moddil_rt_scope_run", scope_run as *const u8),
        ("moddil_rt_stop", stop as *const u8),
        ("moddil_rt_join", join as *const u8),
        ("moddil_rt_detach", detach as *const u8),
        ("moddil_rt_yield_now", yield_now as *const u8),
        ("moddil_rt_task_id", task_id as *const u8),
        ("moddil_rt_park", park as *const u8),
        ("moddil_rt_unpark", unpark as *const u8),
        (
            "moddil_rt_task",
            std::ptr::addr_of!(run::RUNNING).cast::<u8>(),
        ),
        ("moddil_rt_stop_requested", stop_requested as *const u8),
        ("moddil_rt_channel", channel as *const u8),
        ("moddil_rt_timer", timer as *const u8),
        ("moddil_rt_os_arg_count", os_arg_count as *const u8),
        ("moddil_rt_os_arg_len", os_arg_len as *const u8),
        ("moddil_rt_os_arg_copy", os_arg_copy as *const u8),
        ("moddil_rt_os_write_stderr", os_write_stderr as *const u8),
        ("moddil_rt_os_random", os_random as *const u8),
        ("moddil_rt_os_wall_clock", os_wall_clock as *const u8),
        ("moddil_rt_gpu_dispatch", gpu_dispatch as *const u8),
        ("moddil_rt_gpu_frame", gpu_frame as *const u8),
        ("moddil_rt_gpu_show", gpu_show as *const u8),
        ("moddil_rt_os_listen", os_listen as *const u8),
        ("moddil_rt_os_connect", os_connect as *const u8),
        ("moddil_rt_os_channel", os_channel as *const u8),
        ("moddil_rt_os_take", os_take as *const u8),
        ("moddil_rt_os_send", os_send as *const u8),
        ("moddil_rt_os_close", os_close as *const u8),
        ("moddil_rt_clone_sender", clone_sender as *const u8),
        ("moddil_rt_drop_sender", drop_sender as *const u8),
        ("moddil_rt_drop_receiver", drop_receiver as *const u8),
        ("moddil_rt_send", send as *const u8),
        ("moddil_rt_recv", recv as *const u8),
        ("moddil_rt_select", select as *const u8),
        ("moddil_rt_recv_now", recv_now as *const u8),
        ("moddil_rt_send_now", send_now as *const u8),
        ("moddil_rt_join_now", join_now as *const u8),
    ]
}

/// Takes what this thread's runs have recorded since the last time, leaving it as a new run's.
pub fn take() -> Record {
    run::with(|s| Record {
        trap: s.trap.take(),
        failed: s.failed.take(),
    })
}

/// This crate's version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    thread_local! {
        static SEEN: Cell<u32> = const { Cell::new(0) };
    }

    fn seen(_: *mut (), site: u32, _: *const u8) {
        SEEN.with(|s| s.set(site));
    }

    #[test]
    fn records_and_takes() {
        set_observer(Some((seen, std::ptr::null_mut())));
        observe(7, std::ptr::null());
        set_observer(None);
        assert_eq!(SEEN.with(Cell::get), 7);
        take();
        assert_eq!(enter(5, 0), 0);
        trap_tracked(3);
        trap(4);
        let r = take();
        assert_eq!(
            r.trap,
            Some(Trapped {
                site: 3,
                tracked: true,
                stack: vec![5]
            })
        );
        assert_eq!(take(), Record::default());
        assert_eq!(fmod(7.5, 2.0), 1.5);
        assert_eq!(float_fn(0, 9.0, 0.0), 3.0);
    }

    #[test]
    fn counts_frames() {
        take();
        for _ in 1..MAX_DEPTH {
            assert_eq!(enter(0, 0), 0);
        }
        assert_eq!(enter(9, 0), 1);
        leave();
        assert_eq!(enter(0, 0), 0);
        assert_eq!(take().trap.map(|t| t.site), Some(9));
    }

    #[allow(unsafe_code)]
    extern "C" fn sender(captures: *mut u8, _: u64, _: *mut u8, _: *mut u8) -> i8 {
        // SAFETY: the captures are the channel's number.
        let c = unsafe { *captures.cast::<u64>() };
        let v = 41u64;
        let status = send(c, (&raw const v).cast(), 8, None, 0, 0);
        drop_sender(c);
        status as i8
    }

    #[test]
    #[allow(unsafe_code)]
    fn tasks_take_turns() {
        begin_run();
        let s = scope_open(0);
        let c = channel(0);
        let captures = alloc(8, 8);
        // SAFETY: room for the channel's number.
        unsafe { *captures.cast::<u64>() = c };
        let spec = TaskSpec {
            code: sender,
            captures,
            size: 8,
            align: 8,
            arg: 0,
            out: 0,
            err: 0,
            drop: None,
        };
        // SAFETY: `spec` is a task to start.
        let t = unsafe { spawn(s, &raw const spec) };
        let mut got = 0u64;
        // The sender runs once this task waits; a rendezvous, so it waits for the value to go.
        assert_eq!(recv(c, (&raw mut got).cast(), 0, 0), 0);
        assert_eq!(got, 41);
        assert_eq!(join(t, std::ptr::null_mut(), std::ptr::null_mut(), 0, 0), 0);
        // Its sender is gone: `None`.
        assert_eq!(recv(c, (&raw mut got).cast(), 0, 0), 5);
        end_run();
        assert_eq!(live(), 0);
    }

    #[test]
    fn allocations_are_counted() {
        let before = live();
        let p = alloc(16, 8);
        #[allow(unsafe_code)]
        // SAFETY: each pointer is freed once, with the size it was given for.
        unsafe {
            let q = realloc(p, 16, 32, 8);
            assert_eq!(live(), before + 1);
            free(q, 32, 8);
            let z = alloc(0, 8);
            free(z, 0, 8);
        }
        assert_eq!(live(), before);
    }

    #[test]
    fn text_helpers() {
        let s = "naïve";
        assert_eq!(char_at_byte(s.as_ptr(), s.len() as u64, 2), u32::from('ï'));
        assert_eq!(char_at_byte(s.as_ptr(), s.len() as u64, 3), 0x11_0000);
        assert_eq!(is_between(s.as_ptr(), s.len() as u64, 0, 3), 0);
        assert_eq!(str_cmp("a".as_ptr(), 1, "b".as_ptr(), 1), -1);
    }
}

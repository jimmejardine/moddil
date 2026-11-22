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
//! What they record is kept per thread until the host [`take`]s it.

use std::cell::{Cell, RefCell};

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

/// What a run recorded: the trap, if one happened.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Record {
    /// The trap, if one happened.
    pub trap: Option<Trapped>,
    /// The call sites the run is in, innermost last.
    stack: Vec<u32>,
}

/// What the host gives to see observed values: a function and the context it is called with,
/// with each observation's site and the address of its value.
pub type Observer = (fn(*mut (), u32, *const u8), *mut ());

thread_local! {
    static RECORD: RefCell<Record> = RefCell::new(Record::default());
    static OBSERVER: Cell<Option<Observer>> = const { Cell::new(None) };
}

/// Installs (or, with `None`, removes) this thread's observer.
pub fn set_observer(o: Option<Observer>) {
    OBSERVER.with(|c| c.set(o));
}

fn record_trap(site: u32, tracked: bool) {
    RECORD.with(|r| {
        let mut r = r.borrow_mut();
        if r.trap.is_none() {
            let stack = r.stack.clone();
            r.trap = Some(Trapped {
                site,
                tracked,
                stack,
            });
        }
    });
}

/// Records that the check at `site` failed. The code that calls it returns a trapped status.
pub extern "C" fn trap(site: u32) {
    record_trap(site, false);
}

/// As [`trap`], in tracked code: `#[track_caller]`, the prelude's or derived code.
pub extern "C" fn trap_tracked(site: u32) {
    record_trap(site, true);
}

/// Hands the value at `addr` of the observed expression at `site` to the observer, if there is
/// one.
pub extern "C" fn observe(site: u32, addr: *const u8) {
    if let Some((f, cx)) = OBSERVER.with(Cell::get) {
        f(cx, site, addr);
    }
}

/// Before the call at `site`: 0 if it may go a frame deeper, or 1, recording the stack
/// overflow as a trap at `site`, if it may not. `tracked` says whether the calling code is.
pub extern "C" fn enter(site: u32, tracked: u8) -> u8 {
    let full = RECORD.with(|r| r.borrow().stack.len() as u32 + 1 >= MAX_DEPTH);
    if full {
        record_trap(site, tracked != 0);
        return 1;
    }
    RECORD.with(|r| r.borrow_mut().stack.push(site));
    0
}

/// After a call returns, however it returns.
pub extern "C" fn leave() {
    RECORD.with(|r| {
        r.borrow_mut().stack.pop();
    });
}

/// `a % b` on `f64`s, as IEEE 754 and the interpreters compute it.
pub extern "C" fn fmod(a: f64, b: f64) -> f64 {
    libm::fmod(a, b)
}

/// `a % b` on `f32`s.
pub extern "C" fn fmodf(a: f32, b: f32) -> f32 {
    libm::fmodf(a, b)
}

/// The float functions, by number ([`FLOAT_FNS`]), on `f64`s.
pub extern "C" fn float_fn(which: u32, x: f64, y: f64) -> f64 {
    match FLOAT_FNS.get(which as usize).copied().unwrap_or("") {
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
        _ => f64::NAN,
    }
}

/// The same on `f32`s, by `libm`'s `f32` functions, as the interpreters compute them.
pub extern "C" fn float_fn_f32(which: u32, x: f32, y: f32) -> f32 {
    match FLOAT_FNS.get(which as usize).copied().unwrap_or("") {
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
        _ => f32::NAN,
    }
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
pub extern "C" fn char_at_byte(ptr: *const u8, len: u64, at: i64) -> u32 {
    let s = text(ptr, len);
    usize::try_from(at)
        .ok()
        .and_then(|at| s.get(at..))
        .and_then(|rest| rest.chars().next())
        .map_or(0x11_0000, u32::from)
}

/// Whether bytes `a` to `b` of the text are text: in range, in order, on character boundaries.
pub extern "C" fn is_between(ptr: *const u8, len: u64, a: i64, b: i64) -> u8 {
    let s = text(ptr, len);
    let ok = usize::try_from(a)
        .ok()
        .zip(usize::try_from(b).ok())
        .is_some_and(|(a, b)| s.get(a..b).is_some());
    u8::from(ok)
}

/// How two texts compare, byte by byte: -1, 0 or 1.
pub extern "C" fn str_cmp(p: *const u8, n: u64, q: *const u8, m: u64) -> i8 {
    match text(p, n).cmp(text(q, m)) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

/// A `char`'s class, by number: alphabetic, numeric, white space, upper and lower case.
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

thread_local! {
    /// How many allocations are live.
    static LIVE: Cell<i64> = const { Cell::new(0) };
}

/// How many allocations made with [`alloc`] are not yet freed, on this thread.
pub fn live() -> i64 {
    LIVE.with(Cell::get)
}

fn layout(size: u64, align: u64) -> std::alloc::Layout {
    std::alloc::Layout::from_size_align(size as usize, align.max(1) as usize)
        .expect("a layout the backend computed")
}

/// `size` bytes aligned to `align`: a `Buf`'s memory. Nothing (a dangling, aligned address) for
/// no bytes.
pub extern "C" fn alloc(size: u64, align: u64) -> *mut u8 {
    if size == 0 {
        return std::ptr::without_provenance_mut(align.max(1) as usize);
    }
    LIVE.with(|l| l.set(l.get() + 1));
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
pub unsafe extern "C" fn free(ptr: *mut u8, size: u64, align: u64) {
    if size == 0 {
        return;
    }
    LIVE.with(|l| l.set(l.get() - 1));
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
pub unsafe extern "C" fn memmove(dst: *mut u8, src: *const u8, n: u64) {
    // SAFETY: as the caller says.
    unsafe {
        std::ptr::copy(src, dst, n as usize);
    }
}

/// Takes what this thread's runs have recorded since the last time, leaving it as a new run's.
pub fn take() -> Record {
    RECORD.with(|r| std::mem::take(&mut *r.borrow_mut()))
}

/// This crate's version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
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

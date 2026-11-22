//! `moddil-rt`: the runtime that native Moddil code calls into.
//!
//! JIT-compiled code is given the addresses of these functions.
//!
//! - A failed check calls [`trap`] with its site, then returns a "trapped" status that each
//!   caller passes on (the status-return ABI, `specs/compiler.md`).
//! - A test's `// =>` expression calls [`observe`] with its value's address, which hands it to
//!   the observer the host installed ([`set_observer`]) while the value is still there to read.
//! - Each call goes through [`enter`] and [`leave`], which count the frames, so that recursion
//!   without end traps as the interpreters' does, rather than overflowing the native stack.
//!
//! What they record is kept per thread until the host [`take`]s it.

use std::cell::{Cell, RefCell};

/// How many calls deep a run may go, the frame it starts in included: as the MIR interpreter's
/// limit (`moddil_interp::MAX_DEPTH`).
pub const MAX_DEPTH: u32 = 10_000;

/// What a run recorded: the trap, if one happened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    /// The site of the check that failed, if one did.
    pub trap: Option<u32>,
    /// How many frames deep the run is.
    depth: u32,
}

impl Default for Record {
    fn default() -> Self {
        Record {
            trap: None,
            depth: 1,
        }
    }
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

/// Records that the check at `site` failed. The code that calls it returns a trapped status.
pub extern "C" fn trap(site: u32) {
    RECORD.with(|r| {
        r.borrow_mut().trap.get_or_insert(site);
    });
}

/// Hands the value at `addr` of the observed expression at `site` to the observer, if there is
/// one.
pub extern "C" fn observe(site: u32, addr: *const u8) {
    if let Some((f, cx)) = OBSERVER.with(Cell::get) {
        f(cx, site, addr);
    }
}

/// Before the call at `site`: 0 if it may go a frame deeper, or 1, recording the stack
/// overflow as a trap at `site`, if it may not.
pub extern "C" fn enter(site: u32) -> u8 {
    RECORD.with(|r| {
        let mut r = r.borrow_mut();
        if r.depth >= MAX_DEPTH {
            r.trap.get_or_insert(site);
            1
        } else {
            r.depth += 1;
            0
        }
    })
}

/// After a call returns, however it returns.
pub extern "C" fn leave() {
    RECORD.with(|r| {
        let mut r = r.borrow_mut();
        r.depth = r.depth.saturating_sub(1);
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
        trap(3);
        trap(4);
        let r = take();
        assert_eq!(r.trap, Some(3));
        assert_eq!(take(), Record::default());
        assert_eq!(fmod(7.5, 2.0), 1.5);
    }

    #[test]
    fn counts_frames() {
        take();
        for _ in 1..MAX_DEPTH {
            assert_eq!(enter(0), 0);
        }
        assert_eq!(enter(9), 1);
        leave();
        assert_eq!(enter(0), 0);
        assert_eq!(take().trap, Some(9));
    }
}

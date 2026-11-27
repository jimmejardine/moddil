//! The allocator in scope (`alloc.with.1`): each task's stack of the allocators `with Alloc`
//! installs, each a `&dyn Alloc` (its data and vtable words), innermost last. It is kept in the
//! run's state by task, since a task may move between threads; a spawned task starts with its
//! spawner's innermost one (`alloc.task.1`).

use crate::run::{State, me, with};

/// `__alloc_push`: `with Alloc = a` installs `a`, whose `&dyn Alloc` is `data` and `vtable`.
#[allow(unsafe_code)]
#[unsafe(export_name = "moddil_rt_alloc_push")]
pub(crate) extern "C" fn alloc_push(data: u64, vtable: u64) {
    let t = me();
    with(|s| s.allocs.entry(t).or_default().push([data, vtable]));
}

/// `__alloc_pop`: the innermost one's block is left.
#[allow(unsafe_code)]
#[unsafe(export_name = "moddil_rt_alloc_pop")]
pub(crate) extern "C" fn alloc_pop() {
    let t = me();
    with(|s| {
        if let Some(v) = s.allocs.get_mut(&t) {
            v.pop();
        }
    });
}

/// `__alloc_top`: the innermost one, as an `Option<&dyn Alloc>` at `out` (two zero words for
/// `None`).
///
/// # Safety
///
/// `out` has room for two words.
#[allow(unsafe_code)]
#[unsafe(export_name = "moddil_rt_alloc_top")]
pub(crate) unsafe extern "C" fn alloc_top(out: *mut u64) {
    let t = me();
    let top = with(|s| s.allocs.get(&t).and_then(|v| v.last()).copied()).unwrap_or([0, 0]);
    // SAFETY: as the caller says.
    unsafe {
        *out = top[0];
        *out.add(1) = top[1];
    }
}

/// Task `t`, started by task `me`, starts with `me`'s allocator in scope (`alloc.task.1`).
pub(crate) fn inherit(s: &mut State, me: usize, t: usize) {
    match s.allocs.get(&me).and_then(|v| v.last()).copied() {
        Some(top) => {
            s.allocs.insert(t, vec![top]);
        }
        None => {
            s.allocs.remove(&t);
        }
    }
}

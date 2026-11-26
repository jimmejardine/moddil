//! The baton (`task.sched.1`): how the one task that runs hands the turn to another and waits
//! for its own, how a task starts, and how the code of a task and the drop glue of a value are
//! called. Everything else about tasks ([`crate::run`], [`crate::sched`]) is the same on every
//! target; this is what is not.
//!
//! - **Threads** (native code): each task is a thread, waiting on the run's condition variable
//!   until it holds the baton.
//! - **JSPI** (`wasm32`): each task is a call the host started as a promising one; handing over
//!   calls the host's suspending `moddil_host_switch`, which resumes the next task's call and
//!   returns once this one's turn comes again. The code and the glue are slots in the program's
//!   table, called through the shim module the program is linked with.

#![allow(unsafe_code)]

#[cfg(not(target_arch = "wasm32"))]
pub(crate) use threads::*;

#[cfg(target_arch = "wasm32")]
pub(crate) use jspi::*;

#[cfg(target_arch = "wasm32")]
pub use jspi::{DropFn, TaskFn, task};
#[cfg(not(target_arch = "wasm32"))]
pub use threads::{DropFn, TaskFn};

#[cfg(not(target_arch = "wasm32"))]
mod threads {
    use std::sync::{Arc, MutexGuard};

    use crate::run::{Ctx, Start, State, enter_task, me, run_task, running};

    /// How much stack a task's thread has.
    const TASK_STACK: usize = 256 * 1024 * 1024;

    /// A task's code: its closure's body, given its captures and its argument, writing its
    /// result to `out` and an error to `err`, in the platform's C convention: its status.
    pub type TaskFn = extern "C" fn(*mut u8, u64, *mut u8, *mut u8) -> i8;

    /// A function that drops the value at its argument (the value's drop glue, in the
    /// platform's C convention): 0, or the status of a trap in a destructor.
    pub type DropFn = extern "C" fn(*mut u8) -> i8;

    /// What the baton keeps for a run: its tasks' threads.
    #[derive(Default)]
    pub(crate) struct Baton {
        threads: Vec<std::thread::JoinHandle<()>>,
    }

    /// Starts task `t`'s thread, which waits for its turn and then runs.
    pub(crate) fn start(s: &mut State, c: &Arc<Ctx>, t: usize, spec: Start) {
        let c = c.clone();
        let thread = std::thread::Builder::new()
            .stack_size(TASK_STACK)
            .spawn(move || {
                enter_task(&c, t);
                {
                    let mut s = c.lock();
                    while s.current != t && !s.aborted {
                        s = c
                            .turn
                            .wait(s)
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                    }
                    if s.aborted {
                        return;
                    }
                }
                running(t);
                run_task(&c, t, spec);
            })
            .expect("a thread for a task");
        s.baton.threads.push(thread);
    }

    /// Gives the turn to task `next` and waits for this one's: 0, or 1 if the run is aborted.
    pub(crate) fn hand_over(c: &Ctx, mut s: MutexGuard<'_, State>, next: usize) -> u8 {
        let me = me();
        s.current = next;
        c.turn.notify_all();
        while s.current != me && !s.aborted {
            s = c
                .turn
                .wait(s)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        running(me);
        u8::from(s.aborted)
    }

    /// Wakes whoever the state says goes on: the task holding the turn, or, in an aborted run,
    /// every one waiting.
    pub(crate) fn pass_on(c: &Ctx, _: &State) {
        c.turn.notify_all();
    }

    /// Waits for the run's threads to end.
    pub(crate) fn end_run(s: &mut State) -> Vec<std::thread::JoinHandle<()>> {
        std::mem::take(&mut s.baton.threads)
    }

    /// Runs a task's code.
    pub(crate) fn call_code(spec: &Start, out: *mut u8, err: *mut u8) -> i8 {
        (spec.code)(spec.captures as *mut u8, spec.arg, out, err)
    }

    /// Drops the value at `p` with glue `d`.
    pub(crate) fn call_drop(d: DropFn, p: *mut u8) -> i8 {
        d(p)
    }

    /// `drop`, the `TaskSpec` word naming a task's drop glue, as the glue.
    pub(crate) fn drop_of(d: Option<DropFn>) -> Option<DropFn> {
        d
    }
}

#[cfg(target_arch = "wasm32")]
mod jspi {
    use std::collections::HashMap;
    use std::num::NonZeroU32;
    use std::sync::{Arc, MutexGuard};

    use crate::run::{Ctx, STACK, Start, State, ctx, enter_task, me, run_task};

    /// A task's code: its slot in the program's table.
    pub type TaskFn = u64;

    /// A value's drop glue: its slot in the program's table.
    pub type DropFn = NonZeroU32;

    #[link(wasm_import_module = "host")]
    unsafe extern "C" {
        /// Resumes task `next` (starting it, if it has not run), and returns once this task's
        /// turn comes again: 0, or 1 if the run is aborted. It suspends this task's call.
        fn moddil_host_switch(next: u32) -> u32;
        /// Resumes task `next` once the running task's call has returned.
        fn moddil_host_resume(next: u32);
        /// Resumes every task waiting, to unwind: the run is aborted.
        fn moddil_host_abort();
    }

    #[link(wasm_import_module = "shim")]
    unsafe extern "C" {
        /// Calls the task code in slot `code`.
        fn moddil_call_task(code: u64, captures: u64, arg: u64, out: *mut u8, err: *mut u8) -> i32;
        /// Calls the drop glue in slot `f`.
        fn moddil_call_drop(f: u32, p: *mut u8) -> i32;
    }

    /// What the baton keeps for a run: the tasks not yet started, and the call-site stacks of
    /// those waiting.
    #[derive(Default)]
    pub(crate) struct Baton {
        pending: HashMap<usize, Start>,
        stacks: HashMap<usize, Vec<u32>>,
    }

    /// Queues task `t`: the host starts its call when it first gets the turn.
    pub(crate) fn start(s: &mut State, _: &Arc<Ctx>, t: usize, spec: Start) {
        s.baton.pending.insert(t, spec);
    }

    /// Gives the turn to task `next` and waits for this one's: 0, or 1 if the run is aborted.
    pub(crate) fn hand_over(c: &Ctx, mut s: MutexGuard<'_, State>, next: usize) -> u8 {
        let me = me();
        s.current = next;
        let stack = STACK.with(|st| std::mem::take(&mut *st.borrow_mut()));
        s.baton.stacks.insert(me, stack);
        drop(s);
        // SAFETY: the host's function takes a task's number.
        let r = unsafe { moddil_host_switch(next as u32) };
        enter_task(&ctx(), me);
        let mut s = c.lock();
        let stack = s.baton.stacks.remove(&me).unwrap_or_default();
        STACK.with(|st| *st.borrow_mut() = stack);
        u8::from(r != 0 || s.aborted)
    }

    /// Asks the host to go on with whoever the state says: the task holding the turn, or, in
    /// an aborted run, every one waiting.
    pub(crate) fn pass_on(_: &Ctx, s: &State) {
        // SAFETY: the host's functions take a task's number, or nothing.
        unsafe {
            if s.aborted {
                moddil_host_abort();
            } else {
                moddil_host_resume(s.current as u32);
            }
        }
    }

    /// Nothing waits for a run's end: its tasks are calls the host has.
    pub(crate) fn end_run(_: &mut State) -> Vec<std::thread::JoinHandle<()>> {
        Vec::new()
    }

    /// Runs a task's code.
    pub(crate) fn call_code(spec: &Start, out: *mut u8, err: *mut u8) -> i8 {
        // SAFETY: the shim calls the program's code in the slot the program gave.
        unsafe { moddil_call_task(spec.code, spec.captures as u64, spec.arg, out, err) as i8 }
    }

    /// Drops the value at `p` with glue `d`.
    pub(crate) fn call_drop(d: DropFn, p: *mut u8) -> i8 {
        // SAFETY: the shim calls the program's glue in the slot the program gave.
        unsafe { moddil_call_drop(d.get(), p) as i8 }
    }

    /// `drop`, the `TaskSpec` word naming a task's drop glue, as the glue.
    pub(crate) fn drop_of(d: u64) -> Option<DropFn> {
        NonZeroU32::new(d as u32)
    }

    /// Task `t`'s call, which the host starts the first time `t` gets the turn: its code, run
    /// to its end.
    pub fn task(t: usize) {
        let c = ctx();
        enter_task(&c, t);
        STACK.with(|st| st.borrow_mut().clear());
        let Some(spec) = c.lock().baton.pending.remove(&t) else {
            return;
        };
        run_task(&c, t, spec);
    }
}

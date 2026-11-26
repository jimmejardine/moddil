//! The baton (`task.sched.1`): how the one task that runs hands the turn to another and waits
//! for its own, how a task starts, and how the code of a task and the drop glue of a value are
//! called. Everything else about tasks ([`crate::run`], [`crate::sched`]) is the same on every
//! target; this is what is not.
//!
//! - **Workers** (native code): each task but the run's own is a coroutine (`corosensei`), with
//!   a stack of its own, which a worker thread resumes once the task has a turn and which gives
//!   the worker back where it waits for its turn again; there is a worker for each turn. The run's
//!   own task runs on the thread that began the run, waiting on the run's condition variable.
//! - **JSPI** (`wasm32`): each task is a call the host started as a promising one; handing over
//!   calls the host's suspending `moddil_host_switch`, which resumes the next task's call and
//!   returns once this one's turn comes again. The code and the glue are slots in the program's
//!   table, called through the shim module the program is linked with.

#![allow(unsafe_code)]

#[cfg(not(target_arch = "wasm32"))]
pub(crate) use workers::*;

#[cfg(target_arch = "wasm32")]
pub(crate) use jspi::*;

#[cfg(target_arch = "wasm32")]
pub use jspi::{DropFn, TaskFn, task};
#[cfg(not(target_arch = "wasm32"))]
pub use workers::{DropFn, TaskFn};

#[cfg(not(target_arch = "wasm32"))]
mod workers {
    use std::cell::Cell;
    use std::collections::{HashMap, HashSet, VecDeque};
    use std::sync::{Arc, MutexGuard};

    use corosensei::stack::DefaultStack;
    use corosensei::{Coroutine, CoroutineResult, Yielder};

    use crate::run::{Ctx, Start, State, TaskLocals, enter_task, me, run_task, running};

    /// How much stack a task reserves: committed as it is used, with a guard page below.
    const TASK_STACK: usize = 256 * 1024 * 1024;

    /// The run's own task, which runs on the thread that began the run, not on a worker.
    const HOST: usize = 0;

    /// A task's code: its closure's body, given its captures and its argument, writing its
    /// result to `out` and an error to `err`, in the platform's C convention: its status.
    pub type TaskFn = extern "C" fn(*mut u8, u64, *mut u8, *mut u8) -> i8;

    /// A function that drops the value at its argument (the value's drop glue, in the
    /// platform's C convention): 0, or the status of a trap in a destructor.
    pub type DropFn = extern "C" fn(*mut u8) -> i8;

    /// Why a task's coroutine gives its worker back.
    enum Pause {
        /// It waits for its turn.
        Wait,
        /// The run is over and the task runs no more of its code: its coroutine is left.
        Abandon,
    }

    /// A task's coroutine, and what it keeps of its thread's state while it is not running.
    struct Task {
        co: Coroutine<(), Pause, (), DefaultStack>,
        locals: TaskLocals,
    }

    // SAFETY: a coroutine runs on one worker at a time, which the baton's lock hands it to;
    // nothing on its stack refers to a thread's locals across a suspension (the runtime reads
    // them afresh, and compiled code keeps none).
    unsafe impl Send for Task {}

    /// What the baton keeps for a run: its tasks' coroutines, those with a turn waiting for a
    /// worker, and the workers.
    #[derive(Default)]
    pub(crate) struct Baton {
        /// The tasks not yet started.
        starts: HashMap<usize, Start>,
        /// The tasks suspended.
        tasks: HashMap<usize, Task>,
        /// The tasks with a turn, waiting for a worker to run them, in the order given.
        queue: VecDeque<usize>,
        /// The tasks a worker runs now.
        busy: HashSet<usize>,
        workers: Vec<std::thread::JoinHandle<()>>,
        /// Whether the run has ended, and the workers with it.
        over: bool,
    }

    thread_local! {
        /// On a worker, the running coroutine's yielder: what a task suspends through.
        static YIELDER: Cell<*const Yielder<(), Pause>> = const { Cell::new(std::ptr::null()) };
    }

    /// Queues task `t`, to start when it gets a turn; and starts a worker for each turn.
    pub(crate) fn start(s: &mut State, c: &Arc<Ctx>, t: usize, spec: Start) {
        s.baton.starts.insert(t, spec);
        while s.baton.workers.len() < s.turns.slots() {
            let c = c.clone();
            let w = std::thread::Builder::new()
                .name("moddil-worker".to_string())
                .spawn(move || work(&c))
                .expect("a worker thread");
            s.baton.workers.push(w);
        }
    }

    /// A worker: runs the tasks given turns until the run ends.
    fn work(c: &Arc<Ctx>) {
        loop {
            let (t, mut task) = {
                let mut s = c.lock();
                loop {
                    if s.baton.over {
                        return;
                    }
                    if let Some(t) = s.baton.queue.pop_front() {
                        if let Some(spec) = s.baton.starts.remove(&t) {
                            let c = c.clone();
                            let stack = DefaultStack::new(TASK_STACK).expect("a task's stack");
                            let co =
                                Coroutine::with_stack(stack, move |y: &Yielder<(), Pause>, ()| {
                                    set_yielder(y);
                                    run_task(&c, t, spec);
                                });
                            s.baton.busy.insert(t);
                            break (
                                t,
                                Task {
                                    co,
                                    locals: TaskLocals::default(),
                                },
                            );
                        }
                        if let Some(task) = s.baton.tasks.remove(&t) {
                            s.baton.busy.insert(t);
                            break (t, task);
                        }
                        // Running elsewhere, about to suspend: that worker queues it again.
                        continue;
                    }
                    s = c
                        .turn
                        .wait(s)
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                }
            };
            enter_task(c, t);
            running(t);
            crate::run::restore_locals(std::mem::take(&mut task.locals));
            let r = task.co.resume(());
            set_yielder(std::ptr::null());
            task.locals = crate::run::save_locals();
            let mut s = c.lock();
            s.baton.busy.remove(&t);
            match r {
                CoroutineResult::Return(()) => {}
                // Left, not dropped: its stack holds frames that must not be unwound.
                CoroutineResult::Yield(Pause::Abandon) => std::mem::forget(task),
                CoroutineResult::Yield(Pause::Wait) => {
                    // Given a turn again while it suspended, or to unwind an aborted run.
                    let again = s.turns.has(t) || s.aborted;
                    s.baton.tasks.insert(t, task);
                    if again {
                        s.baton.queue.push_back(t);
                        c.turn.notify_all();
                    }
                }
            }
        }
    }

    /// Suspends the running task's coroutine for `why`, giving its worker back. It may resume
    /// on another worker: what it reads of its thread's locals after that, it reads afresh, in
    /// functions of their own, as the compiler may keep a thread local's address within one.
    fn pause(why: Pause) {
        let y = yielder();
        // SAFETY: the yielder is the running coroutine's, which lives while it runs.
        unsafe { (*y).suspend(why) };
        set_yielder(y);
    }

    /// This thread's running coroutine's yielder (null if none).
    #[inline(never)]
    fn yielder() -> *const Yielder<(), Pause> {
        YIELDER.with(Cell::get)
    }

    /// Makes `y` this thread's running coroutine's yielder.
    #[inline(never)]
    fn set_yielder(y: *const Yielder<(), Pause>) {
        YIELDER.with(|x| x.set(y));
    }

    /// Whether the running code is a task's coroutine on a worker.
    fn on_worker() -> bool {
        !yielder().is_null()
    }

    /// Wakes the tasks given turns, and waits for this one's: 0, or 1 if the run is aborted.
    pub(crate) fn wait_turn(c: &Ctx, mut s: MutexGuard<'_, State>) -> u8 {
        let me = me();
        pass_on(c, &mut s);
        if !on_worker() {
            while !s.turns.has(me) && !s.aborted {
                s = c
                    .turn
                    .wait(s)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
            }
            running(me);
            return u8::from(s.aborted);
        }
        if s.turns.has(me) || s.aborted {
            return u8::from(s.aborted);
        }
        drop(s);
        pause(Pause::Wait);
        u8::from(c.lock().aborted)
    }

    /// Hands the tasks given turns to the workers (the run's own task waits on its thread), and
    /// wakes whoever goes on; in an aborted run, every task waiting, to unwind.
    pub(crate) fn pass_on(c: &Ctx, s: &mut State) {
        let b = &mut s.baton;
        let mut go: Vec<usize> = if s.aborted {
            b.tasks.keys().copied().collect()
        } else {
            s.turns.holders().filter(|&t| t != HOST).collect()
        };
        go.sort_unstable();
        for t in go {
            if !b.busy.contains(&t) && !b.queue.contains(&t) {
                b.queue.push_back(t);
            }
        }
        c.turn.notify_all();
    }

    /// At a safe point of an aborted run: a task on a worker runs no more of its code. The
    /// run's own task, on the thread that began the run, goes on to its next call of the
    /// runtime, which gives it the trapped status.
    pub(crate) fn abandon() {
        if on_worker() {
            pause(Pause::Abandon);
        }
    }

    /// Ends the run's workers: the threads to wait for.
    pub(crate) fn end_run(s: &mut State) -> Vec<std::thread::JoinHandle<()>> {
        s.baton.over = true;
        std::mem::take(&mut s.baton.workers)
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

    /// Goes on with the task holding the one turn, and waits for this one's: 0, or 1 if the
    /// run is aborted.
    pub(crate) fn wait_turn(c: &Ctx, mut s: MutexGuard<'_, State>) -> u8 {
        let me = me();
        let next = s.turns.first().unwrap_or(me);
        if next == me {
            return u8::from(s.aborted);
        }
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
    pub(crate) fn pass_on(_: &Ctx, s: &mut State) {
        // SAFETY: the host's functions take a task's number, or nothing.
        unsafe {
            if s.aborted {
                moddil_host_abort();
            } else {
                moddil_host_resume(s.turns.first().unwrap_or(0) as u32);
            }
        }
    }

    /// Nothing waits for a run's end: its tasks are calls the host has.
    pub(crate) fn end_run(_: &mut State) -> Vec<std::thread::JoinHandle<()>> {
        Vec::new()
    }

    /// At a safe point of an aborted run (WASM has none: its code checks for none).
    pub(crate) fn abandon() {}

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

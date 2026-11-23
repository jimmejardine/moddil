//! A run: the state its tasks share, and its tasks (`task.sched.1`).
//!
//! Only the task holding the baton (the running task's number) runs; a task that suspends hands
//! it to the next ready one, as the scheduler says, and waits for its turn ([`crate::baton`]:
//! a thread per task natively, a JSPI call per task on `wasm32`). So one task runs at a time, in
//! the interpreters' order. A trap outside an isolated scope aborts the run: each task waiting
//! is woken to return a trapped status, running no more of its code.
//!
//! The statuses the code and these functions use: 0 returned, 1 trapped (the run ends),
//! 2 failed (an error), 3 unwinding (cancelled, or a trap in an isolated scope).

#![allow(unsafe_code)]

use std::cell::{Cell, RefCell};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use crate::baton::{self, Baton, DropFn, TaskFn};
use crate::sched::{Failure, Op, Outcome, Scheduler, Wait};
use crate::{Observer, Trapped};

/// What a run's threads share.
pub(crate) struct State {
    pub(crate) trap: Option<Trapped>,
    pub(crate) failed: Option<u32>,
    pub(crate) live: i64,
    pub(crate) static_from: i64,
    pub(crate) observer: Option<Observer>,
    pub(crate) texts: Vec<Box<str>>,
    /// What the host gives its functions (a pointer it owns for the run).
    pub(crate) host: usize,
    /// The trap of the last isolated scope that failed with one.
    pub(crate) last_trapped: Option<Trapped>,
    /// What gives a `Trapped`'s text.
    pub(crate) trap_text: Option<crate::host::TrapText>,
    sched: Scheduler,
    /// The running task.
    pub(crate) current: usize,
    pub(crate) aborted: bool,
    pub(crate) baton: Baton,
}

// SAFETY: the observer's context pointer is used only by the task holding the baton, while the
// host's thread, which owns it, waits.
unsafe impl Send for State {}

pub(crate) struct Ctx {
    state: Mutex<State>,
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(crate) turn: Condvar,
}

impl Ctx {
    /// The state, locked.
    pub(crate) fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

thread_local! {
    static CTX: RefCell<Option<Arc<Ctx>>> = const { RefCell::new(None) };
    /// The call sites this thread's task is in, innermost last.
    pub(crate) static STACK: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
    /// The task this thread runs.
    static TASK: Cell<usize> = const { Cell::new(0) };
}

fn new_ctx() -> Arc<Ctx> {
    Arc::new(Ctx {
        state: Mutex::new(State {
            trap: None,
            failed: None,
            live: 0,
            static_from: 0,
            observer: None,
            texts: Vec::new(),
            host: 0,
            last_trapped: None,
            trap_text: None,
            sched: Scheduler::new(),
            current: 0,
            aborted: false,
            baton: Baton::default(),
        }),
        turn: Condvar::new(),
    })
}

/// This thread's run.
pub(crate) fn ctx() -> Arc<Ctx> {
    CTX.with(|c| c.borrow_mut().get_or_insert_with(new_ctx).clone())
}

/// The run's state, locked.
pub(crate) fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let c = ctx();
    let mut s = c
        .state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    f(&mut s)
}

/// The running task's number.
pub(crate) fn me() -> usize {
    TASK.with(Cell::get)
}

/// Makes this thread run task `t` of run `c`.
pub(crate) fn enter_task(c: &Arc<Ctx>, t: usize) {
    CTX.with(|x| *x.borrow_mut() = Some(c.clone()));
    TASK.with(|x| x.set(t));
}

/// Starts a new run on this thread, as its first task; what earlier runs left is forgotten.
pub fn begin_run() {
    CTX.with(|c| *c.borrow_mut() = Some(new_ctx()));
    STACK.with(|s| s.borrow_mut().clear());
    TASK.with(|t| t.set(0));
}

/// Ends this thread's run: waits for its tasks' threads to end.
pub fn end_run() {
    let threads = with(baton::end_run);
    for t in threads {
        let _ = t.join();
    }
}

/// Gives the host's functions `host`, for the run.
pub fn set_host(host: usize) {
    with(|s| s.host = host);
}

/// What the host gave, for the run.
pub fn host() -> usize {
    with(|s| s.host)
}

/// Keeps text `t` until the run ends, and gives its address.
pub fn keep_text(t: Box<str>) -> *const u8 {
    let at = t.as_ptr();
    with(|s| s.texts.push(t));
    at
}

/// Installs what gives a `Trapped`'s text, for the run.
pub fn set_trap_text(f: crate::host::TrapText) {
    with(|s| s.trap_text = Some(f));
}

/// The trap of the last isolated scope that failed with one, as [`Trapped`].
pub fn last_trapped() -> Option<Trapped> {
    with(|s| s.last_trapped.clone())
}

/// Records a trap at `site` in the running task: 3 if an isolated scope takes it (the scope
/// fails with it, and the task unwinds), else 1, the trap ending the run.
pub(crate) fn record_trap(site: u32, tracked: bool) -> u8 {
    let stack = STACK.with(|s| s.borrow().clone());
    let t = Trapped {
        site,
        tracked,
        stack,
    };
    let me = me();
    with(|s| {
        if !s.sched.unwinding(me)
            && let Some(scope) = s.sched.isolating(me)
        {
            s.sched.fail(scope, Failure::Trapped(t));
            s.sched.start_unwinding(me);
            return 3;
        }
        if s.trap.is_none() {
            s.trap = Some(t);
        }
        1
    })
}

/// Hands the baton to the next ready task and waits for this one's turn again: 0, or 1 if the
/// run is aborted, or, with no task ready, the status of a deadlock trap at `site`.
fn suspend(c: &Ctx, mut s: MutexGuard<'_, State>, site: u32, tracked: bool) -> u8 {
    let Some(next) = s.sched.next() else {
        drop(s);
        return record_trap(site, tracked);
    };
    baton::hand_over(c, s, next)
}

/// Runs `op` until it gives a status: it is tried, and while it says to wait (`None`, having
/// recorded what it waits for), the task suspends and it is tried again.
fn waiting(site: u32, tracked: bool, mut op: impl FnMut(&mut State, usize) -> Option<u8>) -> u8 {
    let c = ctx();
    let me = me();
    loop {
        let mut s = c
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if s.aborted {
            return 1;
        }
        if let Some(status) = op(&mut s, me) {
            return status;
        }
        let r = suspend(&c, s, site, tracked);
        if r != 0 {
            return r;
        }
    }
}

/// Drops each value with its glue, running the code outside the lock: 0, or the status of the
/// first destructor that trapped (the rest then left).
fn drop_all(dropped: Vec<(DropFn, Vec<u8>)>) -> u8 {
    for (d, mut v) in dropped {
        let status = baton::call_drop(d, v.as_mut_ptr());
        if status != 0 {
            return status as u8;
        }
    }
    0
}

/// Copies `v` to `to`.
fn put(v: &[u8], to: *mut u8) {
    if !v.is_empty() {
        // SAFETY: the code passes room for a value of the type `v` holds.
        unsafe { std::ptr::copy_nonoverlapping(v.as_ptr(), to, v.len()) };
    }
}

/// `n` bytes from `p`.
fn bytes(p: *const u8, n: u64) -> Vec<u8> {
    if n == 0 {
        return Vec::new();
    }
    // SAFETY: the code passes a value of `n` bytes.
    unsafe { std::slice::from_raw_parts(p, n as usize).to_vec() }
}

/// A task to start, as native code writes it: its code, captures (freed once it ends, `size`
/// bytes aligned to `align`), argument, and room for its result and its error.
#[cfg(not(target_arch = "wasm32"))]
#[repr(C)]
pub struct TaskSpec {
    /// Its code.
    pub code: TaskFn,
    /// Its closure's captures, which it owns.
    pub captures: *mut u8,
    /// The captures' size.
    pub size: u64,
    /// The captures' alignment.
    pub align: u64,
    /// Its argument: the scope's borrow, for a scope's body.
    pub arg: u64,
    /// How many bytes its result has.
    pub out: u64,
    /// How many bytes its error may have.
    pub err: u64,
    /// What drops its result, if nobody takes it (null if it needs no dropping).
    pub drop: Option<DropFn>,
}

/// A task to start, as a WASM module writes it: eight words, as native code's, the code and the
/// drop glue slots in the program's table (0 for none) and the captures an address.
#[cfg(target_arch = "wasm32")]
#[repr(C)]
pub struct TaskSpec {
    /// Its code's slot.
    pub code: TaskFn,
    /// Its closure's captures, which it owns.
    pub captures: u64,
    /// The captures' size.
    pub size: u64,
    /// The captures' alignment.
    pub align: u64,
    /// Its argument: the scope's borrow, for a scope's body.
    pub arg: u64,
    /// How many bytes its result has.
    pub out: u64,
    /// How many bytes its error may have.
    pub err: u64,
    /// Its result's drop glue's slot, or 0.
    pub drop: u64,
}

/// A task as the baton starts it.
#[derive(Clone, Copy)]
pub(crate) struct Start {
    pub(crate) code: TaskFn,
    pub(crate) captures: usize,
    size: u64,
    align: u64,
    pub(crate) arg: u64,
    out: u64,
    err: u64,
}

/// Task `t`'s code, run in its turn, to its end; then the turn is given on (or the run aborted).
pub(crate) fn run_task(c: &Arc<Ctx>, t: usize, spec: Start) {
    let mut out = vec![0u128; (spec.out as usize).div_ceil(16).max(1)];
    let mut err = vec![0u128; (spec.err as usize).div_ceil(16).max(1)];
    let status = baton::call_code(&spec, out.as_mut_ptr().cast(), err.as_mut_ptr().cast());
    // SAFETY: the captures are the task's, given by `alloc`; it has ended.
    unsafe { crate::free(spec.captures as *mut u8, spec.size, spec.align) };
    let mut s = c.lock();
    if s.aborted {
        return;
    }
    let outcome = match status {
        0 => Outcome::Value(bytes(out.as_ptr().cast(), spec.out)),
        2 => Outcome::Error(bytes(err.as_ptr().cast(), spec.err)),
        3 => Outcome::Cancelled,
        // A trap: the run ends.
        _ => {
            s.aborted = true;
            baton::pass_on(c, &s);
            return;
        }
    };
    let dropped = s.sched.end(t, outcome);
    if !dropped.is_empty() {
        drop(s);
        let status = drop_all(dropped);
        s = c.lock();
        if status != 0 {
            s.aborted = true;
            baton::pass_on(c, &s);
            return;
        }
    }
    match s.sched.next() {
        Some(next) => s.current = next,
        None => {
            // Every other task waits: a deadlock, where nothing says.
            if s.trap.is_none() {
                s.trap = Some(Trapped {
                    site: u32::MAX,
                    tracked: false,
                    stack: Vec::new(),
                });
            }
            s.aborted = true;
        }
    }
    baton::pass_on(c, &s);
}

fn spec_of(spec: &TaskSpec) -> Start {
    Start {
        code: spec.code,
        captures: spec.captures as usize,
        size: spec.size,
        align: spec.align,
        arg: spec.arg,
        out: spec.out,
        err: spec.err,
    }
}

/// `__scope_open`: a scope for the running task's `scope` call.
#[allow(unsafe_code)]
#[unsafe(export_name = "moddil_rt_scope_open")]
pub extern "C" fn scope_open(isolated: u8) -> u64 {
    let me = me();
    with(|s| s.sched.open_scope(me, isolated != 0) as u64)
}

/// `Scope::spawn`: starts task `spec` in scope `scope`, queued; its number.
///
/// # Safety
///
/// `spec` points to a task to start.
#[allow(unsafe_code)]
#[unsafe(export_name = "moddil_rt_spawn")]
pub unsafe extern "C" fn spawn(scope: u64, spec: *const TaskSpec) -> u64 {
    // SAFETY: as the caller says.
    let spec = unsafe { &*spec };
    let c = ctx();
    let mut s = c
        .state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let t = s
        .sched
        .spawn(scope as usize, false, baton::drop_of(spec.drop));
    baton::start(&mut s, &c, t, spec_of(spec));
    t as u64
}

/// `__scope_run`: runs `spec` as the scope's body, at once, then waits for every task of the
/// scope: its body's value at `out` (0), its error at `err` (2), 3 to unwind, 1 for a trap, or
/// 4 if it failed with a trap ([`last_trapped`]).
///
/// # Safety
///
/// `spec` points to a task to start; `out` and `err` have room for its result and error.
#[allow(unsafe_code)]
#[unsafe(export_name = "moddil_rt_scope_run")]
pub unsafe extern "C" fn scope_run(
    scope: u64,
    spec: *const TaskSpec,
    out: *mut u8,
    err: *mut u8,
    site: u32,
    tracked: u8,
) -> u8 {
    // SAFETY: as the caller says.
    let spec = unsafe { &*spec };
    let c = ctx();
    let me = me();
    let s_id = scope as usize;
    {
        let mut s = c.lock();
        let t = s.sched.spawn(s_id, true, baton::drop_of(spec.drop));
        baton::start(&mut s, &c, t, spec_of(spec));
        s.sched.block(me, Wait::Scope(s_id));
        // The body runs at once.
        if baton::hand_over(&c, s, t) != 0 {
            return 1;
        }
    }
    let mut dropped = Vec::new();
    let status = waiting(site, tracked != 0, |s, me| {
        if !s.sched.scope_done(s_id) {
            s.sched.block(me, Wait::Scope(s_id));
            return None;
        }
        let (r, d) = s.sched.scope_result(s_id);
        dropped = d;
        if s.sched.cancelled(me) {
            s.sched.start_unwinding(me);
            return Some(3);
        }
        Some(match r {
            Ok(v) => {
                if let Some(v) = v {
                    put(&v, out);
                }
                0
            }
            Err(Failure::Error(e)) => {
                put(&e, err);
                2
            }
            Err(Failure::Trapped(t)) => {
                s.last_trapped = Some(t);
                4
            }
        })
    });
    let dropped = drop_all(dropped);
    if dropped != 0 { dropped } else { status }
}

/// `Scope::stop`.
#[allow(unsafe_code)]
#[unsafe(export_name = "moddil_rt_stop")]
pub extern "C" fn stop(scope: u64) {
    with(|s| s.sched.stop(scope as usize));
}

/// `Task::join`: its value at `out` (0), its error at `err` (2), or 3 to unwind.
#[allow(unsafe_code)]
#[unsafe(export_name = "moddil_rt_join")]
pub extern "C" fn join(t: u64, out: *mut u8, err: *mut u8, site: u32, tracked: u8) -> u8 {
    waiting(site, tracked != 0, |s, me| {
        if s.sched.cancelled(me) {
            s.sched.start_unwinding(me);
            return Some(3);
        }
        match s.sched.take(t as usize) {
            Some(Outcome::Value(v)) => {
                put(&v, out);
                Some(0)
            }
            Some(Outcome::Error(e)) => {
                put(&e, err);
                Some(2)
            }
            Some(Outcome::Cancelled) => {
                s.sched.start_unwinding(me);
                Some(3)
            }
            None => {
                s.sched.block(me, Wait::Join(t as usize));
                None
            }
        }
    })
}

/// A `Task` dropped without a `join`: 0, or the status of a trap dropping its value.
#[allow(unsafe_code)]
#[unsafe(export_name = "moddil_rt_detach")]
pub extern "C" fn detach(t: u64) -> u8 {
    let dropped = with(|s| s.sched.detach(t as usize));
    drop_all(dropped.into_iter().collect())
}

/// `yield_now`.
#[allow(unsafe_code)]
#[unsafe(export_name = "moddil_rt_yield_now")]
pub extern "C" fn yield_now(site: u32, tracked: u8) -> u8 {
    let mut yielded = false;
    waiting(site, tracked != 0, |s, me| {
        if s.sched.cancelled(me) {
            s.sched.start_unwinding(me);
            return Some(3);
        }
        if yielded {
            return Some(0);
        }
        yielded = true;
        s.sched.requeue(me);
        None
    })
}

/// `stop_requested`.
#[allow(unsafe_code)]
#[unsafe(export_name = "moddil_rt_stop_requested")]
pub extern "C" fn stop_requested(site: u32, tracked: u8) -> u8 {
    waiting(site, tracked != 0, |s, me| {
        if s.sched.cancelled(me) {
            s.sched.start_unwinding(me);
            return Some(3);
        }
        if s.sched.stop_reached(me) {
            return Some(0);
        }
        s.sched.block(me, Wait::Stop);
        None
    })
}

/// `channel`: a channel holding up to `capacity` values.
#[allow(unsafe_code)]
#[unsafe(export_name = "moddil_rt_channel")]
pub extern "C" fn channel(capacity: u64) -> u64 {
    with(|s| s.sched.channel(capacity as usize) as u64)
}

/// `__timer`: a timer's channel, whose one value arrives `nanos` from now (`task.timer.1`).
#[allow(unsafe_code)]
#[unsafe(export_name = "moddil_rt_timer")]
pub extern "C" fn timer(nanos: i64) -> u64 {
    with(|s| s.sched.timer(nanos) as u64)
}

/// For a run of `main`: the OS, with the program's arguments `args` (`os.1`).
pub fn set_os(args: Vec<String>) {
    with(|s| s.sched.os = Some((crate::os::Os::new(args), crate::os::Start::now())));
}

/// `core::os::arg_count`.
#[unsafe(export_name = "moddil_rt_os_arg_count")]
pub extern "C" fn os_arg_count() -> u64 {
    with(|s| s.sched.os().map_or(0, |os| os.arg_count()) as u64)
}

/// `core::os::arg_len`.
#[unsafe(export_name = "moddil_rt_os_arg_len")]
pub extern "C" fn os_arg_len(i: u64) -> u64 {
    with(|s| s.sched.os().map_or(0, |os| os.arg(i as usize).len()) as u64)
}

/// `core::os::arg_copy`: argument `i` into the `n` bytes at `out`.
///
/// # Safety
///
/// `out` points to `n` writable bytes.
#[allow(unsafe_code)]
#[unsafe(export_name = "moddil_rt_os_arg_copy")]
pub unsafe extern "C" fn os_arg_copy(i: u64, out: *mut u8, n: u64) {
    let arg = with(|s| {
        s.sched
            .os()
            .map(|os| os.arg(i as usize))
            .unwrap_or_default()
    });
    let k = arg.len().min(n as usize);
    // SAFETY: the caller gives `n` bytes at `out`.
    unsafe { std::ptr::copy_nonoverlapping(arg.as_ptr(), out, k) };
}

/// `core::os::write_stderr`: the `n` bytes of text at `p`.
///
/// # Safety
///
/// `p` points to `n` bytes of UTF-8.
#[allow(unsafe_code)]
#[unsafe(export_name = "moddil_rt_os_write_stderr")]
pub unsafe extern "C" fn os_write_stderr(p: *const u8, n: u64) {
    // SAFETY: the caller gives `n` bytes of text at `p`.
    let text = unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(p, n as usize)) };
    crate::os::write_stderr(text);
}

/// `core::os::random`: under a test, a fixed number, so a run replays.
#[unsafe(export_name = "moddil_rt_os_random")]
pub extern "C" fn os_random() -> u64 {
    let real = with(|s| s.sched.os().is_some());
    if real {
        crate::os::random().max(1) as u64
    } else {
        4
    }
}

/// `core::gpu::dispatch` (`gpu.1`): kernel `source` (`sn` bytes at `sp`) with the `pn` bytes of
/// parameters at `pp`, over a `width` by `height` image, its `on` pixels at `op`, as `mode` says
/// (1 show it, 2 read it back); 1 if it ran.
/// Only a run of `main` has a GPU, and only in a browser: a test's never does, so it replays.
///
/// # Safety
///
/// Each pointer points to as many bytes (or, for `op`, words) as its length says.
#[allow(unsafe_code, clippy::too_many_arguments)]
#[unsafe(export_name = "moddil_rt_gpu_dispatch")]
pub unsafe extern "C" fn gpu_dispatch(
    sp: *const u8,
    sn: u64,
    pp: *const u8,
    pn: u64,
    width: u64,
    height: u64,
    op: *mut u32,
    on: u64,
    mode: u64,
) -> u8 {
    if !with(|s| s.sched.os().is_some()) {
        return 0;
    }
    // SAFETY: as the caller promises.
    let (source, params, out) = unsafe {
        (
            std::str::from_utf8_unchecked(std::slice::from_raw_parts(sp, sn as usize)),
            std::slice::from_raw_parts(pp, pn as usize),
            std::slice::from_raw_parts_mut(op, on as usize),
        )
    };
    u8::from(crate::os::gpu_dispatch(
        source,
        params,
        width as i64,
        height as i64,
        out,
        mode as u32,
    ))
}

/// `core::gpu::show` (`gpu.1`): shows the `n` pixels at `p`, a `width` by `height` image drawn
/// on the CPU, in the page. Only a run of `main` in a browser has a page.
///
/// # Safety
///
/// `p` points to `n` words.
#[allow(unsafe_code)]
#[unsafe(export_name = "moddil_rt_gpu_show")]
pub unsafe extern "C" fn gpu_show(p: *const u32, n: u64, width: u64, height: u64) {
    if !with(|s| s.sched.os().is_some()) {
        return;
    }
    // SAFETY: as the caller promises.
    let pixels = unsafe { std::slice::from_raw_parts(p, n as usize) };
    crate::os::gpu_show(pixels, width as i64, height as i64);
}

/// `core::gpu::frame` (`gpu.1`): the next frame's 8 words into `out`. Outside a run of `main`
/// (a test) there is no window, so the frame asks to stop.
///
/// # Safety
///
/// `out` points to 8 writable words.
#[allow(unsafe_code)]
#[unsafe(export_name = "moddil_rt_gpu_frame")]
pub unsafe extern "C" fn gpu_frame(out: *mut i64) {
    let mut words = [0, 0, 0, 0, 0, 0, 0, 1];
    if with(|s| s.sched.os().is_some()) {
        crate::os::gpu_frame(&mut words);
    }
    // SAFETY: as the caller promises.
    unsafe { std::ptr::copy_nonoverlapping(words.as_ptr(), out, 8) };
}

/// `core::os::listen`: a listener's handle, or -1.
#[unsafe(export_name = "moddil_rt_os_listen")]
pub extern "C" fn os_listen(port: u64) -> u64 {
    let port = u16::try_from(port).unwrap_or(0);
    with(|s| s.sched.os().map_or(-1, |os| os.listen(port))) as u64
}

/// `core::os::connect`: to `port` at the host named by the `n` bytes at `p`; a connection's
/// handle, or -1.
///
/// # Safety
///
/// `p` points to `n` bytes of UTF-8.
#[allow(unsafe_code)]
#[unsafe(export_name = "moddil_rt_os_connect")]
pub unsafe extern "C" fn os_connect(p: *const u8, n: u64, port: u64) -> u64 {
    // SAFETY: the caller gives `n` bytes of text at `p`.
    let host = unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(p, n as usize)) }
        .to_string();
    let port = u16::try_from(port).unwrap_or(0);
    with(|s| s.sched.os().map_or(-1, |os| os.connect(&host, port))) as u64
}

/// `core::os::incoming`, `arrivals` and `signals` (kinds 0, 1 and 2): the channel the OS feeds
/// for `id`.
#[unsafe(export_name = "moddil_rt_os_channel")]
pub extern "C" fn os_channel(kind: u64, id: u64) -> u64 {
    use crate::sched::OsSource;
    let kind = match kind {
        0 => OsSource::Incoming,
        1 => OsSource::Arrivals,
        _ => OsSource::Signals,
    };
    with(|s| s.sched.os_channel(kind, id as usize) as u64)
}

/// `core::os::take`: connection `conn`'s bytes into the `n` at `out`; how many.
///
/// # Safety
///
/// `out` points to `n` writable bytes.
#[allow(unsafe_code)]
#[unsafe(export_name = "moddil_rt_os_take")]
pub unsafe extern "C" fn os_take(conn: u64, out: *mut u8, n: u64) -> u64 {
    // SAFETY: the caller gives `n` bytes at `out`.
    let buf = unsafe { std::slice::from_raw_parts_mut(out, n as usize) };
    with(|s| s.sched.os().map_or(0, |os| os.take(conn as usize, buf))) as u64
}

/// `core::os::send`: the `n` bytes at `p` on connection `conn`; 1 if they went.
///
/// # Safety
///
/// `p` points to `n` bytes.
#[allow(unsafe_code)]
#[unsafe(export_name = "moddil_rt_os_send")]
pub unsafe extern "C" fn os_send(conn: u64, p: *const u8, n: u64) -> u8 {
    // SAFETY: the caller gives `n` bytes at `p`.
    let bytes = unsafe { std::slice::from_raw_parts(p, n as usize) }.to_vec();
    u8::from(with(|s| {
        s.sched
            .os()
            .is_some_and(|os| os.write(conn as usize, &bytes))
    }))
}

/// `core::os::close`.
#[unsafe(export_name = "moddil_rt_os_close")]
pub extern "C" fn os_close(id: u64) {
    with(|s| {
        if let Some(os) = s.sched.os() {
            os.close(id as usize);
        }
    });
}

/// `Sender::clone`.
#[unsafe(export_name = "moddil_rt_clone_sender")]
pub extern "C" fn clone_sender(c: u64) {
    with(|s| s.sched.clone_sender(c as usize));
}

/// A `Sender` dropped.
#[allow(unsafe_code)]
#[unsafe(export_name = "moddil_rt_drop_sender")]
pub extern "C" fn drop_sender(c: u64) {
    with(|s| s.sched.drop_sender(c as usize));
}

/// The receiver of channel `c` is gone: what it holds is dropped with `drop`. 0, or the status
/// of a destructor's trap.
#[allow(unsafe_code)]
#[unsafe(export_name = "moddil_rt_drop_receiver")]
pub extern "C" fn drop_receiver(c: u64, drop: Option<DropFn>) -> u8 {
    let left = with(|s| s.sched.drop_receiver(c as usize));
    match drop {
        Some(d) => drop_all(left.into_iter().map(|v| (d, v)).collect()),
        None => 0,
    }
}

/// `Sender::send` of the `n` bytes at `v`: 0, or 2 if the receiver is gone (the value then
/// dropped with `drop`), or 3 to unwind.
#[allow(unsafe_code)]
#[unsafe(export_name = "moddil_rt_send")]
pub extern "C" fn send(
    c: u64,
    v: *const u8,
    n: u64,
    drop: Option<DropFn>,
    site: u32,
    tracked: u8,
) -> u8 {
    let c = c as usize;
    let value = bytes(v, n);
    let mut pending = Some(value);
    let mut sent: Option<u64> = None;
    let mut closed = None;
    let status = waiting(site, tracked != 0, |s, me| {
        if s.sched.cancelled(me) {
            s.sched.start_unwinding(me);
            return Some(3);
        }
        if let Some(k) = sent {
            if !s.sched.taken(c, k) {
                s.sched.block(me, Wait::Taken(c, k));
                return None;
            }
            return Some(if s.sched.receiver_alive(c) { 0 } else { 2 });
        }
        if !s.sched.receiver_alive(c) {
            closed = pending.take();
            return Some(2);
        }
        if !s.sched.can_send(c) {
            s.sched.block(me, Wait::Send(c));
            return None;
        }
        let (k, rendezvous) = s.sched.push(c, pending.take().expect("a value to send"));
        if rendezvous {
            sent = Some(k);
            s.sched.block(me, Wait::Taken(c, k));
            return None;
        }
        Some(0)
    });
    if let (Some(v), Some(d)) = (closed.or(if status == 3 { pending } else { None }), drop) {
        let dropped = drop_all(vec![(d, v)]);
        if dropped != 0 {
            return dropped;
        }
    }
    status
}

/// `Receiver::recv`: 0 and the value at `out`, 5 for `None` (no senders left), or 3 to
/// unwind.
#[allow(unsafe_code)]
#[unsafe(export_name = "moddil_rt_recv")]
pub extern "C" fn recv(c: u64, out: *mut u8, site: u32, tracked: u8) -> u8 {
    let c = c as usize;
    waiting(site, tracked != 0, |s, me| {
        if s.sched.cancelled(me) {
            s.sched.start_unwinding(me);
            return Some(3);
        }
        if !s.sched.can_recv(c) {
            s.sched.block(me, Wait::Recv(c));
            return None;
        }
        Some(match s.sched.pop(c) {
            Some(v) => {
                put(&v, out);
                0
            }
            None => 5,
        })
    })
}

/// `__select`'s wait: the first of the `n` operations (each a kind and a handle: 0 receive,
/// 1 send, 2 join, 3 stop) that is ready, by its index; or -1 for a trap, -3 to unwind.
///
/// # Safety
///
/// `ops` points to `n` pairs of words.
#[allow(unsafe_code)]
#[unsafe(export_name = "moddil_rt_select")]
pub unsafe extern "C" fn select(n: u64, ops: *const u64, site: u32, tracked: u8) -> i64 {
    // SAFETY: as the caller says.
    let raw = unsafe { std::slice::from_raw_parts(ops, 2 * n as usize) };
    let ops: Vec<Op> = raw
        .chunks(2)
        .map(|p| match p[0] {
            0 => Op::Recv(p[1] as usize),
            1 => Op::Send(p[1] as usize),
            2 => Op::Join(p[1] as usize),
            _ => Op::Stop,
        })
        .collect();
    let mut chosen = 0;
    let status = waiting(site, tracked != 0, |s, me| {
        if s.sched.cancelled(me) {
            s.sched.start_unwinding(me);
            return Some(3);
        }
        match ops.iter().position(|o| s.sched.op_ready(me, o)) {
            Some(i) => {
                chosen = i;
                Some(0)
            }
            None => {
                s.sched.block(me, Wait::Select(ops.clone()));
                None
            }
        }
    });
    match status {
        0 => chosen as i64,
        3 => -3,
        _ => -1,
    }
}

/// A `select` arm's receive, which is ready: as [`recv`].
#[allow(unsafe_code)]
#[unsafe(export_name = "moddil_rt_recv_now")]
pub extern "C" fn recv_now(c: u64, out: *mut u8) -> u8 {
    with(|s| match s.sched.pop(c as usize) {
        Some(v) => {
            put(&v, out);
            0
        }
        None => 5,
    })
}

/// A `select` arm's send, which is ready: 0, or 2 if the receiver is gone (the value dropped).
#[allow(unsafe_code)]
#[unsafe(export_name = "moddil_rt_send_now")]
pub extern "C" fn send_now(c: u64, v: *const u8, n: u64, drop: Option<DropFn>) -> u8 {
    let value = bytes(v, n);
    let closed = with(|s| {
        if s.sched.receiver_alive(c as usize) {
            s.sched.push(c as usize, value);
            None
        } else {
            Some(value)
        }
    });
    match (closed, drop) {
        (Some(v), Some(d)) => match drop_all(vec![(d, v)]) {
            0 => 2,
            trapped => trapped,
        },
        (Some(_), None) => 2,
        (None, _) => 0,
    }
}

/// A `select` arm's join, which is ready: as [`join`].
#[allow(unsafe_code)]
#[unsafe(export_name = "moddil_rt_join_now")]
pub extern "C" fn join_now(t: u64, out: *mut u8, err: *mut u8) -> u8 {
    let me = me();
    with(|s| match s.sched.take(t as usize) {
        Some(Outcome::Value(v)) => {
            put(&v, out);
            0
        }
        Some(Outcome::Error(e)) => {
            put(&e, err);
            2
        }
        _ => {
            s.sched.start_unwinding(me);
            3
        }
    })
}

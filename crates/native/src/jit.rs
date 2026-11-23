//! Running compiled code, and reading the values it observes: the one place `unsafe` is allowed,
//! to call the code Cranelift made and to read its values' bytes while they are alive.

#![allow(unsafe_code)]

use moddil_mir::TrapClass;

use moddil_syntax::TextRange;

use crate::layout::Layouts;
use crate::lower::{Compiled, Site};
use crate::show::{Memory, Show};
use crate::{Native, Observed};

/// How much stack a run has: deep enough for the frames `moddil-rt` lets a run have.
const STACK: usize = 512 * 1024 * 1024;

/// What the observer needs while a run goes on: the sites, the layouts, and what it has seen.
struct Seen<'a> {
    sites: &'a [Site],
    show: Show<'a>,
    observed: Vec<Observed>,
    /// The parts of the last `assert` that failed, each with its value if it was evaluated.
    parts: Vec<(TextRange, Option<String>)>,
}

/// The process's own memory, read while the values the code shows are alive.
struct RawMemory;

impl Memory for RawMemory {
    fn bits(&self, addr: u64, n: u32) -> u128 {
        let mut bytes = [0u8; 16];
        // SAFETY: the renderer reads only inside live values the code hands it.
        unsafe {
            std::ptr::copy_nonoverlapping(
                addr as usize as *const u8,
                bytes.as_mut_ptr(),
                n as usize,
            );
        }
        u128::from_le_bytes(bytes)
    }

    fn bytes(&self, addr: u64, n: usize) -> Vec<u8> {
        if n == 0 {
            return Vec::new();
        }
        // SAFETY: as above.
        unsafe { std::slice::from_raw_parts(addr as usize as *const u8, n).to_vec() }
    }
}

/// `moddil-rt`'s observer: renders the value at `addr` for `site`.
fn observe(cx: *mut (), site: u32, addr: *const u8) {
    // SAFETY: `cx` is the `Seen` the run installed, on the running thread, alive until the run
    // ends and the observer is removed.
    let seen = unsafe { &mut *cx.cast::<Seen<'_>>() };
    match seen.sites.get(site as usize) {
        Some(Site::Observe(file, range, ty)) => {
            let ty = ty.clone();
            // SAFETY: the generated code passes the address of a live value of type `ty`.
            let text = seen.show.render(&RawMemory, &ty, addr as u64);
            seen.observed.push((*file, *range, text));
        }
        Some(Site::Part(range, ty, first)) => {
            if *first {
                seen.parts.clear();
            }
            let text = ty.clone().map(|ty| {
                // SAFETY: as above, for an evaluated part.
                seen.show.render(&RawMemory, &ty, addr as u64)
            });
            seen.parts.push((*range, text));
        }
        _ => {}
    }
}

impl Compiled<'_> {
    /// Runs the entry point, on a thread of its own with a large stack: the values it observed,
    /// and its result as Moddil source, or the trap.
    /// With it, if the result is an `ExitCode`, the process's exit code for it (`main.exit.1`).
    pub(crate) fn run(self) -> (Vec<Observed>, Result<String, Native>, Option<u8>) {
        let ptr = self.module.get_finalized_function(self.entry) as usize;
        let drop_ptr = self.module.get_finalized_function(self.drop_result) as usize;
        let (sites, items, ret) = (&self.sites, self.items, &self.ret);
        let dyn_types = &self.dyn_types;
        let errors = &self.errors;
        let host = crate::host::HostData {
            sites: &self.sites,
            files: &self.files,
        };
        let host = &host;
        let (status, text, observed, record, leaked, parts, exit) = std::thread::scope(|s| {
            std::thread::Builder::new()
                .stack_size(STACK)
                .spawn_scoped(s, move || {
                    // A new run, its tasks' threads joining this one's.
                    moddil_rt::begin_run();
                    moddil_rt::set_host(std::ptr::from_ref(host) as usize);
                    moddil_rt::set_trap_text(crate::host::trap_text);
                    let live = moddil_rt::live();
                    let mut seen = Seen {
                        sites,
                        show: Show {
                            items,
                            layouts: Layouts::new(items),
                            dyn_types,
                        },
                        observed: Vec::new(),
                        parts: Vec::new(),
                    };
                    moddil_rt::set_observer(Some((observe, (&raw mut seen).cast())));
                    // Room for any result, aligned for any field.
                    let size = seen.show.layouts.of(ret).map_or(16, |l| l.size.max(16)) as usize;
                    let mut out = vec![0u128; size.div_ceil(16)];
                    // And for any error, after its id.
                    let room = errors
                        .iter()
                        .map(|t| seen.show.layouts.of(t).map_or(0, |l| l.size))
                        .max()
                        .unwrap_or(0) as usize;
                    let mut err = vec![0u128; (8 + room).div_ceil(16).max(1)];
                    // SAFETY: `ptr` is the finished entry trampoline, whose signature is
                    // `fn(*mut u8, *mut u8) -> i8` in the platform's C calling convention; the
                    // module that owns its memory lives until this scope ends; `out` has room
                    // for the result and `err` for an error.
                    let entry: extern "C" fn(*mut u8, *mut u8) -> i8 =
                        unsafe { std::mem::transmute::<usize, _>(ptr) };
                    let status = entry(out.as_mut_ptr().cast(), err.as_mut_ptr().cast());
                    // Every task has ended, or (after a trap) is ending, running no more code.
                    moddil_rt::end_run();
                    moddil_rt::set_observer(None);
                    let text = match status {
                        // SAFETY: the run returned, so `out` holds a value of type `ret`.
                        0 => Some(seen.show.render(&RawMemory, ret, out.as_ptr() as u64)),
                        // SAFETY: the run failed, so `err` holds an error of those types.
                        2 => Some(seen.show.error(&RawMemory, errors, err.as_ptr() as u64)),
                        _ => None,
                    };
                    // SAFETY: as for the text, when the run returned.
                    let exit = (status == 0)
                        .then(|| seen.show.exit_code(&RawMemory, ret, out.as_ptr() as u64))
                        .flatten();
                    // Once shown, the result is dropped; then a run that returned has freed
                    // everything it allocated.
                    let mut leaked = 0;
                    let mut status = status;
                    if status == 0 {
                        // SAFETY: `drop_ptr` is the finished `drop_result`, of the same
                        // signature as the entry, and `out` holds the result it drops.
                        let drop: extern "C" fn(*mut u8) -> i8 =
                            unsafe { std::mem::transmute::<usize, _>(drop_ptr) };
                        status = drop(out.as_mut_ptr().cast());
                        if status == 0 {
                            leaked = moddil_rt::live() - live;
                        }
                    }
                    (
                        status,
                        text,
                        seen.observed,
                        moddil_rt::take(),
                        leaked,
                        seen.parts,
                        exit,
                    )
                })
                .expect("a thread to run on")
                .join()
                .expect("native code does not panic")
        });
        let result = match text {
            Some(_) if status == 0 && leaked != 0 => Err(Native::Leak(leaked)),
            Some(t) if status == 0 => Ok(t),
            // The error left the run: where the last `fail` was.
            Some(t) if status == 2 => match record.failed.map(|s| &self.sites[s as usize]) {
                Some(Site::Fail(file, range)) => Err(Native::Failed(*file, *range, t)),
                _ => Err(Native::Unsupported("an error without a site".to_string())),
            },
            _ => match record.trap {
                // A failed `assert`'s parts, if it is reported where it is (not in its caller).
                Some(t) => Err(match reported(&self.sites, &t) {
                    Native::Trap(class, file, range, _)
                        if t.reported_at(|_| false) == t.site && !t.tracked =>
                    {
                        Native::Trap(class, file, range, parts)
                    }
                    other => other,
                }),
                None => Err(Native::Unsupported("a trap without a site".to_string())),
            },
        };
        // SAFETY: nothing runs the module's code after this.
        unsafe { self.module.free_memory() };
        (observed, result, exit)
    }
}

/// A trap as the interpreters report it: where it happened, or, in tracked code, at the
/// innermost call untracked code made. A deadlock no site says the place of is reported at the
/// start of the first file.
pub(crate) fn reported(sites: &[Site], t: &moddil_rt::Trapped) -> Native {
    {
        let Some(site) = sites.get(t.site as usize) else {
            return Native::Trap(TrapClass::Deadlock, 0, TextRange::default(), Vec::new());
        };
        let (class, file, range) = match site {
            Site::Trap(class, file, range) => (*class, *file, *range),
            Site::Call(file, range, _) => (TrapClass::StackOverflow, *file, *range),
            Site::Observe(..) | Site::Fail(..) | Site::Part(..) => {
                return Native::Unsupported("a trap at an observation".to_string());
            }
        };
        let at = t.reported_at(|s| matches!(sites.get(s as usize), Some(Site::Call(_, _, false))));
        let (file, range) = match sites.get(at as usize) {
            Some(Site::Call(file, range, false)) if at != t.site => (*file, *range),
            _ => (file, range),
        };
        Native::Trap(class, file, range, Vec::new())
    }
}

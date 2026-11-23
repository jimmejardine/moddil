//! `moddil-rt-main`: a built Moddil program's `main` (`lifecycle.md`).
//!
//! The program's object (`moddil build`, through `moddil_native::build_object`) defines
//! `moddil_entry`, `moddil_drop_result` and `moddil_exit_code`, and the tables a trap is reported
//! from; this library defines `main`, which runs the program and exits as `lifecycle.md` says.

// Built for tests (`cargo test --all-targets`), it would bring a second `main` and symbols only a
// program's object defines: so it is empty then.
#![cfg(not(test))]
#![allow(unsafe_code)]

unsafe extern "C" {
    /// `main`, its result written to its first argument.
    fn moddil_entry(out: *mut u8, err: *mut u8) -> i8;
    /// Drops `main`'s result.
    fn moddil_drop_result(out: *mut u8) -> i8;
    /// The exit code for `main`'s result.
    fn moddil_exit_code(out: *const u8) -> i32;
    static moddil_site_count: u64;
    /// Eight words a site: kind (0 a check, 1 a call), whether it is tracked, its class's text
    /// and its file's (offsets and lengths in `moddil_strings`), its line and its column.
    static moddil_sites: [u32; 0];
    static moddil_strings: [u8; 0];
}

/// What a built program exits with after a trap (`EX_SOFTWARE`).
const TRAPPED: i32 = 70;

/// The object's site tables.
fn tables() -> moddil_rt::sites::Tables {
    // SAFETY: the object defines the count, the sites and their strings.
    unsafe {
        moddil_rt::sites::Tables {
            count: moddil_site_count,
            sites: (&raw const moddil_sites).cast::<u32>(),
            strings: (&raw const moddil_strings).cast::<u8>(),
        }
    }
}

/// The text of a `Trapped` (`task.isolate.1`): its class, or where it was.
fn trapped(t: &moddil_rt::Trapped, which: u32) -> String {
    tables().trapped(t, which)
}

/// The process's `main`: the program's, then its exit code (`main.exit.1`).
#[unsafe(no_mangle)]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    moddil_rt::begin_run();
    moddil_rt::set_trap_text(trapped);
    // Room for `main`'s result, `()` or an `ExitCode`, and for an error (`main` cannot fail).
    let mut out = [0u128; 16];
    let mut err = [0u128; 16];
    // SAFETY: the object's `moddil_entry` is `main`, given room for its result and an error.
    let status = unsafe { moddil_entry(out.as_mut_ptr().cast(), err.as_mut_ptr().cast()) };
    moddil_rt::end_run();
    if status == 0 {
        // SAFETY: `main` returned, so `out` holds its result.
        let code = unsafe { moddil_exit_code(out.as_ptr().cast()) };
        // SAFETY: as above; nothing reads the result after.
        let dropped = unsafe { moddil_drop_result(out.as_mut_ptr().cast()) };
        if dropped == 0 {
            return code;
        }
    }
    match moddil_rt::take().trap {
        Some(t) => {
            let (class, at) = tables().trap_text(&t);
            eprintln!("trap: {class}");
            eprintln!("  at {at}");
        }
        None => eprintln!("trap"),
    }
    TRAPPED
}

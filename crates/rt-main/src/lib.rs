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
    /// The size of `main`'s result.
    static moddil_result_size: u64;
    /// How to show `main`'s result, if it is shown: a table's length (0 if it is not shown),
    /// the result's index in it, then the table (`moddil_show::Types`).
    static moddil_result_types: [u64; 1];
}

/// Native memory, read where the result's bytes are.
struct Native;

impl moddil_show::Memory for Native {
    fn bits(&self, addr: u64, n: u32) -> u128 {
        let mut b = [0u8; 16];
        let n = n.min(16) as usize;
        // SAFETY: the description says what the result's bytes are, so `addr` is a live value
        // of at least `n` bytes, which the process holds until the result is dropped.
        unsafe { std::ptr::copy_nonoverlapping(addr as usize as *const u8, b.as_mut_ptr(), n) };
        u128::from_le_bytes(b)
    }

    fn bytes(&self, addr: u64, n: usize) -> Vec<u8> {
        if n == 0 {
            return Vec::new();
        }
        // SAFETY: as above, for `n` bytes of text or elements.
        unsafe { std::slice::from_raw_parts(addr as usize as *const u8, n).to_vec() }
    }
}

/// `main`'s result at `out`, as `moddil run` prints it, if it is printed (not `()` or an
/// `ExitCode`).
fn shown(out: *const u8) -> Option<String> {
    // SAFETY: the object defines the table, its length first, then the root, then its bytes.
    let (len, root, table) = unsafe {
        let p = (&raw const moddil_result_types).cast::<u64>();
        let len = *p as usize;
        if len == 0 {
            return None;
        }
        let root = *p.add(1) as u32;
        let table = std::slice::from_raw_parts(p.add(2).cast::<u8>(), len);
        (len, root, table)
    };
    let _ = len;
    let types = moddil_show::Types::decode(table)?;
    Some(types.render(&Native, root, out as u64))
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
    // The OS, with the program's arguments after its name (`os.1`).
    moddil_rt::set_os(std::env::args().skip(1).collect());
    moddil_rt::set_trap_text(trapped);
    // Room for `main`'s result, aligned for any field, and for an error (`main` cannot fail).
    // SAFETY: the object defines the result's size.
    let size = unsafe { moddil_result_size } as usize;
    let mut out = vec![0u128; size.div_ceil(16).max(16)];
    let mut err = [0u128; 16];
    // SAFETY: the object's `moddil_entry` is `main`, given room for its result and an error.
    let status = unsafe { moddil_entry(out.as_mut_ptr().cast(), err.as_mut_ptr().cast()) };
    moddil_rt::end_run();
    if status == 0 {
        // SAFETY: `main` returned, so `out` holds its result.
        let code = unsafe { moddil_exit_code(out.as_ptr().cast()) };
        // Printed as `moddil run` prints it, unless it is `()` or an `ExitCode`.
        if let Some(text) = shown(out.as_ptr().cast()) {
            println!("{text}");
        }
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

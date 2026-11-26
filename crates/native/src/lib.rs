//! `moddil-native`: the native backend, MIR to Cranelift IR, run in process by Cranelift's JIT
//! (M1, slice C0).
//!
//! [`run_test`] and [`run_main`] compile what a test or `main` reaches, lazily, and run it. Code
//! outside the subset compiled so far gives [`Native::Unsupported`], so a caller can say so or
//! fall back to an interpreter.

pub mod backend;
mod dwarf;
mod host;
mod jit;
pub mod layout;
mod lower;
pub mod show;

use moddil_mir::TrapClass;
use moddil_sema::{Db, Item, Program};
use moddil_syntax::TextRange;

/// Why a run did not give a result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Native {
    /// Something the program reaches is not compiled natively yet: what.
    Unsupported(String),
    /// A check failed: its class, where (the file, by index, and its source), and, for a failed
    /// `assert`, each part of its condition with its value if it was evaluated.
    Trap(TrapClass, u32, TextRange, Vec<(TextRange, Option<String>)>),
    /// A run that returned left this many allocations unfreed: a bug in the backend's drops.
    Leak(i64),
    /// An error left the run: where it was raised (the file and its source), and the error as
    /// Moddil source.
    Failed(u32, TextRange, String),
}

/// An observed `// =>` value: the file, the expression and its value as Moddil source.
pub type Observed = (u32, TextRange, String);

pub use lower::Threads;

/// How a run or a build goes: the order its tasks run in, and how many threads its code may
/// run on.
#[derive(Clone, Copy, Debug, Default)]
pub struct Options {
    /// The deterministic runtime's seed (`task.runtime.1`): with 0, first in, first out.
    pub seed: u64,
    /// How many threads its code may run on.
    pub threads: Threads,
    /// Whether its tasks run under the parallel runtime, one a core (`task.runtime.1`), when its
    /// `main` installs no runtime.
    pub parallel: bool,
}

/// Runs test `index` of `program` natively: its observed values, or why it gave none. The values
/// observed before a trap are given with it.
///
/// # Errors
///
/// [`Native::Unsupported`] if the test reaches code not compiled natively yet, and
/// [`Native::Trap`] if it traps.
pub fn run_test(db: &dyn Db, program: Program, index: u32) -> (Vec<Observed>, Result<(), Native>) {
    run_test_with(db, program, index, Options::default())
}

/// As [`run_test`], with `options`.
pub fn run_test_with(
    db: &dyn Db,
    program: Program,
    index: u32,
    options: Options,
) -> (Vec<Observed>, Result<(), Native>) {
    match lower::compile(db, program, Item::Test(index), options.threads) {
        Err(why) => (Vec::new(), Err(Native::Unsupported(why))),
        Ok(compiled) => {
            let (observed, result, _) = compiled.run(None, options);
            (observed, result.map(|_| ()))
        }
    }
}

/// Runs `fn main()` (function `index`) of `program` natively: its result, as Moddil source.
///
/// # Errors
///
/// [`Native::Unsupported`] if it reaches code not compiled natively yet, and [`Native::Trap`]
/// if it traps.
pub fn run_main(db: &dyn Db, program: Program, index: u32) -> Result<String, Native> {
    run_main_exit(db, program, index, Vec::new()).map(|(shown, _)| shown)
}

/// As [`run_main`], with the program's arguments `args` and the OS (`os.1`), and with the
/// process's exit code if `main` gives an `ExitCode` (`main.exit.1`).
///
/// # Errors
///
/// As [`run_main`].
pub fn run_main_exit(
    db: &dyn Db,
    program: Program,
    index: u32,
    args: Vec<String>,
) -> Result<(String, Option<u8>), Native> {
    run_main_with(db, program, index, args, Options::default())
}

/// As [`run_main_exit`], with `options`.
///
/// # Errors
///
/// As [`run_main`].
pub fn run_main_with(
    db: &dyn Db,
    program: Program,
    index: u32,
    args: Vec<String>,
    options: Options,
) -> Result<(String, Option<u8>), Native> {
    let compiled = lower::compile(db, program, Item::Fn(index), options.threads)
        .map_err(Native::Unsupported)?;
    let (_, result, exit) = compiled.run(Some(args), options);
    result.map(|shown| (shown, exit))
}

/// Compiles `main` (function `index` of `program`) ahead of time: an object file to link with
/// the `moddil-rt-main` static library into an executable.
///
/// # Errors
///
/// [`Native::Unsupported`] if it reaches code not compiled natively yet, or if `main` takes root
/// capabilities or gives something other than `()` or `ExitCode`.
pub fn build_object(db: &dyn Db, program: Program, index: u32) -> Result<Vec<u8>, Native> {
    build_object_with(db, program, index, false, Threads::Many)
}

/// As [`build_object`], with line tables if `release` (as [`build_release_object`]), for code
/// that runs on `threads` threads.
///
/// # Errors
///
/// As [`build_object`].
pub fn build_object_with(
    db: &dyn Db,
    program: Program,
    index: u32,
    release: bool,
    threads: Threads,
) -> Result<Vec<u8>, Native> {
    lower::compile_object(db, program, Item::Fn(index), release, threads)
        .map_err(Native::Unsupported)
}

/// As [`build_object`], for a release build: with the functions' line tables (DWARF), in an
/// ELF or Mach-O object (a COFF one has none yet: debuggers there read PDB).
///
/// # Errors
///
/// As [`build_object`].
pub fn build_release_object(db: &dyn Db, program: Program, index: u32) -> Result<Vec<u8>, Native> {
    build_object_with(db, program, index, true, Threads::Many)
}

/// Compiles the `#[export]` functions `exports` of `program` (`ffi.export.1`) into an object
/// file, each also under its own name with the platform's C convention, to link with the
/// runtime into a shared library.
///
/// # Errors
///
/// What the functions reach that is not compiled to native code yet.
pub fn build_library(db: &dyn Db, program: Program, exports: &[u32]) -> Result<Vec<u8>, Native> {
    lower::compile_library(db, program, exports).map_err(Native::Unsupported)
}

/// The C header for the `#[export]` functions `exports` of `program` (`ffi.export.1`): their
/// prototypes, and the structs their signatures name (each `#[repr(C)]` one in full, the others
/// as incomplete types), as `name.h` gives them.
pub fn header(db: &dyn Db, program: Program, exports: &[u32], name: &str) -> String {
    lower::header(db, program, exports, name)
}

/// This crate's version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests;

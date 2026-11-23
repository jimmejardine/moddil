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

/// Runs test `index` of `program` natively: its observed values, or why it gave none. The values
/// observed before a trap are given with it.
///
/// # Errors
///
/// [`Native::Unsupported`] if the test reaches code not compiled natively yet, and
/// [`Native::Trap`] if it traps.
pub fn run_test(db: &dyn Db, program: Program, index: u32) -> (Vec<Observed>, Result<(), Native>) {
    match lower::compile(db, program, Item::Test(index)) {
        Err(why) => (Vec::new(), Err(Native::Unsupported(why))),
        Ok(compiled) => {
            let (observed, result, _) = compiled.run();
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
    run_main_exit(db, program, index).map(|(shown, _)| shown)
}

/// As [`run_main`], with the process's exit code if `main` gives an `ExitCode` (`main.exit.1`).
///
/// # Errors
///
/// As [`run_main`].
pub fn run_main_exit(
    db: &dyn Db,
    program: Program,
    index: u32,
) -> Result<(String, Option<u8>), Native> {
    let compiled = lower::compile(db, program, Item::Fn(index)).map_err(Native::Unsupported)?;
    let (_, result, exit) = compiled.run();
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
    lower::compile_object(db, program, Item::Fn(index), false).map_err(Native::Unsupported)
}

/// As [`build_object`], for a release build: with the functions' line tables (DWARF), in an
/// ELF or Mach-O object (a COFF one has none yet: debuggers there read PDB).
///
/// # Errors
///
/// As [`build_object`].
pub fn build_release_object(db: &dyn Db, program: Program, index: u32) -> Result<Vec<u8>, Native> {
    lower::compile_object(db, program, Item::Fn(index), true).map_err(Native::Unsupported)
}

/// This crate's version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests;

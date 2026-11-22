//! `moddil-native`: the native backend, MIR to Cranelift IR, run in process by Cranelift's JIT
//! (M1, slice C0).
//!
//! [`run_test`] and [`run_main`] compile what a test or `main` reaches, lazily, and run it. Code
//! outside the subset compiled so far gives [`Native::Unsupported`], so a caller can say so or
//! fall back to an interpreter.

mod host;
mod jit;
mod layout;
mod lower;

use moddil_mir::TrapClass;
use moddil_sema::{Db, Item, Program};
use moddil_syntax::TextRange;

/// Why a run did not give a result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Native {
    /// Something the program reaches is not compiled natively yet: what.
    Unsupported(String),
    /// A check failed: its class, and where (the file, by index, and its source).
    Trap(TrapClass, u32, TextRange),
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
            let (observed, result) = compiled.run();
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
    let compiled = lower::compile(db, program, Item::Fn(index)).map_err(Native::Unsupported)?;
    compiled.run().1
}

/// This crate's version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests;

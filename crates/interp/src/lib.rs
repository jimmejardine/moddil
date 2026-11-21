//! The MIR interpreter: runs analysed or runtime MIR for compile-time evaluation, tests before
//! any backend, the Miri-style checker and the playground.
//!
//! It runs calls on its own stack of frames, not the host's, so deep recursion in a Moddil
//! program is a [`TrapClass::StackOverflow`] trap and never crashes the interpreter. A borrow is
//! a pointer to a frame's local and a path of fields in it; borrows cannot outlive what they
//! point to (the borrow checker sees to that), so the frame is always there. See this crate's
//! `README.md`.
//!
//! Generic code runs once for every type: each frame carries its type arguments, and a call of a
//! trait's method is resolved from them, through a **witness table** built the first time a type
//! is asked for that trait (`types.md`: Generics). Moving a value out of a place leaves the place
//! empty, which is the drop flag: dropping an empty place does nothing.

mod calls;
mod dispatch;
mod machine;
mod natives;
mod ops;
mod stack;
mod value;

use moddil_mir::TrapClass;
use moddil_sema::{Db, Item, Program};
use moddil_syntax::TextRange;

pub use value::{Pointer, Value};

/// A trap: what kind, and the source of the check that failed (or of the call, for a stack
/// overflow).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trap {
    /// What kind of trap.
    pub class: TrapClass,
    /// The file it is in, by its index in the program.
    pub file: u32,
    /// Where.
    pub range: TextRange,
    /// For a failed `assert`: each part of the condition and its value, or `None` if it was not
    /// evaluated.
    pub parts: Vec<(TextRange, Option<Value>)>,
    /// For a failed `?!`: its reason.
    pub reason: Option<String>,
}

/// Why running a function or test stopped without a result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stop {
    /// A trap.
    Trap(Trap),
    /// An error escaped it (a test's, or a function asked to run that can fail): where it left
    /// the function or test, and the error.
    Failed {
        /// The file the error left from.
        file: u32,
        /// Where the error left.
        range: TextRange,
        /// The error.
        error: Value,
    },
}

/// How deeply calls may nest before a [`TrapClass::StackOverflow`] trap.
pub const MAX_DEPTH: usize = 10_000;

/// Reports the values of observed expressions (those followed by `// =>`): their file, range and
/// value.
pub type Observer<'a> = &'a mut dyn FnMut(u32, TextRange, Value);

/// Calls function `index` of `program` with `args`.
///
/// # Panics
///
/// If the file has errors (check `moddil_mir::has_errors` first), or `args` does not match the
/// function's parameters.
///
/// # Errors
///
/// The trap, if the program traps, or the error, if one escapes.
pub fn call_fn(
    db: &dyn Db,
    program: Program,
    index: u32,
    args: &[Value],
    observer: Observer<'_>,
) -> Result<Value, Stop> {
    machine::run(db, program, Item::Fn(index), args, observer)
}

/// Runs test `index` of `program`.
///
/// # Panics
///
/// If the file has errors.
///
/// # Errors
///
/// The trap, if the test traps, or the error, if one escapes it (which fails it).
pub fn run_test(
    db: &dyn Db,
    program: Program,
    index: u32,
    observer: Observer<'_>,
) -> Result<(), Stop> {
    machine::run(db, program, Item::Test(index), &[], observer).map(|_| ())
}

/// This crate's version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests;

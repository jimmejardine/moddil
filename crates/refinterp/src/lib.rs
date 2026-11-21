//! The reference interpreter: the executable form of the language reference, running the
//! canonical AST directly. It depends only on `moddil-syntax`; a test enforces this.
//!
//! It is written for clarity, not speed, and follows the reference paragraph by paragraph rather
//! than the compiler's design: it walks the syntax tree, keeps variables in a list of scopes, and
//! leaves loops and functions through Rust's `?`. It runs **checked programs only**: a file the
//! compiler rejects has no meaning to run, so it does not repeat the checks and may panic on one.
//!
//! Slices S0 to S5. A borrow is a pointer to a variable of some call and a path of fields in it.
//! The prelude (`reference/2027/prelude.moddil`) is read with the program. Values carry enough
//! of their type to choose a method: a trait's method is found from the type of the value it is
//! called on, and a type parameter (for `T::f()`) is known from the values of the arguments
//! given for it, or from type arguments written at the call. Moving a value out of a variable
//! leaves it empty, so it is not dropped again. Derives are built in, structurally. Handlers are
//! kept on a stack in dynamic scope. See this crate's `README.md`.

use std::fmt;

use moddil_syntax::TextRange;

mod machine;
mod memory;
mod program;
mod value;

use program::Program;
use value::Value;

/// The prelude's source.
pub(crate) const PRELUDE: &str = include_str!("../../../reference/2027/prelude.moddil");

/// Why a program traps.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TrapClass {
    /// Integer overflow: arithmetic, negation or a shift (`trap.overflow`).
    Overflow,
    /// Integer division or remainder by zero.
    DivisionByZero,
    /// Calls nested more than [`MAX_DEPTH`] deep.
    StackOverflow,
    /// A failed `assert`.
    Assertion,
    /// An error that `?!` said cannot happen.
    Unexpected,
}

impl fmt::Display for TrapClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            TrapClass::Overflow => "overflow",
            TrapClass::DivisionByZero => "division by zero",
            TrapClass::StackOverflow => "stack overflow",
            TrapClass::Assertion => "assertion failed",
            TrapClass::Unexpected => "unexpected error",
        })
    }
}

/// A trap: what kind, and the source of the operation that failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Trap {
    /// What kind of trap.
    pub class: TrapClass,
    /// Where.
    pub range: TextRange,
}

/// How deeply calls may nest: the call that would make the stack deeper than this traps.
pub const MAX_DEPTH: usize = 10_000;

/// What running one `test` item did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TestRun {
    /// The test's name.
    pub name: String,
    /// Each observed value (an expression followed by `// =>`), in the order observed: its
    /// range and the value as Moddil source.
    pub observed: Vec<(TextRange, String)>,
    /// The trap that stopped the test, if one did.
    pub trap: Option<Trap>,
    /// Where an error left the test, failing it, if one did.
    pub error: Option<TextRange>,
}

/// Runs every `test` item of a checked program, in order.
///
/// The program runs on its own thread with a large stack, since this interpreter recurses on the
/// host's stack.
///
/// `None` if the program could not run: it does not pass the compiler's checks, which this
/// interpreter does not repeat.
pub fn run_tests(text: &str) -> Option<Vec<TestRun>> {
    let text = text.to_string();
    std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(move || {
            let program = Program::new(&text);
            (0..program.tests.len())
                .map(|i| program.run_test(i))
                .collect()
        })
        .ok()?
        .join()
        .ok()
}

/// Leaving an expression other than by producing its value.
pub(crate) enum Flow {
    /// A trap, and whether it has been placed in the file's code yet: a trap in the prelude's
    /// code is reported where the file's code called it.
    Trap(Trap, bool),
    Return(Value),
    /// `break`, with its label (including the `'`) and value.
    Break(Option<String>, Value),
    /// `continue`, with its label.
    Continue(Option<String>),
    /// An error, and where it was last raised (by `fail`, `?`, or a `catch` that let it go on).
    Fail(Value, TextRange),
    /// `?` on `None`: the function returns `None`, or the `try` block is `None`.
    Absent,
}

pub(crate) type Eval<T = Value> = Result<T, Flow>;

pub(crate) fn trap(class: TrapClass, range: TextRange) -> Flow {
    Flow::Trap(Trap { class, range }, false)
}

// Blocks and statements.

// Expressions.

// Patterns.

/// This crate's version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests;

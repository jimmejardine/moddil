//! Types.

use std::fmt;

/// A type. S0 has only the built-in scalar types.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Ty {
    /// `int`: a signed 64-bit integer.
    Int,
    /// `bool`.
    Bool,
    /// `()`.
    Unit,
    /// `!`: the type of expressions that never produce a value, such as `return`.
    Never,
    /// The type of an expression that already has an error; it matches everything, so one error
    /// is reported once.
    Error,
}

impl Ty {
    /// Whether a value of type `self` can be used where `expected` is wanted.
    pub fn fits(self, expected: Ty) -> bool {
        self == expected || matches!(self, Ty::Never | Ty::Error) || expected == Ty::Error
    }
}

impl fmt::Debug for Ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Ty::Int => "int",
            Ty::Bool => "bool",
            Ty::Unit => "()",
            Ty::Never => "!",
            Ty::Error => "{error}",
        })
    }
}

impl fmt::Display for Ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

//! Types.

use std::fmt;
use std::sync::Arc;

/// A type: the built-in scalars, tuples, and the file's structs and enums.
#[derive(Clone, PartialEq, Eq, Hash)]
pub enum Ty {
    /// `int`: a signed 64-bit integer.
    Int,
    /// `bool`.
    Bool,
    /// `()`: the empty tuple.
    Unit,
    /// `!`: the type of expressions that never produce a value, such as `return`.
    Never,
    /// A tuple of one or more elements: `(int, bool)`, `(int,)`.
    Tuple(Arc<[Ty]>),
    /// A struct or enum of this file, by its index in the item tree, with its name.
    Adt(u32, Arc<str>),
    /// The type of an expression that already has an error; it matches everything, so one error
    /// is reported once.
    Error,
}

impl Ty {
    /// Whether a value of type `self` can be used where `expected` is wanted.
    pub fn fits(&self, expected: &Ty) -> bool {
        self == expected || matches!(self, Ty::Never | Ty::Error) || *expected == Ty::Error
    }

    /// Whether the type is or contains an error, so that errors about it would be noise.
    pub fn has_error(&self) -> bool {
        match self {
            Ty::Error => true,
            Ty::Tuple(ts) => ts.iter().any(Ty::has_error),
            _ => false,
        }
    }

    /// The tuple type of `elements` (`()` when there are none).
    pub fn tuple(elements: Vec<Ty>) -> Ty {
        if elements.is_empty() {
            Ty::Unit
        } else {
            Ty::Tuple(elements.into())
        }
    }

    /// The struct or enum's index in the item tree, if it is one.
    pub fn adt(&self) -> Option<u32> {
        match self {
            Ty::Adt(i, _) => Some(*i),
            _ => None,
        }
    }
}

impl fmt::Debug for Ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Ty::Int => f.write_str("int"),
            Ty::Bool => f.write_str("bool"),
            Ty::Unit => f.write_str("()"),
            Ty::Never => f.write_str("!"),
            Ty::Error => f.write_str("{error}"),
            Ty::Adt(_, name) => f.write_str(name),
            Ty::Tuple(ts) => {
                f.write_str("(")?;
                for (i, t) in ts.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{t:?}")?;
                }
                if ts.len() == 1 {
                    f.write_str(",")?;
                }
                f.write_str(")")
            }
        }
    }
}

impl fmt::Display for Ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

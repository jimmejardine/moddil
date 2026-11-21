//! Values, the pointers borrows hold, and the kinds of value methods are chosen by.

/// A value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Value {
    Int(i64),
    Bool(bool),
    Unit,
    Tuple(Vec<Value>),
    /// A struct (whose variant is named like it) or an enum's variant.
    Adt {
        ty: String,
        variant: String,
        fields: Vec<Value>,
    },
    /// A borrow, and whether it is `&mut`.
    Ref(Pointer, bool),
    /// Nothing: a variable whose value was moved out (`own.move.1`), so it is not dropped.
    Moved,
}

/// Where a borrow points: a call's frame, a variable in it, and a path of field positions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Pointer {
    pub(crate) frame: usize,
    pub(crate) slot: usize,
    pub(crate) path: Vec<usize>,
}

/// The kind of a value, which decides which impl a method comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Head {
    Int,
    Bool,
    Unit,
    Tuple,
    Ref,
    Adt(String),
}

impl Value {
    /// The kind of a value, for choosing a method.
    pub(crate) fn head(&self) -> Head {
        match self {
            Value::Int(_) => Head::Int,
            Value::Bool(_) => Head::Bool,
            Value::Unit | Value::Moved => Head::Unit,
            Value::Tuple(_) => Head::Tuple,
            Value::Ref(..) => Head::Ref,
            Value::Adt { ty, .. } => Head::Adt(ty.clone()),
        }
    }
}

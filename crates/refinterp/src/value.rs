//! Values, the pointers borrows hold, and the kinds of value methods are chosen by.

/// A value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Value {
    Int(i64),
    Bool(bool),
    Unit,
    Char(char),
    /// A `&str`: its text (what it points to cannot change while it lives).
    Str(std::sync::Arc<str>),
    String(String),
    Tuple(Vec<Value>),
    /// An array's elements. A slice borrow points to one, or to a `Buf`.
    Array(Vec<Value>),
    /// A `Buf`: its capacity and its elements.
    Buf(usize, Vec<Value>),
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
    /// A closure: its expression and where it was made.
    Closure(std::sync::Arc<Closure>),
    /// An anonymous impl's value (`expr.anon-impl.1`): its trait's key, and its methods by name,
    /// each a closure whose node is the method.
    Anon(String, Vec<(String, std::sync::Arc<Closure>)>),
    /// A variable of another call, seen from a closure's body: the variable itself.
    Alias(Pointer),
}

/// A closure value (`expr.closure.1`): its expression, the variables in scope where it was
/// written (which its body names as its own), and the handlers installed there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Closure {
    pub(crate) node: moddil_syntax::SyntaxNode,
    pub(crate) vars: Vec<(String, Pointer)>,
    pub(crate) handlers: Vec<(String, Value)>,
    pub(crate) types: std::collections::HashMap<String, Head>,
    pub(crate) file: Option<u32>,
    pub(crate) module: usize,
    /// For the closure a wrapper is given (`attr.around.1`): the wrapped function's key, and
    /// the wrapper after which the closure runs the rest.
    pub(crate) wrapped: Option<(String, usize)>,
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
    Char,
    Str,
    String,
    Unit,
    Tuple,
    /// An array or a slice.
    Array,
    /// The argument of a const parameter: a length.
    Len(i64),
    /// A `Buf`, which is also a slice's storage.
    Buf,
    Ref,
    Adt(String),
    Closure,
}

impl Value {
    /// The kind of a value, for choosing a method.
    pub(crate) fn head(&self) -> Head {
        match self {
            Value::Int(_) => Head::Int,
            Value::Bool(_) => Head::Bool,
            Value::Char(_) => Head::Char,
            Value::Str(_) => Head::Str,
            Value::String(_) => Head::String,
            Value::Unit | Value::Moved | Value::Alias(_) => Head::Unit,
            Value::Closure(_) | Value::Anon(..) => Head::Closure,
            Value::Tuple(_) => Head::Tuple,
            Value::Array(_) => Head::Array,
            Value::Buf(..) => Head::Buf,
            Value::Ref(..) => Head::Ref,
            Value::Adt { ty, .. } => Head::Adt(ty.clone()),
        }
    }
}

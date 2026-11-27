//! Values, the pointers borrows hold, and the kinds of value methods are chosen by.

/// A value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Value {
    Int(i64),
    /// A sized integer or a float (`num.types.1`): its kind and its bits.
    Num(crate::num::Kind, u128),
    /// A fractional literal not yet of a type: its text (`num.lit.1`).
    FloatLit(std::sync::Arc<str>),
    /// An integer literal too large for an `int`, not yet of a type.
    BigLit(i128),
    /// An integer literal past `i128::MAX`, not yet of a type: only a `u128` (or a float) holds
    /// it, so the checker let it through only for one.
    HugeLit(u128),
    Bool(bool),
    Unit,
    Char(char),
    /// A `&str`: its text (what it points to cannot change while it lives).
    Str(std::sync::Arc<str>),
    Tuple(Vec<Value>),
    /// An array's elements. A slice borrow points to one, or to a `Buf`.
    Array(Vec<Value>),
    /// A `Buf`: its capacity and its elements.
    Buf(Cap, Vec<Value>),
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
    pub(crate) node: NodeRef,
    pub(crate) vars: Vec<(String, Pointer)>,
    pub(crate) handlers: Vec<(String, Value)>,
    pub(crate) types: std::collections::HashMap<String, Head>,
    pub(crate) file: Option<u32>,
    pub(crate) module: usize,
    /// For the closure a wrapper is given (`attr.around.1`): the wrapped function's key, and
    /// the wrapper after which the closure runs the rest.
    pub(crate) wrapped: Option<(String, usize)>,
}

/// A node of the program's syntax, as values hold it: which tree (the prelude's, `core`'s or a
/// file's), where, and its kind. Every task parses the program itself (a syntax node is not
/// shared between threads), and finds the node in its own trees.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct NodeRef {
    pub(crate) tree: usize,
    pub(crate) range: moddil_syntax::TextRange,
    pub(crate) kind: moddil_syntax::SyntaxKind,
}

/// Where a borrow points: a task's call's frame, a variable in it, and a path of field
/// positions.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Pointer {
    pub(crate) task: usize,
    pub(crate) frame: usize,
    pub(crate) slot: usize,
    pub(crate) path: Vec<usize>,
    /// For a borrow of part of an array (`expr.slice.1`): where the part starts in the array at
    /// `path`, and how long it is.
    pub(crate) span: Option<(usize, usize)>,
}

/// The kind of a value, which decides which impl a method comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Head {
    Int,
    /// A sized integer or a float.
    Num(crate::num::Kind),
    Bool,
    Char,
    Str,
    Unit,
    /// A tuple of this many elements: an impl for pairs is not one for triples.
    Tuple(usize),
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
            Value::Num(k, _) => Head::Num(*k),
            Value::FloatLit(_) => Head::Num(crate::num::Kind::F64),
            Value::BigLit(_) => Head::Num(crate::num::Kind::I128),
            Value::HugeLit(_) => Head::Num(crate::num::Kind::U128),
            Value::Bool(_) => Head::Bool,
            Value::Char(_) => Head::Char,
            Value::Str(_) => Head::Str,
            Value::Unit | Value::Moved | Value::Alias(_) => Head::Unit,
            Value::Closure(_) | Value::Anon(..) => Head::Closure,
            Value::Tuple(xs) => Head::Tuple(xs.len()),
            Value::Array(_) => Head::Array,
            Value::Buf(..) => Head::Buf,
            Value::Ref(..) => Head::Ref,
            Value::Adt { ty, .. } => Head::Adt(ty.clone()),
        }
    }
}

/// A prelude `String` holding `text` (`text.string.inline.1`): inline if it fits in 23 bytes,
/// else on the heap.
pub(crate) fn make_string(text: &str) -> Value {
    let byte = |b: u8| Value::Num(crate::num::Kind::U8, u128::from(b));
    let bytes = text.as_bytes();
    let (variant, fields) = if bytes.len() <= 23 {
        let mut xs: Vec<Value> = bytes.iter().map(|&b| byte(b)).collect();
        xs.resize(23, byte(0));
        ("Inline", vec![Value::Array(xs), byte(bytes.len() as u8)])
    } else {
        (
            "Heap",
            vec![Value::Buf(
                Cap::new(bytes.len()),
                bytes.iter().map(|&b| byte(b)).collect(),
            )],
        )
    };
    Value::Adt {
        ty: "String".to_string(),
        variant: "String".to_string(),
        fields: vec![Value::Adt {
            ty: "StringRepr".to_string(),
            variant: variant.to_string(),
            fields,
        }],
    }
}

/// The text of a prelude `String` value: its bytes, inline or on the heap.
pub(crate) fn string_text(v: &Value) -> Option<String> {
    let Value::Adt { ty, fields, .. } = v else {
        return None;
    };
    if ty != "String" {
        return None;
    }
    let Some(Value::Adt { fields: repr, .. }) = fields.first() else {
        return None;
    };
    let byte = |b: &Value| match b {
        Value::Num(_, b) => *b as u8,
        _ => 0,
    };
    let bytes: Vec<u8> = match repr.as_slice() {
        [Value::Array(xs), n] => {
            let n = match n {
                Value::Num(_, n) => *n as usize,
                Value::Int(n) => *n as usize,
                _ => 0,
            };
            xs.iter().take(n).map(byte).collect()
        }
        [Value::Buf(_, xs)] => xs.iter().map(byte).collect(),
        _ => return None,
    };
    String::from_utf8(bytes).ok()
}

/// A `Buf`'s capacity, and, if it was made where `with Alloc` installed an allocator
/// (`alloc.with.1`), what that allocator gave for it, as `__free_by` takes it.
#[derive(Clone, Debug)]
pub(crate) struct Cap {
    pub(crate) n: usize,
    pub(crate) owner: Option<Box<Value>>,
}

impl Cap {
    /// A capacity of `n`, from the default allocator.
    pub(crate) fn new(n: usize) -> Cap {
        Cap { n, owner: None }
    }
}

// Two buffers are alike by their capacity: where they came from is not part of their value.
impl PartialEq for Cap {
    fn eq(&self, other: &Cap) -> bool {
        self.n == other.n
    }
}

impl Eq for Cap {}

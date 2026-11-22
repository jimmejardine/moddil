//! Values, and the pointers borrows hold.

use moddil_sema::{ItemTree, Shape, Src, Ty};

/// A value.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Value {
    /// An `int`.
    Int(i64),
    /// A sized integer or a float (`num.types.1`): its type and its bits.
    Num(moddil_sema::Num, u128),
    /// A `bool`.
    Bool(bool),
    /// `()`.
    Unit,
    /// A `char`.
    Char(char),
    /// A `&str`: the text it points to. A shared borrow keeps what it points to from changing,
    /// so the text is held, not where it is.
    Str(std::sync::Arc<str>),
    /// A `String`.
    String(String),
    /// A tuple of one or more elements.
    Tuple(Vec<Value>),
    /// An array's elements, in order. A slice borrow points to one, or to a `Buf`.
    Array(Vec<Value>),
    /// A `Buf`: its capacity, and the elements it holds.
    Buf(usize, Vec<Value>),
    /// A struct (variant 0) or an enum's variant, with its fields in order.
    Adt {
        /// The struct or enum, by its index in the file's item tree.
        adt: u32,
        /// The variant.
        variant: u32,
        /// The fields' values.
        fields: Vec<Value>,
    },
    /// A borrow, while the program runs: where it points, and whether it is `&mut`.
    Ref(Pointer, bool),
    /// A `dyn` borrow (`&dyn Time`): where it points, whether it is `&mut`, and the type of what
    /// it points to, which chooses the methods called through it (its **vtable**).
    Dyn(Pointer, bool, Ty),
    /// A closure: what it captures, in order. Its type says which body it runs.
    Closure(Vec<Value>),
    /// A borrow as reported outside the program (an observed value, a result): what it pointed
    /// to, and whether it was `&mut`.
    Borrowed(Box<Value>, bool),
    /// Nothing: a place not assigned yet, or moved out of.
    Empty,
}

/// Where a borrow points: a local of a frame of a task, and a path of field indices into it.
/// Only the frame stack makes one.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Pointer {
    pub(crate) task: usize,
    pub(crate) frame: usize,
    pub(crate) local: u32,
    pub(crate) path: Vec<u32>,
}

impl Value {
    /// The `int` this is.
    pub(crate) fn as_int(&self) -> i64 {
        match self {
            Value::Int(i) => *i,
            other => unreachable!("expected an int, found {other:?}"),
        }
    }

    /// The value as written in Moddil source and in `// =>` expectations: `3`, `-1`, `true`,
    /// `()`, `(1, true)`, `Point { x: 1, y: 2 }`, `Shape::Circle(3)`, `&3`.
    pub fn render(&self, items: &ItemTree) -> String {
        match self {
            Value::Int(v) => v.to_string(),
            Value::Num(n, bits) => crate::num::text(*n, *bits),
            Value::Bool(v) => v.to_string(),
            Value::Unit => "()".to_string(),
            Value::Char(c) => format!("{c:?}"),
            Value::Str(s) => format!("{:?}", &**s),
            Value::String(s) => format!("{s:?}"),
            Value::Array(xs) | Value::Buf(_, xs) => {
                let xs: Vec<String> = xs.iter().map(|x| x.render(items)).collect();
                format!("[{}]", xs.join(", "))
            }
            // A `Vec` is shown as the literal that makes one.
            Value::Adt { adt, fields, .. }
                if items.adts[*adt as usize].src == Src::Prelude
                    && items.adts[*adt as usize].name == "Vec"
                    && fields.len() == 1 =>
            {
                fields[0].render(items)
            }
            Value::Tuple(xs) if xs.len() == 1 => format!("({},)", xs[0].render(items)),
            Value::Tuple(xs) => {
                let xs: Vec<String> = xs.iter().map(|x| x.render(items)).collect();
                format!("({})", xs.join(", "))
            }
            Value::Adt {
                adt,
                variant,
                fields,
            } => {
                let path = items.adts[*adt as usize].path(*variant);
                let def = items.variant(*adt, *variant);
                match def.shape {
                    Shape::Unit => path,
                    Shape::Tuple => {
                        let xs: Vec<String> = fields.iter().map(|x| x.render(items)).collect();
                        format!("{path}({})", xs.join(", "))
                    }
                    Shape::Named => {
                        let xs: Vec<String> = def
                            .fields
                            .iter()
                            .zip(fields)
                            .map(|(f, x)| format!("{}: {}", f.name, x.render(items)))
                            .collect();
                        format!("{path} {{ {} }}", xs.join(", "))
                    }
                }
            }
            Value::Borrowed(v, true) => format!("&mut {}", v.render(items)),
            Value::Borrowed(v, false) => format!("&{}", v.render(items)),
            Value::Ref(..) | Value::Dyn(..) => "&…".to_string(),
            Value::Closure(_) => "{closure}".to_string(),
            Value::Empty => "<moved>".to_string(),
        }
    }
}

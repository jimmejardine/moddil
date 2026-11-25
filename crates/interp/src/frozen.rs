//! Const items' values as compiling freezes them (`comptime.static.1`): evaluated once, then
//! written as static data, with what each pointer in them points to frozen too.

use std::collections::HashMap;
use std::sync::Arc;

use moddil_sema::Num;

use crate::stack::Stack;
use crate::value::{Pointer, Value};

/// A part of a frozen value.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Frozen {
    /// An `int`.
    Int(i64),
    /// A sized integer or a float: its type and its bits.
    Num(Num, u128),
    /// A `bool`.
    Bool(bool),
    /// A `char`.
    Char(char),
    /// `()`.
    Unit,
    /// A `&str`'s text.
    Str(Arc<str>),
    /// A tuple's elements.
    Tuple(Vec<Frozen>),
    /// An array's elements.
    Array(Vec<Frozen>),
    /// A `Buf`'s elements (its capacity is how many there are).
    Buf(Vec<Frozen>),
    /// A struct (variant 0) or an enum's variant, by its index in the item tree, with its
    /// fields.
    Adt {
        /// The struct or enum.
        adt: u32,
        /// The variant.
        variant: u32,
        /// The fields.
        fields: Vec<Frozen>,
    },
    /// A borrow or a raw pointer: to the frozen value's pointee of this index.
    Ptr(usize),
}

/// A const item's value as compiling froze it: the value, and what its pointers point to, each
/// once however many point to it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FrozenValue {
    /// The value.
    pub root: Frozen,
    /// What its pointers point to, by [`Frozen::Ptr`]'s index.
    pub pointees: Vec<Frozen>,
}

/// Freezes `v`, a value a run of `stack` gave: its pointers followed into the stack's static
/// data and heap. What cannot be data (a closure, a `dyn` value or borrow, a C pointer or
/// function pointer, a borrow of part of an array) is the error, named.
pub(crate) fn freeze(stack: &Stack<'_>, v: &Value) -> Result<FrozenValue, String> {
    let mut f = Freezer {
        stack,
        seen: HashMap::new(),
        pointees: Vec::new(),
    };
    let root = f.part(v)?;
    Ok(FrozenValue {
        root,
        pointees: f.pointees,
    })
}

struct Freezer<'s, 'a> {
    stack: &'s Stack<'a>,
    /// Each pointer followed so far: its pointee's index.
    seen: HashMap<Pointer, usize>,
    pointees: Vec<Frozen>,
}

impl Freezer<'_, '_> {
    fn parts(&mut self, xs: &[Value]) -> Result<Vec<Frozen>, String> {
        xs.iter().map(|x| self.part(x)).collect()
    }

    fn part(&mut self, v: &Value) -> Result<Frozen, String> {
        Ok(match v {
            Value::Int(i) => Frozen::Int(*i),
            Value::Num(n, bits) => Frozen::Num(*n, *bits),
            Value::Bool(b) => Frozen::Bool(*b),
            Value::Char(c) => Frozen::Char(*c),
            Value::Unit => Frozen::Unit,
            Value::Str(s) => Frozen::Str(s.clone()),
            Value::Tuple(xs) => Frozen::Tuple(self.parts(xs)?),
            Value::Array(xs) => Frozen::Array(self.parts(xs)?),
            Value::Buf(_, xs) => Frozen::Buf(self.parts(xs)?),
            Value::Adt {
                adt,
                variant,
                fields,
            } => Frozen::Adt {
                adt: *adt,
                variant: *variant,
                fields: self.parts(fields)?,
            },
            // A slice borrow: the elements it covers.
            Value::Ref(p, _) if let Some((start, len)) = p.span => {
                if let Some(&i) = self.seen.get(p) {
                    return Ok(Frozen::Ptr(i));
                }
                let i = self.pointees.len();
                self.seen.insert(p.clone(), i);
                self.pointees.push(Frozen::Unit);
                let mut whole = p.clone();
                whole.span = None;
                let xs = match self.stack.load(&whole) {
                    Value::Array(xs) | Value::Buf(_, xs) => xs,
                    other => return Err(format!("a borrow of part of {other:?}")),
                };
                let part = xs
                    .get(start as usize..(start + len) as usize)
                    .ok_or("a borrow past an array's end")?
                    .to_vec();
                self.pointees[i] = Frozen::Array(self.parts(&part)?);
                Frozen::Ptr(i)
            }
            Value::Ref(p, _) => {
                if let Some(&i) = self.seen.get(p) {
                    return Ok(Frozen::Ptr(i));
                }
                let i = self.pointees.len();
                self.seen.insert(p.clone(), i);
                self.pointees.push(Frozen::Unit);
                let pointee = self.stack.load(p);
                self.pointees[i] = self.part(&pointee)?;
                Frozen::Ptr(i)
            }
            Value::Dyn(..) | Value::Owned(_) => return Err("a `dyn` value".to_string()),
            Value::Closure(_) => return Err("a closure".to_string()),
            Value::Foreign(_) => return Err("a pointer C gave".to_string()),
            Value::FnPtr(_) => return Err("a function pointer".to_string()),
            Value::Borrowed(..) | Value::Empty => {
                return Err("a value moved out of".to_string());
            }
        })
    }
}

/// A frozen value as a run holds it (`comptime.static.1`): what its pointers point to put on
/// `stack`'s heap, where nothing frees it.
pub(crate) fn thaw(stack: &mut Stack<'_>, fv: &FrozenValue) -> Value {
    let slots: Vec<Pointer> = fv
        .pointees
        .iter()
        .map(|_| match stack.alloc(Value::Unit) {
            Value::Ref(p, _) => p,
            other => unreachable!("a heap pointer: {other:?}"),
        })
        .collect();
    for (p, v) in slots.iter().zip(&fv.pointees) {
        let v = thawed(&slots, v);
        stack.store(p, v);
    }
    thawed(&slots, &fv.root)
}

fn thawed(slots: &[Pointer], v: &Frozen) -> Value {
    let all = |xs: &[Frozen]| xs.iter().map(|x| thawed(slots, x)).collect();
    match v {
        Frozen::Int(i) => Value::Int(*i),
        Frozen::Num(n, bits) => Value::Num(*n, *bits),
        Frozen::Bool(b) => Value::Bool(*b),
        Frozen::Char(c) => Value::Char(*c),
        Frozen::Unit => Value::Unit,
        Frozen::Str(s) => Value::Str(s.clone()),
        Frozen::Tuple(xs) => Value::Tuple(all(xs)),
        Frozen::Array(xs) => Value::Array(all(xs)),
        Frozen::Buf(xs) => Value::Buf(xs.len(), all(xs)),
        Frozen::Adt {
            adt,
            variant,
            fields,
        } => Value::Adt {
            adt: *adt,
            variant: *variant,
            fields: all(fields),
        },
        Frozen::Ptr(i) => Value::Ref(slots[*i].clone(), true),
    }
}

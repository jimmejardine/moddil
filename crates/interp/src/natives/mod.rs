//! The prelude's built-in functions (`#[intrinsic]`): the core ones here, `Buf`'s in [`buf`],
//! and the text types' in [`text`].

mod buf;
mod text;

use moddil_mir::TrapClass;
use moddil_sema::{Container, ItemTree, Src, Ty};

use crate::stack::Stack;
use crate::value::{Pointer, Value};

/// Runs the built-in functions, and makes the prelude's values they return.
pub(crate) struct Natives<'a> {
    items: &'a ItemTree,
    prelude: Prelude,
}

/// The prelude's `Option` and `Ordering`, by index, found once.
struct Prelude {
    option: u32,
    some: u32,
    none: u32,
    ordering: u32,
    less: u32,
    equal: u32,
    greater: u32,
    task: u32,
}

impl<'a> Natives<'a> {
    pub(crate) fn new(items: &'a ItemTree) -> Self {
        let adt = |name: &str| {
            items
                .adts
                .iter()
                .position(|a| a.src == Src::Prelude && a.name == name)
                .expect("a prelude type") as u32
        };
        let variant = |adt: u32, name: &str| {
            items.adts[adt as usize]
                .variants
                .iter()
                .position(|v| v.name == name)
                .expect("a variant") as u32
        };
        let (option, ordering) = (adt("Option"), adt("Ordering"));
        Natives {
            items,
            prelude: Prelude {
                option,
                some: variant(option, "Some"),
                none: variant(option, "None"),
                ordering,
                less: variant(ordering, "Less"),
                equal: variant(ordering, "Equal"),
                greater: variant(ordering, "Greater"),
                task: adt("Task"),
            },
        }
    }

    /// Runs built-in function `f` on `args`, reading and writing through borrows in `stack`.
    pub(crate) fn call(
        &self,
        f: u32,
        args: &[Value],
        stack: &mut Stack<'_>,
    ) -> Result<Value, TrapClass> {
        let sig = &self.items.fns[f as usize];
        let self_ty = match sig.container {
            Container::Impl(k) => self.items.impls[k as usize].self_ty.clone(),
            _ => Ty::Unit,
        };
        let name = sig.name.as_str();
        Ok(match (&self_ty, name) {
            (Ty::Buf(_), _) => return self.buf(name, args, stack),
            (Ty::Unit, "replace") => {
                let p = pointer(&args[0]);
                let old = stack.load(&p);
                stack.store(&p, args[1].clone());
                old
            }
            (Ty::Unit, "swap") => {
                let (a, b) = (pointer(&args[0]), pointer(&args[1]));
                let (x, y) = (stack.load(&a), stack.load(&b));
                stack.store(&a, y);
                stack.store(&b, x);
                Value::Unit
            }
            (Ty::Int, "wrapping_add") => {
                Value::Int(args[0].as_int().wrapping_add(args[1].as_int()))
            }
            (Ty::Int, "wrapping_sub") => {
                Value::Int(args[0].as_int().wrapping_sub(args[1].as_int()))
            }
            (Ty::Int, "wrapping_mul") => {
                Value::Int(args[0].as_int().wrapping_mul(args[1].as_int()))
            }
            (Ty::Int, "rotate_left") => Value::Int(
                args[0]
                    .as_int()
                    .rotate_left(args[1].as_int().rem_euclid(64) as u32),
            ),
            _ => self.text(&self_ty, name, args, stack),
        })
    }

    /// A value of the prelude's `Task`, for task `t`.
    pub(crate) fn task(&self, t: usize) -> Value {
        Value::Adt {
            adt: self.prelude.task,
            variant: 0,
            fields: vec![Value::Int(t as i64)],
        }
    }

    /// A value of the prelude's `Option`.
    fn option(&self, v: Option<Value>) -> Value {
        let p = &self.prelude;
        Value::Adt {
            adt: p.option,
            variant: if v.is_some() { p.some } else { p.none },
            fields: v.into_iter().collect(),
        }
    }

    /// A value of the prelude's `Ordering`.
    fn ordering(&self, o: std::cmp::Ordering) -> Value {
        let p = &self.prelude;
        Value::Adt {
            adt: p.ordering,
            variant: match o {
                std::cmp::Ordering::Less => p.less,
                std::cmp::Ordering::Equal => p.equal,
                std::cmp::Ordering::Greater => p.greater,
            },
            fields: Vec::new(),
        }
    }
}

/// Where a borrow argument points.
fn pointer(v: &Value) -> Pointer {
    match v {
        Value::Ref(p, _) => p.clone(),
        other => unreachable!("a borrow: {other:?}"),
    }
}

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
            },
        }
    }

    /// Runs built-in function `f` on `args`, reading and writing through borrows in `stack`.
    pub(crate) fn call(
        &self,
        f: u32,
        args: &[Value],
        type_args: &[Ty],
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
            // A byte of the text (`text.string.inline.1`).
            (Ty::Str, "byte_at") => {
                let Value::Str(s) = stack.deref(&args[0]) else {
                    unreachable!("text")
                };
                let i = args[1].as_int();
                return usize::try_from(i)
                    .ok()
                    .and_then(|i| s.as_bytes().get(i).copied())
                    .map(|b| Value::Num(moddil_sema::Num::U8, u128::from(b)))
                    .ok_or(TrapClass::IndexOutOfBounds);
            }
            // The conversions (`num.convert.1`): to the type the call names, its last type
            // argument.
            (Ty::Int | Ty::Num(_), "checked_to" | "truncate" | "wrap" | "saturate") => {
                let to = type_args.last().expect("the type converted to");
                let v = crate::num::convert(name, &stack.deref(&args[0]), to);
                if name == "checked_to" {
                    self.option(v)
                } else {
                    v.expect("an integer conversion always gives a value")
                }
            }
            // The float functions (`num.float.fn.1`).
            (Ty::Num(n), _) if n.is_float() && name != "text" => {
                let x = |i: usize| match args.get(i).map(|a| stack.deref(a)) {
                    Some(Value::Num(_, bits)) => crate::num::float(*n, bits),
                    _ => 0.0,
                };
                crate::num::float_fn(*n, name, x(0), x(1)).expect("a float function")
            }
            (Ty::Unit, "replace") => {
                let p = pointer(&args[0]);
                let old = stack.load(&p);
                stack.store(&p, args[1].clone());
                old
            }
            // The text between two byte offsets (`text.pos.1`): they are boundaries, in order.
            (Ty::Str, "bytes_between") => {
                let Value::Str(s) = stack.deref(&args[0]) else {
                    unreachable!("text")
                };
                let (a, b) = (args[1].as_int(), args[2].as_int());
                return usize::try_from(a)
                    .ok()
                    .zip(usize::try_from(b).ok())
                    .and_then(|(a, b)| s.get(a..b))
                    .map(|t| Value::Str(t.into()))
                    .ok_or(TrapClass::IndexOutOfBounds);
            }
            // The two parts of an `&mut` slice borrow, disjoint (`expr.slice.1`).
            (Ty::Slice(_), "split_at_mut_unchecked") => {
                let p = pointer(&args[0]);
                let (at, n) = (args[1].as_int(), args[2].as_int());
                if at < 0 || at > n {
                    return Err(TrapClass::IndexOutOfBounds);
                }
                let (at, n) = (at as u32, n as u32);
                let start = p.span.map_or(0, |(s, _)| s);
                let mut a = p.clone();
                a.span = Some((start, at));
                let mut b = p;
                b.span = Some((start + at, n - at));
                Value::Tuple(vec![Value::Ref(a, true), Value::Ref(b, true)])
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
            // The ambient log (`obs.log.1`).
            (Ty::Adt(a, ..), "debug" | "info" | "warn" | "error")
                if self.items.adts[*a as usize].name == "Log" =>
            {
                let msg = crate::value::string_text(self.items, &args[1])
                    .unwrap_or_else(|| format!("{:?}", args[1]));
                eprintln!("[{name}] {msg}");
                Value::Unit
            }
            _ => self.text(&self_ty, name, args, stack),
        })
    }

    /// A handle of the prelude's: a `Task`, `Sender` or `Receiver` for index `i`.
    pub(crate) fn handle(&self, name: &str, i: usize) -> Value {
        Value::Adt {
            adt: self.prelude_adt(name),
            variant: 0,
            fields: vec![Value::Int(i as i64)],
        }
    }

    /// A value of the prelude's struct `name`, which has no fields (`Closed`).
    pub(crate) fn unit_struct(&self, name: &str) -> Value {
        Value::Adt {
            adt: self.prelude_adt(name),
            variant: 0,
            fields: Vec::new(),
        }
    }

    /// The prelude's `Trapped { class, at }`.
    pub(crate) fn trapped(&self, class: &str, at: &str) -> Value {
        Value::Adt {
            adt: self.prelude_adt("Trapped"),
            variant: 0,
            fields: vec![
                crate::value::make_string(self.items, class),
                crate::value::make_string(self.items, at),
            ],
        }
    }

    /// `Some(v)`, or `None`.
    pub(crate) fn option_of(&self, v: Option<Value>) -> Value {
        self.option(v)
    }

    fn prelude_adt(&self, name: &str) -> u32 {
        self.items
            .adts
            .iter()
            .position(|a| a.src == Src::Prelude && a.name == name)
            .expect("a prelude type") as u32
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

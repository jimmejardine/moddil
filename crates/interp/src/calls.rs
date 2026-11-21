//! What a call runs. A `Call` terminator names a function or a trait's method; this decides the
//! body that runs and the arguments it gets, looking through `dyn` borrows, anonymous impls and
//! closures.

use moddil_sema::{Callee, Item, Ty};

use crate::dispatch::{Dispatch, Target};
use crate::stack::Stack;
use crate::value::Value;

/// How to run a call.
pub(crate) enum CallPlan {
    /// A closure's body, with its captures and its arguments as two values, and the type
    /// arguments of the body it is written in.
    Closure {
        item: Item,
        args: Vec<Value>,
        type_args: Vec<Ty>,
    },
    /// A function's body.
    Fn(Target, Vec<Value>),
    /// One of the prelude's built-in functions.
    Intrinsic(u32, Vec<Value>),
    /// The built-in `Clone` of a borrow or tuple: this copy is the result.
    Copy(Value),
}

/// The plan for calling `callee` with `values`, from the top frame of `stack`.
pub(crate) fn plan(
    callee: &Callee,
    mut values: Vec<Value>,
    stack: &Stack<'_>,
    dispatch: &mut Dispatch<'_>,
) -> CallPlan {
    let concrete = |d: &Dispatch<'_>, t: &Ty| d.concrete(t, stack.type_args());
    // A method called through a `dyn` borrow is the one of the type it points to.
    let callee = match callee {
        Callee::Trait { func, args } if matches!(concrete(dispatch, &args[0]), Ty::Dyn(_)) => {
            let mut args = args.clone();
            if let Some(Value::Dyn(ptr, mutable, t)) = values.first().cloned() {
                values[0] = Value::Ref(ptr, mutable);
                args[0] = t;
            }
            Callee::Trait { func: *func, args }
        }
        c => c.clone(),
    };
    // An anonymous impl's method is a closure of its own: its value is the method's, the
    // arguments but `self` its arguments. One it leaves out is the trait's default.
    let mut closure_ty = None;
    if let Callee::Trait { func, args } = &callee {
        match concrete(dispatch, &args[0]) {
            Ty::Anon(a) => {
                if let Some(k) = a.methods.iter().position(|(f, _)| f == func) {
                    let Value::Tuple(methods) = stack.deref(&values[0]) else {
                        unreachable!("an anonymous impl's value")
                    };
                    let rest = values.split_off(1);
                    values = vec![methods[k].clone(), tuple(rest)];
                    closure_ty = Some(a.methods[k].1.clone());
                }
            }
            t @ Ty::Closure(_) => closure_ty = Some(t),
            _ => {}
        }
    }
    if let Some(Ty::Closure(c)) = closure_ty {
        let Value::Closure(captures) = stack.deref(&values[0]) else {
            unreachable!("a closure's value")
        };
        return CallPlan::Closure {
            item: Item::closure(c.owner, c.expr),
            args: vec![tuple(captures), values[1].clone()],
            type_args: c.args.to_vec(),
        };
    }
    match dispatch.target(&callee, stack.type_args()) {
        Some((f, _)) if dispatch.items().fns[f as usize].intrinsic => {
            CallPlan::Intrinsic(f, values)
        }
        Some(target) => CallPlan::Fn(target, values),
        None => CallPlan::Copy(stack.deref(&values[0])),
    }
}

/// Values as one argument: `()` for none, else a tuple.
fn tuple(xs: Vec<Value>) -> Value {
    if xs.is_empty() {
        Value::Unit
    } else {
        Value::Tuple(xs)
    }
}

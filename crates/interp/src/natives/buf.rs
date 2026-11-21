//! The methods of `Buf<T>` (`collections.buf.1`).

use moddil_mir::TrapClass;

use super::{Natives, pointer};
use crate::stack::Stack;
use crate::value::Value;

impl Natives<'_> {
    pub(super) fn buf(
        &self,
        name: &str,
        args: &[Value],
        stack: &mut Stack<'_>,
    ) -> Result<Value, TrapClass> {
        let load = |stack: &Stack<'_>, v: &Value| match stack.load(&pointer(v)) {
            Value::Buf(cap, xs) => (cap, xs),
            other => unreachable!("a `Buf`: {other:?}"),
        };
        Ok(match name {
            "with_capacity" => {
                Value::Buf(usize::try_from(args[0].as_int()).unwrap_or(0), Vec::new())
            }
            "from_array" => match args[0].clone() {
                Value::Array(xs) => Value::Buf(xs.len(), xs),
                other => unreachable!("an array: {other:?}"),
            },
            "len" => Value::Int(load(stack, &args[0]).1.len() as i64),
            "capacity" => Value::Int(load(stack, &args[0]).0 as i64),
            "as_slice" => Value::Ref(pointer(&args[0]), false),
            "as_mut_slice" => Value::Ref(pointer(&args[0]), true),
            _ => {
                let (mut cap, mut xs) = load(stack, &args[0]);
                let at =
                    |i: &Value, len: usize| usize::try_from(i.as_int()).ok().filter(|&i| i < len);
                let out = match name {
                    "set_capacity" => {
                        cap = usize::try_from(args[1].as_int()).unwrap_or(0).max(xs.len());
                        Value::Unit
                    }
                    "push" | "insert" if xs.len() >= cap => {
                        return Err(TrapClass::CapacityExceeded);
                    }
                    "push" => {
                        xs.push(args[1].clone());
                        Value::Unit
                    }
                    "pop" => self.option(xs.pop()),
                    "insert" => {
                        let i = at(&args[1], xs.len() + 1).ok_or(TrapClass::IndexOutOfBounds)?;
                        xs.insert(i, args[2].clone());
                        Value::Unit
                    }
                    "remove" => {
                        let i = at(&args[1], xs.len()).ok_or(TrapClass::IndexOutOfBounds)?;
                        xs.remove(i)
                    }
                    "swap" => {
                        let i = at(&args[1], xs.len()).ok_or(TrapClass::IndexOutOfBounds)?;
                        let j = at(&args[2], xs.len()).ok_or(TrapClass::IndexOutOfBounds)?;
                        xs.swap(i, j);
                        Value::Unit
                    }
                    other => unreachable!("no built-in `{other}` for `Buf`"),
                };
                stack.store(&pointer(&args[0]), Value::Buf(cap, xs));
                out
            }
        })
    }
}

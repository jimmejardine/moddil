//! The built-in methods of the text types (`char`, `str`, `String`), their `Display`, and the
//! slice `len`.

use moddil_sema::Ty;

use super::{Natives, pointer};
use crate::stack::Stack;
use crate::value::Value;

impl Natives<'_> {
    pub(super) fn text(
        &self,
        self_ty: &Ty,
        name: &str,
        args: &[Value],
        stack: &mut Stack<'_>,
    ) -> Value {
        // An argument as a value: a borrow is read, a `&str` is its text.
        let arg = |stack: &Stack<'_>, i: usize| stack.deref(&args[i]);
        // A `Pattern` (`text.pattern.1`): text, or a character.
        let pattern = |stack: &Stack<'_>, i: usize| -> String {
            match arg(stack, i) {
                Value::Str(s) => s.to_string(),
                Value::String(s) => s,
                Value::Char(c) => c.to_string(),
                other => unreachable!("a pattern: {other:?}"),
            }
        };
        let text = |stack: &Stack<'_>, i: usize| -> String {
            match arg(stack, i) {
                Value::Str(s) => s.to_string(),
                Value::String(s) => s,
                other => unreachable!("text: {other:?}"),
            }
        };
        let ch = |stack: &Stack<'_>, i: usize| match arg(stack, i) {
            Value::Char(c) => c,
            other => unreachable!("a `char`: {other:?}"),
        };
        match (self_ty, name) {
            (Ty::Int | Ty::Num(_) | Ty::Bool | Ty::Char, "fmt") => {
                let s = match arg(stack, 0) {
                    Value::Int(v) => v.to_string(),
                    Value::Num(n, bits) => crate::num::text(n, bits),
                    Value::Bool(v) => v.to_string(),
                    Value::Char(c) => c.to_string(),
                    other => unreachable!("`Display` of {other:?}"),
                };
                edit(stack, &args[1], |out| {
                    out.push_str(&s);
                    Value::Unit
                })
            }
            (Ty::Char, "code") => Value::Int(i64::from(u32::from(ch(stack, 0)))),
            (Ty::Char, "from_code") => {
                let c = u32::try_from(arg(stack, 0).as_int())
                    .ok()
                    .and_then(char::from_u32);
                self.option(c.map(Value::Char))
            }
            (Ty::Char, "is_alphabetic") => Value::Bool(ch(stack, 0).is_alphabetic()),
            (Ty::Char, "is_numeric") => Value::Bool(ch(stack, 0).is_numeric()),
            (Ty::Char, "is_whitespace") => Value::Bool(ch(stack, 0).is_whitespace()),
            (Ty::Char, "is_uppercase") => Value::Bool(ch(stack, 0).is_uppercase()),
            (Ty::Char, "is_lowercase") => Value::Bool(ch(stack, 0).is_lowercase()),
            (Ty::Char, "len_utf8") => Value::Int(ch(stack, 0).len_utf8() as i64),
            (Ty::Str, "len") => Value::Int(text(stack, 0).len() as i64),
            (Ty::Str, "quoted") => Value::String(format!("{:?}", text(stack, 0))),
            (Ty::Char, "quoted") => Value::String(format!("{:?}", ch(stack, 0))),
            (Ty::Str, "char_at_byte") => {
                let at = arg(stack, 1).as_int();
                let s = text(stack, 0);
                let c = usize::try_from(at)
                    .ok()
                    .and_then(|at| s.get(at..))
                    .and_then(|rest| rest.chars().next());
                self.option(c.map(Value::Char))
            }
            (Ty::Slice(_), "len") => match arg(stack, 0) {
                Value::Array(xs) | Value::Buf(_, xs) => Value::Int(xs.len() as i64),
                other => unreachable!("a slice: {other:?}"),
            },
            (Ty::Str, "starts_with") => Value::Bool(text(stack, 0).starts_with(&pattern(stack, 1))),
            (Ty::Str, "ends_with") => Value::Bool(text(stack, 0).ends_with(&pattern(stack, 1))),
            (Ty::Str, "contains") => Value::Bool(text(stack, 0).contains(&pattern(stack, 1))),
            (Ty::Str, "trim") => Value::Str(text(stack, 0).trim().into()),
            (Ty::Str, "to_uppercase") => Value::String(text(stack, 0).to_uppercase()),
            (Ty::Str, "to_lowercase") => Value::String(text(stack, 0).to_lowercase()),
            (Ty::Str, "eq") => Value::Bool(text(stack, 0) == text(stack, 1)),
            (Ty::Str, "cmp") => self.ordering(text(stack, 0).cmp(&text(stack, 1))),
            (Ty::String, "new") => Value::String(String::new()),
            (Ty::String, "as_str") => Value::Str(text(stack, 0).into()),
            (Ty::String, "clone") => Value::String(text(stack, 0)),
            (Ty::String, "push_str") => {
                let s = text(stack, 1);
                edit(stack, &args[0], |out| {
                    out.push_str(&s);
                    Value::Unit
                })
            }
            (Ty::String, "push") => {
                let c = ch(stack, 1);
                edit(stack, &args[0], |out| {
                    out.push(c);
                    Value::Unit
                })
            }
            (Ty::String, "clear") => edit(stack, &args[0], |out| {
                out.clear();
                Value::Unit
            }),
            (Ty::String, "pop") => {
                let c = edit(stack, &args[0], |out| {
                    out.pop().map_or(Value::Empty, Value::Char)
                });
                self.option((c != Value::Empty).then_some(c))
            }
            (t, n) => unreachable!("no built-in `{n}` for `{t}`"),
        }
    }
}

/// Changes the `String` that `target` (a `&mut String`) borrows.
fn edit(stack: &mut Stack<'_>, target: &Value, f: impl FnOnce(&mut String) -> Value) -> Value {
    let p = pointer(target);
    let Value::String(mut s) = stack.load(&p) else {
        unreachable!("a `String`")
    };
    let v = f(&mut s);
    stack.store(&p, Value::String(s));
    v
}

//! The built-in methods of the text types (`char`, `str`, `String`), their `Display`, and the
//! slice `len`.

use moddil_sema::Ty;

use super::Natives;
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
        let text = |stack: &Stack<'_>, i: usize| -> String {
            match arg(stack, i) {
                Value::Str(s) => s.to_string(),
                other => unreachable!("text: {other:?}"),
            }
        };
        let ch = |stack: &Stack<'_>, i: usize| match arg(stack, i) {
            Value::Char(c) => c,
            other => unreachable!("a `char`: {other:?}"),
        };
        let str_of = |s: String| Value::Str(s.into());
        match (self_ty, name) {
            // What `Display` writes (`text.display.1`).
            (Ty::Int | Ty::Num(_) | Ty::Bool | Ty::Char, "text") => str_of(match arg(stack, 0) {
                Value::Int(v) => v.to_string(),
                Value::Num(n, bits) => crate::num::text(n, bits),
                Value::Bool(v) => v.to_string(),
                Value::Char(c) => c.to_string(),
                other => unreachable!("`Display` of {other:?}"),
            }),
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
            (Ty::Str, "quoted_text") => str_of(format!("{:?}", text(stack, 0))),
            (Ty::Char, "quoted_text") => str_of(format!("{:?}", ch(stack, 0))),
            (Ty::Str, "char_at_byte") => {
                let at = arg(stack, 1).as_int();
                let s = text(stack, 0);
                let c = usize::try_from(at)
                    .ok()
                    .and_then(|at| s.get(at..))
                    .and_then(|rest| rest.chars().next());
                self.option(c.map(Value::Char))
            }
            // Its bytes (`text.bytes.1`): a copy on the heap, borrowed, as text holds none.
            (Ty::Str, "as_bytes") => {
                let bytes = text(stack, 0)
                    .as_bytes()
                    .iter()
                    .map(|b| Value::Num(moddil_sema::Num::U8, u128::from(*b)))
                    .collect();
                match stack.alloc(Value::Array(bytes)) {
                    Value::Ref(p, _) => Value::Ref(p, false),
                    other => unreachable!("a heap pointer: {other:?}"),
                }
            }
            // The first `len` bytes of a byte slice, which are UTF-8, as text.
            (Ty::Str, "view") => {
                let bytes = match arg(stack, 0) {
                    Value::Array(xs) | Value::Buf(_, xs) => xs,
                    other => unreachable!("bytes: {other:?}"),
                };
                let len = usize::try_from(arg(stack, 1).as_int()).unwrap_or(0);
                let bytes: Vec<u8> = bytes
                    .iter()
                    .take(len)
                    .map(|b| match b {
                        Value::Num(_, b) => *b as u8,
                        other => unreachable!("a byte: {other:?}"),
                    })
                    .collect();
                str_of(String::from_utf8(bytes).expect("a `String` holds UTF-8"))
            }
            (Ty::Slice(_), "len") => match arg(stack, 0) {
                Value::Array(xs) | Value::Buf(_, xs) => Value::Int(xs.len() as i64),
                other => unreachable!("a slice: {other:?}"),
            },
            (Ty::Str, "trim") => str_of(text(stack, 0).trim().to_string()),
            (Ty::Str, "upper_text") => str_of(text(stack, 0).to_uppercase()),
            (Ty::Str, "lower_text") => str_of(text(stack, 0).to_lowercase()),
            (Ty::Str, "eq") => Value::Bool(text(stack, 0) == text(stack, 1)),
            (Ty::Str, "cmp") => self.ordering(text(stack, 0).cmp(&text(stack, 1))),
            (t, n) => unreachable!("no built-in `{n}` for `{t}`"),
        }
    }
}

//! Tagged literals with fields (`comptime.tag.fields.1`), as the reference interpreter runs
//! them: it has no types to make a struct of, so a literal whose value implements `TagFields`
//! is its value wrapped with its fields' names, and `read` gives a value whose fields are found
//! by those names. Anything else asked of the wrapper is asked of its value, as through `Deref`.

use moddil_syntax::TextRange;

use super::Machine;
use crate::value::{Pointer, Value};
use crate::{Eval, TrapClass, trap};

/// The type of a tagged literal's wrapper: its variant is its fields, each name followed by `!`
/// if every reading has it or `?`, and `,`.
pub(crate) const WRAPPER: &str = "<tagged literal>";
/// The type of what a wrapper's `read` gives: its variant is its fields' names, and `,`.
pub(crate) const FIELDS: &str = "<tagged literal's fields>";

impl Machine<'_> {
    /// `value`, a tag's, wrapped with its fields if it implements `TagFields`.
    pub(super) fn with_fields(&mut self, value: Value, range: TextRange) -> Eval {
        let head = value.head();
        let Some(m) = self.program.trait_method("TagFields", &head, "field_names") else {
            return Ok(value);
        };
        let at = self.memory.temp(value.clone());
        let names = self.run_method(m, &head, vec![Value::Ref(at, false)], range)?;
        let mut spec = String::new();
        for name in elements(&names) {
            if let Value::Tuple(parts) = name
                && let [n, Value::Bool(every)] = parts.as_slice()
            {
                spec.push_str(&crate::value::string_text(n).unwrap_or_default());
                spec.push(if *every { '!' } else { '?' });
                spec.push(',');
            }
        }
        Ok(Value::Adt {
            ty: WRAPPER.to_string(),
            variant: spec,
            fields: vec![value],
        })
    }

    /// `w.read(text)` for the wrapper at `w` with fields `spec`: `None` if its value finds none
    /// in `text`, or else its fields; a field every reading has but this one does not traps.
    pub(super) fn read_fields(
        &mut self,
        w: &Pointer,
        spec: &str,
        text: Value,
        range: TextRange,
    ) -> Eval {
        let mut inner = w.clone();
        inner.path.push(0);
        let head = self.memory.load(&inner).head();
        let m = self
            .program
            .trait_method("TagFields", &head, "field_values")
            .expect("a checked `TagFields`");
        let got = self.run_method(m, &head, vec![Value::Ref(inner, false), text], range)?;
        let Value::Adt {
            variant, fields, ..
        } = &got
        else {
            unreachable!("an `Option`")
        };
        if variant == "None" {
            return Ok(got);
        }
        let values = elements(&fields[0]);
        let mut names = Vec::new();
        let mut out = Vec::new();
        for (field, v) in spec.split(',').filter(|f| !f.is_empty()).zip(values) {
            let every = field.ends_with('!');
            names.push(&field[..field.len() - 1]);
            out.push(match v {
                Value::Adt {
                    variant,
                    mut fields,
                    ..
                } if every => {
                    if variant == "None" {
                        return Err(trap(TrapClass::Unreachable, range));
                    }
                    fields.remove(0)
                }
                v => v,
            });
        }
        Ok(Value::Adt {
            ty: "Option".to_string(),
            variant: "Some".to_string(),
            fields: vec![Value::Adt {
                ty: FIELDS.to_string(),
                variant: names.join(","),
                fields: out,
            }],
        })
    }
}

/// The elements of a `Vec`.
fn elements(v: &Value) -> Vec<Value> {
    match v {
        Value::Adt { ty, fields, .. } if ty == "Vec" => match fields.first() {
            Some(Value::Buf(_, xs) | Value::Array(xs)) => xs.clone(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

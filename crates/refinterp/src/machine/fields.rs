//! Tagged literals with fields (`comptime.tag.fields.1`), as the reference interpreter runs
//! them: it has no types to make a struct of, so a literal whose value implements `TagFields`
//! is its value wrapped with its fields' names, and `read` gives a value whose fields are found
//! by those names. Anything else asked of the wrapper is asked of its value, as through `Deref`.

use moddil_syntax::TextRange;

use super::Machine;
use crate::value::{Pointer, Value};
use crate::{Eval, TrapClass, trap};

/// The type of a tagged literal's wrapper: its variant is its fields, each name followed by `!`
/// if every reading has it or `?`, its type's text (`comptime.tag.fields.3`), and `,`.
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
        let names = self.run_method(m, &head, vec![Value::Ref(at.clone(), false)], range)?;
        let types = match self.program.trait_method("TagFields", &head, "field_types") {
            Some(m) => elements(&self.run_method(m, &head, vec![Value::Ref(at, false)], range)?),
            None => Vec::new(),
        };
        let mut spec = String::new();
        for (i, name) in elements(&names).into_iter().enumerate() {
            if let Value::Tuple(parts) = name
                && let [n, Value::Bool(every)] = parts.as_slice()
            {
                spec.push_str(&crate::value::string_text(n).unwrap_or_default());
                spec.push(if *every { '!' } else { '?' });
                let ty = types.get(i).and_then(crate::value::string_text);
                spec.push_str(ty.as_deref().unwrap_or("String"));
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
        let got = self.run_method(
            m,
            &head,
            vec![Value::Ref(inner.clone(), false), text.clone()],
            range,
        )?;
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
        let specs: Vec<(&str, bool, &str)> = spec
            .split(',')
            .filter(|f| !f.is_empty())
            .filter_map(|f| {
                let at = f.find(['!', '?'])?;
                Some((&f[..at], f[at..].starts_with('!'), &f[at + 1..]))
            })
            .collect();
        // Where the fields are, for a `Match` field.
        let spans = if specs.iter().any(|(_, _, t)| *t == "Match") {
            let m = self
                .program
                .trait_method("TagFields", &head, "field_spans")
                .expect("a checked `TagFields`");
            match self.run_method(m, &head, vec![Value::Ref(inner, false), text], range)? {
                Value::Adt {
                    variant, fields, ..
                } if variant == "Some" => elements(&fields[0]),
                none => return Ok(none),
            }
        } else {
            Vec::new()
        };
        let mut names = Vec::new();
        let mut out = Vec::new();
        for (k, ((name, every, ty), v)) in specs.into_iter().zip(values).enumerate() {
            names.push(name);
            // The reading's value of this field, if it has one.
            let found = match (ty, v) {
                ("Match", _) => match spans.get(k) {
                    Some(Value::Adt {
                        variant, fields, ..
                    }) if variant == "Some" => Some(self.span_match(&fields[0])),
                    _ => None,
                },
                (
                    _,
                    Value::Adt {
                        variant,
                        mut fields,
                        ..
                    },
                ) if variant == "Some" => {
                    let t = fields.remove(0);
                    if ty == "String" {
                        Some(t)
                    } else {
                        let text = crate::value::string_text(&t).unwrap_or_default();
                        match self.read_typed(ty, &text, range)? {
                            Some(v) => Some(v),
                            None => return Ok(none_value()),
                        }
                    }
                }
                _ => None,
            };
            out.push(match found {
                Some(v) if every => v,
                Some(v) => some_value(v),
                None if every => return Err(trap(TrapClass::Unreachable, range)),
                None => none_value(),
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

impl Machine<'_> {
    /// `core::text::Match` of the span `(start, end)`.
    fn span_match(&self, span: &Value) -> Value {
        let (s, e) = match span {
            Value::Tuple(p) if p.len() == 2 => (p[0].clone(), p[1].clone()),
            _ => (Value::Int(0), Value::Int(0)),
        };
        let path = ["core", "text", "Match"].map(String::from);
        let key = match self.resolve(&path) {
            Some((crate::program::Def::Item(k), _)) => k,
            _ => "Match".to_string(),
        };
        Value::Adt {
            ty: key,
            variant: "Match".to_string(),
            fields: vec![s, e],
        }
    }

    /// `text` read as a `ty` with its `FromStr` (`text.fromstr.1`): the value, or `None` if
    /// `text` is not one.
    fn read_typed(&mut self, ty: &str, text: &str, range: TextRange) -> Eval<Option<Value>> {
        let head = self.type_head(&[ty.to_string()]);
        let m = self
            .program
            .trait_method("FromStr", &head, "from_str")
            .expect("the prelude's `FromStr` of a field's type");
        match self.run_method(m, &head, vec![Value::Str(text.into())], range)? {
            Value::Adt {
                variant,
                mut fields,
                ..
            } if variant == "Some" => Ok(Some(fields.remove(0))),
            _ => Ok(None),
        }
    }
}

fn some_value(v: Value) -> Value {
    Value::Adt {
        ty: "Option".to_string(),
        variant: "Some".to_string(),
        fields: vec![v],
    }
}

fn none_value() -> Value {
    Value::Adt {
        ty: "Option".to_string(),
        variant: "None".to_string(),
        fields: Vec::new(),
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

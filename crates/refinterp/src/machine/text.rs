//! Text (`text.*`): the values of text literals, `fmt"…"`, and the prelude's built-in methods
//! of `char`, `str` and `String` (`#[intrinsic]`).

use moddil_syntax::ast::{self, AstNode};
use moddil_syntax::{SyntaxKind as K, SyntaxToken};

use crate::Eval;
use crate::program::FnDef;
use crate::value::{Head, Value};

use super::Machine;

/// The text of a string literal: plain with escapes (`text.lit.escape.1`), raw
/// (`text.lit.raw.1`), or a dedented block (`text.lit.block.1`).
pub(super) fn string(tok: &str) -> String {
    let (raw, rest) = match tok.strip_prefix('r') {
        Some(r) => (true, r),
        None => (false, tok),
    };
    if let Some(body) = rest.strip_prefix("\"\"\"") {
        return block(body.strip_suffix("\"\"\"").expect("a closed block"), raw);
    }
    if raw {
        let hashes = rest.chars().take_while(|&c| c == '#').count();
        let inner = &rest[hashes + 1..rest.len() - hashes - 1];
        return inner.replace("\r\n", "\n");
    }
    unescape(&rest[1..rest.len() - 1])
}

/// The value of a character literal (`text.char.1`).
pub(super) fn character(tok: &str) -> char {
    unescape(&tok[1..tok.len() - 1])
        .chars()
        .next()
        .expect("a checked character")
}

/// `s` with its escapes replaced; `\s` is a space.
fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('\\') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let c = after.chars().next().expect("a checked escape");
        rest = &after[c.len_utf8()..];
        match c {
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            '0' => out.push('\0'),
            's' => out.push(' '),
            'x' => {
                let v = u32::from_str_radix(&rest[..2], 16).expect("a checked escape");
                out.push(char::from_u32(v).expect("ASCII"));
                rest = &rest[2..];
            }
            'u' => {
                let close = rest.find('}').expect("a checked escape");
                let digits: String = rest[1..close].chars().filter(|&c| c != '_').collect();
                let v = u32::from_str_radix(&digits, 16).expect("a checked escape");
                out.push(char::from_u32(v).expect("a scalar value"));
                rest = &rest[close + 1..];
            }
            // A line continuation: the break and the next line's leading white space go.
            '\n' | '\r' => rest = rest.trim_start(),
            other => out.push(other),
        }
    }
    out.push_str(rest);
    out
}

/// A block's text: its lines after the opening quotes, less the closing line's indentation,
/// with trailing white space removed and each line ended by a line break.
fn block(body: &str, raw: bool) -> String {
    let body = body.replace("\r\n", "\n");
    let body = body.strip_prefix('\n').expect("a checked block");
    let Some(last_break) = body.rfind('\n') else {
        return String::new();
    };
    let indent = &body[last_break + 1..];
    let mut out = String::new();
    for line in body[..last_break].split('\n') {
        let line = line.strip_prefix(indent).unwrap_or("");
        let line = line.trim_end_matches([' ', '\t']);
        if raw {
            out.push_str(line);
        } else {
            out.push_str(&unescape(line));
        }
        out.push('\n');
    }
    out
}

/// The text `v` holds, through any borrows: a `&str` or a `String`.
pub(super) fn text_of(m: &Machine<'_>, v: &Value) -> Option<String> {
    match v {
        Value::Str(s) => Some(s.to_string()),
        Value::String(s) => Some(s.clone()),
        Value::Ref(p, _) => text_of(m, &m.memory.load(p)),
        _ => None,
    }
}

impl Machine<'_> {
    /// `fmt"…"` (`expr.fmt.1`): the text, and each hole's value written by its `Display`.
    pub(super) fn fmt(&mut self, tok: &SyntaxToken) -> Eval {
        let text = tok.text();
        let open = text.find('"').expect("a tagged literal");
        let inner: Vec<char> = text[open + 1..text.len() - 1].chars().collect();
        // Where the literal's inside starts in the file, for the holes' ranges.
        let mut at = tok.text_range().start as usize + open + 1;
        let out = self.memory.temp(Value::String(String::new()));
        let mut lit = String::new();
        let mut i = 0;
        while i < inner.len() {
            let c = inner[i];
            if (c == '{' || c == '}') && inner.get(i + 1) == Some(&c) {
                lit.push(c);
                lit.push(c);
                i += 2;
                at += 2;
                continue;
            }
            if c != '{' {
                lit.push(c);
                i += 1;
                at += c.len_utf8();
                continue;
            }
            self.push_text(&out, &unescape(&lit.replace("{{", "{").replace("}}", "}")));
            lit.clear();
            let mut depth = 0;
            let mut j = i + 1;
            while j < inner.len() && !(inner[j] == '}' && depth == 0) {
                match inner[j] {
                    '{' => depth += 1,
                    '}' => depth -= 1,
                    _ => {}
                }
                j += 1;
            }
            let hole: String = inner[i + 1..j].iter().collect();
            let start = at + 1;
            self.hole(&hole, start, &out)?;
            at = start + hole.len() + 1;
            i = j + 1;
        }
        self.push_text(&out, &unescape(&lit.replace("{{", "{").replace("}}", "}")));
        Ok(self.memory.load(&out))
    }

    fn push_text(&mut self, out: &crate::value::Pointer, s: &str) {
        if let Value::String(mut t) = self.memory.load(out) {
            t.push_str(s);
            self.memory.store(out, Value::String(t));
        }
    }

    /// One hole: its expression, parsed where it stands in the file so its ranges are the
    /// file's, then written onto `out` by `Display::fmt` with a borrow of its value.
    fn hole(&mut self, text: &str, start: usize, out: &crate::value::Pointer) -> Eval<()> {
        let wrapper = "fn f(){(";
        let pad = start.saturating_sub(wrapper.len());
        let source = format!("{}{wrapper}{text})}}", " ".repeat(pad));
        let parse = moddil_syntax::parse(&source);
        let expr = parse
            .syntax()
            .descendants()
            .into_iter()
            .find(|n| n.kind() == K::PAREN_EXPR)
            .and_then(|p| p.children().find_map(ast::Expr::cast))
            .expect("a checked hole");
        let this = match self.place_of(&expr)? {
            Some((p, _)) => match self.memory.load(&p) {
                v @ (Value::Ref(..) | Value::Str(_)) => v,
                _ => Value::Ref(p, false),
            },
            None => match self.eval(&expr)? {
                v @ (Value::Ref(..) | Value::Str(_)) => v,
                v => Value::Ref(self.memory.temp(v), false),
            },
        };
        let range = expr.syntax().text_range();
        self.call_trait(
            "Display",
            "fmt",
            vec![this, Value::Ref(out.clone(), true)],
            range,
        )?;
        Ok(())
    }

    /// The prelude's built-in function `f`, for values of kind `head`.
    pub(super) fn intrinsic(&mut self, f: &FnDef, head: &Head, args: &[Value]) -> Eval {
        let name = f.node.name().map(|n| n.text()).unwrap_or_default();
        let text = |m: &Self, i: usize| text_of(m, &args[i]).expect("text");
        let ch = |m: &Self, i: usize| match m.target(args[i].clone()) {
            Value::Char(c) => c,
            other => unreachable!("a `char`: {other:?}"),
        };
        // The `String` a `&mut String` argument points to, changed by `change`.
        let change = |m: &mut Self, i: usize, change: &mut dyn FnMut(&mut String) -> Value| {
            let mut p = match &args[i] {
                Value::Ref(p, _) => p.clone(),
                other => unreachable!("a `&mut String`: {other:?}"),
            };
            while let Value::Ref(q, _) = m.memory.load(&p) {
                p = q;
            }
            let Value::String(mut s) = m.memory.load(&p) else {
                unreachable!("a `String`")
            };
            let v = change(&mut s);
            m.memory.store(&p, Value::String(s));
            v
        };
        Ok(match (head, name.as_str()) {
            (Head::Int | Head::Bool | Head::Char, "fmt") => {
                let s = match self.target(args[0].clone()) {
                    Value::Int(v) => v.to_string(),
                    Value::Bool(v) => v.to_string(),
                    Value::Char(c) => c.to_string(),
                    other => unreachable!("`Display` of {other:?}"),
                };
                change(self, 1, &mut |out| {
                    out.push_str(&s);
                    Value::Unit
                })
            }
            (Head::Char, "code") => Value::Int(i64::from(u32::from(ch(self, 0)))),
            (Head::Char, "from_code") => {
                let Value::Int(n) = self.target(args[0].clone()) else {
                    unreachable!("an `int`")
                };
                option(
                    u32::try_from(n)
                        .ok()
                        .and_then(char::from_u32)
                        .map(Value::Char),
                )
            }
            (Head::Char, "is_alphabetic") => Value::Bool(ch(self, 0).is_alphabetic()),
            (Head::Char, "is_numeric") => Value::Bool(ch(self, 0).is_numeric()),
            (Head::Char, "is_whitespace") => Value::Bool(ch(self, 0).is_whitespace()),
            (Head::Char, "is_uppercase") => Value::Bool(ch(self, 0).is_uppercase()),
            (Head::Char, "is_lowercase") => Value::Bool(ch(self, 0).is_lowercase()),
            (Head::Char, "len_utf8") => Value::Int(ch(self, 0).len_utf8() as i64),
            (Head::Str, "len") => Value::Int(text(self, 0).len() as i64),
            (Head::Str, "starts_with") => Value::Bool(text(self, 0).starts_with(&text(self, 1))),
            (Head::Str, "ends_with") => Value::Bool(text(self, 0).ends_with(&text(self, 1))),
            (Head::Str, "contains") => Value::Bool(text(self, 0).contains(&text(self, 1))),
            (Head::Str, "trim") => Value::Str(text(self, 0).trim().into()),
            (Head::Str, "to_uppercase") => Value::String(text(self, 0).to_uppercase()),
            (Head::Str, "to_lowercase") => Value::String(text(self, 0).to_lowercase()),
            (Head::Str, "eq") => Value::Bool(text(self, 0) == text(self, 1)),
            (Head::Str, "cmp") => {
                let variant = match text(self, 0).cmp(&text(self, 1)) {
                    std::cmp::Ordering::Less => "Less",
                    std::cmp::Ordering::Equal => "Equal",
                    std::cmp::Ordering::Greater => "Greater",
                };
                Value::Adt {
                    ty: "Ordering".to_string(),
                    variant: variant.to_string(),
                    fields: Vec::new(),
                }
            }
            (Head::String, "new") => Value::String(String::new()),
            (Head::String, "as_str") => Value::Str(text(self, 0).into()),
            (Head::String, "clone") => Value::String(text(self, 0)),
            (Head::String, "push_str") => {
                let s = text(self, 1);
                change(self, 0, &mut |out| {
                    out.push_str(&s);
                    Value::Unit
                })
            }
            (Head::String, "push") => {
                let c = ch(self, 1);
                change(self, 0, &mut |out| {
                    out.push(c);
                    Value::Unit
                })
            }
            (Head::String, "clear") => change(self, 0, &mut |out| {
                out.clear();
                Value::Unit
            }),
            (Head::String, "pop") => {
                let c = change(self, 0, &mut |out| {
                    out.pop().map_or(Value::Unit, Value::Char)
                });
                option((c != Value::Unit).then_some(c))
            }
            (h, n) => unreachable!("no built-in `{n}` for {h:?}"),
        })
    }
}

/// A value of the prelude's `Option`.
fn option(v: Option<Value>) -> Value {
    Value::Adt {
        ty: "Option".to_string(),
        variant: if v.is_some() { "Some" } else { "None" }.to_string(),
        fields: v.into_iter().collect(),
    }
}

/// Whether `a op b` holds for two texts, or two characters, compared by code point
/// (`text.cmp.1`).
pub(super) fn ordered<T: Ord + ?Sized>(op: K, a: &T, b: &T) -> bool {
    match op {
        K::EQ_EQ => a == b,
        K::BANG_EQ => a != b,
        K::LT => a < b,
        K::LT_EQ => a <= b,
        K::GT => a > b,
        _ => a >= b,
    }
}

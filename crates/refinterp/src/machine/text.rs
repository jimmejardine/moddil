//! Text (`text.*`): the values of text literals, `fmt"…"`, and the prelude's built-in methods
//! of `char`, `str` and `String` (`#[intrinsic]`).

use std::collections::HashMap;

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

/// A block literal's lines (from its opening quotes), as `block` takes them: for each, `None`
/// if it is blank, or where its text starts in the literal and the text, less the closing
/// line's indentation and trailing white space.
fn block_lines(lit: &str) -> Vec<Option<(usize, &str)>> {
    let body = &lit[3..lit.len() - 3];
    let mut out = Vec::new();
    let (Some(first), Some(last)) = (body.find('\n'), body.rfind('\n')) else {
        return out;
    };
    let indent = body.len() - last - 1;
    let mut at = first + 1;
    if last < at {
        return out;
    }
    for line in body[at..last].split('\n') {
        let text = line.trim_end_matches('\r');
        if text.chars().all(|c| c == ' ' || c == '\t') {
            out.push(None);
        } else {
            let rest = text.get(indent..).unwrap_or("");
            out.push(Some((3 + at + indent, rest.trim_end_matches([' ', '\t']))));
        }
        at += line.len() + 1;
    }
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
        Value::Ref(p, _) => text_of(m, &m.memory.load(p)),
        v @ Value::Adt { .. } => crate::value::string_text(v),
        _ => None,
    }
}

impl Machine<'_> {
    /// `v` stored where type `ty` is written: a literal takes a number type
    /// (`num.lit.1`), and a `&String` where a `&str` is written is its text
    /// (`text.coerce.1`), as the prelude's `as_str` gives it.
    pub(super) fn typed(&mut self, v: Value, ty: Option<&ast::Type>) -> Value {
        if let Some(t) = ty
            && t.syntax().text() == "&str"
            && let Value::Ref(..) = v
            && let Some(s) = self.string_str(v.clone())
        {
            return s;
        }
        // A number where a `Wrapping<T>` is written (`num.wrap.3`): the number, wrapped.
        if let Some(t) = ty
            && let Some(inner) = t
                .syntax()
                .text()
                .to_string()
                .strip_prefix("Wrapping<")
                .and_then(|s| s.strip_suffix('>'))
                .map(str::to_string)
            && matches!(v, Value::Int(_) | Value::Num(..))
        {
            let inner_ty = t
                .syntax()
                .descendants()
                .into_iter()
                .skip(1)
                .find_map(ast::Type::cast);
            let v = match inner_ty {
                Some(it) if it.syntax().text() == inner.as_str() => crate::num::typed(v, Some(&it)),
                _ => v,
            };
            return Value::Adt {
                ty: "Wrapping".to_string(),
                variant: "Wrapping".to_string(),
                fields: vec![v],
            };
        }
        crate::num::typed(v, ty)
    }

    /// The text of the `String` `v` points to (through any borrows), as its `as_str` gives it,
    /// so it is the same text each time (`text.pos.1`); `None` if it is not a `String`.
    pub(super) fn string_str(&mut self, v: Value) -> Option<Value> {
        let mut at = None;
        let mut v = v;
        while let Value::Ref(q, _) = v {
            v = self.memory.load(&q);
            at = Some(q);
        }
        crate::value::string_text(&v)?;
        let at = at?;
        let range = moddil_syntax::TextRange::default();
        self.string_call("as_str", vec![Value::Ref(at, false)], range)
            .ok()
    }

    /// `tag"… {e} …"` for a tag `f` taking `&[&str]` (`comptime.tag.2`): `f` called with the
    /// text's parts between the holes, as written but for `{{` and `}}`, and with holes, its
    /// value's `bind` with a tuple of the holes' values, left to right.
    pub(super) fn parts_tag(
        &mut self,
        f: &FnDef,
        tok: &SyntaxToken,
        range: moddil_syntax::TextRange,
    ) -> Eval {
        let text = tok.text();
        let t = moddil_syntax::Tagged::of(text);
        let open = t.lit_at;
        let base = tok.text_range().start as usize + open;
        let mut parts = Vec::new();
        let mut holes = Vec::new();
        let mut cur = String::new();
        if !t.raw && text[open..].starts_with("\"\"\"") {
            for line in block_lines(&text[open..]) {
                if let Some((off, line)) = line {
                    parts_scan(line, base + off, &mut parts, &mut holes, &mut cur);
                }
                cur.push('\n');
            }
        } else {
            // A raw literal (`sqlr#"…"#`) is between its `#"` and `"#`.
            let (inner, at) = t.body();
            parts_scan(
                inner,
                tok.text_range().start as usize + at,
                &mut parts,
                &mut holes,
                &mut cur,
            );
        }
        parts.push(cur);
        let strings = parts.into_iter().map(|p| Value::Str(p.into())).collect();
        let array = self.memory.temp(Value::Array(strings));
        let value = self.invoke(f, vec![Value::Ref(array, false)], HashMap::new(), range)?;
        if holes.is_empty() {
            return Ok(value);
        }
        let mut values = Vec::new();
        for (hole, start) in holes {
            values.push(self.eval(&hole_expr(&hole, start))?);
        }
        let head = value.head();
        let bind = self
            .program
            .methods(&head, "bind")
            .into_iter()
            .next()
            .expect("a checked `bind`");
        self.run_method(bind, &head, vec![value, Value::Tuple(values)], range)
    }

    /// `fmt"…"` (`expr.fmt.1`): the text, and each hole's value written by its `Display`.
    pub(super) fn fmt(&mut self, tok: &SyntaxToken) -> Eval {
        let empty = self.string_call("new", Vec::new(), tok.text_range())?;
        let out = self.memory.temp(empty);
        self.fmt_into(tok, &out, false)?;
        Ok(self.memory.load(&out))
    }

    /// `fmt"…"` written onto the sink at `out` (`text.write.2`): each run of its text through
    /// the sink's `write_str`, and each hole as `fmt"…"` writes it.
    /// `straight` for `w.write_fmt(fmt"…")`, whose padded holes go onto the sink through the
    /// prelude's `Measure` and `Padded`, as compiled code writes them.
    pub(super) fn fmt_into(
        &mut self,
        tok: &SyntaxToken,
        out: &crate::value::Pointer,
        straight: bool,
    ) -> Eval<()> {
        let text = tok.text();
        let open = text.find('"').expect("a tagged literal");
        let base = tok.text_range().start as usize + open;
        let mut lit = String::new();
        // A block (`text.lit.block.1`): its lines dedented, each with its holes.
        if text[open..].starts_with("\"\"\"") {
            for line in block_lines(&text[open..]) {
                if let Some((off, line)) = line {
                    self.fmt_scan(line, base + off, out, &mut lit, straight)?;
                }
                lit.push('\n');
            }
        } else {
            // Where the literal's inside starts in the file, for the holes' ranges.
            self.fmt_scan(
                &text[open + 1..text.len() - 1],
                base + 1,
                out,
                &mut lit,
                straight,
            )?;
        }
        self.push_text(out, &unescape(&lit.replace("{{", "{").replace("}}", "}")))?;
        Ok(())
    }

    /// The text and holes of `inner`, which starts at byte `at` of the file, written onto the
    /// sink at `out`, the text since the last hole kept in `lit`.
    fn fmt_scan(
        &mut self,
        inner: &str,
        at: usize,
        out: &crate::value::Pointer,
        lit: &mut String,
        straight: bool,
    ) -> Eval<()> {
        let out = out.clone();
        let inner: Vec<char> = inner.chars().collect();
        let mut at = at;
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
            self.push_text(&out, &unescape(&lit.replace("{{", "{").replace("}}", "}")))?;
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
            self.hole(&hole, start, &out, straight)?;
            at = start + hole.len() + 1;
            i = j + 1;
        }
        Ok(())
    }

    /// Writes `s` to the sink at `out`, a `String` or another `TextWrite` (`text.write.1`),
    /// through its `write_str`, as the compiled code does.
    pub(super) fn push_text(&mut self, out: &crate::value::Pointer, s: &str) -> Eval<()> {
        if s.is_empty() {
            return Ok(());
        }
        let args = vec![Value::Ref(out.clone(), true), Value::Str(s.into())];
        self.call_trait(
            "TextWrite",
            "write_str",
            args,
            moddil_syntax::TextRange::default(),
        )?;
        Ok(())
    }

    /// The prelude `String`'s function `name` (`text.string.1`), run on `args`.
    pub(super) fn string_call(
        &mut self,
        name: &str,
        args: Vec<Value>,
        range: moddil_syntax::TextRange,
    ) -> Eval {
        let head = Head::Adt("String".to_string());
        let m = self
            .program
            .methods(&head, name)
            .into_iter()
            .next()
            .expect("the prelude's `String`");
        self.run_method(m, &head, args, range)
    }

    /// `this` written onto the sink at `out` by its `Display`, or for `?` its `Debug`
    /// (`expr.fmt.debug.1`), across lines for `#?` (`expr.fmt.debug.2`).
    fn write_value(
        &mut self,
        spec: &Spec,
        this: Value,
        out: &crate::value::Pointer,
        range: moddil_syntax::TextRange,
    ) -> Eval<()> {
        let args = vec![this, Value::Ref(out.clone(), true)];
        if spec.debug && spec.prefix {
            let f = self.program.fns["debug_pretty"].clone();
            self.invoke(&f, args, HashMap::new(), range)?;
        } else {
            let tr = if spec.debug { "Debug" } else { "Display" };
            self.call_trait(tr, "fmt", args, range)?;
        }
        Ok(())
    }

    /// One hole: its expression, parsed where it stands in the file so its ranges are the
    /// file's, then written onto `out` by `Display::fmt` with a borrow of its value. A spec
    /// (`expr.fmt.spec.1`) writes it into a `String` of its own first, then cuts and pads it.
    fn hole(
        &mut self,
        text: &str,
        start: usize,
        out: &crate::value::Pointer,
        straight: bool,
    ) -> Eval<()> {
        let (text, spec) = match spec_colon(text) {
            Some(k) => {
                let mut spec = parse_spec(&text[k + 1..]).expect("a checked spec");
                // A width or precision named `name$`, evaluated where it stands.
                for (named, precision) in [
                    (spec.width_name.take(), false),
                    (spec.precision_name.take(), true),
                ] {
                    let Some((name, off)) = named else { continue };
                    let x = hole_expr(&name, start + k + 1 + off);
                    let n = match self.eval(&x)? {
                        Value::Int(n) => u32::try_from(n).unwrap_or(0),
                        other => unreachable!("a checked `int`: {other:?}"),
                    };
                    if precision {
                        spec.precision = Some(n);
                    } else {
                        spec.width = Some(n);
                    }
                }
                (&text[..k], spec)
            }
            None => (text, Spec::default()),
        };
        let expr = hole_expr(text, start);
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
        if spec.width.is_none()
            && spec.precision.is_none()
            && !spec.plus
            && spec.radix.is_none()
            && spec.exponent.is_none()
        {
            self.write_value(&spec, this, out, range)?;
            return Ok(());
        }
        let value = self.target(this.clone());
        let number = matches!(value, Value::Int(_) | Value::Num(..) | Value::FloatLit(_));
        let text_value =
            matches!(value, Value::Str(_)) || crate::value::string_text(&value).is_some();
        // An integer's digits in a base (`expr.fmt.spec.4`), a float's fixed digits
        // (`expr.fmt.spec.3`), or what its `fmt` writes.
        let written = match (&value, spec.precision) {
            _ if spec.radix.is_some() => {
                radix_text(&value, spec.radix.expect("a base"), spec.prefix)
            }
            // A float in exponent form (`expr.fmt.spec.5`).
            (Value::Num(k, bits), digits) if spec.exponent.is_some() => exp_text(
                *k == crate::num::Kind::F32,
                *bits as u64,
                digits,
                spec.exponent == Some(true),
            ),
            (Value::FloatLit(t), digits) if spec.exponent.is_some() => {
                let x: f64 = t.replace('_', "").parse().expect("a float literal");
                exp_text(false, x.to_bits(), digits, spec.exponent == Some(true))
            }
            (Value::Num(k, bits), Some(digits)) if k.is_float() => {
                let digits = digits as usize;
                if *k == crate::num::Kind::F32 {
                    format!("{:.*}", digits, f32::from_bits(*bits as u32))
                } else {
                    format!("{:.*}", digits, f64::from_bits(*bits as u64))
                }
            }
            // A float literal whose type is not written is an `f64` (`num.lit.1`).
            (Value::FloatLit(t), Some(digits)) => {
                let x: f64 = t.replace('_', "").parse().expect("a float literal");
                format!("{:.*}", digits as usize, x)
            }
            _ => {
                let empty = self.string_call("new", Vec::new(), range)?;
                let piece = self.memory.temp(empty);
                self.write_value(&spec, this, &piece, range)?;
                crate::value::string_text(&self.memory.load(&piece)).expect("a `String`")
            }
        };
        // `+` before a number with no `-` (`expr.fmt.spec.4`).
        if straight {
            return self.padded_onto(out, &written, &spec, number, text_value, range);
        }
        let written = if spec.plus && !written.starts_with('-') {
            format!("+{written}")
        } else {
            written
        };
        let keep = match spec.precision {
            Some(k) if text_value && !spec.debug => k as usize,
            _ => usize::MAX,
        };
        let align = spec.align.unwrap_or(if number { '>' } else { '<' });
        let padded = padded(
            &written,
            spec.fill,
            align,
            spec.width.unwrap_or(0) as usize,
            spec.zero,
            keep,
        );
        self.push_text(out, &padded)
    }

    /// `written`, a hole's text, onto the sink at `out` padded by `spec`, as `w.write_fmt(…)`
    /// writes it (`text.write.2`): counted by the prelude's `Measure`, then written through its
    /// `Padded`.
    fn padded_onto(
        &mut self,
        out: &crate::value::Pointer,
        written: &str,
        spec: &Spec,
        number: bool,
        text_value: bool,
        range: moddil_syntax::TextRange,
    ) -> Eval<()> {
        let call = |m: &mut Self, name: &str, args: Vec<Value>| -> Eval {
            let f = m.program.fns[name].clone();
            m.invoke(&f, args, HashMap::new(), range)
        };
        let measure = call(self, "measure_new", Vec::new())?;
        let mp = self.memory.temp(measure);
        self.push_text(&mp, written)?;
        let keep = match spec.precision {
            Some(k) if text_value && !spec.debug => i64::from(k),
            _ => -1,
        };
        let align = match spec.align.unwrap_or(if number { '>' } else { '<' }) {
            '^' => 1,
            '>' => 2,
            _ => 0,
        };
        let args = vec![
            Value::Ref(out.clone(), true),
            Value::Ref(mp, false),
            Value::Char(spec.fill),
            Value::Int(align),
            Value::Int(i64::from(spec.width.unwrap_or(0))),
            Value::Bool(spec.zero),
            Value::Int(keep),
            Value::Bool(spec.plus),
        ];
        let padded = call(self, "padded_start", args)?;
        let pp = self.memory.temp(padded);
        self.push_text(&pp, written)?;
        let head = Head::Adt("Padded".to_string());
        let finish = self
            .program
            .methods(&head, "finish")
            .into_iter()
            .next()
            .expect("the prelude's `Padded::finish`");
        self.run_method(finish, &head, vec![Value::Ref(pp, true)], range)?;
        Ok(())
    }

    /// The prelude's built-in function `f`, for values of kind `head`.
    pub(super) fn intrinsic(
        &mut self,
        f: &FnDef,
        head: &Head,
        args: &[Value],
        range: moddil_syntax::TextRange,
    ) -> Eval {
        let name = f.node.name().map(|n| n.text()).unwrap_or_default();
        // The `core` mod's `os` module (`os.1`): a test has no OS.
        if f.module as usize == self.program.core_os {
            return Ok(self.no_os(&name, args));
        }
        if f.module as usize == self.program.core_gpu {
            return Ok(self.no_gpu(&name, args));
        }
        if let Some(r) = self.task_intrinsic(&name, head, args, range) {
            return r;
        }
        let text = |m: &Self, i: usize| text_of(m, &args[i]).expect("text");
        // A `Pattern` (`text.pattern.1`): text, or a character.
        let pattern = |m: &Self, i: usize| match m.target(args[i].clone()) {
            Value::Char(c) => c.to_string(),
            _ => text_of(m, &args[i]).expect("a pattern"),
        };
        let ch = |m: &Self, i: usize| match m.target(args[i].clone()) {
            Value::Char(c) => c,
            other => unreachable!("a `char`: {other:?}"),
        };
        Ok(match (head, name.as_str()) {
            // The conversions (`num.convert.1`), to the type the call names.
            (Head::Int | Head::Num(_), "checked_to" | "truncate" | "wrap" | "saturate") => {
                let to = match self.convert_to.clone() {
                    Some(Head::Num(k)) => Some(k),
                    _ => None,
                };
                let x = self.target(args[0].clone());
                let x = match head {
                    Head::Num(k) => crate::num::as_kind(x, *k),
                    _ => x,
                };
                let v = crate::num::convert(&name, &x, to);
                if name == "checked_to" {
                    match v {
                        Some(v) => Value::Adt {
                            ty: "Option".to_string(),
                            variant: "Some".to_string(),
                            fields: vec![v],
                        },
                        None => Value::Adt {
                            ty: "Option".to_string(),
                            variant: "None".to_string(),
                            fields: Vec::new(),
                        },
                    }
                } else {
                    v.expect("an integer conversion gives a value")
                }
            }
            // The float functions (`num.float.fn.1`).
            (Head::Num(k), _) if k.is_float() && name != "text" => {
                let x = self.target(args[0].clone());
                let y = args.get(1).map(|a| self.target(a.clone()));
                crate::num::float_fn(*k, &name, &x, y.as_ref()).expect("a float function")
            }
            // What `Display` writes (`text.display.1`).
            (Head::Num(k), "text") => match crate::num::as_kind(self.target(args[0].clone()), *k) {
                Value::Num(k, bits) => Value::Str(crate::num::text(k, bits).into()),
                other => unreachable!("`Display` of {other:?}"),
            },
            (Head::Int | Head::Bool | Head::Char, "text") => Value::Str(
                match self.target(args[0].clone()) {
                    Value::Int(v) => v.to_string(),
                    Value::Bool(v) => v.to_string(),
                    Value::Char(c) => c.to_string(),
                    other => unreachable!("`Display` of {other:?}"),
                }
                .into(),
            ),
            // The ambient log (`obs.log.1`).
            (Head::Adt(k), "debug" | "info" | "warn" | "error") if k == "Log" => {
                eprintln!("[{name}] {}", text(self, 1));
                Value::Unit
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
            (Head::Str, "quoted_text") => Value::Str(format!("{:?}", text(self, 0)).into()),
            (Head::Char, "quoted_text") => Value::Str(format!("{:?}", ch(self, 0)).into()),
            // A byte of the text, and the first `len` bytes of a byte slice as text
            // (`text.string.inline.1`).
            (Head::Str, "byte_at") => {
                let t = text(self, 0);
                let Value::Int(i) = self.target(args[1].clone()) else {
                    unreachable!("an `int`")
                };
                match usize::try_from(i)
                    .ok()
                    .and_then(|i| t.as_bytes().get(i).copied())
                {
                    Some(b) => Value::Num(crate::num::Kind::U8, u128::from(b)),
                    None => return Err(crate::trap(crate::TrapClass::IndexOutOfBounds, range)),
                }
            }
            // Its bytes (`text.bytes.1`): a copy on the heap, borrowed.
            (Head::Str, "as_bytes") => {
                let bytes = text(self, 0)
                    .as_bytes()
                    .iter()
                    .map(|b| Value::Num(crate::num::Kind::U8, u128::from(*b)))
                    .collect();
                Value::Ref(self.memory.alloc(Value::Array(bytes)), false)
            }
            (Head::Str, "view") => {
                let bytes = match self.target(args[0].clone()) {
                    Value::Array(xs) | Value::Buf(_, xs) => xs,
                    other => unreachable!("bytes: {other:?}"),
                };
                let Value::Int(len) = self.target(args[1].clone()) else {
                    unreachable!("an `int`")
                };
                let bytes: Vec<u8> = bytes
                    .iter()
                    .take(usize::try_from(len).unwrap_or(0))
                    .map(|b| match b {
                        Value::Num(_, b) => *b as u8,
                        // A byte written as a literal the context did not type.
                        Value::Int(b) => *b as u8,
                        other => unreachable!("a byte: {other:?}"),
                    })
                    .collect();
                let text = String::from_utf8(bytes).expect("UTF-8");
                // The same bytes give the same text while they hold it (`text.pos.1`).
                match &args[0] {
                    Value::Ref(at, _) => Value::Str(self.texts.view(at.clone(), text)),
                    _ => Value::Str(text.into()),
                }
            }
            // Whether two texts are the same text, not only the same characters (`text.pos.1`).
            (Head::Str, "same_text") => {
                match (self.target(args[0].clone()), self.target(args[1].clone())) {
                    (Value::Str(a), Value::Str(b)) => Value::Bool(crate::texts::same(&a, &b)),
                    other => unreachable!("texts: {other:?}"),
                }
            }
            // The text between two byte offsets (`text.pos.1`).
            (Head::Str, "bytes_between") => {
                let t = match self.target(args[0].clone()) {
                    Value::Str(t) => t,
                    _ => text(self, 0).into(),
                };
                let (Value::Int(a), Value::Int(b)) =
                    (self.target(args[1].clone()), self.target(args[2].clone()))
                else {
                    unreachable!("`int`s")
                };
                match usize::try_from(a)
                    .ok()
                    .zip(usize::try_from(b).ok())
                    .and_then(|(a, b)| self.texts.slice(&t, a, b))
                {
                    Some(part) => Value::Str(part),
                    None => return Err(crate::trap(crate::TrapClass::IndexOutOfBounds, range)),
                }
            }
            (Head::Str, "char_at_byte") => {
                let Value::Int(at) = self.target(args[1].clone()) else {
                    unreachable!("an `int`")
                };
                let s = text(self, 0);
                let c = usize::try_from(at)
                    .ok()
                    .and_then(|at| s.get(at..))
                    .and_then(|rest| rest.chars().next());
                option(c.map(Value::Char))
            }
            // Whether two pointers point to the same place (`core.ptr.1`).
            (Head::Unit, "addr_eq") => match (args[0].clone(), args[1].clone()) {
                (Value::Ref(a, _), Value::Ref(b, _)) => Value::Bool(a == b),
                _ => unreachable!("pointers"),
            },
            // The same pointer, to change through (`core.ptr.1`).
            (Head::Unit, "cast_mut") => {
                let Value::Ref(p, _) = args[0].clone() else {
                    unreachable!("a pointer")
                };
                Value::Ref(p, true)
            }
            // A box's value, kept on the heap behind a raw pointer, and taken back
            // (`ffi.core.1`).
            (Head::Unit, "into_raw") => match &args[0] {
                Value::Adt { ty, fields, .. } if ty == "Box" => match &fields[..] {
                    [Value::Buf(_, xs)] if xs.len() == 1 => {
                        Value::Ref(self.memory.alloc(xs[0].clone()), true)
                    }
                    _ => unreachable!("a box's buffer"),
                },
                _ => unreachable!("a box"),
            },
            (Head::Unit, "from_raw") => {
                let Value::Ref(p, _) = args[0].clone() else {
                    unreachable!("a pointer")
                };
                Value::Adt {
                    ty: "Box".to_string(),
                    variant: "Box".to_string(),
                    fields: vec![Value::Buf(
                        crate::value::Cap::new(1),
                        vec![self.memory.free(&p)],
                    )],
                }
            }
            // Compile-time code's error, reached when the program runs (`comptime.error.1`).
            (Head::Unit, "compile_error") => {
                return Err(crate::trap(crate::TrapClass::Unreachable, range));
            }
            (Head::Unit, "replace") => {
                let Value::Ref(p, _) = args[0].clone() else {
                    unreachable!("a borrow")
                };
                let old = self.memory.load(&p);
                self.memory.store(&p, args[1].clone());
                old
            }
            (Head::Unit, "swap") => {
                let (Value::Ref(a, _), Value::Ref(b, _)) = (args[0].clone(), args[1].clone())
                else {
                    unreachable!("two borrows")
                };
                let (x, y) = (self.memory.load(&a), self.memory.load(&b));
                self.memory.store(&a, y);
                self.memory.store(&b, x);
                Value::Unit
            }
            (Head::Int, "rotate_left") => {
                let (Value::Int(a), Value::Int(b)) =
                    (self.target(args[0].clone()), self.target(args[1].clone()))
                else {
                    unreachable!("two `int`s")
                };
                Value::Int(a.rotate_left(b.rem_euclid(64) as u32))
            }
            // The two parts of an `&mut` slice borrow (`expr.slice.1`).
            (Head::Array | Head::Buf, "split_at_mut_unchecked") => {
                let Value::Ref(p, _) = args[0].clone() else {
                    unreachable!("a borrow")
                };
                let (Value::Int(at), Value::Int(n)) =
                    (self.target(args[1].clone()), self.target(args[2].clone()))
                else {
                    unreachable!("`int`s")
                };
                if at < 0 || at > n {
                    return Err(crate::trap(crate::TrapClass::IndexOutOfBounds, range));
                }
                let (at, n) = (at as usize, n as usize);
                let start = p.span.map_or(0, |(s, _)| s);
                let mut a = p.clone();
                a.span = Some((start, at));
                let mut b = p;
                b.span = Some((start + at, n - at));
                Value::Tuple(vec![Value::Ref(a, true), Value::Ref(b, true)])
            }
            (Head::Array | Head::Buf, "len") => match self.target(args[0].clone()) {
                Value::Array(xs) | Value::Buf(_, xs) => Value::Int(xs.len() as i64),
                other => unreachable!("a slice: {other:?}"),
            },
            (Head::Buf, _) => return self.buf_intrinsic(&name, args, range),
            (Head::Str, "starts_with") => Value::Bool(text(self, 0).starts_with(&pattern(self, 1))),
            (Head::Str, "ends_with") => Value::Bool(text(self, 0).ends_with(&pattern(self, 1))),
            (Head::Str, "contains") => Value::Bool(text(self, 0).contains(&pattern(self, 1))),
            (Head::Str, "trim") => Value::Str(text(self, 0).trim().into()),
            (Head::Str, "upper_text") => Value::Str(text(self, 0).to_uppercase().into()),
            (Head::Str, "lower_text") => Value::Str(text(self, 0).to_lowercase().into()),
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
            (h, n) => unreachable!("no built-in `{n}` for {h:?}"),
        })
    }
}

impl Machine<'_> {
    /// The methods of `Buf<T>` (`collections.buf.1`).
    fn buf_intrinsic(
        &mut self,
        name: &str,
        args: &[Value],
        range: moddil_syntax::TextRange,
    ) -> Eval {
        let int = |v: &Value| match v {
            Value::Int(i) => *i,
            other => unreachable!("an `int`: {other:?}"),
        };
        // The place of the `Buf` a borrow argument points to.
        let place = |m: &Self, v: &Value| {
            let Value::Ref(mut p, _) = v.clone() else {
                unreachable!("a borrow of a `Buf`")
            };
            while let Value::Ref(q, _) = m.memory.load(&p) {
                p = q;
            }
            p
        };
        let oob = || crate::trap(crate::TrapClass::IndexOutOfBounds, range);
        let full = || crate::trap(crate::TrapClass::CapacityExceeded, range);
        match name {
            "with_capacity" => {
                let cap = self.buf_cap(usize::try_from(int(&args[0])).unwrap_or(0), range)?;
                return Ok(Value::Buf(cap, Vec::new()));
            }
            "from_array" => {
                let Value::Array(xs) = args[0].clone() else {
                    unreachable!("an array")
                };
                let cap = self.buf_cap(xs.len(), range)?;
                return Ok(Value::Buf(cap, xs));
            }
            "into_array" => {
                let Value::Buf(cap, xs) = args[0].clone() else {
                    unreachable!("a `Buf`")
                };
                self.buf_free(cap, range)?;
                return Ok(Value::Array(xs));
            }
            "as_slice" => return Ok(Value::Ref(place(self, &args[0]), false)),
            "as_mut_slice" => return Ok(Value::Ref(place(self, &args[0]), true)),
            _ => {}
        }
        let p = place(self, &args[0]);
        let Value::Buf(mut cap, mut xs) = self.memory.load(&p) else {
            unreachable!("a `Buf`")
        };
        let at = |i: &Value, len: usize| usize::try_from(int(i)).ok().filter(|&i| i < len);
        let out = match name {
            "len" => Value::Int(xs.len() as i64),
            "capacity" => Value::Int(cap.n as i64),
            "set_capacity" => {
                let n = usize::try_from(int(&args[1])).unwrap_or(0).max(xs.len());
                cap = self.buf_recap(cap, n, range)?;
                Value::Unit
            }
            "push" | "insert" if xs.len() >= cap.n => return Err(full()),
            "push" => {
                xs.push(args[1].clone());
                Value::Unit
            }
            "pop" => option(xs.pop()),
            "insert" => {
                let i = at(&args[1], xs.len() + 1).ok_or_else(oob)?;
                xs.insert(i, args[2].clone());
                Value::Unit
            }
            "remove" => {
                let i = at(&args[1], xs.len()).ok_or_else(oob)?;
                xs.remove(i)
            }
            "swap" => {
                let i = at(&args[1], xs.len()).ok_or_else(oob)?;
                let j = at(&args[2], xs.len()).ok_or_else(oob)?;
                xs.swap(i, j);
                Value::Unit
            }
            other => unreachable!("no built-in `{other}` for `Buf`"),
        };
        self.memory.store(&p, Value::Buf(cap, xs));
        Ok(out)
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

/// A hole's format spec (`expr.fmt.spec.1`):
/// `[[fill]align][+][#][0][width][.precision][type]`, the type `?`, `x`, `X`, `b` or `o`.
#[derive(Default)]
struct Spec {
    debug: bool,
    fill: char,
    align: Option<char>,
    plus: bool,
    prefix: bool,
    zero: bool,
    width: Option<u32>,
    precision: Option<u32>,
    radix: Option<char>,
    /// `e` (`false`) or `E` (`true`).
    exponent: Option<bool>,
    /// A width or precision named `name$`: the name and its byte offset in the spec.
    width_name: Option<(String, usize)>,
    precision_name: Option<(String, usize)>,
}

/// A float's text in exponent form (`expr.fmt.spec.5`), `digits` after the point or as many as
/// it needs, the `e` upper-case for `upper`: an `f32` if `f32` says so, from its bits.
fn exp_text(f32: bool, bits: u64, digits: Option<u32>, upper: bool) -> String {
    let s = match (digits, f32) {
        (Some(d), true) => format!("{:.*e}", d as usize, f32::from_bits(bits as u32)),
        (Some(d), false) => format!("{:.*e}", d as usize, f64::from_bits(bits)),
        (None, true) => format!("{:e}", f32::from_bits(bits as u32)),
        (None, false) => format!("{:e}", f64::from_bits(bits)),
    };
    if upper { s.replace('e', "E") } else { s }
}

/// The expression `text` of a hole that starts at byte `start` of the file, parsed where it
/// stands so its ranges are the file's.
/// The parts and holes of `inner`, which starts at byte `at` of the file, of a tag taking
/// `&[&str]` (`comptime.tag.2`): each hole, with where its text starts, ends the part in `cur`.
fn parts_scan(
    inner: &str,
    at: usize,
    parts: &mut Vec<String>,
    holes: &mut Vec<(String, usize)>,
    cur: &mut String,
) {
    let inner: Vec<char> = inner.chars().collect();
    let mut at = at;
    let mut i = 0;
    while i < inner.len() {
        let c = inner[i];
        if (c == '{' || c == '}') && inner.get(i + 1) == Some(&c) {
            cur.push(c);
            i += 2;
            at += 2;
            continue;
        }
        if c != '{' {
            cur.push(c);
            i += 1;
            at += c.len_utf8();
            continue;
        }
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
        at = start + hole.len() + 1;
        holes.push((hole, start));
        parts.push(std::mem::take(cur));
        i = j + 1;
    }
}

fn hole_expr(text: &str, start: usize) -> ast::Expr {
    let wrapper = "fn f(){(";
    let pad = start.saturating_sub(wrapper.len());
    let source = format!("{}{wrapper}{text})}}", " ".repeat(pad));
    moddil_syntax::parse(&source)
        .syntax()
        .descendants()
        .into_iter()
        .find(|n| n.kind() == K::PAREN_EXPR)
        .and_then(|p| p.children().find_map(ast::Expr::cast))
        .expect("a checked hole")
}

/// Integer `v`'s digits in the base of `radix` (`x`, `X`, `b` or `o`), its bits as its type holds
/// them, after the base's prefix for `prefix` (`expr.fmt.spec.4`).
fn radix_text(v: &Value, radix: char, prefix: bool) -> String {
    let (bits, width) = match v {
        Value::Int(i) => (u128::from(*i as u64), 64),
        Value::Num(k, b) => (*b, k.width()),
        Value::BigLit(i) => (*i as u128, 128),
        Value::HugeLit(u) => (*u, 128),
        _ => unreachable!("a checked integer"),
    };
    let bits = if width == 128 {
        bits
    } else {
        bits & ((1u128 << width) - 1)
    };
    let (p, digits) = match radix {
        'x' => ("0x", format!("{bits:x}")),
        'X' => ("0x", format!("{bits:X}")),
        'b' => ("0b", format!("{bits:b}")),
        _ => ("0o", format!("{bits:o}")),
    };
    if prefix {
        format!("{p}{digits}")
    } else {
        digits
    }
}

/// Where a hole's spec starts: its first `:` outside brackets and strings, not in a `::`.
fn spec_colon(hole: &str) -> Option<usize> {
    let cs: Vec<(usize, char)> = hole.char_indices().collect();
    let mut depth = 0;
    let mut i = 0;
    while i < cs.len() {
        match cs[i].1 {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            '"' => {
                i += 1;
                while i < cs.len() && cs[i].1 != '"' {
                    i += 1;
                }
            }
            ':' if cs.get(i + 1).map(|c| c.1) == Some(':') => i += 1,
            ':' if depth == 0 && (i == 0 || cs[i - 1].1 != ':') => return Some(cs[i].0),
            _ => {}
        }
        i += 1;
    }
    None
}

/// A checked hole's spec.
fn parse_spec(text: &str) -> Option<Spec> {
    let cs: Vec<char> = text.chars().collect();
    let align = |c: &char| matches!(c, '<' | '^' | '>');
    let mut spec = Spec {
        fill: ' ',
        ..Spec::default()
    };
    let mut i = 0;
    if cs.get(1).is_some_and(align) {
        spec.fill = cs[0];
        spec.align = Some(cs[1]);
        i = 2;
    } else if cs.first().is_some_and(align) {
        spec.align = Some(cs[0]);
        i = 1;
    }
    if cs.get(i) == Some(&'+') {
        spec.plus = true;
        i += 1;
    }
    if cs.get(i) == Some(&'#') {
        spec.prefix = true;
        i += 1;
    }
    if cs.get(i) == Some(&'0') {
        spec.zero = true;
        i += 1;
    }
    let number = |i: &mut usize| {
        let from = *i;
        while cs.get(*i).is_some_and(char::is_ascii_digit) {
            *i += 1;
        }
        cs[from..*i].iter().collect::<String>().parse().ok()
    };
    // `name$`, and its offset in bytes.
    let named = |i: &mut usize| {
        let from = *i;
        let mut j = from;
        while cs.get(j).is_some_and(|c| c.is_alphanumeric() || *c == '_') {
            j += 1;
        }
        let first = cs.get(from).is_some_and(|c| c.is_alphabetic() || *c == '_');
        if !first || cs.get(j) != Some(&'$') {
            return None;
        }
        *i = j + 1;
        let off: usize = cs[..from].iter().map(|c| c.len_utf8()).sum();
        Some((cs[from..j].iter().collect::<String>(), off))
    };
    spec.width_name = named(&mut i);
    if spec.width_name.is_none() {
        spec.width = number(&mut i);
    }
    if cs.get(i) == Some(&'.') {
        i += 1;
        spec.precision_name = named(&mut i);
        if spec.precision_name.is_none() {
            spec.precision = Some(number(&mut i)?);
        }
    }
    match cs.get(i) {
        Some('?') => spec.debug = true,
        Some(&c @ ('x' | 'X' | 'b' | 'o')) => spec.radix = Some(c),
        Some('e') => spec.exponent = Some(false),
        Some('E') => spec.exponent = Some(true),
        _ => return (i == cs.len()).then_some(spec),
    }
    (i + 1 == cs.len()).then_some(spec)
}

/// `s` cut to its first `keep` characters, then filled with `fill` to `width` characters, to the
/// left (`<`), centred (`^`) or to the right (`>`); with `zero`, filled with `0`s after a leading
/// sign (`expr.fmt.spec.1`, `expr.fmt.spec.2`).
fn padded(s: &str, fill: char, align: char, width: usize, zero: bool, keep: usize) -> String {
    let kept: String = s.chars().take(keep).collect();
    let room = width.saturating_sub(kept.chars().count());
    if zero {
        // The zeros go after a sign and a base's prefix (`{n:#06x}` is `0x002a`).
        let mut at = usize::from(kept.starts_with(['-', '+']));
        if ["0x", "0b", "0o"].iter().any(|p| kept[at..].starts_with(p)) {
            at += 2;
        }
        return format!("{}{}{}", &kept[..at], "0".repeat(room), &kept[at..]);
    }
    let before = match align {
        '<' => 0,
        '^' => room / 2,
        _ => room,
    };
    let fill = |n: usize| fill.to_string().repeat(n);
    format!("{}{kept}{}", fill(before), fill(room - before))
}

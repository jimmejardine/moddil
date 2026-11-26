//! Text literals (`expressions/literals.md`): `'c'`, `"…"` with escapes, raw `r#"…"#`, dedented
//! `"""` blocks, and `fmt"…"` with its holes.

use moddil_syntax::ast;
use moddil_syntax::{SyntaxKind as K, SyntaxToken, TextRange};

use super::Lowerer;
use crate::hir::{Align, ExprId, ExprKind, FmtPiece, Radix, Spec};
use crate::{Code, Diagnostic, Ty};

/// The text of a string literal token, or why it is not one.
pub(crate) fn string_value(tok: &str) -> Result<String, String> {
    let (raw, rest) = match tok.strip_prefix('r') {
        Some(r) => (true, r),
        None => (false, tok),
    };
    if let Some(block) = rest.strip_prefix("\"\"\"") {
        return block_value(block.strip_suffix("\"\"\"").unwrap_or(block), raw);
    }
    if raw {
        let hashes = rest.len() - rest.trim_start_matches('#').len();
        let inner = rest[hashes..]
            .strip_suffix(&rest[..hashes])
            .and_then(|s| s.strip_prefix('"'))
            .and_then(|s| s.strip_suffix('"'))
            .ok_or("an unterminated raw string")?;
        return Ok(inner.replace("\r\n", "\n"));
    }
    let inner = rest
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .ok_or("an unterminated string")?;
    unescape(inner)
}

/// For a block literal token with a wrongly indented line, the byte range of its closing quotes,
/// whose indentation the lines are measured against: the error is reported there.
fn block_error_at(tok: &str) -> Option<(usize, usize)> {
    let start = tok.find("\"\"\"")? + 3;
    let end = tok.rfind("\"\"\"")?;
    let body = tok.get(start..end)?;
    let indent = &body[body.rfind('\n')? + 1..];
    // The first piece is the rest of the opening line, and the last the closing one's indentation.
    let mut pieces = body.split_inclusive('\n');
    let mut at = start + pieces.next()?.len();
    for line in pieces {
        let text = line.trim_end_matches(['\n', '\r']);
        let blank = text.chars().all(|c| c == ' ' || c == '\t');
        if !blank && !text.starts_with(indent) && at + line.len() < end {
            return Some((end, end + 3));
        }
        at += line.len();
    }
    None
}

/// The value of a `char` literal token.
pub(crate) fn char_value(tok: &str) -> Result<char, String> {
    let inner = tok
        .strip_prefix('\'')
        .and_then(|s| s.strip_suffix('\''))
        .ok_or("an unterminated character")?;
    let s = unescape(inner)?;
    let mut cs = s.chars();
    match (cs.next(), cs.next()) {
        (Some(c), None) => Ok(c),
        _ => Err("a character literal holds one character".to_string()),
    }
}

/// `s` with its escapes processed: `\n`, `\r`, `\t`, `\\`, `\0`, `\'`, `\"`, `\s` (a space,
/// which a block keeps at the end of a line), `\x7F`, `\u{…}`, and a `\` before a line break
/// (which skips the break and the next line's indentation).
pub(crate) fn unescape(s: &str) -> Result<String, String> {
    let mut out = String::new();
    let mut cs = s.chars().peekable();
    while let Some(c) = cs.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match cs.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            Some('0') => out.push('\0'),
            Some('\'') => out.push('\''),
            Some('"') => out.push('"'),
            Some('s') => out.push(' '),
            Some('x') => {
                let h: String = cs.by_ref().take(2).collect();
                match u8::from_str_radix(&h, 16) {
                    Ok(v) if v <= 0x7F => out.push(v as char),
                    _ => {
                        return Err(format!(
                            "`\\x{h}` is not an ASCII character (`\\x00` to `\\x7F`)"
                        ));
                    }
                }
            }
            Some('u') => {
                if cs.next() != Some('{') {
                    return Err("`\\u` is written `\\u{…}`".to_string());
                }
                let h: String = cs.by_ref().take_while(|&c| c != '}').collect();
                let h = h.replace('_', "");
                match u32::from_str_radix(&h, 16).ok().and_then(char::from_u32) {
                    Some(c) => out.push(c),
                    None => return Err(format!("`\\u{{{h}}}` is not a Unicode scalar value")),
                }
            }
            Some('\n') => {
                while cs.peek().is_some_and(|c| c.is_whitespace()) {
                    cs.next();
                }
            }
            Some('\r') if cs.peek() == Some(&'\n') => {
                cs.next();
                while cs.peek().is_some_and(|c| c.is_whitespace()) {
                    cs.next();
                }
            }
            Some(other) => return Err(format!("`\\{other}` is not an escape")),
            None => return Err("a `\\` at the end of the string".to_string()),
        }
    }
    Ok(out)
}

/// A dedented block's text (`text.md`: Literals): from the line after the opening quotes to the
/// line of the closing ones, whose indentation is taken from every line.
fn block_value(body: &str, raw: bool) -> Result<String, String> {
    let body = body.replace("\r\n", "\n");
    let body = body
        .strip_prefix('\n')
        .ok_or("a `\"\"\"` block starts on the next line")?;
    let (lines, last) = match body.rfind('\n') {
        Some(i) => (&body[..i], &body[i + 1..]),
        None => ("", body),
    };
    if !last.chars().all(|c| c == ' ' || c == '\t') {
        return Err("the closing `\"\"\"` of a block is on a line of its own".to_string());
    }
    let indent = last;
    let mut out = String::new();
    if body.find('\n').is_none() {
        return Ok(out);
    }
    for line in lines.split('\n') {
        if line.chars().all(|c| c == ' ' || c == '\t') {
            out.push('\n');
            continue;
        }
        let Some(rest) = line.strip_prefix(indent) else {
            let lead: String = line
                .chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .collect();
            return Err(if lead.len() >= indent.len() {
                "a block mixes tabs and spaces in its indentation".to_string()
            } else {
                "a line of a block is indented less than its closing `\"\"\"`".to_string()
            });
        };
        let rest = rest.trim_end_matches([' ', '\t']);
        if raw {
            out.push_str(rest);
        } else {
            out.push_str(&unescape(rest)?);
        }
        out.push('\n');
    }
    Ok(out)
}

/// A `"""` block literal's lines (from its opening quotes), dedented as `block_value` does: for
/// each, `None` if it is blank, or where its text starts in the literal and the text, without its
/// indentation or trailing white space. The block's errors are `block_value`'s.
fn block_lines(lit: &str) -> Result<Vec<Option<(usize, &str)>>, String> {
    let body = &lit[3..lit.len() - 3];
    block_value(body, true)?;
    let last = body.rfind('\n').unwrap_or(0);
    let indent = body[last + 1..].len();
    let mut out = Vec::new();
    // After the opening line, up to the closing one.
    let mut at = 3 + body.find('\n').map_or(body.len(), |i| i + 1);
    if last < at - 3 {
        return Ok(out);
    }
    for line in body[at - 3..last].split('\n') {
        let text = line.trim_end_matches('\r');
        if text.chars().all(|c| c == ' ' || c == '\t') {
            out.push(None);
        } else {
            out.push(Some((
                at + indent,
                text[indent..].trim_end_matches([' ', '\t']),
            )));
        }
        at += line.len() + 1;
    }
    Ok(out)
}

impl Lowerer<'_> {
    /// A string, raw string or block literal: a `&str`.
    pub(super) fn string_lit(&mut self, tok: &SyntaxToken, range: TextRange) -> ExprId {
        match string_value(tok.text()) {
            Ok(s) => self.alloc(ExprKind::Str(s), range),
            Err(why) => {
                let at = match block_error_at(tok.text()) {
                    Some((a, b)) => TextRange::new(range.start + a as u32, range.start + b as u32),
                    None => range,
                };
                self.report(Code::E0902, at, why);
                self.missing(range)
            }
        }
    }

    /// `fmt"… {e} …"` (`expr.fmt.1`): the text, with each hole an expression of the body; a
    /// `fmt"""` block (`text.lit.block.1`) dedented as a block is, each hole on one line.
    pub(super) fn tagged(&mut self, tok: &SyntaxToken, range: TextRange) -> ExprId {
        let text = tok.text();
        let t = moddil_syntax::Tagged::of(text);
        if t.tag != "fmt" {
            return self.tag_call(&t, range);
        }
        if t.raw {
            self.report(
                Code::E0902,
                range,
                "a `fmt` literal cannot be raw: its holes need `{{` and `}}` to write braces",
            );
            return self.missing(range);
        }
        let lit = t.lit.to_string();
        let start = range.start as usize + t.lit_at;
        let mut pieces = Vec::new();
        let mut lit_buf = String::new();
        if lit.starts_with("\"\"\"") {
            let lines = match block_lines(&lit) {
                Ok(lines) => lines,
                Err(why) => {
                    let at = match block_error_at(&lit) {
                        Some((a, b)) => TextRange::new((start + a) as u32, (start + b) as u32),
                        None => range,
                    };
                    self.report(Code::E0902, at, why);
                    return self.missing(range);
                }
            };
            for line in lines {
                if let Some((off, text)) = line
                    && !self.fmt_scan(text, start + off, range, &mut pieces, &mut lit_buf)
                {
                    return self.missing(range);
                }
                lit_buf.push('\n');
            }
        } else {
            let inner = &lit[1..lit.len() - 1];
            if !self.fmt_scan(inner, start + 1, range, &mut pieces, &mut lit_buf) {
                return self.missing(range);
            }
        }
        if !self.fmt_flush(&mut lit_buf, &mut pieces, range) {
            return self.missing(range);
        }
        self.alloc(ExprKind::Fmt(pieces), range)
    }

    /// The text run in `buf`, unescaped, as a piece; false, with an error, if an escape is bad.
    fn fmt_flush(
        &mut self,
        buf: &mut String,
        pieces: &mut Vec<FmtPiece>,
        range: TextRange,
    ) -> bool {
        if buf.is_empty() {
            return true;
        }
        match unescape(buf) {
            Ok(s) => {
                pieces.push(FmtPiece::Lit(s));
                buf.clear();
                true
            }
            Err(why) => {
                self.report(Code::E0902, range, why);
                false
            }
        }
    }

    /// `tag"…"` for a tag other than `fmt` (`comptime.tag.1`): a call of the `#[tag]` function
    /// `tag` in scope with the literal's text, in a `const { }` block, so it runs when
    /// compiling. A tag taking `&[&str]` has holes (`comptime.tag.2`): it is called with the
    /// text's parts between them, and its value's `bind` with the holes' values.
    fn tag_call(&mut self, t: &moddil_syntax::Tagged<'_>, range: TextRange) -> ExprId {
        let (tag, lit) = (t.tag, t.lit);
        let f = match self
            .types
            .tree
            .lookup(self.types.module, tag, self.types.prelude_first)
        {
            Some(crate::Def::Fn(f))
                if self.items.fns[f as usize].tag && self.items.fns[f as usize].is_const =>
            {
                f
            }
            _ => {
                self.report(
                    Code::E1011,
                    range,
                    format!(
                        "`{tag}` is not a `#[tag]` function in scope, so `{tag}\"…\"` means nothing"
                    ),
                );
                return self.missing(range);
            }
        };
        let parts = match self.items.fns[f as usize].params.as_slice() {
            [p] if is_str_ref(&p.ty) => false,
            [p] if matches!(&p.ty, Ty::Ref(false, t)
                if matches!(&**t, Ty::Slice(e) if is_str_ref(e))) =>
            {
                true
            }
            _ => {
                self.report(
                    Code::E1011,
                    range,
                    format!("`{tag}` is a `#[tag]` function, but takes one `&str` or `&[&str]`"),
                );
                return self.missing(range);
            }
        };
        if parts {
            return self.parts_call(f, t, range);
        }
        // As written, as a raw string's text (`comptime.tag.1`).
        let text = match string_value(&format!("r{lit}")) {
            Ok(t) => t,
            Err(why) => {
                self.report(Code::E0902, range, why);
                return self.missing(range);
            }
        };
        let s = self.alloc(ExprKind::Str(text), range);
        let callee = self.alloc(
            ExprKind::Fn {
                func: f,
                args: Vec::new(),
            },
            range,
        );
        let call = self.alloc(
            ExprKind::Call {
                callee,
                args: vec![s],
            },
            range,
        );
        let block = self.alloc(ExprKind::ConstBlock(call), range);
        // Where the text starts: after the tag and the opening quotes.
        let (_, at) = t.body();
        let text = crate::hir::TagText::at(range.start + at as u32, range.end);
        self.out.body.tagged.insert(block, text);
        // A literal with fields is its value in the wrapper that reads them
        // (`comptime.tag.fields.1`).
        let (_, make) = crate::collect::wrapper_names(range);
        if let Some(crate::Def::Fn(w)) =
            self.types
                .tree
                .lookup(self.types.module, &make, self.types.prelude_first)
        {
            // Its type parameter, the value's type, inferred.
            let callee = self.alloc(
                ExprKind::Fn {
                    func: w,
                    args: vec![Ty::Infer(Ty::HOLE)],
                },
                range,
            );
            return self.alloc(
                ExprKind::Call {
                    callee,
                    args: vec![block],
                },
                range,
            );
        }
        block
    }

    /// `tag"… {e} …"` for a tag taking `&[&str]` (`comptime.tag.2`): `const { tag(&[parts]) }`,
    /// and with holes, its value's `bind((e, …,))`. The text is as written, but `{{` and `}}`
    /// are braces.
    fn parts_call(&mut self, f: u32, t: &moddil_syntax::Tagged<'_>, range: TextRange) -> ExprId {
        let (tag, lit) = (t.tag, t.lit);
        let start = range.start as usize + t.lit_at;
        let mut parts = Vec::new();
        let mut holes = Vec::new();
        let mut cur = String::new();
        let mut text = crate::hir::TagText {
            runs: Vec::new(),
            end: range.end,
        };
        if !t.raw && lit.starts_with("\"\"\"") {
            let lines = match block_lines(lit) {
                Ok(lines) => lines,
                Err(why) => {
                    let at = match block_error_at(lit) {
                        Some((a, b)) => TextRange::new((start + a) as u32, (start + b) as u32),
                        None => range,
                    };
                    self.report(Code::E0902, at, why);
                    return self.missing(range);
                }
            };
            for line in lines {
                if let Some((off, line)) = line
                    && !self.parts_scan(
                        line,
                        start + off,
                        &mut parts,
                        &mut holes,
                        &mut cur,
                        &mut text,
                    )
                {
                    return self.missing(range);
                }
                cur.push('\n');
            }
        } else {
            // A raw literal (`sqlr#"…"#`) is between its `#"` and `"#`, so a hole may hold a `"`.
            let (inner, at) = t.body();
            let from = range.start as usize + at;
            if !self.parts_scan(inner, from, &mut parts, &mut holes, &mut cur, &mut text) {
                return self.missing(range);
            }
        }
        parts.push(cur);
        // With holes, the `const { }` block is at the tag's name: the holes and the `bind` call
        // run where the literal is, not when compiling, and compile-time code is found by range.
        let whole = range;
        let range = if holes.is_empty() {
            range
        } else {
            TextRange::new(range.start, range.start + tag.len() as u32)
        };
        let elems = parts
            .into_iter()
            .map(|t| self.alloc(ExprKind::Str(t), range))
            .collect();
        let array = self.alloc(ExprKind::Array(elems), range);
        let arg = self.alloc(
            ExprKind::Ref {
                mutable: false,
                expr: array,
            },
            range,
        );
        let callee = self.alloc(
            ExprKind::Fn {
                func: f,
                args: Vec::new(),
            },
            range,
        );
        let call = self.alloc(
            ExprKind::Call {
                callee,
                args: vec![arg],
            },
            range,
        );
        let block = self.alloc(ExprKind::ConstBlock(call), range);
        self.out.body.tagged.insert(block, text);
        if holes.is_empty() {
            return block;
        }
        let values = self.alloc(ExprKind::Tuple(holes), whole);
        self.alloc(
            ExprKind::MethodCall {
                receiver: block,
                name: "bind".to_string(),
                name_range: whole,
                generic_args: None,
                args: vec![values],
            },
            whole,
        )
    }

    /// The parts and holes of `inner`, which starts at byte `start` of the file, for a tag taking
    /// `&[&str]`: each hole ends the part in `cur`; false, with an error, if a hole is bad.
    fn parts_scan(
        &mut self,
        inner: &str,
        start: usize,
        parts: &mut Vec<String>,
        holes: &mut Vec<ExprId>,
        cur: &mut String,
        text: &mut crate::hir::TagText,
    ) -> bool {
        let chars: Vec<char> = inner.chars().collect();
        let mut i = 0;
        let mut offset = 0;
        // Where the next byte of the parts joined is.
        let joined = |parts: &[String], cur: &str| {
            (parts.iter().map(String::len).sum::<usize>() + cur.len()) as u32
        };
        while i < chars.len() {
            match chars[i] {
                c @ ('{' | '}') if chars.get(i + 1) == Some(&c) => {
                    text.mark(joined(parts, cur), (start + offset) as u32);
                    cur.push(c);
                    i += 2;
                    offset += 2;
                }
                '}' => {
                    let at = (start + offset) as u32;
                    self.report(
                        Code::E0902,
                        TextRange::new(at, at + 1),
                        "a `}` in a tagged literal with holes is written `}}`",
                    );
                    return false;
                }
                '{' => {
                    let end = hole_end(&chars, i + 1);
                    let hole: String = chars[i + 1..end].iter().collect();
                    let hole_start = start + offset + 1;
                    let at = TextRange::new(hole_start as u32, (hole_start + hole.len()) as u32);
                    match self.hole(&hole, hole_start, at) {
                        Some(e) => holes.push(e),
                        None => return false,
                    }
                    parts.push(std::mem::take(cur));
                    let taken: usize = chars[i..=end.min(chars.len() - 1)]
                        .iter()
                        .map(|c| c.len_utf8())
                        .sum();
                    offset += taken;
                    i = end + 1;
                }
                c => {
                    text.mark(joined(parts, cur), (start + offset) as u32);
                    cur.push(c);
                    offset += c.len_utf8();
                    i += 1;
                }
            }
        }
        true
    }

    /// The text and holes of `inner`, which starts at byte `start` of the file, onto `pieces`,
    /// the text run so far in `lit_buf`; false, with an error, if a hole is bad.
    fn fmt_scan(
        &mut self,
        inner: &str,
        start: usize,
        range: TextRange,
        pieces: &mut Vec<FmtPiece>,
        lit_buf: &mut String,
    ) -> bool {
        let bytes: Vec<char> = inner.chars().collect();
        let mut i = 0;
        let mut offset = 0;
        while i < bytes.len() {
            let c = bytes[i];
            match c {
                '{' if bytes.get(i + 1) == Some(&'{') => {
                    lit_buf.push('{');
                    i += 2;
                    offset += 2;
                }
                '}' if bytes.get(i + 1) == Some(&'}') => {
                    lit_buf.push('}');
                    i += 2;
                    offset += 2;
                }
                '{' => {
                    if !self.fmt_flush(lit_buf, pieces, range) {
                        return false;
                    }
                    let end = hole_end(&bytes, i + 1);
                    let hole: String = bytes[i + 1..end].iter().collect();
                    let hole_start = start + offset + 1;
                    let at = TextRange::new(hole_start as u32, (hole_start + hole.len()) as u32);
                    // `{x:spec}`: `?` writes `x`'s `Debug` (`expr.fmt.debug.1`), and the rest
                    // pads and cuts it (`expr.fmt.spec.1`).
                    let (expr_text, mut spec, names) = match spec_colon(&hole) {
                        Some(k) => match parse_spec(&hole[k + 1..]) {
                            Ok((spec, names)) => (&hole[..k], spec, (k + 1, names)),
                            Err(why) => {
                                self.report(Code::E0902, at, why);
                                return false;
                            }
                        },
                        None => (hole.as_str(), Spec::default(), (0, [None, None])),
                    };
                    // A width or precision named `name$`: an expression where it stands.
                    let (from, [width, precision]) = names;
                    for (named, slot) in [(width, 0), (precision, 1)] {
                        let Some((name, off)) = named else { continue };
                        let Some(x) = self.hole(&name, hole_start + from + off, at) else {
                            return false;
                        };
                        if slot == 0 {
                            spec.width_arg = Some(x);
                        } else {
                            spec.precision_arg = Some(x);
                        }
                    }
                    match self.hole(expr_text, hole_start, at) {
                        Some(e) => pieces.push(FmtPiece::Hole(e, spec)),
                        None => return false,
                    }
                    let taken: usize = bytes[i..=end.min(bytes.len() - 1)]
                        .iter()
                        .map(|c| c.len_utf8())
                        .sum();
                    offset += taken;
                    i = end + 1;
                }
                c => {
                    lit_buf.push(c);
                    offset += c.len_utf8();
                    i += 1;
                }
            }
        }
        true
    }

    /// The expression of a hole (of `fmt`, or of a tag taking `&[&str]`) whose text starts at
    /// byte `at` of the file, parsed where it stands so its ranges are the file's.
    fn hole(&mut self, text: &str, at: usize, range: TextRange) -> Option<ExprId> {
        let wrapper = "fn f(){(";
        if at < wrapper.len() {
            self.error(Diagnostic::unsupported(range, "this hole"));
            return None;
        }
        let source = format!("{}{wrapper}{text})}}", " ".repeat(at - wrapper.len()));
        let parse = moddil_syntax::parse(&source);
        if let Some(e) = parse.errors().first() {
            self.report(Code::E0001, range, e.message.clone());
            return None;
        }
        let expr = parse
            .syntax()
            .descendants()
            .into_iter()
            .find(|n| n.kind() == K::PAREN_EXPR)
            .and_then(|p| p.children().find_map(ast::Expr::cast));
        match expr {
            Some(e) if !text.trim().is_empty() => Some(self.expr(&e)),
            _ => {
                self.report(Code::E0902, range, "a hole holds an expression: `{x}`");
                None
            }
        }
    }
}

/// Where the hole starting at `from` ends: its closing `}`, skipping nested braces, strings and
/// characters.
fn hole_end(cs: &[char], from: usize) -> usize {
    let mut depth = 0;
    let mut i = from;
    while i < cs.len() {
        match cs[i] {
            '"' => {
                i += 1;
                while i < cs.len() && cs[i] != '"' {
                    if cs[i] == '\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            '{' => depth += 1,
            '}' if depth == 0 => return i,
            '}' => depth -= 1,
            _ => {}
        }
        i += 1;
    }
    cs.len()
}

/// Where the hole's format spec starts: the byte of its `:`, the first not in brackets, a string
/// or a path's `::`.
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

/// A width or precision named in a spec, `name$`: the name, and its byte offset in the spec.
type Named = Option<(String, usize)>;

/// A format spec, `[[fill]align][+][#][0][width][.precision][type]` with `type` one of `?`, `x`,
/// `X`, `b`, `o`, `e` and `E`, and a width or precision a number or `name$`
/// (`expr.fmt.spec.1`), with the names of those named; or what is wrong with it.
fn parse_spec(text: &str) -> Result<(Spec, [Named; 2]), String> {
    let bad = || {
        format!(
            "`{text}` is not a format spec: `[[fill]align][+][#][0][width][.precision][type]`, \
             the width and precision a number or `name$`, the type `?`, `x`, `X`, `b`, `o`, `e` \
             or `E`"
        )
    };
    let align_of = |c: char| match c {
        '<' => Some(Align::Left),
        '^' => Some(Align::Center),
        '>' => Some(Align::Right),
        _ => None,
    };
    let cs: Vec<char> = text.chars().collect();
    let mut spec = Spec::default();
    let mut i = 0;
    if cs.len() >= 2
        && let Some(a) = align_of(cs[1])
    {
        spec.fill = Some(cs[0]);
        spec.align = Some(a);
        i = 2;
    } else if let Some(a) = cs.first().copied().and_then(align_of) {
        spec.align = Some(a);
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
    let number = |i: &mut usize| -> Option<u32> {
        let start = *i;
        while cs.get(*i).is_some_and(char::is_ascii_digit) {
            *i += 1;
        }
        cs[start..*i].iter().collect::<String>().parse().ok()
    };
    // `name$`: an identifier, then `$`; its offset in bytes.
    let named = |i: &mut usize| -> Named {
        let start = *i;
        let mut j = start;
        while cs.get(j).is_some_and(|c| c.is_alphanumeric() || *c == '_') {
            j += 1;
        }
        let first = cs
            .get(start)
            .is_some_and(|c| c.is_alphabetic() || *c == '_');
        if !first || cs.get(j) != Some(&'$') {
            return None;
        }
        *i = j + 1;
        let off = cs[..start].iter().map(|c| c.len_utf8()).sum();
        Some((cs[start..j].iter().collect(), off))
    };
    let width_name = named(&mut i);
    if width_name.is_none() {
        spec.width = number(&mut i);
    }
    let mut precision_name = None;
    if cs.get(i) == Some(&'.') {
        i += 1;
        precision_name = named(&mut i);
        if precision_name.is_none() {
            spec.precision = Some(number(&mut i).ok_or_else(bad)?);
        }
    }
    match cs.get(i) {
        Some('?') => spec.debug = true,
        Some('x') => spec.radix = Some(Radix::Hex),
        Some('X') => spec.radix = Some(Radix::UpperHex),
        Some('b') => spec.radix = Some(Radix::Binary),
        Some('o') => spec.radix = Some(Radix::Octal),
        Some('e') => spec.exponent = Some(false),
        Some('E') => spec.exponent = Some(true),
        _ => i = i.wrapping_sub(1),
    }
    i = i.wrapping_add(1);
    // An alignment or `0` says how to fill a width, so it needs one; `#` is a base's prefix.
    let width = spec.width.is_some() || width_name.is_some();
    if i != cs.len()
        || ((spec.align.is_some() || spec.zero) && !width)
        || (spec.prefix && spec.radix.is_none() && !spec.debug)
    {
        return Err(bad());
    }
    Ok((spec, [width_name, precision_name]))
}

/// Whether `t` is `&str`.
fn is_str_ref(t: &Ty) -> bool {
    matches!(t, Ty::Ref(false, s) if **s == Ty::Str)
}

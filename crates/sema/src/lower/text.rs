//! Text literals (`expressions/literals.md`): `'c'`, `"…"` with escapes, raw `r#"…"#`, dedented
//! `"""` blocks, and `fmt"…"` with its holes.

use moddil_syntax::ast;
use moddil_syntax::{SyntaxKind as K, SyntaxToken, TextRange};

use super::Lowerer;
use crate::hir::{Align, ExprId, ExprKind, FmtPiece, Radix, Spec};
use crate::{Code, Diagnostic};

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

    /// `fmt"… {e} …"` (`expr.fmt.1`): the text, with each hole an expression of the body.
    pub(super) fn tagged(&mut self, tok: &SyntaxToken, range: TextRange) -> ExprId {
        let text = tok.text();
        let tag_len = text.find('"').unwrap_or(0);
        let (tag, lit) = text.split_at(tag_len);
        let lit = lit.to_string();
        if tag != "fmt" {
            return self.unsupported(range, &format!("the tagged literal `{tag}\"…\"`"));
        }
        if lit.starts_with("\"\"\"") {
            return self.unsupported(range, "a `fmt` block");
        }
        let start = range.start as usize + tag_len + 1;
        let inner = &lit[1..lit.len() - 1];
        let mut pieces = Vec::new();
        let mut lit_buf = String::new();
        let bytes: Vec<char> = inner.chars().collect();
        let mut i = 0;
        let mut offset = 0;
        let flush = |cx: &mut Self, buf: &mut String, pieces: &mut Vec<FmtPiece>| -> bool {
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
                    cx.report(Code::E0902, range, why);
                    false
                }
            }
        };
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
                    if !flush(self, &mut lit_buf, &mut pieces) {
                        return self.missing(range);
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
                                return self.missing(range);
                            }
                        },
                        None => (hole.as_str(), Spec::default(), (0, [None, None])),
                    };
                    // A width or precision named `name$`: an expression where it stands.
                    let (from, [width, precision]) = names;
                    for (named, slot) in [(width, 0), (precision, 1)] {
                        let Some((name, off)) = named else { continue };
                        let Some(x) = self.hole(&name, hole_start + from + off, at) else {
                            return self.missing(range);
                        };
                        if slot == 0 {
                            spec.width_arg = Some(x);
                        } else {
                            spec.precision_arg = Some(x);
                        }
                    }
                    match self.hole(expr_text, hole_start, at) {
                        Some(e) => pieces.push(FmtPiece::Hole(e, spec)),
                        None => return self.missing(range),
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
        if !flush(self, &mut lit_buf, &mut pieces) {
            return self.missing(range);
        }
        self.alloc(ExprKind::Fmt(pieces), range)
    }

    /// The expression of a `fmt` hole whose text starts at byte `at` of the file, parsed where it
    /// stands so its ranges are the file's.
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
                self.report(
                    Code::E0902,
                    range,
                    "a `fmt` hole holds an expression: `{x}`",
                );
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
        || (spec.prefix && spec.radix.is_none())
    {
        return Err(bad());
    }
    Ok((spec, [width_name, precision_name]))
}

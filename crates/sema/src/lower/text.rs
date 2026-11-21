//! Text literals (`expressions/literals.md`): `'c'`, `"…"` with escapes, raw `r#"…"#`, dedented
//! `"""` blocks, and `fmt"…"` with its holes.

use moddil_syntax::ast;
use moddil_syntax::{SyntaxKind as K, SyntaxToken, TextRange};

use super::Lowerer;
use crate::hir::{ExprId, ExprKind, FmtPiece};
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
                self.report(Code::E0902, range, why);
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
                    match self.hole(&hole, hole_start, at) {
                        Some(e) => pieces.push(FmtPiece::Hole(e)),
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
        if format_spec(text) {
            self.error(Diagnostic::unsupported(range, "a format spec (`{x:…}`)"));
            return None;
        }
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

/// Whether a hole has a format spec: a `:` outside brackets, strings and `::`.
fn format_spec(hole: &str) -> bool {
    let cs: Vec<char> = hole.chars().collect();
    let mut depth = 0;
    let mut i = 0;
    while i < cs.len() {
        match cs[i] {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            '"' => {
                i += 1;
                while i < cs.len() && cs[i] != '"' {
                    i += 1;
                }
            }
            ':' if cs.get(i + 1) == Some(&':') => i += 1,
            ':' if depth == 0 && (i == 0 || cs[i - 1] != ':') => return true,
            _ => {}
        }
        i += 1;
    }
    false
}

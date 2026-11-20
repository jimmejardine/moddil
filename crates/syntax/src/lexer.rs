//! The lexer: the Tokens section of `reference/grammar/moddil.ebnf`, implemented by hand.
//!
//! It never fails: text it cannot read becomes an `ERROR` token plus an error, and every byte of
//! the input is in exactly one token, so the tree built from the tokens is lossless. Punctuation
//! comes out one character at a time with a joint flag (except `#[` and `#![`); the parser glues
//! compounds such as `>>=` from joint characters.

use unicode_ident::{is_xid_continue, is_xid_start};
use unicode_normalization::is_nfc;

use crate::{SyntaxError, SyntaxKind, TextRange};

/// A token: its kind, its length in bytes, and whether the next token follows with no trivia
/// between.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    /// The kind.
    pub kind: SyntaxKind,
    /// The length in bytes.
    pub len: u32,
    /// The next token starts right after this one (no whitespace or comment between).
    pub joint: bool,
}

/// Splits `text` into tokens, trivia included.
pub fn lex(text: &str) -> (Vec<Token>, Vec<SyntaxError>) {
    let mut lexer = Lexer {
        text,
        pos: 0,
        tokens: Vec::new(),
        errors: Vec::new(),
    };
    lexer.run();
    let Lexer {
        mut tokens,
        mut errors,
        ..
    } = lexer;
    for i in 0..tokens.len() {
        tokens[i].joint =
            !tokens[i].kind.is_trivia() && tokens.get(i + 1).is_some_and(|t| !t.kind.is_trivia());
    }
    // Bidirectional control characters can only be written as escapes.
    for (at, c) in text.char_indices() {
        if is_bidi_control(c) {
            let at = at as u32;
            errors.push(SyntaxError::new(
                TextRange::new(at, at + c.len_utf8() as u32),
                format!(
                    "bidirectional control character U+{:04X}: write it as an escape",
                    c as u32
                ),
            ));
        }
    }
    errors.sort_by_key(|e| e.range.start);
    (tokens, errors)
}

fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

fn is_ident_start(c: char) -> bool {
    c == '_' || is_xid_start(c)
}

struct Lexer<'t> {
    text: &'t str,
    pos: usize,
    tokens: Vec<Token>,
    errors: Vec<SyntaxError>,
}

impl Lexer<'_> {
    fn rest(&self) -> &str {
        &self.text[self.pos..]
    }

    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn peek_at(&self, n: usize) -> Option<char> {
        self.rest().chars().nth(n)
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.pos += c.len_utf8();
            true
        } else {
            false
        }
    }

    fn eat_while(&mut self, f: impl Fn(char) -> bool) -> usize {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if !f(c) {
                break;
            }
            self.pos += c.len_utf8();
        }
        self.pos - start
    }

    fn error(&mut self, start: usize, end: usize, message: impl Into<String>) {
        self.errors.push(SyntaxError::new(
            TextRange::new(start as u32, end as u32),
            message,
        ));
    }

    fn push(&mut self, start: usize, kind: SyntaxKind) {
        self.tokens.push(Token {
            kind,
            len: (self.pos - start) as u32,
            joint: false,
        });
    }

    /// The previous token is a lone `.` with nothing between: a tuple index follows (`t.0.1`).
    fn after_field_dot(&self) -> bool {
        let n = self.tokens.len();
        n >= 1
            && self.tokens[n - 1].kind == T!["."]
            && !(n >= 2 && self.tokens[n - 2].kind == T!["."])
    }

    fn run(&mut self) {
        while let Some(c) = self.peek() {
            let start = self.pos;
            let kind = self.token(c, start);
            debug_assert!(self.pos > start, "the lexer must make progress");
            self.push(start, kind);
        }
    }

    fn token(&mut self, c: char, start: usize) -> SyntaxKind {
        match c {
            ' ' | '\t' | '\r' | '\n' => {
                self.eat_while(|c| matches!(c, ' ' | '\t' | '\r' | '\n'));
                SyntaxKind::WHITESPACE
            }
            '/' if self.peek_at(1) == Some('/') => self.line_comment(),
            '/' if self.peek_at(1) == Some('*') => self.block_comment(start),
            '0'..='9' => self.number(),
            '"' => self.string(start, false),
            '\'' => self.quote(start),
            c if is_ident_start(c) => self.word(start),
            _ => {
                if let Some((len, kind)) = SyntaxKind::lexed_punct(self.rest()) {
                    self.pos += len;
                    return kind;
                }
                self.bump();
                let what = if c.is_whitespace() {
                    format!(
                        "whitespace character U+{:04X}: only spaces, tabs and newlines are allowed",
                        c as u32
                    )
                } else {
                    format!("unexpected character `{c}`")
                };
                self.error(start, self.pos, what);
                SyntaxKind::ERROR
            }
        }
    }

    fn line_comment(&mut self) -> SyntaxKind {
        let rest = self.rest();
        let kind = if rest.starts_with("////") {
            SyntaxKind::LINE_COMMENT
        } else if rest.starts_with("///") {
            SyntaxKind::DOC_OUTER
        } else if rest.starts_with("//!") {
            SyntaxKind::DOC_INNER
        } else {
            SyntaxKind::LINE_COMMENT
        };
        self.eat_while(|c| c != '\n');
        kind
    }

    fn block_comment(&mut self, start: usize) -> SyntaxKind {
        self.pos += 2;
        let mut depth = 1;
        while depth > 0 {
            let rest = self.rest();
            if rest.starts_with("/*") {
                self.pos += 2;
                depth += 1;
            } else if rest.starts_with("*/") {
                self.pos += 2;
                depth -= 1;
            } else if self.bump().is_none() {
                self.error(start, self.pos, "unterminated block comment");
                break;
            }
        }
        SyntaxKind::BLOCK_COMMENT
    }

    fn number(&mut self) -> SyntaxKind {
        let field = self.after_field_dot();
        if self.peek() == Some('0') {
            let digits: Option<fn(char) -> bool> = match self.peek_at(1) {
                Some('x') => Some(|c: char| c.is_ascii_hexdigit() || c == '_'),
                Some('o') => Some(|c: char| matches!(c, '0'..='7' | '_')),
                Some('b') => Some(|c: char| matches!(c, '0' | '1' | '_')),
                _ => None,
            };
            if let Some(digits) = digits
                && self.peek_at(2).is_some_and(digits)
            {
                self.pos += 2;
                self.eat_while(digits);
                return SyntaxKind::INTEGER;
            }
        }
        self.eat_while(|c| c.is_ascii_digit() || c == '_');
        if field {
            return SyntaxKind::INTEGER;
        }
        let mut kind = SyntaxKind::INTEGER;
        if self.peek() == Some('.') && self.peek_at(1).is_some_and(|c| c.is_ascii_digit()) {
            self.pos += 1;
            self.eat_while(|c| c.is_ascii_digit() || c == '_');
            kind = SyntaxKind::FLOAT;
        }
        if matches!(self.peek(), Some('e' | 'E')) {
            let sign = usize::from(matches!(self.peek_at(1), Some('+' | '-')));
            if self
                .peek_at(1 + sign)
                .is_some_and(|c| c.is_ascii_digit() || c == '_')
            {
                self.pos += 1 + sign;
                self.eat_while(|c| c.is_ascii_digit() || c == '_');
                kind = SyntaxKind::FLOAT;
            }
        }
        kind
    }

    /// At `"`: a block string if `"""` and a newline follow, else a string.
    fn string(&mut self, start: usize, tagged: bool) -> SyntaxKind {
        if self.block_opener() {
            self.block_body(start, false);
            return if tagged {
                SyntaxKind::TAGGED_STRING
            } else {
                SyntaxKind::BLOCK_STRING
            };
        }
        self.pos += 1;
        self.quoted(start, '"', tagged);
        if tagged {
            SyntaxKind::TAGGED_STRING
        } else {
            SyntaxKind::STRING
        }
    }

    /// `"""` then spaces or tabs and a newline.
    fn block_opener(&self) -> bool {
        let Some(after) = self.rest().strip_prefix("\"\"\"") else {
            return false;
        };
        let after = after.trim_start_matches([' ', '\t']);
        after.starts_with('\n') || after.starts_with("\r\n")
    }

    /// From `"""`: lines up to one holding only spaces or tabs and `"""`.
    fn block_body(&mut self, start: usize, raw: bool) {
        self.pos += 3;
        self.eat_while(|c| c == ' ' || c == '\t');
        self.eat('\r');
        self.eat('\n');
        loop {
            let line_start = self.pos;
            self.eat_while(|c| c == ' ' || c == '\t');
            if self.rest().starts_with("\"\"\"") {
                self.pos += 3;
                return;
            }
            self.pos = line_start;
            let line_end = self.rest().find('\n').map(|i| self.pos + i + 1);
            if !raw {
                let end = line_end.unwrap_or(self.text.len());
                self.check_escapes(self.pos, end);
            }
            match line_end {
                Some(end) => self.pos = end,
                None => {
                    self.pos = self.text.len();
                    self.error(
                        start,
                        self.pos,
                        "unterminated block string: it ends at a line holding only `\"\"\"`",
                    );
                    return;
                }
            }
        }
    }

    /// The body of a quoted literal after its opening quote, up to the closing `close`.
    /// In a tagged literal, `{` opens a hole that runs to its matching `}`.
    fn quoted(&mut self, start: usize, close: char, tagged: bool) {
        loop {
            let Some(c) = self.peek() else {
                self.error(start, self.pos, "unterminated literal");
                return;
            };
            if c == close {
                self.pos += 1;
                return;
            }
            if c == '\\' {
                self.escape();
            } else if tagged && c == '{' {
                if self.rest().starts_with("{{") {
                    self.pos += 2;
                } else {
                    self.pos += 1;
                    if !self.hole() {
                        self.error(start, self.pos, "unterminated hole in a tagged literal");
                        return;
                    }
                }
            } else {
                self.pos += c.len_utf8();
            }
        }
    }

    /// Inside a hole: up to its matching `}`, skipping string, raw string and char literals.
    fn hole(&mut self) -> bool {
        let mut depth = 1;
        while let Some(c) = self.peek() {
            match c {
                '{' => {
                    depth += 1;
                    self.pos += 1;
                }
                '}' => {
                    depth -= 1;
                    self.pos += 1;
                    if depth == 0 {
                        return true;
                    }
                }
                '"' => {
                    let s = self.pos;
                    self.pos += 1;
                    self.quoted(s, '"', false);
                }
                '\'' => {
                    if let Some(len) = self.char_len() {
                        self.pos += len;
                    } else {
                        self.pos += 1;
                    }
                }
                'r' if self.raw_ahead(1) => {
                    let s = self.pos;
                    self.pos += 1;
                    self.raw(s);
                }
                _ => self.pos += c.len_utf8(),
            }
        }
        false
    }

    /// `#`s then `"` at offset `n`.
    fn raw_ahead(&self, n: usize) -> bool {
        self.rest()[n..].trim_start_matches('#').starts_with('"')
    }

    /// After `r`/`br`: `#`*n, `"`, anything up to `"` followed by n `#`.
    fn raw(&mut self, start: usize) {
        let hashes = self.eat_while(|c| c == '#');
        self.pos += 1; // the '"'
        let closing: String = std::iter::once('"')
            .chain(std::iter::repeat_n('#', hashes))
            .collect();
        match self.rest().find(&closing) {
            Some(i) => self.pos += i + closing.len(),
            None => {
                self.pos = self.text.len();
                self.error(start, self.pos, "unterminated raw string");
            }
        }
    }

    /// A backslash escape: `\n`, `\x41`, `\u{1F600}`.
    fn escape(&mut self) {
        let start = self.pos;
        self.pos += 1;
        let ok = match self.bump() {
            Some('n' | 'r' | 't' | '0' | '\\' | '"' | '\'' | 's') => true,
            Some('x') => (0..2).all(|_| {
                let hex = self.peek().is_some_and(|c| c.is_ascii_hexdigit());
                if hex {
                    self.pos += 1;
                }
                hex
            }),
            Some('u') if self.eat('{') => {
                let n = self.eat_while(|c| c.is_ascii_hexdigit());
                self.eat('}') && n > 0
            }
            _ => false,
        };
        if !ok {
            self.error(
                start,
                self.pos,
                "invalid escape: use \\n \\r \\t \\0 \\\\ \\\" \\' \\s \\xHH or \\u{H…}",
            );
        }
    }

    fn check_escapes(&mut self, from: usize, to: usize) {
        let save = self.pos;
        self.pos = from;
        while self.pos < to {
            if self.rest().starts_with('\\') {
                self.escape();
            } else {
                self.bump();
            }
        }
        self.pos = save;
    }

    /// The length of a char literal starting here (`'a'`, `'\n'`), if one does.
    fn char_len(&self) -> Option<usize> {
        let rest = self.rest();
        let mut chars = rest.char_indices().skip(1);
        let (_, c) = chars.next()?;
        if c == '\\' {
            // An escape, then a closing quote.
            let close = rest[2..].find('\'')?;
            let body = &rest[1..2 + close];
            let ok = body.len() >= 2 && !body.contains('\n');
            return ok.then_some(2 + close + 1);
        }
        if c == '\'' || c == '\n' {
            return None;
        }
        let (i, close) = chars.next()?;
        (close == '\'').then_some(i + 1)
    }

    /// At `'`: a char literal, else a label.
    fn quote(&mut self, start: usize) -> SyntaxKind {
        if self.char_len().is_some() {
            self.pos += 1;
            if self.peek() == Some('\\') {
                self.escape();
            } else {
                self.bump();
            }
            if !self.eat('\'') {
                self.error(start, self.pos, "a char literal holds one character");
                self.eat_while(|c| c != '\'' && c != '\n');
                self.eat('\'');
            }
            return SyntaxKind::CHAR;
        }
        self.pos += 1;
        if self.peek().is_some_and(is_ident_start) && self.ident_text().is_some() {
            return SyntaxKind::LABEL;
        }
        self.error(start, self.pos, "expected a char literal or a label");
        SyntaxKind::ERROR
    }

    /// IDENT_TEXT: `XID_START XID_CONTINUE*` or `_ XID_CONTINUE+`. `None` (and nothing eaten)
    /// for a lone `_`.
    fn ident_text(&mut self) -> Option<&str> {
        let start = self.pos;
        let first = self.bump()?;
        let n = self.eat_while(is_xid_continue);
        if first == '_' && n == 0 {
            self.pos = start;
            return None;
        }
        Some(&self.text[start..self.pos])
    }

    /// An identifier, keyword, `_`, or a literal with a word prefix: raw strings (`r#"…"#`),
    /// byte strings (`b"…"`, `b'…'`) and tagged literals (`fmt"…"`).
    fn word(&mut self, start: usize) -> SyntaxKind {
        let Some(word) = self.ident_text() else {
            self.pos += 1;
            return T![_];
        };
        let word = word.to_string();
        let next = self.peek();
        match (word.as_str(), next) {
            ("r" | "br", Some('"' | '#')) if self.raw_ahead(0) => {
                if word == "r" && self.block_opener() {
                    self.block_body(start, true);
                    return SyntaxKind::BLOCK_STRING;
                }
                self.raw(start);
                return SyntaxKind::RAW_STRING;
            }
            ("b", Some('"')) => {
                self.pos += 1;
                self.quoted(start, '"', false);
                return SyntaxKind::BYTE_STRING;
            }
            ("b", Some('\'')) if self.char_len().is_some() => {
                self.quote(start);
                return SyntaxKind::BYTE;
            }
            (_, Some('"')) if !matches!(word.as_str(), "r" | "br" | "b") => {
                return self.string(start, true);
            }
            // A tag before a raw string: `sqlr#"…"#` (a tag ending in `r` then a plain string is
            // read as that: `fmtr"…"` has the tag `fmtr`).
            (w, Some('#')) if w.len() > 1 && w.ends_with('r') && self.raw_ahead(0) => {
                self.raw(start);
                return SyntaxKind::TAGGED_STRING;
            }
            _ => {}
        }
        if let Some(kw) = SyntaxKind::keyword(&word) {
            return kw;
        }
        if !word.is_ascii() && !is_nfc(&word) {
            self.error(
                start,
                self.pos,
                format!("identifier `{word}` is not in Unicode normal form C (NFC)"),
            );
        }
        SyntaxKind::IDENT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(text: &str) -> Vec<(SyntaxKind, &str)> {
        let (tokens, errors) = lex(text);
        assert!(errors.is_empty(), "{text:?}: {errors:?}");
        let mut at = 0;
        tokens
            .iter()
            .filter_map(|t| {
                let s = &text[at..at + t.len as usize];
                at += t.len as usize;
                (t.kind != SyntaxKind::WHITESPACE).then_some((t.kind, s))
            })
            .collect()
    }

    fn one(text: &str) -> SyntaxKind {
        let k = kinds(text);
        assert_eq!(k.len(), 1, "{text:?} lexes as {k:?}");
        k[0].0
    }

    #[test]
    fn comments() {
        assert_eq!(one("// x"), SyntaxKind::LINE_COMMENT);
        assert_eq!(one("//"), SyntaxKind::LINE_COMMENT);
        assert_eq!(one("//// x"), SyntaxKind::LINE_COMMENT);
        assert_eq!(one("/// x"), SyntaxKind::DOC_OUTER);
        assert_eq!(one("///"), SyntaxKind::DOC_OUTER);
        assert_eq!(one("//! x"), SyntaxKind::DOC_INNER);
        assert_eq!(one("/* a /* b */ c */"), SyntaxKind::BLOCK_COMMENT);
    }

    #[test]
    fn names() {
        assert_eq!(one("x"), SyntaxKind::IDENT);
        assert_eq!(one("_x"), SyntaxKind::IDENT);
        assert_eq!(one("_"), T![_]);
        assert_eq!(one("fn"), T![fn]);
        assert_eq!(one("Self"), T![Self]);
        assert_eq!(one("größe"), SyntaxKind::IDENT);
        assert_eq!(one("'outer"), SyntaxKind::LABEL);
        assert_eq!(one("'a'"), SyntaxKind::CHAR);
        assert_eq!(one("'\\n'"), SyntaxKind::CHAR);
        assert_eq!(one("'\\u{1F600}'"), SyntaxKind::CHAR);
        assert_eq!(one("b'x'"), SyntaxKind::BYTE);
    }

    #[test]
    fn numbers() {
        assert_eq!(one("1_000"), SyntaxKind::INTEGER);
        assert_eq!(one("0xff"), SyntaxKind::INTEGER);
        assert_eq!(one("0b1010"), SyntaxKind::INTEGER);
        assert_eq!(one("1.5"), SyntaxKind::FLOAT);
        assert_eq!(one("1e9"), SyntaxKind::FLOAT);
        assert_eq!(one("2.5E-3"), SyntaxKind::FLOAT);
        let k: Vec<_> = kinds("1..2").into_iter().map(|t| t.0).collect();
        assert_eq!(
            k,
            [SyntaxKind::INTEGER, T!["."], T!["."], SyntaxKind::INTEGER]
        );
        let k: Vec<_> = kinds("t.0.1").into_iter().map(|t| t.0).collect();
        assert_eq!(
            k,
            [
                SyntaxKind::IDENT,
                T!["."],
                SyntaxKind::INTEGER,
                T!["."],
                SyntaxKind::INTEGER
            ]
        );
        let k: Vec<_> = kinds("1.max()").into_iter().map(|t| t.0).collect();
        assert_eq!(k[0], SyntaxKind::INTEGER);
        let k: Vec<_> = kinds("x . 1.5").into_iter().map(|t| t.0).collect();
        assert_eq!(k[2], SyntaxKind::FLOAT);
    }

    #[test]
    fn strings() {
        assert_eq!(one(r#""a \"b\" \u{41}""#), SyntaxKind::STRING);
        assert_eq!(one(r#"b"bytes""#), SyntaxKind::BYTE_STRING);
        assert_eq!(one(r##"r#"a "quoted" raw"#"##), SyntaxKind::RAW_STRING);
        assert_eq!(one(r#"r"raw""#), SyntaxKind::RAW_STRING);
        assert_eq!(one(r#"br"x""#), SyntaxKind::RAW_STRING);
        assert_eq!(one("\"\"\"\n  a\n  \"\"\""), SyntaxKind::BLOCK_STRING);
        assert_eq!(one("r\"\"\"\n  \\a\n  \"\"\""), SyntaxKind::BLOCK_STRING);
        assert_eq!(
            one(r#"fmt"Hi {name.or("guest")} {{braces}}""#),
            SyntaxKind::TAGGED_STRING
        );
        assert_eq!(one(r#"fmt"{ {x} }""#), SyntaxKind::TAGGED_STRING);
        assert_eq!(
            one("sql\"\"\"\n  SELECT {x}\n  \"\"\""),
            SyntaxKind::TAGGED_STRING
        );
        assert_eq!(one(r##"sqlr#"raw"#"##), SyntaxKind::TAGGED_STRING);
        assert_eq!(one(r#"fmt"""#), SyntaxKind::TAGGED_STRING);
    }

    #[test]
    fn punctuation_is_single_and_joint() {
        let (tokens, _) = lex(">>= > >");
        let shape: Vec<_> = tokens.iter().map(|t| (t.kind, t.joint)).collect();
        assert_eq!(
            shape,
            [
                (T![">"], true),
                (T![">"], true),
                (T!["="], false),
                (SyntaxKind::WHITESPACE, false),
                (T![">"], false),
                (SyntaxKind::WHITESPACE, false),
                (T![">"], false),
            ]
        );
        assert_eq!(one("#["), T!["#["]);
        assert_eq!(one("#!["), T!["#!["]);
    }

    #[test]
    fn errors_are_tokens() {
        for bad in [
            "#", "\"open", "/* open", "'", "\u{202E}", "\u{00A0}", "\"\\q\"",
        ] {
            let (tokens, errors) = lex(bad);
            assert!(!errors.is_empty(), "{bad:?}");
            assert_eq!(
                tokens.iter().map(|t| t.len as usize).sum::<usize>(),
                bad.len()
            );
        }
    }

    #[test]
    fn identifiers_must_be_nfc() {
        let (_, errors) = lex("cafe\u{301}");
        assert_eq!(errors.len(), 1);
        assert!(lex("caf\u{e9}").1.is_empty());
    }
}

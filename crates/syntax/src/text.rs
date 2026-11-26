//! Offsets and ranges in source text (UTF-8 bytes).

use std::fmt;
use std::ops::Range;

/// A half-open byte range in the source text.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct TextRange {
    /// The first byte.
    pub start: u32,
    /// One past the last byte.
    pub end: u32,
}

impl TextRange {
    /// The range from `start` to `end`.
    pub fn new(start: u32, end: u32) -> Self {
        debug_assert!(start <= end);
        TextRange { start, end }
    }

    /// The range's length in bytes.
    pub fn len(self) -> u32 {
        self.end - self.start
    }

    /// Whether the range is empty.
    pub fn is_empty(self) -> bool {
        self.start == self.end
    }

    /// As a `Range<usize>`, for slicing the text.
    pub fn to_range(self) -> Range<usize> {
        self.start as usize..self.end as usize
    }
}

impl fmt::Debug for TextRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}..{}", self.start, self.end)
    }
}

/// A syntax error: where, and what was wrong.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyntaxError {
    /// Where the error is (empty when something is missing).
    pub range: TextRange,
    /// The message: `expected ';'`.
    pub message: String,
}

impl SyntaxError {
    pub(crate) fn new(range: TextRange, message: impl Into<String>) -> Self {
        SyntaxError {
            range,
            message: message.into(),
        }
    }
}

impl fmt::Display for SyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.range, self.message)
    }
}

/// The 1-based line and column (in characters) of byte `offset` in `text`.
pub fn line_col(text: &str, offset: u32) -> (u32, u32) {
    let before = &text[..(offset as usize).min(text.len())];
    let line = before.matches('\n').count() + 1;
    let col = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    (line as u32, col as u32)
}

/// A tagged string token taken apart (`comptime.tag.1`): `tag"…"`, `tag"""…"""`, or a raw one,
/// `tagr#"…"#`, whose tag is `tag` and whose text is raw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tagged<'a> {
    /// The tag: `sql` of `sql"…"` and of `sqlr#"…"#`.
    pub tag: &'a str,
    /// The literal as the text of a raw string after its `r`: `"…"`, `"""…"""` or `#"…"#`.
    pub lit: &'a str,
    /// Where `lit` starts in the token.
    pub lit_at: usize,
    /// Whether the literal is raw (`sqlr#"…"#`).
    pub raw: bool,
}

impl<'a> Tagged<'a> {
    /// Takes apart the text of a `TAGGED_STRING` token.
    pub fn of(token: &'a str) -> Tagged<'a> {
        let quote = token.find('"').unwrap_or(token.len());
        let head = &token[..quote];
        let hashes = head.len() - head.trim_end_matches('#').len();
        if hashes > 0 && head[..quote - hashes].ends_with('r') {
            let tag_len = quote - hashes - 1;
            return Tagged {
                tag: &token[..tag_len],
                lit: &token[tag_len + 1..],
                lit_at: tag_len + 1,
                raw: true,
            };
        }
        Tagged {
            tag: head,
            lit: &token[quote..],
            lit_at: quote,
            raw: false,
        }
    }

    /// The literal's text between its quotes, and where that starts in the token: for a block,
    /// what is between its `"""`s, lines and indentation as written.
    pub fn body(&self) -> (&'a str, usize) {
        let hashes = self.lit.len() - self.lit.trim_start_matches('#').len();
        let quotes = if self.lit[hashes..].starts_with("\"\"\"") {
            3
        } else {
            1
        };
        let open = hashes + quotes;
        let close = self.lit.len().saturating_sub(open).max(open);
        (&self.lit[open..close], self.lit_at + open)
    }
}

#[cfg(test)]
mod tagged_tests {
    use super::Tagged;

    #[test]
    fn a_tagged_literal_comes_apart_into_its_tag_and_text() {
        let t = Tagged::of("sql\"a {x}\"");
        assert_eq!((t.tag, t.lit, t.raw), ("sql", "\"a {x}\"", false));
        assert_eq!(t.body(), ("a {x}", 4));
        let r = Tagged::of("sqlr#\"say \"hi\"\"#");
        assert_eq!((r.tag, r.lit, r.raw), ("sql", "#\"say \"hi\"\"#", true));
        assert_eq!(r.body(), ("say \"hi\"", 6));
        let f = Tagged::of("fmtr\"x\"");
        assert_eq!((f.tag, f.raw), ("fmtr", false));
        let b = Tagged::of("re\"\"\"\n  a\n  \"\"\"");
        assert_eq!(b.body(), ("\n  a\n  ", 5));
    }
}

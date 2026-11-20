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

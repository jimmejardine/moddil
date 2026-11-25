//! The output, line by line. The printer hands it tokens and trivia in source order, and says
//! where the structure breaks lines and indents; the output decides the rest: spaces within a
//! line ([`crate::space`]), the line a comment ends, and the blank lines kept.

use moddil_syntax::{SyntaxKind as K, SyntaxToken};

use crate::space;

/// A break waiting for the next token.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Break {
    None,
    /// A new line, without a blank line before it (after a comment, or inside a list).
    Soft,
    /// A new line where the structure has one: a blank line before it is kept.
    Hard,
}

/// One line of output: its code, and the comment that ends it.
struct Line {
    indent: usize,
    code: String,
    comment: Option<String>,
    /// Where the line's `=>` goes, padded to line up with its neighbours': the match it is an
    /// arm of, and the byte offset in `code` before the space ahead of it.
    arrow: Option<(u32, usize)>,
}

pub(crate) struct Out {
    lines: Vec<Line>,
    cur: Option<Line>,
    indent: usize,
    pending: Break,
    /// The line breaks in the source since the last token or comment.
    newlines: usize,
    /// Whether the source had whitespace since the last token or comment.
    ws: bool,
    /// The last token printed.
    prev: Option<SyntaxToken>,
    /// A block comment was printed on a line of its own: the next token goes on a new line only
    /// if the source has one.
    after_own_comment: bool,
    /// A block comment was printed within a line: a space follows it.
    after_inline_comment: bool,
    /// The next token is a match arm's `=>`, of the match whose arms start at this offset.
    arrow: Option<u32>,
    /// Indentation that starts after the line the next token opens.
    deferred: usize,
}

impl Out {
    pub(crate) fn new() -> Out {
        Out {
            lines: Vec::new(),
            cur: None,
            indent: 0,
            // The file starts as if after a structural break, so the blank line after a
            // leading comment is kept.
            pending: Break::Hard,
            newlines: 0,
            ws: false,
            prev: None,
            after_own_comment: false,
            after_inline_comment: false,
            arrow: None,
            deferred: 0,
        }
    }

    pub(crate) fn indent(&mut self) {
        self.indent += 1;
    }

    /// Indents the lines after the one the next token is on: a method chain's links, whose
    /// head may start a new line itself.
    pub(crate) fn indent_after(&mut self) {
        if self.cur.is_none() || self.pending != Break::None {
            self.deferred += 1;
        } else {
            self.indent += 1;
        }
    }

    pub(crate) fn dedent(&mut self) {
        self.indent -= 1;
    }

    /// A line break where the structure has one.
    pub(crate) fn line(&mut self) {
        self.pending = Break::Hard;
    }

    /// A line break inside a list broken by a comment.
    pub(crate) fn soft_line(&mut self) {
        self.pending = self.pending.max(Break::Soft);
    }

    /// Whitespace or a comment from the source.
    pub(crate) fn trivia(&mut self, t: &SyntaxToken) {
        match t.kind() {
            K::WHITESPACE => {
                self.newlines += t.text().matches('\n').count();
                self.ws = true;
            }
            K::LINE_COMMENT => self.line_comment(t.text().trim_end()),
            _ => self.block_comment(t.text()),
        }
    }

    /// Whether the open line has code and no comment yet: a comment that follows on the same
    /// source line ends it.
    fn trails(&self) -> bool {
        self.newlines == 0
            && self
                .cur
                .as_ref()
                .is_some_and(|l| !l.code.is_empty() && l.comment.is_none())
    }

    fn line_comment(&mut self, text: &str) {
        if self.trails() {
            if let Some(l) = &mut self.cur {
                l.comment = Some(text.to_string());
            }
            self.soft_line();
        } else {
            let after = self.pending.max(Break::Soft);
            self.open(false);
            self.push(text);
            self.pending = after;
        }
        self.reset();
    }

    fn block_comment(&mut self, text: &str) {
        if self.trails() {
            self.push(" ");
            self.push(text);
            self.after_inline_comment = true;
        } else {
            let after = self.pending;
            if self.cur.is_some() && self.pending == Break::None {
                self.pending = Break::Soft;
            }
            self.open(false);
            self.push(text);
            self.pending = after;
            self.after_own_comment = true;
        }
        self.newlines = 0;
        self.ws = false;
    }

    fn reset(&mut self) {
        self.newlines = 0;
        self.ws = false;
        self.after_own_comment = false;
        self.after_inline_comment = false;
    }

    /// Starts a new line if a break is waiting (or none is open yet). A blank line goes before
    /// it if the break is structural, the source has one there, and the line is neither the
    /// first in a block nor its closer.
    fn open(&mut self, closer: bool) {
        if self.cur.is_some() && self.pending == Break::None {
            return;
        }
        let opener = self
            .cur
            .as_ref()
            .is_some_and(|l| l.comment.is_none() && l.code.ends_with(['{', '(', '[', '<', '|']));
        let blank = self.pending == Break::Hard
            && self.newlines >= 2
            && self.cur.is_some()
            && !opener
            && !closer;
        if let Some(l) = self.cur.take() {
            self.lines.push(l);
        }
        if blank {
            self.lines.push(Line {
                indent: 0,
                code: String::new(),
                comment: None,
                arrow: None,
            });
        }
        self.cur = Some(Line {
            indent: self.indent,
            code: String::new(),
            comment: None,
            arrow: None,
        });
        self.indent += self.deferred;
        self.deferred = 0;
        self.pending = Break::None;
        self.prev = None;
    }

    fn push(&mut self, s: &str) {
        if let Some(l) = &mut self.cur {
            l.code.push_str(s);
        }
    }

    /// A token of the source.
    pub(crate) fn token(&mut self, t: &SyntaxToken) {
        let k = t.kind();
        // A comma or semicolon stays at the end of the line it follows, even across a break the
        // structure asked for after the node before it.
        let sticks = matches!(k, K::COMMA | K::SEMI)
            && self
                .cur
                .as_ref()
                .is_some_and(|l| !l.code.is_empty() && l.comment.is_none())
            && self.pending != Break::None
            && !self.after_own_comment;
        // A doc comment is on lines of its own.
        let doc = matches!(k, K::DOC_OUTER | K::DOC_INNER);
        if (self.after_own_comment && self.newlines > 0) || doc {
            self.pending = self.pending.max(Break::Soft);
        }
        if !sticks {
            self.open(matches!(k, K::R_CURLY | K::R_PAREN | K::R_BRACK));
        }
        if let Some(group) = self.arrow.take()
            && let Some(l) = &mut self.cur
            && !l.code.contains('\n')
        {
            l.arrow = Some((group, l.code.len()));
        }
        let space = match &self.prev {
            Some(p) if !sticks => self.after_inline_comment || space::between(p, t, self.ws),
            Some(_) => false,
            None => self.after_inline_comment,
        };
        if space {
            self.push(" ");
        }
        self.push(t.text());
        self.prev = Some(t.clone());
        self.reset();
        if doc {
            self.soft_line();
        }
    }

    /// The next token is a match arm's `=>`, lined up with those of the arms of the same match
    /// (`group`) on the lines around it.
    pub(crate) fn arrow(&mut self, group: u32) {
        self.arrow = Some(group);
    }

    /// Pads the `=>` of match arms on consecutive lines, of the same match and indentation, to
    /// one column.
    fn align_arrows(&mut self) {
        let mut i = 0;
        while i < self.lines.len() {
            let Some((group, _)) = self.lines[i].arrow else {
                i += 1;
                continue;
            };
            let mut j = i;
            while j < self.lines.len() && self.lines[j].arrow.is_some_and(|(g, _)| g == group) {
                j += 1;
            }
            let width = |l: &Line| l.arrow.map_or(0, |(_, at)| l.code[..at].chars().count());
            let col = self.lines[i..j].iter().map(width).max().unwrap_or(0);
            for l in &mut self.lines[i..j] {
                let pad = col - width(l);
                if let Some((_, at)) = l.arrow {
                    l.code.insert_str(at, &" ".repeat(pad));
                }
            }
            i = j;
        }
    }

    /// A comma the layout adds after a list's last element.
    pub(crate) fn comma(&mut self) {
        self.push(",");
    }

    /// The text: each line indented, trailing comments of consecutive lines aligned, one `\n`
    /// at the end.
    pub(crate) fn finish(mut self) -> String {
        if let Some(l) = self.cur.take() {
            self.lines.push(l);
        }
        self.align_arrows();
        let width = |l: &Line| {
            let last = l.code.rsplit('\n').next().unwrap_or("");
            if l.code.contains('\n') {
                last.chars().count()
            } else {
                l.indent * 4 + last.chars().count()
            }
        };
        let mut column = vec![0; self.lines.len()];
        let mut i = 0;
        while i < self.lines.len() {
            if self.lines[i].comment.is_none() {
                i += 1;
                continue;
            }
            let indent = self.lines[i].indent;
            let mut j = i;
            while j < self.lines.len()
                && self.lines[j].comment.is_some()
                && self.lines[j].indent == indent
            {
                j += 1;
            }
            let col = self.lines[i..j].iter().map(width).max().unwrap_or(0) + 2;
            for c in &mut column[i..j] {
                *c = col;
            }
            i = j;
        }
        let mut s = String::new();
        for (l, col) in self.lines.iter().zip(column) {
            let mut line = String::new();
            if !l.code.is_empty() || l.comment.is_some() {
                line.push_str(&"    ".repeat(l.indent));
            }
            line.push_str(&l.code);
            if let Some(c) = &l.comment {
                let pad = col.saturating_sub(width(l)).max(1);
                line.push_str(&" ".repeat(pad));
                line.push_str(c);
            }
            s.push_str(line.trim_end());
            s.push('\n');
        }
        let trimmed = s.trim_end_matches('\n').trim_start_matches('\n');
        if trimmed.is_empty() {
            return String::new();
        }
        format!("{trimmed}\n")
    }
}

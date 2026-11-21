//! Where collecting reports a problem, by the source it is in.
//!
//! Only problems in the file reach the user. Those in derived code are moved to the `derive`
//! attribute they come from, and the rest (the prelude's, or derived code with no attribute in
//! the file) are kept hidden, for tests.

use moddil_syntax::TextRange;

use crate::Diagnostic;
use crate::derive::Expansion;
use crate::items::Src;

pub(super) struct Reporter<'a> {
    expansion: &'a Expansion,
    shown: Vec<Diagnostic>,
    hidden: Vec<Diagnostic>,
}

impl<'a> Reporter<'a> {
    pub(super) fn new(expansion: &'a Expansion) -> Self {
        Reporter {
            expansion,
            shown: Vec::new(),
            hidden: Vec::new(),
        }
    }

    /// Reports `ds`, found in source `src` (at `origin`, for a derived item).
    pub(super) fn report(&mut self, src: Src, origin: Option<TextRange>, ds: Vec<Diagnostic>) {
        for mut d in ds {
            match src {
                Src::File => self.shown.push(d),
                Src::Derived => match origin {
                    Some(o) => {
                        d.range = o;
                        d.labels.clear();
                        self.shown.push(d);
                    }
                    None => self.hidden.push(d),
                },
                Src::Prelude => self.hidden.push(d),
            }
        }
    }

    /// Reports `d`, found in the file.
    pub(super) fn push(&mut self, d: Diagnostic) {
        self.shown.push(d);
    }

    /// For an item in the derived source: the `derive` attribute in the file it comes from.
    pub(super) fn origin(&self, src: Src, range: TextRange) -> Option<TextRange> {
        if src != Src::Derived {
            return None;
        }
        self.expansion
            .origins
            .iter()
            .find(|(r, _, _)| r.start <= range.start && range.end <= r.end)
            .and_then(|&(_, s, attr)| (s == Src::File).then_some(attr))
    }

    /// The problems shown, and those hidden.
    pub(super) fn finish(self) -> (Vec<Diagnostic>, Vec<Diagnostic>) {
        (self.shown, self.hidden)
    }
}

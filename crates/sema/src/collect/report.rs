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

    /// Reports `ds`, found in source `src` (at `origin`, for a derived item: the `derive`
    /// attribute, in the file the derived code comes from).
    pub(super) fn report(&mut self, src: Src, origin: Option<TextRange>, ds: Vec<Diagnostic>) {
        for mut d in ds {
            match src {
                Src::File(f) => self.shown.push(d.in_file(f)),
                Src::Derived => match origin.and_then(|o| self.origin_file(o).map(|f| (o, f))) {
                    Some((o, f)) => {
                        d.range = o;
                        d.labels.clear();
                        self.shown.push(d.in_file(f));
                    }
                    None => self.hidden.push(d),
                },
                Src::Prelude => self.hidden.push(d),
            }
        }
    }

    /// Reports `d`, found in file `file`.
    pub(super) fn push_in(&mut self, file: u32, d: Diagnostic) {
        self.shown.push(d.in_file(file));
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
            .and_then(|&(_, s, attr)| s.is_file().then_some(attr))
    }

    /// For an item in the derived source: the source its `derive` attribute is in.
    pub(super) fn origin_src(&self, range: TextRange) -> Option<Src> {
        self.expansion
            .origins
            .iter()
            .find(|(r, _, _)| r.start <= range.start && range.end <= r.end)
            .map(|&(_, s, _)| s)
    }

    /// The file of the `derive` attribute at `attr`.
    fn origin_file(&self, attr: TextRange) -> Option<u32> {
        self.expansion
            .origins
            .iter()
            .find(|(_, _, a)| *a == attr)
            .and_then(|&(_, s, _)| s.file())
    }

    /// The problems shown, and those hidden.
    pub(super) fn finish(self) -> (Vec<Diagnostic>, Vec<Diagnostic>) {
        (self.shown, self.hidden)
    }
}

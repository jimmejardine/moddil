//! Where text is (`text.pos.1`): an interpreter holds a `&str` as its text, not as where it is,
//! so two texts with the same characters would be one. A position belongs to the text it was
//! found in, and slicing other text with it traps, so each text a string's bytes or a slice make
//! is kept as one `Arc` while it lives: the same bytes viewed again, or the same part of the same
//! text sliced again, give that `Arc`, and texts are the same text when their `Arc`s are, as
//! native code's are when their addresses and lengths are.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{Arc, Weak};

/// The texts made so far, by where they are: `K` is where a string's bytes are.
pub(crate) struct Texts<K> {
    /// A string's text by where its bytes are and how many.
    views: HashMap<(K, usize), Weak<str>>,
    /// A slice by the outermost text it is in (its address) and its byte range there.
    slices: HashMap<(usize, usize, usize), Weak<str>>,
    /// Each slice, by its address: itself, the outermost text it is in (kept while the slice
    /// is, so the same bytes viewed again are that text), and where it starts.
    within: HashMap<usize, (Weak<str>, Arc<str>, usize)>,
    /// How many entries there were when dead ones were last let go.
    kept: usize,
}

impl<K> Default for Texts<K> {
    fn default() -> Self {
        Texts {
            views: HashMap::new(),
            slices: HashMap::new(),
            within: HashMap::new(),
            kept: 0,
        }
    }
}

/// Where a text is: its address.
fn addr(t: &Arc<str>) -> usize {
    Arc::as_ptr(t).cast::<u8>() as usize
}

impl<K: Hash + Eq> Texts<K> {
    /// The text of `len` bytes at `at`, which are `text`: the text made of them before, if they
    /// still hold it.
    pub(crate) fn view(&mut self, at: K, text: String) -> Arc<str> {
        let key = (at, text.len());
        if let Some(t) = self.views.get(&key).and_then(Weak::upgrade)
            && *t == *text
        {
            return t;
        }
        let t: Arc<str> = text.into();
        self.views.insert(key, Arc::downgrade(&t));
        self.tidy();
        t
    }

    /// The part of `base` from byte `a` to byte `b`, if they are in it: `base` itself if it is
    /// all of it, else the slice of the outermost text there.
    pub(crate) fn slice(&mut self, base: &Arc<str>, a: usize, b: usize) -> Option<Arc<str>> {
        let part = base.get(a..b)?;
        if a == 0 && b == base.len() {
            return Some(base.clone());
        }
        let (root, off) = match self.within.get(&addr(base)) {
            Some((me, root, off)) if me.upgrade().is_some_and(|me| Arc::ptr_eq(&me, base)) => {
                (root.clone(), *off)
            }
            _ => (base.clone(), 0),
        };
        let key = (addr(&root), off + a, off + b);
        if let Some(t) = self.slices.get(&key).and_then(Weak::upgrade) {
            return Some(t);
        }
        let t: Arc<str> = part.into();
        self.slices.insert(key, Arc::downgrade(&t));
        self.within
            .insert(addr(&t), (Arc::downgrade(&t), root, off + a));
        self.tidy();
        Some(t)
    }

    /// Lets go of the entries of texts that are gone, once there are twice as many as before.
    fn tidy(&mut self) {
        let n = self.views.len() + self.slices.len();
        if n < 64 || n < 2 * self.kept {
            return;
        }
        self.views.retain(|_, t| t.strong_count() > 0);
        self.slices.retain(|_, t| t.strong_count() > 0);
        self.within.retain(|_, (t, _, _)| t.strong_count() > 0);
        self.kept = self.views.len() + self.slices.len();
    }
}

/// Whether two texts are the same text, not only the same characters.
pub(crate) fn same(a: &Arc<str>, b: &Arc<str>) -> bool {
    Arc::ptr_eq(a, b)
}

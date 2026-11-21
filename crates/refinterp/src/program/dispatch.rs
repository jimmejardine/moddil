//! Method dispatch: whether a kind of value implements a trait, and which method a call on it
//! runs: an impl's, a trait's default, a derived one, or the built-in copy.

use super::{Method, Program};
use crate::value::Head;

impl Program {
    /// Whether values of kind `head` implement trait `name`: by an impl, a derive, or (for
    /// `Clone` and `Copy`) built in.
    pub(crate) fn implements(&self, head: &Head, name: &str) -> bool {
        // A borrow of a `Buf` is a slice too.
        if *head == Head::Buf && self.implements(&Head::Array, name) {
            return true;
        }
        if self
            .impls
            .iter()
            .any(|i| i.head == *head && i.trait_.as_deref() == Some(name))
        {
            return true;
        }
        match head {
            Head::Adt(ty) => self.adts[ty].derives.iter().any(|d| d == name),
            Head::Ref | Head::Tuple | Head::Unit | Head::Array => matches!(name, "Clone" | "Copy"),
            _ => false,
        }
    }

    /// The methods called `name` that values of kind `head` have, in the order they are
    /// chosen (`expr.method.1`): an inherent impl's, then a trait's (from an impl, a derive, or
    /// the trait's default).
    pub(crate) fn methods(&self, head: &Head, name: &str) -> Vec<Method> {
        if *head == Head::Buf {
            let mut out = self.methods_of(head, name);
            out.extend(self.methods_of(&Head::Array, name));
            return out;
        }
        self.methods_of(head, name)
    }

    fn methods_of(&self, head: &Head, name: &str) -> Vec<Method> {
        let mut out = Vec::new();
        for (k, i) in self.impls.iter().enumerate() {
            if i.trait_.is_none()
                && i.head == *head
                && let Some(f) = i.methods.get(name)
            {
                out.push(Method::Code(f.clone(), Some(k)));
            }
        }
        for (k, i) in self.impls.iter().enumerate() {
            if i.trait_.is_some()
                && i.head == *head
                && let Some(f) = i.methods.get(name)
            {
                out.push(Method::Code(f.clone(), Some(k)));
            }
        }
        if let Head::Adt(ty) = head {
            for d in &self.adts[ty].derives {
                let provides = matches!(
                    (d.as_str(), name),
                    ("Clone", "clone")
                        | ("PartialEq", "eq")
                        | ("PartialOrd", "partial_cmp")
                        | ("Ord", "cmp")
                        | ("Hash", "hash")
                );
                if provides {
                    out.push(Method::Derived(d.clone()));
                }
            }
        }
        if name == "clone" && matches!(head, Head::Ref | Head::Tuple | Head::Unit | Head::Array) {
            out.push(Method::Copy);
        }
        for (tname, t) in &self.traits {
            if let Some(f) = t.methods.get(name)
                && f.node.body().is_some()
                && self.implements(head, tname)
                && !out.iter().any(|m| matches!(m, Method::Code(g, Some(k)) if g.node == f.node || self.impls[*k].trait_.as_deref() == Some(tname)))
            {
                out.push(Method::Code(f.clone(), None));
            }
        }
        out
    }

    /// The method called `name` of trait `tr` for values of kind `head`.
    pub(crate) fn trait_method(&self, tr: &str, head: &Head, name: &str) -> Option<Method> {
        if *head == Head::Buf {
            return self
                .trait_method_of(tr, head, name)
                .or_else(|| self.trait_method_of(tr, &Head::Array, name));
        }
        self.trait_method_of(tr, head, name)
    }

    fn trait_method_of(&self, tr: &str, head: &Head, name: &str) -> Option<Method> {
        for (k, i) in self.impls.iter().enumerate() {
            if i.trait_.as_deref() == Some(tr)
                && i.head == *head
                && let Some(f) = i.methods.get(name)
            {
                return Some(Method::Code(f.clone(), Some(k)));
            }
        }
        if let Head::Adt(ty) = head
            && self.adts[ty].derives.iter().any(|d| d == tr)
            && matches!(
                (tr, name),
                ("Clone", "clone")
                    | ("PartialEq", "eq")
                    | ("PartialOrd", "partial_cmp")
                    | ("Ord", "cmp")
                    | ("Hash", "hash")
            )
        {
            return Some(Method::Derived(tr.to_string()));
        }
        if tr == "Clone" && matches!(head, Head::Ref | Head::Tuple | Head::Unit) {
            return Some(Method::Copy);
        }
        let f = self.traits.get(tr)?.methods.get(name)?;
        f.node
            .body()
            .is_some()
            .then(|| Method::Code(f.clone(), None))
    }
}

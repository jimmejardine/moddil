//! Patterns: checked against the type of the value they match, and the names they bind.

use moddil_syntax::TextRange;

use crate::hir::{LocalId, PatId, PatKind};
use crate::{Code, Ty};

use super::finish::Deferred;
use super::{Checker, index, tuple_positions};

impl Checker<'_> {
    /// Whether `p` contains a pattern already reported as an error while lowering.
    pub(super) fn has_missing(&self, p: PatId) -> bool {
        match &self.body.pats[p].kind {
            PatKind::Missing => true,
            PatKind::Bind { sub, .. } => sub.is_some_and(|s| self.has_missing(s)),
            PatKind::Tuple { pats, .. } | PatKind::Or(pats) => {
                pats.iter().any(|&s| self.has_missing(s))
            }
            PatKind::Adt { fields, .. } => fields.iter().any(|&(_, s)| self.has_missing(s)),
            PatKind::Ref { pat, .. } => self.has_missing(*pat),
            _ => false,
        }
    }

    /// Checks that `p` can match a value of type `expected`, and gives its bindings their types.
    /// `mode` is `Some(mutable)` while matching through a borrow: names then bind borrows of the
    /// parts they match (`syntax.md`: binding modes).
    pub(super) fn check_pat(&mut self, p: PatId, expected: &Ty, mode: Option<bool>) {
        let expected = self.norm(expected);
        self.out.pat_types[index(p)] = expected.clone();
        let tests_value = matches!(
            self.body.pats[p].kind,
            PatKind::Int(_)
                | PatKind::Char(_)
                | PatKind::Str(_)
                | PatKind::Bool(_)
                | PatKind::Range { .. }
                | PatKind::Unit
                | PatKind::Tuple { .. }
                | PatKind::Adt { .. }
        );
        // A string literal matches a `&str` itself.
        let str_lit = matches!(self.body.pats[p].kind, PatKind::Str(_))
            && matches!(&expected, Ty::Ref(_, t) if **t == Ty::Str);
        if tests_value
            && !str_lit
            && let Ty::Ref(m, inner) = &expected
        {
            // Matching through a borrow: test what it points to, and bind borrows.
            self.out.pat_derefs.insert(p);
            let mode = Some(mode.map_or(*m, |outer| outer && *m));
            let inner = self.norm(inner);
            self.check_pat_kind(p, &inner, mode);
        } else {
            self.check_pat_kind(p, &expected, mode);
        }
    }

    pub(super) fn check_pat_kind(&mut self, p: PatId, expected: &Ty, mode: Option<bool>) {
        let range = self.body.pats[p].range;
        let shown = self.show(expected);
        let wrong = |found: &str| -> String {
            format!("expected `{shown}`, but this pattern matches {found}")
        };
        match &self.body.pats[p].kind {
            PatKind::Missing | PatKind::Wild => {}
            &PatKind::OfType { pat, .. } => self.check_pat(pat, expected, mode),
            // `p: X` (`refine.pat.1`): a value of `X`'s base (or an `X`) tested by its predicate.
            &PatKind::Refined { refinement, pat } => {
                let def = &self.items.types[refinement as usize];
                let refined = Ty::Refined(refinement, def.name.as_str().into());
                let base = def.ty.clone();
                if !self.coerce(expected, &base) && !self.coerce(expected, &refined) {
                    let shown = self.show(expected);
                    let name = def.name.clone();
                    self.report(
                        Code::E0201,
                        range,
                        format!("`{name}` refines `{base}`, but the value matched is a `{shown}`"),
                    );
                    return;
                }
                // The predicate runs on a copy of the value.
                if !self.solver.is_copy(&base) && base != Ty::Error {
                    self.out.diagnostics.push(crate::Diagnostic::unsupported(
                        range,
                        "a refinement test of a value that is not `Copy`",
                    ));
                }
                self.check_pat(pat, &refined, mode);
            }
            &PatKind::Bind { local, sub } => {
                self.out.local_types[index(local)] = match mode {
                    Some(m) => {
                        self.out.by_ref.insert(p, m);
                        Ty::reference(m, expected.clone())
                    }
                    None => expected.clone(),
                };
                if let Some(s) = sub {
                    self.check_pat(s, expected, mode);
                }
            }
            &PatKind::Ref { mutable, pat } => {
                let inner = match expected {
                    Ty::Ref(m, inner) if *m == mutable && mode.is_none() => (**inner).clone(),
                    Ty::Error => Ty::Error,
                    _ => {
                        let form = if mutable {
                            "`&mut` pattern"
                        } else {
                            "`&` pattern"
                        };
                        self.report(Code::E0201, range, wrong(&format!("a borrow (a {form})")));
                        Ty::Error
                    }
                };
                self.check_pat(pat, &inner, None);
                // `&x` copies what the borrow points to, so only `Copy` values can be bound.
                let mut binds = Vec::new();
                self.value_bindings(pat, &mut binds);
                for (local, at) in binds {
                    self.deferred.push(Deferred::CopyOut(local, at));
                }
            }
            &PatKind::Int(v) => {
                if !self.coerce(&Ty::Int, expected) {
                    self.report(Code::E0201, range, wrong("an `int`"));
                } else if i64::try_from(v).is_err() {
                    self.report(
                        Code::E0204,
                        range,
                        format!("`{v}` does not fit in `int` (a signed 64-bit integer)"),
                    );
                }
            }
            PatKind::Bool(_) => {
                if !self.coerce(&Ty::Bool, expected) {
                    self.report(Code::E0201, range, wrong("a `bool`"));
                }
            }
            PatKind::Char(_) => {
                if !self.coerce(&Ty::Char, expected) {
                    self.report(Code::E0201, range, wrong("a `char`"));
                }
            }
            PatKind::Str(_) => {
                if !self.coerce(&Ty::reference(false, Ty::Str), expected) {
                    self.report(Code::E0201, range, wrong("a `&str`"));
                }
            }
            &PatKind::Range {
                lo,
                hi,
                inclusive,
                chars: true,
            } => {
                if !self.coerce(&Ty::Char, expected) {
                    self.report(Code::E0201, range, wrong("a `char`"));
                    return;
                }
                if let Some(hi) = hi
                    && (hi < lo || (hi == lo && !inclusive))
                {
                    self.report(
                        Code::E0214,
                        range,
                        "this range is empty, so the pattern matches nothing",
                    );
                }
            }
            &PatKind::Range {
                lo, hi, inclusive, ..
            } => {
                if !self.coerce(&Ty::Int, expected) {
                    self.report(Code::E0201, range, wrong("an `int`"));
                    return;
                }
                for v in [Some(lo), hi].into_iter().flatten() {
                    if i64::try_from(v).is_err() {
                        self.report(
                            Code::E0204,
                            range,
                            format!("`{v}` does not fit in `int` (a signed 64-bit integer)"),
                        );
                        return;
                    }
                }
                if let Some(hi) = hi
                    && (hi < lo || (hi == lo && !inclusive))
                {
                    self.report(
                        Code::E0214,
                        range,
                        "this range is empty, so the pattern matches nothing",
                    );
                }
            }
            PatKind::Unit => {
                if !self.coerce(&Ty::Unit, expected) {
                    self.report(Code::E0201, range, wrong("`()`"));
                }
            }
            PatKind::Tuple { pats, rest } => {
                let pats = pats.clone();
                let rest = *rest;
                let elements: Vec<Ty> = match expected {
                    Ty::Tuple(ts) => ts.to_vec(),
                    Ty::Error => vec![Ty::Error; pats.len()],
                    Ty::Infer(_) if rest.is_none() => {
                        let ts: Vec<Ty> = (0..pats.len()).map(|_| self.fresh()).collect();
                        self.solver.unify(expected, &Ty::tuple(ts.clone()));
                        ts
                    }
                    _ => {
                        self.report(Code::E0201, range, wrong("a tuple"));
                        vec![Ty::Error; pats.len()]
                    }
                };
                let fits = match rest {
                    Some(_) => pats.len() <= elements.len(),
                    None => pats.len() == elements.len(),
                };
                if !fits {
                    self.report(
                        Code::E0213,
                        range,
                        format!(
                            "`{shown}` has {} elements, but this pattern has {}",
                            elements.len(),
                            pats.len()
                        ),
                    );
                    for &s in &pats {
                        self.check_pat(s, &Ty::Error, None);
                    }
                    return;
                }
                let positions = tuple_positions(pats.len(), rest, elements.len());
                for (&s, i) in pats.iter().zip(positions) {
                    self.check_pat(s, &elements[i], mode);
                }
            }
            PatKind::Adt {
                adt,
                variant,
                fields,
            } => {
                let (adt, variant, fields) = (*adt, *variant, fields.clone());
                let def = &self.items.adts[adt as usize];
                let n = def.generics.params.len();
                let ty = match expected {
                    Ty::Adt(a, ..) if *a == adt => expected.clone(),
                    Ty::Error => def.ty_with(adt, vec![Ty::Error; n]),
                    Ty::Infer(_) => {
                        let args = self.solver.fresh_n(n);
                        let t = self.items.adts[adt as usize].ty_with(adt, args);
                        self.solver.unify(expected, &t);
                        t
                    }
                    _ => {
                        let path = def.path(variant);
                        self.report(Code::E0201, range, wrong(&format!("`{path}`")));
                        for &(_, s) in &fields {
                            self.check_pat(s, &Ty::Error, None);
                        }
                        return;
                    }
                };
                let types = self.items.field_types(&ty, variant);
                for (i, s) in fields {
                    let t = types[i as usize].clone();
                    self.check_pat(s, &t, mode);
                }
            }
            PatKind::Or(alts) => {
                for &a in &alts.clone() {
                    self.check_pat(a, expected, mode);
                }
            }
        }
    }

    /// The names `p` binds by value (not through a borrow), with where each is bound.
    pub(super) fn value_bindings(&self, p: PatId, out: &mut Vec<(LocalId, TextRange)>) {
        match &self.body.pats[p].kind {
            &PatKind::Bind { local, sub } => {
                if !self.out.by_ref.contains_key(&p) {
                    out.push((local, self.body.pats[p].range));
                }
                if let Some(s) = sub {
                    self.value_bindings(s, out);
                }
            }
            PatKind::Tuple { pats, .. } | PatKind::Or(pats) => {
                for &s in pats {
                    self.value_bindings(s, out);
                }
            }
            PatKind::Adt { fields, .. } => {
                for &(_, s) in fields {
                    self.value_bindings(s, out);
                }
            }
            _ => {}
        }
    }
}

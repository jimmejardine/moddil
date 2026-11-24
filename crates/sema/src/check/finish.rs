//! What checking a body leaves for its end, when its types are known: the trait bounds it
//! found ([`Obligations`]), the checks that need resolved types ([`DeferredChecks`]), and the
//! final pass that replaces each inference variable by what it was bound to.

use moddil_syntax::TextRange;

use super::{Inference, index};
use crate::hir::{Arm, Body, ExprId, LocalId, PatId};
use crate::items::ItemTree;
use crate::traits::{Answer, Solver};
use crate::ty::TraitRef;
use crate::{Code, Diagnostic, Ty, exhaust};

/// A bound a type must meet, found while checking.
pub(super) struct Obligation {
    pub(super) ty: Ty,
    pub(super) trait_ref: TraitRef,
    pub(super) range: TextRange,
}

/// The bounds found while checking, solved at the end.
#[derive(Default)]
pub(super) struct Obligations(Vec<Obligation>);

impl Obligations {
    pub(super) fn push(&mut self, o: Obligation) {
        self.0.push(o);
    }

    /// The trait arguments of the first bound on `ty` (a variable) of a trait `pick` picks: what
    /// a closure passed for a type parameter is expected to take.
    pub(super) fn closure_args(
        &self,
        solver: &mut Solver,
        ty: &Ty,
        pick: impl Fn(u32) -> bool,
    ) -> Option<Ty> {
        if !matches!(ty, Ty::Infer(_)) {
            return None;
        }
        self.0
            .iter()
            .find(|o| pick(o.trait_ref.trait_) && solver.norm(&o.ty) == *ty)
            .and_then(|o| o.trait_ref.args.first().cloned())
    }

    /// Solves what can be solved, until nothing changes: the errors for bounds that do not
    /// hold, and the bounds still ambiguous.
    pub(super) fn solve(self, solver: &mut Solver) -> (Vec<Diagnostic>, Vec<Obligation>) {
        let mut errors = Vec::new();
        let mut pending = self.0;
        loop {
            let before = pending.len();
            let mut still = Vec::new();
            for o in pending {
                let ty = solver.deep(&o.ty);
                let tr = o.trait_ref.map(&mut |t| solver.deep(t));
                match solver.implements(&ty, &tr) {
                    Answer::Yes(_) => {}
                    Answer::No => {
                        if !ty.has_error() {
                            errors.push(Diagnostic::error(
                                Code::E0402,
                                o.range,
                                format!("`{ty}` does not implement `{tr}`"),
                            ));
                        }
                    }
                    Answer::Maybe => still.push(o),
                }
            }
            pending = still;
            if pending.len() == before || pending.is_empty() {
                break;
            }
        }
        (errors, pending)
    }
}

/// A check that needs the body's types resolved first.
pub(super) enum Deferred {
    /// A `match`: its expression, the scrutinee's type and its arms.
    Match(ExprId, Ty, Vec<Arm>),
    /// A pattern that must match every value of its type.
    Irrefutable(PatId, &'static str),
    /// A name bound by copying out of a borrow (`&x`), which must be `Copy`.
    CopyOut(LocalId, TextRange),
}

/// The checks that need the body's types resolved, run at the end in the order found.
#[derive(Default)]
pub(super) struct DeferredChecks(Vec<Deferred>);

impl DeferredChecks {
    pub(super) fn push(&mut self, d: Deferred) {
        self.0.push(d);
    }

    /// Runs the checks against the resolved types in `out`.
    pub(super) fn run(
        self,
        items: &ItemTree,
        body: &Body,
        out: &Inference,
        solver: &mut Solver,
    ) -> Vec<Diagnostic> {
        let mut ds = Vec::new();
        for d in self.0 {
            match d {
                Deferred::Match(e, st, arms) => {
                    let st = erase(&solver.deep(&st));
                    if st.has_error() {
                        continue;
                    }
                    ds.extend(exhaust::check_match(
                        items,
                        body,
                        out,
                        body.exprs[e].range,
                        &st,
                        &arms,
                    ));
                }
                Deferred::Irrefutable(pat, what) => {
                    let t = out.pat(pat);
                    if t.has_error() {
                        continue;
                    }
                    if let Some(missing) = exhaust::refutable(items, body, out, pat) {
                        ds.push(Diagnostic::error(
                            Code::E0209,
                            body.pats[pat].range,
                            format!(
                                "{what} needs a pattern that always matches, but `{missing}` is \
                                 not matched"
                            ),
                        ));
                    }
                }
                Deferred::CopyOut(local, at) => {
                    let t = out.local(local);
                    if !solver.is_copy(&t) {
                        ds.push(Diagnostic::error(
                            Code::E0304,
                            at,
                            format!(
                                "`{t}` is not `Copy`, so it cannot be bound out of a borrow: \
                                 match the borrow without `&` to bind a borrow instead"
                            ),
                        ));
                    }
                }
            }
        }
        ds
    }
}

/// The first type that could not be inferred, at a binding if there is one, or else the first
/// bound whose impl could not be chosen.
pub(super) fn uninferred(
    body: &Body,
    out: &Inference,
    solver: &mut Solver,
    pending: &[Obligation],
) -> Option<Diagnostic> {
    for (l, data) in body.locals.iter() {
        let t = solver.deep(&out.local_types[index(l)]);
        if t.has_infer() {
            return Some(Diagnostic::error(
                Code::E0405,
                data.range,
                format!(
                    "the type of `{}` cannot be inferred (it is `{t}` so far): annotate it",
                    data.name
                ),
            ));
        }
    }
    for (e, _) in body.exprs.iter() {
        let t = solver.deep(&out.expr_types[index(e)]);
        if t.has_infer() {
            return Some(Diagnostic::error(
                Code::E0405,
                body.exprs[e].range,
                format!("the type of this cannot be inferred (it is `{t}` so far): annotate it"),
            ));
        }
    }
    let o = pending.first()?;
    let ty = solver.deep(&o.ty);
    Some(Diagnostic::error(
        Code::E0405,
        o.range,
        format!(
            "which impl of `{}` to use for `{ty}` cannot be inferred",
            o.trait_ref
        ),
    ))
}

impl Inference {
    /// Replaces every inference variable in the recorded types and callees by what it is bound
    /// to, and any left unbound by the error type.
    pub(super) fn resolve_all(&mut self, solver: &mut Solver) {
        let resolve = |solver: &mut Solver, t: &Ty| erase(&solver.deep(t));
        for t in &mut self.expr_types {
            *t = resolve(solver, t);
        }
        for t in &mut self.pat_types {
            *t = resolve(solver, t);
        }
        for t in &mut self.local_types {
            *t = resolve(solver, t);
        }
        for c in self.calls.values_mut() {
            *c = c.map(&mut |t| erase(&solver.deep(t)));
        }
        for c in self.compare_calls.values_mut() {
            *c = c.map(&mut |t| erase(&solver.deep(t)));
        }
        for c in self.fmt_calls.values_mut() {
            *c = c.map(&mut |t| erase(&solver.deep(t)));
        }
        for c in self
            .index_calls
            .values_mut()
            .chain(self.literal_calls.values_mut())
            .chain(self.method_derefs.values_mut())
        {
            *c = c.map(&mut |t| erase(&solver.deep(t)));
        }
        for (c, t) in self.field_through.values_mut() {
            *c = c.map(&mut |t| erase(&solver.deep(t)));
            *t = erase(&solver.deep(t));
        }
    }
}

/// `t` with its inference variables replaced by the error type.
pub(super) fn erase(t: &Ty) -> Ty {
    match t {
        Ty::Infer(_) => Ty::Error,
        t => t.map(&mut erase),
    }
}

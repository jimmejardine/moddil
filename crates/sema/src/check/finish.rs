//! What checking a body leaves for its end, when its types are known: the trait bounds it
//! found ([`Obligations`]), the checks that need resolved types ([`DeferredChecks`]), and the
//! final pass that replaces each inference variable by what it was bound to.

use moddil_syntax::TextRange;

use super::{Inference, index};
use crate::hir::{Arm, Body, ExprId, LocalId, PatId};
use crate::items::ItemTree;
use crate::traits::{Answer, Solver};
use crate::ty::TraitRef;
use crate::{Callee, Code, Diagnostic, Ty, exhaust};

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
    }
}

/// Borrows are second-class: a type argument (which a struct could store) cannot be one. `Self`
/// may be, for the built-in `Clone` of a shared borrow.
pub(super) fn borrowed_type_args(body: &Body, out: &Inference) -> Vec<Diagnostic> {
    let mut found = Vec::new();
    for (&e, c) in &out.calls {
        let skip = usize::from(matches!(c, Callee::Trait { .. }));
        if c.args().iter().skip(skip).any(Ty::has_ref_inside) {
            found.push(body.exprs[e].range);
        }
    }
    for (e, _) in body.exprs.iter() {
        if let Ty::Adt(_, _, args) = &out.expr_types[index(e)]
            && args.iter().any(Ty::has_ref_inside)
        {
            found.push(body.exprs[e].range);
        }
    }
    found.sort_by_key(|r| (r.start, r.end));
    found.dedup();
    found
        .into_iter()
        .map(|r| {
            Diagnostic::error(
                Code::E0309,
                r,
                "a type argument cannot be a borrow: borrows live only in parameters and locals",
            )
        })
        .collect()
}

/// `t` with its inference variables replaced by the error type.
fn erase(t: &Ty) -> Ty {
    match t {
        Ty::Infer(_) => Ty::Error,
        t => t.map(&mut erase),
    }
}

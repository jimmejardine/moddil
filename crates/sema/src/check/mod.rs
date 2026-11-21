//! Type checking a body: the `infer` query. Types are inferred by unification, generic calls
//! are given type arguments, methods and operators are resolved to the functions they call, and
//! trait bounds are checked. It also checks that patterns are exhaustive where they must be
//! ([`crate::exhaust`]).
//!
//! [`Checker`] infers types as it walks the body, by topic: [`exprs`], [`blocks`], [`patterns`],
//! [`operators`] and [`calls`]. What needs the body's types resolved is recorded on the way and
//! run by [`finish`]: the trait bounds found, the deferred checks (exhaustiveness,
//! irrefutability, copying out of a borrow), and the pass that resolves every recorded type.

use std::collections::{HashMap, HashSet};

use la_arena::RawIdx;
use moddil_syntax::TextRange;

use crate::collect::item_tree;
use crate::hir::{Body, ExprId, ExprKind, LocalId, PatId};
use crate::lower::body;
use crate::traits::{Adjust, Solver};
use crate::ty::TraitRef;
use crate::{Code, Db, Diagnostic, Item, ItemTree, Owner, Ty};

mod blocks;
mod calls;
mod closures;
mod errors;
mod exprs;
mod finish;
mod needs;
mod operators;
mod patterns;

pub use closures::{Capture, CaptureMode};
use errors::FailCx;
pub use errors::Propagated;
use finish::{Deferred, DeferredChecks, Obligation, Obligations};
pub(crate) use needs::show_effect;
pub use needs::{Effects, Evidence, FnEffects, effects};
use needs::{EvidenceMap, Mode, Need, inferred};

/// The function a call, method call or operator runs.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Callee {
    /// A free function or a function of an inherent impl, with the type arguments for all its
    /// type parameters (the impl's, then its own).
    Fn {
        /// The function.
        func: u32,
        /// Its type arguments.
        args: Vec<Ty>,
    },
    /// A trait's method, run for the impl of the trait for `Self` (the first type argument):
    /// the method's declaration in the trait, with the type arguments for all its type
    /// parameters (`Self`, the trait's, then its own).
    Trait {
        /// The trait's method.
        func: u32,
        /// Its type arguments.
        args: Vec<Ty>,
    },
}

impl Callee {
    /// The function's declaration: the function, or the trait's method.
    pub fn func(&self) -> u32 {
        match self {
            Callee::Fn { func, .. } | Callee::Trait { func, .. } => *func,
        }
    }

    /// The type arguments.
    pub fn args(&self) -> &[Ty] {
        match self {
            Callee::Fn { args, .. } | Callee::Trait { args, .. } => args,
        }
    }

    fn map(&self, f: &mut impl FnMut(&Ty) -> Ty) -> Callee {
        match self {
            Callee::Fn { func, args } => Callee::Fn {
                func: *func,
                args: args.iter().map(&mut *f).collect(),
            },
            Callee::Trait { func, args } => Callee::Trait {
                func: *func,
                args: args.iter().map(&mut *f).collect(),
            },
        }
    }
}

/// The types of a body's expressions, patterns and locals, the functions its calls and
/// operators run, and the type errors found.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Inference {
    expr_types: Vec<Ty>,
    pat_types: Vec<Ty>,
    local_types: Vec<Ty>,
    field_indices: HashMap<ExprId, u32>,
    field_derefs: HashSet<ExprId>,
    pat_derefs: HashSet<PatId>,
    by_ref: HashMap<PatId, bool>,
    calls: HashMap<ExprId, Callee>,
    compare_calls: HashMap<(ExprId, usize), Callee>,
    fmt_calls: HashMap<ExprId, Callee>,
    adjusts: HashMap<ExprId, Adjust>,
    propagated: HashMap<ExprId, Propagated>,
    try_errors: HashMap<ExprId, Ty>,
    evidence: EvidenceMap,
    dyn_coercions: HashMap<ExprId, Ty>,
    /// For a function whose effects are inferred: the effects its body needs that nothing in it
    /// meets.
    escaping_needs: Vec<Ty>,
    /// For a function whose errors are inferred: the errors that leave its body.
    escaping_errors: Vec<Ty>,
    captures: HashMap<ExprId, Vec<Capture>>,
    passes_on: HashSet<ExprId>,
    /// Type errors, and exhaustiveness errors and warnings.
    pub diagnostics: Vec<Diagnostic>,
}

impl Inference {
    /// The type of expression `e`.
    pub fn expr(&self, e: ExprId) -> Ty {
        self.expr_types.get(index(e)).cloned().unwrap_or(Ty::Error)
    }

    /// The type of the value pattern `p` matches.
    pub fn pat(&self, p: PatId) -> Ty {
        self.pat_types.get(index(p)).cloned().unwrap_or(Ty::Error)
    }

    /// The type of local `l`.
    pub fn local(&self, l: LocalId) -> Ty {
        self.local_types.get(index(l)).cloned().unwrap_or(Ty::Error)
    }

    /// For a field expression `base.name`: the field's index in its struct or tuple.
    pub fn field_index(&self, e: ExprId) -> Option<u32> {
        self.field_indices.get(&e).copied()
    }

    /// Whether field expression `e` reads its field through a borrow (`r.x` for `r: &Point`).
    pub fn field_through_borrow(&self, e: ExprId) -> bool {
        self.field_derefs.contains(&e)
    }

    /// Whether pattern `p` matches through a borrow: the value is a borrow, and `p` tests what
    /// it points to.
    pub fn pat_through_borrow(&self, p: PatId) -> bool {
        self.pat_derefs.contains(&p)
    }

    /// For a binding pattern that binds a borrow of the matched place (matching through a
    /// borrow): whether the borrow is `&mut`.
    pub fn binds_borrow(&self, p: PatId) -> Option<bool> {
        self.by_ref.get(&p).copied()
    }

    /// What a call, method call, or operator on a type other than `int` and `bool` runs.
    pub fn callee(&self, e: ExprId) -> Option<&Callee> {
        self.calls.get(&e)
    }

    /// What comparison `i` of a comparison chain runs, if it is not on `int`, `bool` or `()`.
    pub fn compare_callee(&self, e: ExprId, i: usize) -> Option<&Callee> {
        self.compare_calls.get(&(e, i))
    }

    /// For a hole of `fmt"…"`: the `Display::fmt` it calls.
    pub fn fmt_callee(&self, hole: ExprId) -> Option<&Callee> {
        self.fmt_calls.get(&hole)
    }

    /// For `e?` and `e ?! "…"`: what `e` is (a fallible call, a `Result` or an `Option`).
    pub fn propagated(&self, e: ExprId) -> Option<Propagated> {
        self.propagated.get(&e).copied()
    }

    /// For `try { … } catch { … }`: the errors its block can fail with, as an error set.
    pub fn try_errors(&self, e: ExprId) -> Option<Ty> {
        self.try_errors.get(&e).cloned()
    }

    /// For a call of a function that needs effects: where each of its needs (in the order of
    /// its [`FnEffects::needs`]) is met.
    pub fn evidence(&self, e: ExprId) -> &[Evidence] {
        self.evidence.get(&e).map_or(&[], Vec::as_slice)
    }

    /// For an expression whose borrow is used as a `dyn` borrow (`&clock` as `&dyn Time`): the
    /// `dyn` borrow type.
    pub fn dyn_coercion(&self, e: ExprId) -> Option<&Ty> {
        self.dyn_coercions.get(&e)
    }

    /// Whether call `e`, of a closure parameter and not marked, passes its errors on as `?`
    /// would.
    pub fn passes_on(&self, e: ExprId) -> bool {
        self.passes_on.contains(&e)
    }

    /// For a closure expression: what it captures, in the order of its type's captures.
    pub fn captures(&self, e: ExprId) -> &[Capture] {
        self.captures.get(&e).map_or(&[], Vec::as_slice)
    }

    /// How a method call's receiver is adjusted to the method's `self`.
    pub fn adjust(&self, e: ExprId) -> Adjust {
        self.adjusts.get(&e).copied().unwrap_or(Adjust {
            deref: false,
            autoref: None,
        })
    }
}

pub(crate) fn index<T>(id: la_arena::Idx<T>) -> usize {
    let raw: RawIdx = id.into_raw();
    raw.into_u32() as usize
}

/// Type checks the body of `owner`.
#[salsa::tracked(returns(ref))]
pub fn infer<'db>(db: &'db dyn Db, owner: Owner<'db>) -> Inference {
    check_body(db, owner, effects(db, owner.program(db)))
}

/// Type checks the body of `owner`, with the effects and errors of the functions `fx` says.
pub(crate) fn check_body<'db>(db: &'db dyn Db, owner: Owner<'db>, fx: &Effects) -> Inference {
    let lowered = body(db, owner);
    let items = item_tree(db, owner.program(db));
    let body = &lowered.body;
    let (ret, ret_range, is_test, preds) = match owner.item(db) {
        Item::Fn(i) => {
            let sig = &items.fns[i as usize];
            (
                sig.ret.clone(),
                sig.ret_range,
                false,
                sig.generics.preds.as_slice(),
            )
        }
        Item::Test(_) | Item::Closure(..) => (Ty::Unit, None, true, &[][..]),
    };
    // Where the body's errors go (`err.set.1`, `err.entry.1`).
    let fail_cx = match owner.item(db) {
        Item::Test(_) | Item::Closure(..) => FailCx::Test,
        Item::Fn(i) if items.trait_of(i) == items.lang("Drop") && items.lang("Drop").is_some() => {
            FailCx::Never("a destructor")
        }
        Item::Fn(i) => FailCx::Fn {
            // With its closure parameters' errors (`expr.closure.fails.1`).
            allowed: fx.fns[i as usize].fails.members(),
            option: matches!(&ret, Ty::Adt(a, ..) if Some(*a) == items.lang("Option")),
            infer: inferred(items, i),
        },
    };
    // How its needs are met (`eff.need.1`).
    let (mode, own) = match owner.item(db) {
        Item::Test(_) | Item::Closure(..) => (Mode::Entry("a test"), Vec::new()),
        Item::Fn(i) => {
            let mode = if inferred(items, i) {
                Mode::Inferred
            } else if items.is_main(i) {
                Mode::Entry("`main`")
            } else if items.trait_of(i).is_some() {
                Mode::Method
            } else {
                Mode::Written
            };
            (mode, fx.fns[i as usize].needs.clone())
        }
    };
    let mut cx = Checker {
        body,
        items,
        solver: Solver::new(items, preds),
        ret: ret.clone(),
        ret_range,
        loops: Vec::new(),
        obligations: Obligations::default(),
        deferred: DeferredChecks::default(),
        fail_cx: vec![fail_cx],
        marked: None,
        call_fails: None,
        fx,
        mode,
        own,
        installs: Vec::new(),
        needs: Vec::new(),
        owner: owner.item(db),
        owner_args: match owner.item(db) {
            Item::Fn(i) => items.fns[i as usize].generics.identity(),
            Item::Test(_) | Item::Closure(..) => Vec::new(),
        },
        closures: Vec::new(),
        closure_calls: Vec::new(),
        kind_reqs: Vec::new(),
        assoc_eqs: Vec::new(),
        hint: None,
        sig_hint: None,
        anon_fails: Vec::new(),
        out: Inference {
            expr_types: vec![Ty::Error; body.exprs.len()],
            pat_types: vec![Ty::Error; body.pats.len()],
            local_types: vec![Ty::Error; body.locals.len()],
            ..Inference::default()
        },
    };
    if let Item::Fn(i) = owner.item(db) {
        for (&local, p) in body.params.iter().zip(&items.fns[i as usize].params) {
            cx.out.local_types[index(local)] = p.ty.clone();
        }
        for &(local, pat) in &body.param_pats {
            let t = cx.out.local_types[index(local)].clone();
            cx.check_pat(pat, &t, None);
            cx.deferred.push(Deferred::Irrefutable(pat, "a parameter"));
        }
        cx.effect_locals(i);
    }
    if let Some(root) = body.root {
        let t = cx.infer(root);
        // A test's final value is observed, not returned, so it may have any type.
        if !is_test && !cx.unsize(root, &t, &ret) && !cx.coerce(&t, &ret) {
            let at = match &body.exprs[root].kind {
                ExprKind::Block {
                    tail: Some(tail), ..
                } => body.exprs[*tail].range,
                _ => body.exprs[root].range,
            };
            let t = cx.show(&t);
            let mut d = Diagnostic::error(
                Code::E0201,
                at,
                format!("expected `{ret}` because of the return type, found `{t}`"),
            );
            if let Some(r) = ret_range {
                d = d.with_label(r, "the return type");
            }
            cx.out.diagnostics.push(d);
        }
    }
    cx.finish();
    cx.out
}

/// An enclosing loop while its body is checked.
struct LoopCx {
    /// The `loop` or `while` expression.
    id: ExprId,
    /// Whether it is a `while`, which cannot `break` with a value.
    is_while: bool,
    /// The type of the values `break` gives it so far.
    ty: Option<Ty>,
}

struct Checker<'a> {
    body: &'a Body,
    items: &'a ItemTree,
    solver: Solver<'a>,
    ret: Ty,
    ret_range: Option<TextRange>,
    loops: Vec<LoopCx>,
    obligations: Obligations,
    deferred: DeferredChecks,
    /// Where errors raised here go, innermost last.
    fail_cx: Vec<FailCx>,
    /// The call a `?` or `?!` is around, while it is checked.
    marked: Option<ExprId>,
    /// The errors of the marked call, once it is found to be one that can fail.
    call_fails: Option<Ty>,
    /// Every function's effects and errors.
    fx: &'a Effects,
    /// How the body's needs are met.
    mode: Mode,
    /// The function's own effects, in order.
    own: Vec<Ty>,
    /// The `with` expressions around what is being checked, innermost last, with the effects
    /// each installs.
    installs: Vec<(ExprId, Vec<Ty>)>,
    /// The needs of the calls checked so far.
    needs: Vec<Need>,
    /// The function or test being checked.
    owner: Item,
    /// Its type parameters, as types.
    owner_args: Vec<Ty>,
    /// The closures checked so far, innermost first.
    closures: Vec<ExprId>,
    /// The calls of closures: the call, its callee, the callee's type and its arguments' tuple.
    closure_calls: Vec<(ExprId, ExprId, Ty, Ty)>,
    /// The closure traits bounds need (`F: Fn(…)`), checked once closures' kinds are known.
    kind_reqs: Vec<(Ty, u32, TextRange)>,
    /// Bounds' fixed associated types (`Output = R`) still to settle: the associated type, the
    /// type it must be, and where.
    assoc_eqs: Vec<(Ty, Ty, TextRange)>,
    /// For a closure about to be checked: the type expected of it.
    hint: Option<closures::Hint>,
    /// For an anonymous impl's method about to be checked: its parameters' and result types.
    sig_hint: Option<(Vec<Ty>, Option<Ty>)>,
    /// Anonymous impls' methods, and the errors their trait's methods allow.
    anon_fails: Vec<(ExprId, Ty)>,
    out: Inference,
}

impl Checker<'_> {
    fn range(&self, e: ExprId) -> TextRange {
        self.body.exprs[e].range
    }

    fn report(&mut self, code: Code, range: TextRange, message: impl Into<String>) {
        self.out
            .diagnostics
            .push(Diagnostic::error(code, range, message));
    }

    /// `t` as far as it is known, for messages.
    fn show(&mut self, t: &Ty) -> Ty {
        self.solver.deep(t)
    }

    fn norm(&mut self, t: &Ty) -> Ty {
        self.solver.norm(t)
    }

    fn fresh(&mut self) -> Ty {
        self.solver.table.fresh()
    }

    /// `t` with each hole replaced by a new variable.
    fn instantiate(&mut self, t: &Ty) -> Ty {
        match t {
            Ty::Infer(Ty::HOLE) => self.fresh(),
            t => t.map(&mut |x| self.instantiate(x)),
        }
    }

    fn mismatch(&mut self, e: ExprId, expected: &Ty, found: &Ty) {
        let (expected, found) = (self.show(expected), self.show(found));
        self.report(
            Code::E0201,
            self.range(e),
            format!("expected `{expected}`, found `{found}`"),
        );
    }

    /// Whether a value of type `found` can be used where `expected` is wanted, making them agree
    /// if it can: `!` goes anywhere, and `&mut T` where `&T` is wanted.
    fn coerce(&mut self, found: &Ty, expected: &Ty) -> bool {
        let f = self.norm(found);
        let e = self.norm(expected);
        match (&f, &e) {
            (Ty::Never | Ty::Error, _) | (_, Ty::Error) => true,
            (Ty::Ref(true, a), Ty::Ref(false, b)) => self.solver.unify(a, b),
            _ => self.solver.unify(&f, &e),
        }
    }

    /// The type of an expression whose branches have types `a` and `b`, if they agree.
    fn join(&mut self, a: &Ty, b: &Ty) -> Option<Ty> {
        let (x, y) = (self.norm(a), self.norm(b));
        match (&x, &y) {
            (Ty::Error, _) | (_, Ty::Error) => Some(Ty::Error),
            (Ty::Never, t) | (t, Ty::Never) => Some(t.clone()),
            _ if self.solver.unify(&x, &y) => Some(x),
            _ => None,
        }
    }

    fn check(&mut self, e: ExprId, expected: &Ty) -> Ty {
        if matches!(self.body.exprs[e].kind, ExprKind::Closure { .. }) {
            self.hint = Some(closures::Hint(expected.clone()));
        }
        let t = self.infer(e);
        if !self.unsize(e, &t, expected) && !self.coerce(&t, expected) {
            self.mismatch(e, expected, &t);
        }
        t
    }

    fn infer(&mut self, e: ExprId) -> Ty {
        let t = self.infer_inner(e);
        self.out.expr_types[index(e)] = t.clone();
        t
    }

    /// Reports E0405 for an expression whose type must be known here but is not.
    fn unknown(&mut self, range: TextRange, what: &str) -> Ty {
        self.report(
            Code::E0405,
            range,
            format!("the type of this must be known {what}: annotate it"),
        );
        Ty::Error
    }

    fn require(&mut self, ty: Ty, trait_ref: TraitRef, range: TextRange) {
        if matches!(
            self.closure_trait(trait_ref.trait_),
            Some(crate::ty::ClosureKind::Fn | crate::ty::ClosureKind::FnMut)
        ) {
            self.kind_reqs.push((ty.clone(), trait_ref.trait_, range));
        }
        self.obligations.push(Obligation {
            ty,
            trait_ref,
            range,
        });
    }

    /// Checks the bounds recorded, resolves every type (reporting those that could not be
    /// inferred), and runs the checks that need resolved types.
    fn finish(&mut self) {
        self.settle_assoc_eqs();
        let (errors, pending) = std::mem::take(&mut self.obligations).solve(&mut self.solver);
        self.out.diagnostics.extend(errors);
        if let Some(d) = finish::uninferred(self.body, &self.out, &mut self.solver, &pending) {
            self.out.diagnostics.push(d);
        }
        self.out.resolve_all(&mut self.solver);
        self.meet_needs();
        self.finish_closures();
        let ds = finish::borrowed_type_args(self.items, self.body, &self.out);
        self.out.diagnostics.extend(ds);
        let deferred = std::mem::take(&mut self.deferred);
        let ds = deferred.run(self.items, self.body, &self.out, &mut self.solver);
        self.out.diagnostics.extend(ds);
    }
}

/// The element positions the patterns of a tuple pattern match: `n` patterns, with `..` before
/// pattern `rest` if any, against a tuple of `arity` elements.
pub fn tuple_positions(n: usize, rest: Option<usize>, arity: usize) -> Vec<usize> {
    match rest {
        None => (0..n).collect(),
        Some(at) => (0..n)
            .map(|i| if i < at { i } else { arity - (n - i) })
            .collect(),
    }
}

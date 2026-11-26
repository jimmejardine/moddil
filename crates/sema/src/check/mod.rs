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
pub(crate) mod comptime;
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

/// What a call passes for one of its function's parameters, when its arguments are not simply
/// the parameters in order (`expr.call.named.1`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArgSlot {
    /// The argument written at this position among the call's arguments (the receiver not
    /// counted).
    Arg(usize),
    /// The parameter's default: a call of this hidden const.
    Default(u32),
}

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
    /// For an operand of an operator that is read through borrows (`borrow.deref.2`): how many.
    operand_derefs: HashMap<ExprId, u32>,
    pat_derefs: HashSet<PatId>,
    by_ref: HashMap<PatId, bool>,
    calls: HashMap<ExprId, Callee>,
    compare_calls: HashMap<(ExprId, usize), Callee>,
    fmt_calls: HashMap<ExprId, Callee>,
    index_calls: HashMap<ExprId, Callee>,
    /// For a method call through `Deref` (`expr.method.deref.1`): the `deref` or `deref_mut`
    /// that gives its receiver.
    method_derefs: HashMap<ExprId, Callee>,
    /// For a borrow used where a borrow of what it dereferences to is expected
    /// (`type.deref.coerce.1`): the `deref` or `deref_mut` that converts it, and the borrow
    /// type it gives.
    deref_coercions: HashMap<ExprId, (Callee, Ty)>,
    /// For a field reached through `Deref` (`expr.field.deref.1`): the `deref` or `deref_mut`
    /// that gives the struct, and the struct's type.
    field_through: HashMap<ExprId, (Callee, Ty)>,
    literal_calls: HashMap<ExprId, Callee>,
    refined_literals: Vec<(ExprId, u32)>,
    /// The integer literals taken as a `Wrapping<T>` (`num.wrap.3`), each with that type: the
    /// literal is a `T`, wrapped where it is used.
    wrapped: HashMap<ExprId, Ty>,
    adjusts: HashMap<ExprId, Adjust>,
    propagated: HashMap<ExprId, Propagated>,
    arg_order: HashMap<ExprId, Vec<ArgSlot>>,
    try_errors: HashMap<ExprId, Ty>,
    evidence: EvidenceMap,
    dyn_coercions: HashMap<ExprId, Ty>,
    /// The functions named where a C function pointer is wanted (`type.fnptr.1`), by the
    /// expression naming them.
    fn_ptrs: HashMap<ExprId, u32>,
    /// For a function whose effects are inferred: the effects its body needs that nothing in it
    /// meets.
    escaping_needs: Vec<Ty>,
    /// For a function whose errors are inferred: the errors that leave its body.
    escaping_errors: Vec<Ty>,
    /// For a function whose effects are inferred: the control effects its body needs.
    escaping_control: crate::Control,
    captures: HashMap<ExprId, Vec<Capture>>,
    passes_on: HashSet<ExprId>,
    /// For a function whose result type is `impl Trait`: the type its body gives
    /// (`type.impl.ret.1`).
    hidden: Option<Ty>,
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

    /// How many borrows operand `e` of an operator is read through (`borrow.deref.2`): `r` in
    /// `r * 2.0` for `r: &float` is read as `*r`. Zero for an operand that is not.
    pub fn operand_derefs(&self, e: ExprId) -> u32 {
        self.operand_derefs.get(&e).copied().unwrap_or(0)
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

    /// For `base[i]` on a type other than an array or slice: the `Index::index` or
    /// `IndexMut::index_mut` it calls, which gives a borrow of the element.
    pub fn index_callee(&self, e: ExprId) -> Option<&Callee> {
        self.index_calls.get(&e)
    }

    /// For an array literal whose context expects another type: the
    /// `FromArrayLiteral::from_array` (or `from_repeat`) that makes it.
    pub fn literal_callee(&self, e: ExprId) -> Option<&Callee> {
        self.literal_calls.get(&e)
    }

    /// The literals taken as refined values (`let q: Port = 8080;`), each with its refinement:
    /// compiling checks the predicate holds for them (`refine.lit.1`).
    pub fn refined_literals(&self) -> &[(ExprId, u32)] {
        &self.refined_literals
    }

    /// The `Wrapping<T>` integer literal `e` is taken as, if it is one (`num.wrap.3`): `e`
    /// itself is a `T`.
    pub fn wrapped(&self, e: ExprId) -> Option<&Ty> {
        self.wrapped.get(&e)
    }

    /// For a hole of `fmt"…"`: the `Display::fmt` it calls.
    pub fn fmt_callee(&self, hole: ExprId) -> Option<&Callee> {
        self.fmt_calls.get(&hole)
    }

    /// For a call with named or left-out arguments: what it passes for each of its function's
    /// parameters (the receiver not counted), in order.
    pub fn arg_order(&self, e: ExprId) -> Option<&[ArgSlot]> {
        self.arg_order.get(&e).map(Vec::as_slice)
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

    /// For a function named where a C function pointer is wanted (`type.fnptr.1`): the
    /// function, whose address the expression's value is.
    pub fn fn_ptr(&self, e: ExprId) -> Option<u32> {
        self.fn_ptrs.get(&e).copied()
    }

    /// For a method call whose receiver is reached through `Deref` (`expr.method.deref.1`):
    /// the `deref` (or `deref_mut`) that gives it.
    pub fn method_deref(&self, e: ExprId) -> Option<&Callee> {
        self.method_derefs.get(&e)
    }

    /// For an expression, a borrow, converted to a borrow of what it dereferences to
    /// (`type.deref.coerce.1`): the `deref` (or `deref_mut`) that converts it, and the borrow
    /// type it gives.
    pub fn deref_coercion(&self, e: ExprId) -> Option<&(Callee, Ty)> {
        self.deref_coercions.get(&e)
    }

    /// For a field expression whose struct is reached through `Deref` (`expr.field.deref.1`):
    /// the `deref` (or `deref_mut`) that gives it, and its type.
    pub fn field_deref(&self, e: ExprId) -> Option<&(Callee, Ty)> {
        self.field_through.get(&e)
    }

    /// Whether call `e`, of a closure parameter and not marked, passes its errors on as `?`
    /// would.
    pub fn passes_on(&self, e: ExprId) -> bool {
        self.passes_on.contains(&e)
    }

    /// For a function whose result type is `impl Trait`: the type its body gives.
    pub fn hidden(&self) -> Option<&Ty> {
        self.hidden.as_ref()
    }

    /// The inference with `f` applied to every type it records, as the MIR is built with the
    /// opaque types revealed (`type.impl.ret.1`).
    #[must_use]
    pub fn map_types(&self, f: &mut impl FnMut(&Ty) -> Ty) -> Inference {
        let mut out = self.clone();
        for t in out
            .expr_types
            .iter_mut()
            .chain(out.pat_types.iter_mut())
            .chain(out.local_types.iter_mut())
            .chain(out.try_errors.values_mut())
            .chain(out.dyn_coercions.values_mut())
            .chain(out.field_through.values_mut().map(|(_, t)| t))
            .chain(out.deref_coercions.values_mut().map(|(_, t)| t))
            .chain(out.wrapped.values_mut())
        {
            *t = f(t);
        }
        for c in out
            .calls
            .values_mut()
            .chain(out.compare_calls.values_mut())
            .chain(out.fmt_calls.values_mut())
            .chain(out.index_calls.values_mut())
            .chain(out.method_derefs.values_mut())
            .chain(out.deref_coercions.values_mut().map(|(c, _)| c))
            .chain(out.field_through.values_mut().map(|(c, _)| c))
            .chain(out.literal_calls.values_mut())
        {
            *c = c.map(f);
        }
        out
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
/// The value of `bool` expression `e` of `body`, if it is known when checking: a literal, `!`,
/// `&&` and `||` of known values, or a use of a const item in `consts`.
pub(crate) fn known_bool(body: &Body, consts: &HashMap<u32, bool>, e: ExprId) -> Option<bool> {
    use crate::hir::{LogicOp, UnaryOp};
    match &body.exprs[e].kind {
        ExprKind::Bool(b) => Some(*b),
        ExprKind::Unary(UnaryOp::Not, x) => known_bool(body, consts, *x).map(|b| !b),
        ExprKind::Logic(op, a, b) => {
            let (a, b) = (known_bool(body, consts, *a), known_bool(body, consts, *b));
            // `&&` is false with a false side and true with both; `||` the other way round.
            let (stop, op_value) = match op {
                LogicOp::And => (false, false),
                LogicOp::Or => (true, true),
            };
            if a == Some(stop) || b == Some(stop) {
                Some(op_value)
            } else if a.is_some() && b.is_some() {
                Some(!op_value)
            } else {
                None
            }
        }
        ExprKind::Block {
            stmts,
            tail: Some(t),
            ..
        } if stmts.is_empty() => known_bool(body, consts, *t),
        ExprKind::Call { callee, args, .. } if args.is_empty() => match &body.exprs[*callee].kind {
            ExprKind::Fn { func, .. } => consts.get(func).copied(),
            _ => None,
        },
        _ => None,
    }
}

/// The `bool` const items of `program` whose values [`known_bool`] knows, each from its
/// initialiser (and the consts it names), by passes until nothing more is learned.
fn known_consts(db: &dyn Db, program: crate::Program, items: &ItemTree) -> HashMap<u32, bool> {
    let consts: Vec<u32> = (0..items.fns.len() as u32)
        .filter(|&f| items.fns[f as usize].const_item && items.fns[f as usize].ret == Ty::Bool)
        .collect();
    let mut known = HashMap::new();
    loop {
        let before = known.len();
        for &f in &consts {
            if known.contains_key(&f) {
                continue;
            }
            let lowered = body(db, Owner::new(db, program, Item::Fn(f)));
            if let Some(v) = lowered
                .body
                .root
                .and_then(|r| known_bool(&lowered.body, &known, r))
            {
                known.insert(f, v);
            }
        }
        if known.len() == before {
            return known;
        }
    }
}

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
            taken: items.fns[i as usize]
                .generics
                .preds
                .iter()
                .filter_map(|q| match &q.ty {
                    Ty::Assoc(a) if a.name.as_ref() == "Fails" => Some(a.self_ty.clone()),
                    _ => None,
                })
                .collect(),
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
    let known_bools = known_consts(db, owner.program(db), items);
    let mut cx = Checker {
        known_bools,
        body,
        items,
        solver: Solver::new(items, preds),
        ret: ret.clone(),
        ret_range,
        loops: Vec::new(),
        obligations: Obligations::default(),
        deferred: DeferredChecks::default(),
        late: HashSet::new(),
        fail_cx: vec![fail_cx],
        marked: None,
        call_fails: None,
        fx,
        mode,
        own,
        installs: Vec::new(),
        needs: Vec::new(),
        control_uses: Vec::new(),
        own_control: match owner.item(db) {
            Item::Fn(i) => fx.fns[i as usize].control,
            Item::Test(_) | Item::Closure(..) => 0,
        },
        owner: owner.item(db),
        owner_args: match owner.item(db) {
            Item::Fn(i) => items.fns[i as usize].generics.identity(),
            Item::Test(_) | Item::Closure(..) => Vec::new(),
        },
        closures: Vec::new(),
        closure_calls: Vec::new(),
        stored: Vec::new(),
        boxed_hint: None,
        stored_ctx: Vec::new(),
        bound_control: Vec::new(),
        opaque_hint: None,
        spawns: Vec::new(),
        result_closures: HashSet::new(),
        kind_reqs: Vec::new(),
        assoc_eqs: Vec::new(),
        hint: None,
        refined_results: HashMap::new(),
        sig_hint: None,
        anon_fails: Vec::new(),
        literals: Vec::new(),
        out: Inference {
            expr_types: vec![Ty::Error; body.exprs.len()],
            pat_types: vec![Ty::Error; body.pats.len()],
            local_types: vec![Ty::Error; body.locals.len()],
            ..Inference::default()
        },
    };
    // A result type `impl Trait` (`type.impl.ret.1`): the body gives a type of its own choice
    // that meets the bounds.
    let mut ret = ret;
    let hidden = match (owner.item(db), &ret) {
        (Item::Fn(i), Ty::Opaque(o)) if o.func == i => {
            let h = cx.fresh();
            let range = ret_range.unwrap_or_default();
            for p in items.fns[i as usize].opaque.clone() {
                for (k, fixed) in &p.assoc {
                    let name = items.traits[p.trait_ref.trait_ as usize].assoc[*k as usize]
                        .0
                        .clone();
                    let a = Ty::Assoc(std::sync::Arc::new(crate::ty::AssocTy {
                        self_ty: h.clone(),
                        trait_ref: p.trait_ref.clone(),
                        index: *k,
                        name: name.as_str().into(),
                    }));
                    cx.assoc_eqs.push((a, fixed.clone(), range));
                }
                cx.require(h.clone(), p.trait_ref.clone(), range);
            }
            if let Some(d) = items.fns[i as usize].opaque_fn.clone() {
                cx.opaque_hint = Some((h.clone(), d));
            }
            cx.ret = h.clone();
            ret = h.clone();
            Some(h)
        }
        _ => None,
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
    // Contracts (`contract.1`): each a `bool`; `old(e)` is `e`'s value, of a `Copy` type, and
    // `result` the value returned.
    for &e in &body.requires {
        cx.check(e, &Ty::Bool);
    }
    for &(local, e) in &body.olds {
        let t = cx.infer(e);
        cx.out.local_types[index(local)] = t;
    }
    if let Some(r) = body.result {
        cx.out.local_types[index(r)] = ret.clone();
    }
    for &e in &body.ensures {
        cx.check(e, &Ty::Bool);
    }
    if let Some(root) = body.root {
        // A literal the function's refined result may be is taken as one (`refine.lit.1`).
        if !is_test && matches!(ret, Ty::Refined(..)) {
            cx.result_literals(root, &ret);
            for (_, x) in body.exprs.iter() {
                if let ExprKind::Return(Some(v)) = x.kind {
                    cx.result_literals(v, &ret);
                }
            }
        }
        // The closures it gives as an `impl Fn…` result (`expr.closure.result.1`).
        if cx.opaque_hint.is_some() {
            let mut found = Vec::new();
            result_closure(body, root, &mut found);
            for (_, x) in body.exprs.iter() {
                if let ExprKind::Return(Some(v)) = x.kind {
                    result_closure(body, v, &mut found);
                }
            }
            cx.result_closures.extend(found);
        }
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
    // A selectable function's parts (`task.select.4`): the receiver, and the rest, which takes
    // what was received, an `Option` of what the receiver carries.
    if let Some(parts) = body.select {
        let chan = cx.infer(parts.chan);
        let received = match &chan {
            Ty::Closure(c) => match cx.norm(&c.ret) {
                Ty::Ref(_, r) => match cx.norm(&r) {
                    Ty::Adt(a, _, args)
                        if items.adts[a as usize].src == crate::items::Src::Prelude
                            && items.adts[a as usize].name == "Receiver" =>
                    {
                        items
                            .lang("Option")
                            .map(|o| items.adts[o as usize].ty_with(o, args.to_vec()))
                    }
                    _ => None,
                },
                _ => None,
            },
            _ => None,
        };
        match received {
            Some(t) => {
                let mut params: Vec<Ty> = match owner.item(db) {
                    Item::Fn(i) => items.fns[i as usize]
                        .params
                        .iter()
                        .map(|p| p.ty.clone())
                        .collect(),
                    _ => Vec::new(),
                };
                params.push(t);
                cx.sig_hint = Some((params, Some(ret.clone())));
                cx.infer(parts.done);
            }
            None => cx.report(
                Code::E0217,
                body.exprs[parts.chan].range,
                "a `#[selectable]` function receives first from a channel: \
                 `self.incoming.recv()`, `incoming` a `Receiver`",
            ),
        }
    }
    cx.finish();
    // A kernel's body keeps to the kernel effect set: no handlers, errors or suspension
    // (`gpu.kernel.1`).
    if let Item::Fn(i) = owner.item(db)
        && items.fns[i as usize].kernel
        && (!cx.out.escaping_needs.is_empty()
            || !cx.out.escaping_errors.is_empty()
            || cx.out.escaping_control != 0)
    {
        cx.out.diagnostics.push(Diagnostic::error(
            Code::E0221,
            items.fns[i as usize].name_range,
            "a `#[kernel]`'s body needs no handlers, cannot fail, and does not suspend or spawn \
             (the kernel effect set)",
        ));
    }
    // A selectable function's parts run in its caller's frame, so they capture nothing: not
    // the function's handlers either (`task.select.4`).
    if let Some(parts) = body.select {
        for e in [parts.chan, parts.done] {
            if let Ty::Closure(c) = &cx.out.expr_types[index(e)]
                && !c.captures.is_empty()
            {
                cx.out.diagnostics.push(Diagnostic::error(
                    Code::E0217,
                    body.exprs[e].range,
                    "what a `#[selectable]` function does with what it received runs where it \
                     is selected, so it uses no handler: it may only fail",
                ));
                break;
            }
        }
    }
    if let Some(h) = hidden {
        let t = finish::erase(&cx.solver.deep(&h));
        // A closure it gives, as closure analysis completed it (with what it captures). What a
        // returned closure borrows is the borrow checker's to report (E0310), so the check
        // below looks at the type as inferred.
        let done = completed_closures(&t, &cx.out.expr_types, cx.owner);
        let at = ret_range.unwrap_or_default();
        if (t.has_infer() || t == Ty::Error) && cx.out.diagnostics.is_empty() {
            cx.out.diagnostics.push(Diagnostic::error(
                Code::E0405,
                at,
                "the type this function gives cannot be inferred: annotate it",
            ));
        }
        // A hidden type that holds a borrow (a closure or function value that captured the
        // function's handlers, `type.impl.ret.2`) is the borrow checker's to check.
        cx.out.hidden = Some(done);
    }
    cx.out
}

/// `t` with each closure of `owner`'s body as the body's types record it: completed, with what
/// it captures (the type a result was inferred with predates that).
fn completed_closures(t: &Ty, expr_types: &[Ty], owner: Item) -> Ty {
    match t {
        Ty::Closure(c) if c.owner == owner => match expr_types.get(c.expr as usize) {
            Some(done @ Ty::Closure(_)) => done.clone(),
            _ => t.clone(),
        },
        t => t.map(&mut |x| completed_closures(x, expr_types, owner)),
    }
}

/// The closure `e` gives as its value, through blocks, if it does.
fn result_closure(body: &Body, e: ExprId, out: &mut Vec<ExprId>) {
    match &body.exprs[e].kind {
        ExprKind::Closure { .. } => out.push(e),
        ExprKind::Block { tail: Some(t), .. } => result_closure(body, *t, out),
        _ => {}
    }
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
    /// The `bool` const items whose values are known when checking ([`known_bool`]).
    known_bools: HashMap<u32, bool>,
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
    /// Each call's control effects, with where it is.
    control_uses: Vec<(crate::Control, TextRange)>,
    /// The function's own control effects.
    own_control: crate::Control,
    /// The function or test being checked.
    owner: Item,
    /// Its type parameters, as types.
    owner_args: Vec<Ty>,
    /// The closures checked so far, innermost first.
    closures: Vec<ExprId>,
    /// The calls of closures: the call, its callee, the callee's type and its arguments' tuple.
    closure_calls: Vec<(ExprId, ExprId, Ty, Ty)>,
    /// The closures kept in a `Box<dyn Fn…>`, with that `dyn` type and where: they own what
    /// they capture (`expr.closure.stored.1`).
    stored: Vec<(Ty, Ty, TextRange)>,
    /// A closure passed to a call whose result is expected to be a `Box<dyn Fn…>`, and that
    /// `dyn` type, which gives the closure's parameters their types.
    boxed_hint: Option<(ExprId, Ty)>,
    /// The closures being checked whose control effects are met by their type or bound,
    /// innermost last: stored closures (`expr.closure.stored.1`) and closures passed for a bound
    /// that lists control effects (`expr.closure.control.1`). Each closure, the control effects
    /// listed, and how many handlers are installed around it.
    stored_ctx: Vec<(ExprId, crate::Control, usize)>,
    /// The types a call's closure bounds bound, with the control effects each lists
    /// (`expr.closure.control.1`): a closure checked as one of them has its control met there.
    bound_control: Vec<(Ty, crate::Control)>,
    /// For a function whose result is `impl Fn…`: the type its body gives, and the closure
    /// type as a stored one's, which a closure written for it takes (`expr.closure.result.1`).
    opaque_hint: Option<(Ty, Ty)>,
    /// Each `s.spawn(f)` of a scope: the call, the scope, and the task's closure, checked once
    /// the closures' captures are known (`task.with.1`).
    spawns: Vec<(ExprId, ExprId, ExprId)>,
    /// The closures the body gives as its value or returns, when that is an `impl Fn…`.
    result_closures: HashSet<ExprId>,
    /// The closure traits bounds need (`F: Fn(…)`), checked once closures' kinds are known.
    kind_reqs: Vec<(Ty, u32, TextRange)>,
    /// Bounds' fixed associated types (`Output = R`) still to settle: the associated type, the
    /// type it must be, and where.
    assoc_eqs: Vec<(Ty, Ty, TextRange)>,
    /// For a closure about to be checked: the type expected of it.
    hint: Option<closures::Hint>,
    /// The literals that may be a function's refined result, with the refined type.
    refined_results: HashMap<ExprId, Ty>,
    /// For an anonymous impl's method about to be checked: its parameters' and result types.
    sig_hint: Option<(Vec<Ty>, Option<Ty>)>,
    /// Anonymous impls' methods, and the errors their trait's methods allow.
    anon_fails: Vec<(ExprId, Ty)>,
    /// The integer literals, with their values, to check against the types they get
    /// (`num.lit.1`).
    literals: Vec<(ExprId, moddil_syntax::ast::IntValue)>,
    /// The locals declared without a value (`let x;`), which `=` gives one even if they are not
    /// `mut`: the borrow checker sees that it does so once (`stmt.let.uninit.1`).
    late: HashSet<LocalId>,
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
    /// if it can: `!` goes anywhere, `&mut T` where `&T` is wanted, and a borrow where a raw
    /// pointer is.
    fn coerce(&mut self, found: &Ty, expected: &Ty) -> bool {
        let f = self.norm(found);
        let e = self.norm(expected);
        match (&f, &e) {
            (Ty::Never | Ty::Error, _) | (_, Ty::Error) => true,
            (Ty::Ref(true, a), Ty::Ref(false, b)) | (Ty::Ptr(true, a), Ty::Ptr(false, b)) => {
                self.solver.unify(a, b)
            }
            // A borrow where a raw pointer is wanted (`type.ptr.1`): `&T` as `*const T`, and
            // `&mut T` as either.
            (Ty::Ref(m, a), Ty::Ptr(n, b)) if *m || !*n => self.solver.unify(a, b),
            // A refined value where its base is wanted (`refine.base.1`), and a borrow of one
            // where a borrow of its base is.
            (Ty::Refined(..), _) if !self.solver.can_unify(&f, &e) => {
                let base = self.items.base(&f);
                self.solver.unify(&base, &e)
            }
            (Ty::Ref(m, a), Ty::Ref(n, b))
                if (m == n || *m)
                    && matches!(self.norm(a), Ty::Refined(..))
                    && !self.solver.can_unify(a, b) =>
            {
                let base = self.items.base(&self.norm(a));
                self.solver.unify(&base, b)
            }
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
            // A closure boxed where a `Box<dyn Fn…>` is expected takes its types from that
            // (`expr.closure.stored.1`).
            let hint = match self.boxed_hint.take() {
                Some((c, d)) if c == e => d,
                other => {
                    self.boxed_hint = other;
                    expected.clone()
                }
            };
            self.hint = Some(closures::Hint(hint));
        }
        if let ExprKind::Call { args, .. } = &self.body.exprs[e].kind
            && let [arg] = args.as_slice()
            && matches!(self.body.exprs[*arg].kind, ExprKind::Closure { .. })
            && let Ty::Adt(b, _, xs) = self.norm(expected)
            && Some(b) == self.items.lang("Box")
            && let d @ Ty::Dyn(..) = self.norm(&xs[0])
        {
            self.boxed_hint = Some((*arg, d));
        }
        if let Some(t) = self.array_literal(e, expected) {
            return t;
        }
        if let Some(t) = self.refined_literal(e, expected) {
            return t;
        }
        if let Some(t) = self.wrapped_literal(e, expected) {
            return t;
        }
        let t = self.infer(e);
        self.unsized_value(e);
        if self.fn_pointer(e, expected) {
            return t;
        }
        if !self.unsize(e, &t, expected) && !self.coerce(&t, expected) {
            self.mismatch(e, expected, &t);
        }
        t
    }

    /// Whether `e` names a function where a C function pointer of type `expected` is wanted
    /// (`type.fnptr.1`), recording it if so. The function is free and not generic, its
    /// signature is the pointer's, and it needs no handlers, cannot fail and does not suspend
    /// or spawn (E0222 otherwise).
    fn fn_pointer(&mut self, e: ExprId, expected: &Ty) -> bool {
        let Ty::FnPtr(ps, r) = self.norm(expected) else {
            return false;
        };
        let ExprKind::Closure { body, .. } = self.body.exprs[e].kind else {
            return false;
        };
        let ExprKind::Pass(call) = self.body.exprs[body].kind else {
            return false;
        };
        let ExprKind::Call { callee, .. } = self.body.exprs[call].kind else {
            return false;
        };
        let ExprKind::Fn { func, .. } = self.body.exprs[callee].kind else {
            return false;
        };
        if self.body.exprs[callee].range != self.body.exprs[e].range {
            return false;
        }
        let sig = &self.items.fns[func as usize];
        let fits = sig.generics.params.is_empty()
            && sig.self_kind.is_none()
            && sig.params.len() == ps.len()
            && sig.params.iter().zip(ps.iter()).all(|(p, t)| p.ty == *t)
            && sig.ret == *r;
        let plain = sig.effects.is_empty() && sig.control == 0 && sig.fails == Ty::Never;
        if !fits || !plain {
            let name = sig.name.clone();
            let why = if fits {
                "needs no handlers, cannot fail, and does not suspend or spawn"
            } else {
                "is a free function, not generic, with the pointer's parameters and result"
            };
            let (shown, range) = (self.show(expected), self.range(e));
            self.report(
                Code::E0222,
                range,
                format!(
                    "a function given as a C function pointer `{shown}` {why}: `{name}` is not"
                ),
            );
            return true;
        }
        // A C function named as a value was checked as a closure that calls it, which needs
        // `unsafe` (E0218); as a pointer it is its address, and nothing is called here.
        if self.items.fns[func as usize].foreign {
            let at = self.range(e);
            self.out
                .diagnostics
                .retain(|d| !(d.code == Code::E0218 && d.range == at));
        }
        self.out.fn_ptrs.insert(e, func);
        self.out.dyn_coercions.insert(e, Ty::FnPtr(ps, r));
        true
    }

    /// E0605 for a part of an array or text used as a value, not behind a borrow: it has no
    /// size of its own (`expr.slice.1`).
    pub(super) fn unsized_value(&mut self, e: ExprId) {
        if matches!(self.body.exprs[e].kind, ExprKind::Slice { .. }) {
            self.report(
                Code::E0605,
                self.range(e),
                "a part of an array or of text has no size of its own, so it is used behind a \
                 borrow: `&xs[a..b]`",
            );
        }
    }

    /// Marks the literals that `e`'s value may be (through blocks, `if` and `match`) as of
    /// refined type `ty`.
    fn result_literals(&mut self, e: ExprId, ty: &Ty) {
        match &self.body.exprs[e].kind {
            ExprKind::Block { tail: Some(t), .. } => self.result_literals(*t, ty),
            ExprKind::If { then, else_, .. } => {
                let (then, else_) = (*then, *else_);
                self.result_literals(then, ty);
                if let Some(x) = else_ {
                    self.result_literals(x, ty);
                }
            }
            ExprKind::Match { arms, .. } => {
                let bodies: Vec<ExprId> = arms.iter().map(|a| a.body).collect();
                for b in bodies {
                    self.result_literals(b, ty);
                }
            }
            _ => {
                self.refined_results.insert(e, ty.clone());
            }
        }
    }

    fn infer(&mut self, e: ExprId) -> Ty {
        if let Some(ty) = self.refined_results.remove(&e)
            && let Some(t) = self.refined_literal(e, &ty)
        {
            return t;
        }
        let t = self.infer_inner(e);
        self.out.expr_types[index(e)] = t.clone();
        t
    }

    /// The module the body being checked is in.
    fn here(&self) -> u32 {
        match self.owner.owner() {
            Item::Fn(i) => self.items.fns[i as usize].module,
            Item::Test(i) => self.items.tests[i as usize].module,
            Item::Closure(..) => crate::items::PRELUDE_MODULE,
        }
    }

    /// Whether field `i` of struct `adt` can be named here; E0701 at `range` if not
    /// (`mod.vis.2`).
    fn field_visible(&mut self, adt: u32, i: u32, range: TextRange) -> bool {
        let def = &self.items.adts[adt as usize];
        let Some(f) = def.variants.first().and_then(|v| v.fields.get(i as usize)) else {
            return true;
        };
        if self.items.can_see(f.vis, def.module, self.here()) {
            return true;
        }
        // A derive's template reads the fields of the struct it is instantiated for: the
        // derive is the opt-in (`comptime.template.1`).
        if let Item::Fn(i) = self.owner.owner()
            && self.items.fns[i as usize]
                .template
                .as_ref()
                .is_some_and(|t| t.adt == Some(adt))
        {
            return true;
        }
        let (name, ty) = (f.name.clone(), def.name.clone());
        self.report(
            Code::E0701,
            range,
            format!(
                "the field `{name}` of `{ty}` is private to its module: mark it `pub` to share it"
            ),
        );
        false
    }

    /// Whether function `f` of an inherent impl can be called here; E0701 at `range` if not
    /// (`mod.vis.2`).
    fn method_visible(&mut self, f: u32, range: TextRange) -> bool {
        let sig = &self.items.fns[f as usize];
        let inherent = matches!(sig.container, crate::items::Container::Impl(k)
            if self.items.impls[k as usize].trait_ref.is_none());
        if !inherent || !sig.src.is_file() || self.items.can_see(sig.vis, sig.module, self.here()) {
            return true;
        }
        let name = sig.name.clone();
        self.report(
            Code::E0701,
            range,
            format!("`{name}` is private to its module: mark it `pub` to share it"),
        );
        false
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
        // A literal nothing gives a type to is an `int` or an `f64` (`num.lit.1`).
        self.solver.table.default_literals();
        self.settle_assoc_eqs();
        let (errors, pending) = std::mem::take(&mut self.obligations).solve(&mut self.solver);
        self.out.diagnostics.extend(errors);
        if let Some(d) = finish::uninferred(self.body, &self.out, &mut self.solver, &pending) {
            self.out.diagnostics.push(d);
        }
        self.out.resolve_all(&mut self.solver);
        self.check_literals();
        self.meet_needs();
        self.finish_closures();
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

impl Checker<'_> {
    /// Each integer literal's value fits the type it got (E0204, `num.lit.1`).
    fn check_literals(&mut self) {
        for (e, v) in std::mem::take(&mut self.literals) {
            let t = self.out.expr_types[index(e)].clone();
            let t = self.items.base(&t);
            let (lo, hi, name) = match &t {
                Ty::Int => (i128::from(i64::MIN), i64::MAX as u128, "int".to_string()),
                Ty::Num(crate::ty::Num::U128) => (0, u128::MAX, "u128".to_string()),
                Ty::Num(n) if !n.is_float() => {
                    let (lo, hi) = n.int_range();
                    (lo, hi as u128, n.name().to_string())
                }
                _ => continue,
            };
            if !v.fits(lo, hi) {
                self.report(
                    Code::E0204,
                    self.range(e),
                    format!("`{v}` does not fit in `{name}`, which holds {lo} to {hi}"),
                );
            }
        }
    }
}

/// Whether a value of number type `from` converts to number type `to` implicitly: `to` holds
/// every value of `from` exactly (`num.widen.1`). `i32` to `int`, `u8` to `u32`, `u32` to
/// `int`, `f32` to `f64`, and the integers of up to 32 bits (24 for `f32`) to the floats.
pub(crate) fn widens(from: &Ty, to: &Ty) -> bool {
    use crate::ty::Num;
    let num = |t: &Ty| match t {
        Ty::Int => Some(None),
        Ty::Num(n) => Some(Some(*n)),
        _ => None,
    };
    let (Some(a), Some(b)) = (num(from), num(to)) else {
        return false;
    };
    if a == b {
        return false;
    }
    // Whether it is signed, its width, and whether it is a float; `int` is an `i64`.
    let shape = |n: Option<Num>| match n {
        None => (true, 64u32, false),
        Some(n) => (n.is_signed() || n.is_float(), n.bits(), n.is_float()),
    };
    let ((sa, wa, fa), (sb, wb, fb)) = (shape(a), shape(b));
    match (fa, fb) {
        // Float to float: `f32` to `f64`.
        (true, true) => wa < wb,
        (true, false) => false,
        // Integer to float: exact if the mantissa holds every value (24 bits for `f32`, 53
        // for `f64`, counting the sign).
        (false, true) => {
            let mantissa = if wb == 32 { 24 } else { 53 };
            wa < mantissa
        }
        // Integer to integer: signed to wider signed, unsigned to wider unsigned, unsigned to
        // strictly wider signed.
        (false, false) => match (sa, sb) {
            (true, false) => false,
            _ => wa < wb,
        },
    }
}

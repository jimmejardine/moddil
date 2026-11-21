//! Type checking a body: the `infer` query. Types are inferred by unification, generic calls
//! are given type arguments, methods and operators are resolved to the functions they call, and
//! trait bounds are checked. It also checks that patterns are exhaustive where they must be
//! ([`crate::exhaust`]).

use std::collections::{HashMap, HashSet};

use la_arena::RawIdx;
use moddil_syntax::TextRange;

use crate::collect::item_tree;
use crate::exhaust;
use crate::hir::{
    Arm, BinaryOp, Body, CompareOp, ExprId, ExprKind, LocalId, PatId, PatKind, Stmt, UnaryOp,
};
use crate::items::FnSig;
use crate::lower::body;
use crate::traits::{Adjust, Answer, Solver};
use crate::ty::TraitRef;
use crate::{Code, Db, Diagnostic, Item, ItemTree, Owner, Ty};

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
    adjusts: HashMap<ExprId, Adjust>,
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
    let lowered = body(db, owner);
    let items = item_tree(db, owner.file(db));
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
        Item::Test(_) => (Ty::Unit, None, true, &[][..]),
    };
    let mut cx = Checker {
        body,
        items,
        solver: Solver::new(items, preds),
        ret: ret.clone(),
        ret_range,
        loops: Vec::new(),
        obligations: Vec::new(),
        deferred: Vec::new(),
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
    }
    if let Some(root) = body.root {
        let t = cx.infer(root);
        // A test's final value is observed, not returned, so it may have any type.
        if !is_test && !cx.coerce(&t, &ret) {
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

/// A bound a type must meet, found while checking.
struct Obligation {
    ty: Ty,
    trait_ref: TraitRef,
    range: TextRange,
}

/// A check that needs the body's types resolved first.
enum Deferred {
    /// A `match`: its expression, the scrutinee's type and its arms.
    Match(ExprId, Ty, Vec<Arm>),
    /// A pattern that must match every value of its type.
    Irrefutable(PatId, &'static str),
    /// A name bound by copying out of a borrow (`&x`), which must be `Copy`.
    CopyOut(LocalId, TextRange),
}

struct Checker<'a> {
    body: &'a Body,
    items: &'a ItemTree,
    solver: Solver<'a>,
    ret: Ty,
    ret_range: Option<TextRange>,
    loops: Vec<LoopCx>,
    obligations: Vec<Obligation>,
    deferred: Vec<Deferred>,
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
        let t = self.infer(e);
        if !self.coerce(&t, expected) {
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
        self.obligations.push(Obligation {
            ty,
            trait_ref,
            range,
        });
    }

    fn infer_inner(&mut self, e: ExprId) -> Ty {
        let range = self.range(e);
        match &self.body.exprs[e].kind {
            ExprKind::Missing => Ty::Error,
            &ExprKind::Int(v) => {
                if i64::try_from(v).is_err() {
                    self.report(
                        Code::E0204,
                        range,
                        format!("`{v}` does not fit in `int` (a signed 64-bit integer)"),
                    );
                }
                Ty::Int
            }
            ExprKind::Bool(_) => Ty::Bool,
            ExprKind::Unit => Ty::Unit,
            &ExprKind::Local(l) => self.out.local_types[index(l)].clone(),
            ExprKind::Fn { .. } | ExprKind::AssocFn { .. } | ExprKind::TraitFn { .. } => {
                self.out
                    .diagnostics
                    .push(Diagnostic::unsupported(range, "a function used as a value"));
                Ty::Error
            }
            ExprKind::Tuple(elements) => {
                let ts: Vec<Ty> = elements.iter().map(|&x| self.infer(x)).collect();
                let ts: Vec<Ty> = ts.iter().map(|t| self.norm(t)).collect();
                if ts.iter().any(Ty::has_ref) {
                    self.report(
                        Code::E0309,
                        range,
                        "a tuple cannot hold a borrow: borrows live only in parameters and locals",
                    );
                    return Ty::Error;
                }
                Ty::tuple(ts)
            }
            &ExprKind::Ref { mutable, expr } => {
                let t = self.infer(expr);
                if mutable {
                    self.check_mutable(expr, "borrowed as `&mut`");
                }
                match self.norm(&t) {
                    Ty::Ref(..) => {
                        self.report(
                            Code::E0309,
                            range,
                            "a borrow cannot point to a borrow: to borrow what it points to, \
                             write `&*r`",
                        );
                        Ty::Error
                    }
                    Ty::Error => Ty::Error,
                    t => Ty::reference(mutable, t),
                }
            }
            &ExprKind::Deref(x) => {
                let t = self.infer(x);
                match self.norm(&t) {
                    Ty::Ref(_, t) => (*t).clone(),
                    Ty::Error => Ty::Error,
                    Ty::Infer(_) => self.unknown(range, "to dereference it"),
                    t => {
                        let t = self.show(&t);
                        self.report(
                            Code::E0205,
                            range,
                            format!(
                                "`*` is not defined for `{t}`: only a borrow can be dereferenced"
                            ),
                        );
                        Ty::Error
                    }
                }
            }
            ExprKind::Adt {
                adt,
                variant,
                fields,
                args,
            } => {
                let (adt, variant) = (*adt, *variant);
                let fields = fields.clone();
                let args: Vec<Ty> = args.iter().map(|a| self.instantiate(a)).collect();
                let ty = self.items.adts[adt as usize].ty_with(adt, args);
                let types = self.items.field_types(&ty, variant);
                for (i, value) in fields {
                    let t = types[i as usize].clone();
                    self.check(value, &t);
                }
                ty
            }
            ExprKind::Field {
                base,
                name,
                name_range,
            } => {
                let (base, name, name_range) = (*base, name.clone(), *name_range);
                self.field(e, base, &name, name_range)
            }
            ExprKind::Call { callee, args } => {
                let (callee, args) = (*callee, args.clone());
                self.call(e, callee, &args)
            }
            ExprKind::MethodCall {
                receiver,
                name,
                name_range,
                generic_args,
                args,
            } => {
                let (receiver, name, name_range) = (*receiver, name.clone(), *name_range);
                let (generic_args, args) = (generic_args.clone(), args.clone());
                self.method_call(
                    e,
                    receiver,
                    &name,
                    name_range,
                    generic_args.as_deref(),
                    &args,
                )
            }
            ExprKind::Block { stmts, tail, .. } => {
                let (stmts, tail) = (stmts.clone(), *tail);
                self.block(&stmts, tail)
            }
            &ExprKind::If { cond, then, else_ } => self.if_expr(cond, then, else_),
            &ExprKind::Let { pat, init } => {
                let t = self.infer(init);
                self.check_pat(pat, &t, None);
                Ty::Bool
            }
            ExprKind::Match { scrutinee, arms } => {
                let (scrutinee, arms) = (*scrutinee, arms.clone());
                self.match_expr(e, scrutinee, &arms)
            }
            &ExprKind::Loop { body } => {
                self.loops.push(LoopCx {
                    id: e,
                    is_while: false,
                    ty: None,
                });
                self.check(body, &Ty::Unit);
                let cx = self.loops.pop().expect("the loop");
                // A loop without `break` never finishes.
                cx.ty.unwrap_or(Ty::Never)
            }
            &ExprKind::While { cond, body } => {
                self.check(cond, &Ty::Bool);
                self.loops.push(LoopCx {
                    id: e,
                    is_while: true,
                    ty: None,
                });
                self.check(body, &Ty::Unit);
                self.loops.pop();
                Ty::Unit
            }
            &ExprKind::Break { target, value } => self.break_expr(range, target, value),
            ExprKind::Continue { .. } => Ty::Never,
            &ExprKind::Assert(cond) => {
                self.check(cond, &Ty::Bool);
                Ty::Unit
            }
            &ExprKind::Return(value) => {
                let ret = self.ret.clone();
                match value {
                    Some(v) => {
                        self.check(v, &ret);
                    }
                    None if !self.coerce(&Ty::Unit, &ret) => {
                        let mut d = Diagnostic::error(
                            Code::E0201,
                            range,
                            format!("expected a value of type `{ret}` after `return`"),
                        );
                        if let Some(r) = self.ret_range {
                            d = d.with_label(r, "the return type");
                        }
                        self.out.diagnostics.push(d);
                    }
                    None => {}
                }
                Ty::Never
            }
            &ExprKind::Unary(op, x) => self.unary(e, op, x),
            &ExprKind::Binary(op, l, r) => {
                let lt = self.infer(l);
                self.binary(e, range, op, &lt, r)
            }
            &ExprKind::Logic(_, l, r) => {
                self.check(l, &Ty::Bool);
                self.check(r, &Ty::Bool);
                Ty::Bool
            }
            ExprKind::Compare { first, rest } => {
                let (first, rest) = (*first, rest.clone());
                self.compare(e, first, &rest)
            }
            &ExprKind::Assign { target, op, value } => {
                // Assigning to a function is already reported.
                let lt = match self.body.exprs[target].kind {
                    ExprKind::Fn { .. } | ExprKind::AssocFn { .. } | ExprKind::TraitFn { .. } => {
                        Ty::Error
                    }
                    _ => self.infer(target),
                };
                self.check_mutable(target, "assigned to");
                match op {
                    None => {
                        self.check(value, &lt);
                    }
                    Some(op) => {
                        let lt = self.norm(&lt);
                        match lt {
                            Ty::Int | Ty::Bool | Ty::Error | Ty::Never => {
                                self.binary(e, range, op, &lt, value);
                            }
                            _ => {
                                self.infer(value);
                                let lt = self.show(&lt);
                                self.report(
                                    Code::E0205,
                                    range,
                                    format!(
                                        "`{}=` is defined only for `int` and `bool`, not `{lt}`: \
                                         write `x = x {} y`",
                                        op.symbol(),
                                        op.symbol()
                                    ),
                                );
                            }
                        }
                    }
                }
                Ty::Unit
            }
        }
    }

    fn break_expr(&mut self, range: TextRange, target: ExprId, value: Option<ExprId>) -> Ty {
        let t = match value {
            Some(v) => self.infer(v),
            None => Ty::Unit,
        };
        let Some(at) = self.loops.iter().position(|l| l.id == target) else {
            return Ty::Never;
        };
        if self.loops[at].is_while {
            if value.is_some() {
                self.report(
                    Code::E0212,
                    range,
                    "only `loop` can be left with a value; `while` has type `()`",
                );
            }
            return Ty::Never;
        }
        match self.loops[at].ty.clone() {
            None => self.loops[at].ty = Some(t),
            Some(prev) => match self.join(&prev, &t) {
                Some(j) => self.loops[at].ty = Some(j),
                None => {
                    let (prev, t) = (self.show(&prev), self.show(&t));
                    self.report(
                        Code::E0201,
                        value.map_or(range, |v| self.range(v)),
                        format!(
                            "this loop's earlier `break` gives a `{prev}`, but this one gives a \
                             `{t}`"
                        ),
                    );
                }
            },
        }
        Ty::Never
    }

    /// Reports why the place `e` cannot be changed, if it cannot: a local not declared `mut`
    /// (E0301), or a place reached through a shared borrow (E0302). `what` is how it would be
    /// changed: "assigned to", "borrowed as `&mut`".
    fn check_mutable(&mut self, e: ExprId, what: &str) {
        let range = self.range(e);
        match self.body.exprs[e].kind {
            ExprKind::Local(l) => {
                let data = &self.body.locals[l];
                if !data.mutable {
                    let d = Diagnostic::error(
                        Code::E0301,
                        range,
                        format!(
                            "`{}` is not declared `mut`, so it cannot be {what}",
                            data.name
                        ),
                    )
                    .with_label(data.range, "declared here");
                    self.out.diagnostics.push(d);
                }
            }
            ExprKind::Field { base, .. } if !self.out.field_derefs.contains(&e) => {
                self.check_mutable(base, what);
            }
            ExprKind::Field { base: x, .. } | ExprKind::Deref(x) => {
                let t = self.out.expr(x);
                if let Ty::Ref(false, _) = self.norm(&t) {
                    self.report(
                        Code::E0302,
                        range,
                        format!("this is behind a shared borrow (`&`), so it cannot be {what}"),
                    );
                }
            }
            _ => {}
        }
    }

    fn field(&mut self, e: ExprId, base: ExprId, name: &str, name_range: TextRange) -> Ty {
        let t = self.infer(base);
        let mut bt = self.norm(&t);
        // Field access looks through a borrow.
        if let Ty::Ref(_, inner) = &bt {
            self.out.field_derefs.insert(e);
            bt = self.norm(inner);
        }
        let found = match &bt {
            Ty::Error => return Ty::Error,
            Ty::Infer(_) => return self.unknown(self.range(base), "to read a field of it"),
            Ty::Tuple(ts) => name
                .parse::<usize>()
                .ok()
                .filter(|&i| i < ts.len())
                .map(|i| (i as u32, ts[i].clone())),
            Ty::Adt(a, ..) if !self.items.adts[*a as usize].is_enum => {
                let def = self.items.variant(*a, 0);
                def.field(name)
                    .map(|i| (i, self.items.field_types(&bt, 0)[i as usize].clone()))
            }
            _ => None,
        };
        match found {
            Some((i, t)) => {
                self.out.field_indices.insert(e, i);
                t
            }
            None => {
                let hint = match &bt {
                    Ty::Adt(..) => ": an enum's fields are reached by matching it",
                    _ => "",
                };
                let bt = self.show(&bt);
                self.report(
                    Code::E0107,
                    name_range,
                    format!("`{bt}` has no field called `{name}`{hint}"),
                );
                Ty::Error
            }
        }
    }

    fn if_expr(&mut self, cond: ExprId, then: ExprId, else_: Option<ExprId>) -> Ty {
        self.check(cond, &Ty::Bool);
        let t1 = self.infer(then);
        let Some(else_) = else_ else {
            if !self.coerce(&t1, &Ty::Unit) {
                let t1 = self.show(&t1);
                self.report(
                    Code::E0201,
                    self.range(then),
                    format!(
                        "an `if` without `else` has type `()`, but this branch has type `{t1}`"
                    ),
                );
            }
            return Ty::Unit;
        };
        let t2 = self.infer(else_);
        match self.join(&t1, &t2) {
            Some(t) => t,
            None => {
                let (s1, s2) = (self.show(&t1), self.show(&t2));
                let d = Diagnostic::error(
                    Code::E0201,
                    self.range(else_),
                    format!(
                        "the branches of an `if` must have the same type: expected `{s1}`, found \
                         `{s2}`"
                    ),
                )
                .with_label(self.range(then), format!("this branch has type `{s1}`"));
                self.out.diagnostics.push(d);
                t1
            }
        }
    }

    fn match_expr(&mut self, e: ExprId, scrutinee: ExprId, arms: &[Arm]) -> Ty {
        let st = self.infer(scrutinee);
        let mut result: Option<(Ty, ExprId)> = None;
        let errors_before = self.out.diagnostics.len();
        let mut broken = false;
        for arm in arms {
            broken |= self.has_missing(arm.pat);
            self.check_pat(arm.pat, &st, None);
            broken |= self.out.diagnostics.len() > errors_before;
            if let Some(g) = arm.guard {
                self.check(g, &Ty::Bool);
            }
            let t = self.infer(arm.body);
            result = match result {
                None => Some((t, arm.body)),
                Some((prev, first)) => match self.join(&prev, &t) {
                    Some(j) => Some((j, first)),
                    None => {
                        let (sp, st2) = (self.show(&prev), self.show(&t));
                        let d = Diagnostic::error(
                            Code::E0201,
                            self.range(arm.body),
                            format!(
                                "the arms of a `match` must have the same type: expected \
                                 `{sp}`, found `{st2}`"
                            ),
                        )
                        .with_label(self.range(first), format!("this arm has type `{sp}`"));
                        self.out.diagnostics.push(d);
                        Some((prev, first))
                    }
                },
            };
        }
        // Patterns with errors would make the analysis report noise.
        if !broken {
            self.deferred.push(Deferred::Match(e, st, arms.to_vec()));
        }
        result.map_or(Ty::Never, |(t, _)| t)
    }

    /// Whether `p` contains a pattern already reported as an error while lowering.
    fn has_missing(&self, p: PatId) -> bool {
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
    fn check_pat(&mut self, p: PatId, expected: &Ty, mode: Option<bool>) {
        let expected = self.norm(expected);
        self.out.pat_types[index(p)] = expected.clone();
        let tests_value = matches!(
            self.body.pats[p].kind,
            PatKind::Int(_)
                | PatKind::Bool(_)
                | PatKind::Range { .. }
                | PatKind::Unit
                | PatKind::Tuple { .. }
                | PatKind::Adt { .. }
        );
        if tests_value && let Ty::Ref(m, inner) = &expected {
            // Matching through a borrow: test what it points to, and bind borrows.
            self.out.pat_derefs.insert(p);
            let mode = Some(mode.map_or(*m, |outer| outer && *m));
            let inner = self.norm(inner);
            self.check_pat_kind(p, &inner, mode);
        } else {
            self.check_pat_kind(p, &expected, mode);
        }
    }

    fn check_pat_kind(&mut self, p: PatId, expected: &Ty, mode: Option<bool>) {
        let range = self.body.pats[p].range;
        let shown = self.show(expected);
        let wrong = |found: &str| -> String {
            format!("expected `{shown}`, but this pattern matches {found}")
        };
        match &self.body.pats[p].kind {
            PatKind::Missing | PatKind::Wild => {}
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
            &PatKind::Range { lo, hi, inclusive } => {
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
    fn value_bindings(&self, p: PatId, out: &mut Vec<(LocalId, TextRange)>) {
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

    /// The prelude trait for an operator, and its method.
    fn op_trait(&self, name: &str, method: &str) -> Option<(TraitRef, u32)> {
        let tr = self.items.lang_ref(name)?;
        let def = &self.items.traits[tr.trait_ as usize];
        let m = def
            .methods
            .iter()
            .copied()
            .find(|&m| self.items.fns[m as usize].name == method)?;
        Some((tr, m))
    }

    /// An operator on a type other than `int` and `bool`: through the trait `tr` and its method
    /// `method`, with `operand` as `Self`. The result type is the trait's `Output`.
    fn operator(
        &mut self,
        e: ExprId,
        range: TextRange,
        symbol: &str,
        operand: &Ty,
        (tr, method): (&str, &str),
    ) -> Option<Ty> {
        let (trait_ref, func) = self.op_trait(tr, method)?;
        if self.solver.implements(operand, &trait_ref) == Answer::No {
            let t = self.show(operand);
            self.report(
                Code::E0205,
                range,
                format!("`{symbol}` is not defined for `{t}`: it does not implement `{tr}`"),
            );
            return Some(Ty::Error);
        }
        self.require(operand.clone(), trait_ref.clone(), range);
        self.out.calls.insert(
            e,
            Callee::Trait {
                func,
                args: vec![operand.clone()],
            },
        );
        let def = &self.items.traits[trait_ref.trait_ as usize];
        let out = match def.assoc.iter().position(|(n, _)| n == "Output") {
            Some(i) => Ty::Assoc(std::sync::Arc::new(crate::ty::AssocTy {
                self_ty: operand.clone(),
                trait_ref,
                index: i as u32,
                name: "Output".into(),
            })),
            None => Ty::Bool,
        };
        Some(self.norm(&out))
    }

    fn unary(&mut self, e: ExprId, op: UnaryOp, x: ExprId) -> Ty {
        let range = self.range(e);
        let t = self.infer(x);
        let t = self.norm(&t);
        let sym = if op == UnaryOp::Neg { "-" } else { "!" };
        match (op, &t) {
            (_, Ty::Error | Ty::Never) => Ty::Error,
            (UnaryOp::Neg | UnaryOp::Not, Ty::Int) => Ty::Int,
            (UnaryOp::Not, Ty::Bool) => Ty::Bool,
            (_, Ty::Infer(_)) => self.unknown(range, &format!("to apply `{sym}` to it")),
            (_, Ty::Ref(..)) => {
                let t = self.show(&t);
                self.report(
                    Code::E0205,
                    range,
                    format!("`{sym}` is not defined for `{t}`: dereference it with `*`"),
                );
                Ty::Error
            }
            _ => {
                let names = if op == UnaryOp::Neg {
                    ("Neg", "neg")
                } else {
                    ("Not", "not")
                };
                self.operator(e, range, sym, &t, names).unwrap_or(Ty::Error)
            }
        }
    }

    /// `l op r`, where `l` has type `lt`.
    fn binary(&mut self, e: ExprId, range: TextRange, op: BinaryOp, lt: &Ty, r: ExprId) -> Ty {
        let lt = self.norm(lt);
        let names = match op {
            BinaryOp::Add => Some(("Add", "add")),
            BinaryOp::Sub => Some(("Sub", "sub")),
            BinaryOp::Mul => Some(("Mul", "mul")),
            BinaryOp::Div => Some(("Div", "div")),
            BinaryOp::Rem => Some(("Rem", "rem")),
            _ => None,
        };
        match &lt {
            Ty::Int => {
                self.check(r, &Ty::Int);
                Ty::Int
            }
            Ty::Bool if op.applies_to_bool() => {
                self.check(r, &Ty::Bool);
                Ty::Bool
            }
            Ty::Error | Ty::Never => {
                self.infer(r);
                Ty::Error
            }
            Ty::Infer(_) => {
                self.infer(r);
                self.unknown(range, &format!("to apply `{}` to it", op.symbol()))
            }
            Ty::Ref(..) => {
                self.infer(r);
                let lt = self.show(&lt);
                self.report(
                    Code::E0205,
                    range,
                    format!(
                        "`{}` is not defined for `{lt}`: dereference it with `*`",
                        op.symbol()
                    ),
                );
                Ty::Error
            }
            _ if names.is_some() => {
                let t = self.operator(e, range, op.symbol(), &lt, names.expect("a trait"));
                if t.as_ref().is_some_and(|t| *t != Ty::Error) {
                    self.check(r, &lt);
                } else {
                    self.infer(r);
                }
                t.unwrap_or(Ty::Error)
            }
            _ => {
                self.infer(r);
                let lt = self.show(&lt);
                self.report(
                    Code::E0205,
                    range,
                    format!("`{}` is not defined for `{lt}`", op.symbol()),
                );
                Ty::Error
            }
        }
    }

    fn compare(&mut self, e: ExprId, first: ExprId, rest: &[(CompareOp, ExprId)]) -> Ty {
        let range = self.range(e);
        let t0 = self.infer(first);
        let mut prev = (first, t0);
        for (i, &(op, x)) in rest.iter().enumerate() {
            let t = self.infer(x);
            let (_, pt) = prev.clone();
            let Some(j) = self.join(&pt, &t) else {
                if op.is_ordering() {
                    let st = self.show(&t);
                    let sp = self.show(&pt);
                    self.report(
                        Code::E0201,
                        self.range(x),
                        format!(
                            "`{}` compares values of one type: `{sp}` and `{st}`",
                            op.symbol()
                        ),
                    );
                } else {
                    self.mismatch(x, &pt, &t);
                }
                prev = (x, t);
                continue;
            };
            let j = self.norm(&j);
            match &j {
                Ty::Error | Ty::Never | Ty::Int => {}
                Ty::Bool | Ty::Unit if !op.is_ordering() => {}
                Ty::Infer(_) => {
                    self.unknown(range, &format!("to compare it with `{}`", op.symbol()));
                }
                Ty::Ref(..) => self.report(
                    Code::E0205,
                    range,
                    format!(
                        "`{}` compares values, not borrows: dereference both sides with `*`",
                        op.symbol()
                    ),
                ),
                _ => {
                    let (tr, method) = match op {
                        CompareOp::Eq => ("PartialEq", "eq"),
                        CompareOp::Ne => ("PartialEq", "ne"),
                        CompareOp::Lt => ("PartialOrd", "lt"),
                        CompareOp::Le => ("PartialOrd", "le"),
                        CompareOp::Gt => ("PartialOrd", "gt"),
                        CompareOp::Ge => ("PartialOrd", "ge"),
                    };
                    if let Some((trait_ref, func)) = self.op_trait(tr, method) {
                        if self.solver.implements(&j, &trait_ref) == Answer::No {
                            let js = self.show(&j);
                            self.report(
                                Code::E0205,
                                range,
                                format!(
                                    "`{}` is not defined for `{js}`: it does not implement `{tr}`",
                                    op.symbol()
                                ),
                            );
                        } else {
                            self.require(j.clone(), trait_ref, range);
                            self.out.compare_calls.insert(
                                (e, i),
                                Callee::Trait {
                                    func,
                                    args: vec![j.clone()],
                                },
                            );
                        }
                    }
                }
            }
            prev = (x, t);
        }
        Ty::Bool
    }

    /// The parameter types and return type of `sig` with type arguments `args`, after bounds on
    /// the arguments are recorded for checking.
    fn instantiate_sig(&mut self, func: u32, args: &[Ty], range: TextRange) -> (Vec<Ty>, Ty) {
        let sig: &FnSig = &self.items.fns[func as usize];
        let params: Vec<Ty> = sig.params.iter().map(|p| p.ty.subst(args)).collect();
        let ret = sig.ret.subst(args);
        for p in sig.generics.preds.clone() {
            self.require(p.ty.subst(args), p.trait_ref.subst(args), range);
        }
        (params, ret)
    }

    /// Type arguments for a function's own parameters: those written (`want` of them), or new
    /// variables.
    fn own_args(
        &mut self,
        given: Option<&[Ty]>,
        want: usize,
        range: TextRange,
        name: &str,
    ) -> Vec<Ty> {
        match given {
            Some(g) => {
                let mut ds = Vec::new();
                let ok = crate::collect::arity(&format!("`{name}`"), want, g.len(), range, &mut ds);
                self.out.diagnostics.extend(ds);
                if ok {
                    g.iter().map(|t| self.instantiate(t)).collect()
                } else {
                    vec![Ty::Error; want]
                }
            }
            None => self.solver.fresh_n(want),
        }
    }

    /// Reports E0412 if `func` is a destructor, which is run by dropping, not by calling.
    fn not_drop(&mut self, func: u32, range: TextRange) {
        let drop = self.items.lang("Drop");
        if drop.is_some() && self.items.trait_of(func) == drop {
            self.report(
                Code::E0412,
                range,
                "a destructor runs when its value is dropped, and cannot be called: to end a \
                 value early, write `drop(x)`",
            );
        }
    }

    /// What a callee expression calls: its function and type arguments.
    fn callee(&mut self, callee: ExprId) -> Option<Callee> {
        let range = self.range(callee);
        match self.body.exprs[callee].kind.clone() {
            ExprKind::Fn { func, args } => {
                let args: Vec<Ty> = args.iter().map(|a| self.instantiate(a)).collect();
                Some(Callee::Fn { func, args })
            }
            ExprKind::TraitFn {
                func,
                trait_args,
                args,
                ..
            } => {
                let sig = &self.items.fns[func as usize];
                let own = sig.generics.params.len() - sig.generics.parent as usize;
                let name = sig.name.clone();
                let mut all = vec![self.fresh()];
                all.extend(trait_args.iter().map(|a| self.instantiate(a)));
                all.extend(self.own_args(args.as_deref(), own, range, &name));
                Some(Callee::Trait { func, args: all })
            }
            ExprKind::AssocFn {
                ty,
                name,
                name_range,
                args,
            } => {
                let ty = self.instantiate(&ty);
                self.assoc_fn(&ty, &name, name_range, args.as_deref())
            }
            _ => None,
        }
    }

    /// `Type::name`: a function of an inherent impl of `ty`, or else a method of a trait `ty`
    /// implements.
    fn assoc_fn(
        &mut self,
        ty: &Ty,
        name: &str,
        range: TextRange,
        given: Option<&[Ty]>,
    ) -> Option<Callee> {
        let items = self.items;
        for imp in &items.impls {
            if imp.trait_ref.is_some() {
                continue;
            }
            let Some(&f) = imp
                .methods
                .iter()
                .find(|&&f| items.fns[f as usize].name == name)
            else {
                continue;
            };
            let saved = self.solver.table.clone();
            let mut args = self.solver.fresh_n(imp.generics.params.len());
            if self.solver.unify(&imp.self_ty.subst(&args), ty) {
                let sig = &items.fns[f as usize];
                let own = sig.generics.params.len() - sig.generics.parent as usize;
                args.extend(self.own_args(given, own, range, name));
                return Some(Callee::Fn { func: f, args });
            }
            self.solver.table = saved;
        }
        let mut found: Vec<(u32, Vec<Ty>)> = Vec::new();
        for (t, def) in items.traits.iter().enumerate() {
            let Some(&f) = def
                .methods
                .iter()
                .find(|&&f| items.fns[f as usize].name == name)
            else {
                continue;
            };
            let trait_args = self.solver.fresh_n(def.generics.params.len() - 1);
            let trait_ref = TraitRef {
                trait_: t as u32,
                name: def.name.as_str().into(),
                args: trait_args.clone().into(),
            };
            let saved = self.solver.table.clone();
            let answer = self.solver.implements(ty, &trait_ref);
            self.solver.table = saved;
            if answer != Answer::No {
                let mut args = vec![ty.clone()];
                args.extend(trait_args);
                found.push((f, args));
            }
        }
        match found.len() {
            0 => {
                let t = self.show(ty);
                let (code, what) = match &t {
                    Ty::Adt(a, ..) if items.adts[*a as usize].is_enum => {
                        (Code::E0101, "variant or function")
                    }
                    _ => (Code::E0403, "function"),
                };
                self.report(code, range, format!("`{t}` has no {what} called `{name}`"));
                None
            }
            1 => {
                let (f, mut args) = found.pop().expect("one");
                let sig = &items.fns[f as usize];
                let own = sig.generics.params.len() - sig.generics.parent as usize;
                args.extend(self.own_args(given, own, range, name));
                Some(Callee::Trait { func: f, args })
            }
            _ => {
                let names: Vec<String> = found
                    .iter()
                    .filter_map(|(f, _)| items.trait_of(*f))
                    .map(|t| format!("`{}::{name}`", items.traits[t as usize].name))
                    .collect();
                let t = self.show(ty);
                self.report(
                    Code::E0410,
                    range,
                    format!(
                        "more than one trait gives `{t}` a function `{name}`: write {}",
                        names.join(" or ")
                    ),
                );
                None
            }
        }
    }

    /// Checks the arguments of a call of `func` with parameter types `params`.
    fn args(&mut self, range: TextRange, func: u32, params: &[Ty], args: &[ExprId]) {
        let sig = &self.items.fns[func as usize];
        if args.len() != params.len() {
            let d = Diagnostic::error(
                Code::E0202,
                range,
                format!(
                    "`{}` takes {} argument{}, but {} {} given",
                    sig.name,
                    params.len(),
                    if params.len() == 1 { "" } else { "s" },
                    args.len(),
                    if args.len() == 1 { "was" } else { "were" },
                ),
            )
            .with_label(sig.name_range, "defined here");
            self.out.diagnostics.push(d);
        }
        for (i, &a) in args.iter().enumerate() {
            match params.get(i) {
                Some(t) => {
                    self.check(a, t);
                }
                None => {
                    self.infer(a);
                }
            }
        }
    }

    fn call(&mut self, e: ExprId, callee: ExprId, args: &[ExprId]) -> Ty {
        let range = self.range(e);
        let Some(target) = self.callee(callee) else {
            if !matches!(
                self.body.exprs[callee].kind,
                ExprKind::AssocFn { .. } | ExprKind::Missing
            ) {
                let t = self.infer(callee);
                let t = self.show(&t);
                if t != Ty::Error {
                    self.report(
                        Code::E0203,
                        self.range(callee),
                        format!("this is a `{t}`, not a function"),
                    );
                }
            }
            for &a in args {
                self.infer(a);
            }
            return Ty::Error;
        };
        self.out.expr_types[index(callee)] = Ty::Unit;
        let func = target.func();
        self.not_drop(func, range);
        let (params, ret) = self.instantiate_sig(func, target.args(), range);
        self.out.calls.insert(e, target);
        self.args(range, func, &params, args);
        self.norm(&ret)
    }

    fn method_call(
        &mut self,
        e: ExprId,
        receiver: ExprId,
        name: &str,
        name_range: TextRange,
        given: Option<&[Ty]>,
        args: &[ExprId],
    ) -> Ty {
        let range = self.range(e);
        let rt = self.infer(receiver);
        let r = self.norm(&rt);
        match r {
            Ty::Error | Ty::Never => {
                for &a in args {
                    self.infer(a);
                }
                return Ty::Error;
            }
            Ty::Infer(_) => {
                for &a in args {
                    self.infer(a);
                }
                return self.unknown(self.range(receiver), "to call a method on it");
            }
            _ => {}
        }
        let pick = match self.solver.lookup_method(&r, name) {
            Ok(Some(p)) => p,
            Ok(None) if self.show(&r).has_infer() => {
                for &a in args {
                    self.infer(a);
                }
                return self.unknown(self.range(receiver), "to call a method on it");
            }
            Ok(None) => {
                for &a in args {
                    self.infer(a);
                }
                let t = self.show(&r);
                let hint = match r.strip_ref() {
                    Ty::Adt(a, ..)
                        if !self.items.adts[*a as usize].is_enum
                            && self.items.variant(*a, 0).field(name).is_some() =>
                    {
                        format!(": `{name}` is a field; to call it, write `({name})`")
                    }
                    _ => String::new(),
                };
                self.report(
                    Code::E0403,
                    name_range,
                    format!("`{t}` has no method called `{name}`{hint}"),
                );
                return Ty::Error;
            }
            Err(traits) => {
                for &a in args {
                    self.infer(a);
                }
                let t = self.show(&r);
                self.report(
                    Code::E0410,
                    name_range,
                    format!(
                        "more than one trait gives `{t}` a method `{name}` ({}): call it as \
                         `Trait::{name}(x)`",
                        traits.join(", ")
                    ),
                );
                return Ty::Error;
            }
        };
        let sig = &self.items.fns[pick.func as usize];
        let own = sig.generics.params.len() - sig.generics.parent as usize;
        let mut targs = pick.parent_args.clone();
        targs.extend(self.own_args(given, own, name_range, name));
        let target = match &pick.trait_ref {
            Some(_) => Callee::Trait {
                func: pick.func,
                args: targs,
            },
            None => Callee::Fn {
                func: pick.func,
                args: targs,
            },
        };
        self.not_drop(pick.func, name_range);
        let (params, ret) = self.instantiate_sig(pick.func, target.args(), range);
        // The receiver, adjusted to the method's `self`.
        let base = match (&r, pick.adjust.deref) {
            (Ty::Ref(_, inner), true) => (**inner).clone(),
            _ => r.clone(),
        };
        let adjusted = match pick.adjust.autoref {
            None => base,
            Some(m) => Ty::reference(m, base),
        };
        if let Some(p0) = params.first() {
            self.solver.unify(&adjusted, p0);
        }
        if pick.adjust.autoref == Some(true) {
            if pick.adjust.deref {
                if let Ty::Ref(false, _) = r {
                    self.report(
                        Code::E0302,
                        self.range(receiver),
                        format!(
                            "`{name}` takes `&mut self`, but this is behind a shared borrow (`&`)"
                        ),
                    );
                }
            } else {
                self.check_mutable(receiver, "borrowed as `&mut`");
            }
        }
        self.out.adjusts.insert(e, pick.adjust);
        self.out.calls.insert(e, target);
        let func = pick.func;
        self.args(range, func, params.get(1..).unwrap_or(&[]), args);
        self.norm(&ret)
    }

    fn block(&mut self, stmts: &[Stmt], tail: Option<ExprId>) -> Ty {
        let mut diverges = false;
        for s in stmts {
            match *s {
                Stmt::Let {
                    pat,
                    ref ty,
                    init,
                    else_,
                } => {
                    let t = match ty {
                        Some(ty) => {
                            let ty = self.instantiate(ty);
                            self.check(init, &ty);
                            ty
                        }
                        None => self.infer(init),
                    };
                    diverges |= self.norm(&t) == Ty::Never;
                    self.check_pat(pat, &t, None);
                    match else_ {
                        None => self
                            .deferred
                            .push(Deferred::Irrefutable(pat, "`let` without `else`")),
                        Some(x) => {
                            let et = self.infer(x);
                            let et = self.norm(&et);
                            if !matches!(et, Ty::Never | Ty::Error) {
                                let et = self.show(&et);
                                self.report(
                                    Code::E0210,
                                    self.range(x),
                                    format!(
                                        "the `else` block of `let … else` must not finish \
                                         normally (end it with `return`, `break` or \
                                         `continue`), but it has type `{et}`"
                                    ),
                                );
                            }
                        }
                    }
                }
                Stmt::Defer(body) => {
                    self.check(body, &Ty::Unit);
                }
                Stmt::Expr {
                    expr,
                    semi,
                    observe,
                } => {
                    let t = self.infer(expr);
                    let t = self.norm(&t);
                    diverges |= t == Ty::Never;
                    // A value followed by a `// =>` expectation is used by it.
                    if !semi && !observe && !self.coerce(&t, &Ty::Unit) {
                        let t = self.show(&t);
                        self.report(
                            Code::E0201,
                            self.range(expr),
                            format!(
                                "expected `()`, found `{t}`: a value in the middle of a block \
                                 needs a `;` after it"
                            ),
                        );
                    }
                }
            }
        }
        match tail {
            Some(t) => self.infer(t),
            None if diverges => Ty::Never,
            None => Ty::Unit,
        }
    }

    /// Checks the bounds recorded, resolves every type (reporting those that could not be
    /// inferred), and runs the checks that need resolved types.
    fn finish(&mut self) {
        let mut pending = std::mem::take(&mut self.obligations);
        loop {
            let before = pending.len();
            let mut still = Vec::new();
            for o in pending {
                let ty = self.solver.deep(&o.ty);
                let tr = o.trait_ref.map(&mut |t| self.solver.deep(t));
                match self.solver.implements(&ty, &tr) {
                    Answer::Yes(_) => {}
                    Answer::No => {
                        if !ty.has_error() {
                            self.report(
                                Code::E0402,
                                o.range,
                                format!("`{ty}` does not implement `{tr}`"),
                            );
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
        // Types that could not be inferred: report the first, at a binding if there is one.
        let mut reported = false;
        for (l, data) in self.body.locals.iter() {
            let t = self.solver.deep(&self.out.local_types[index(l)]);
            if t.has_infer() && !reported {
                reported = true;
                self.report(
                    Code::E0405,
                    data.range,
                    format!(
                        "the type of `{}` cannot be inferred (it is `{t}` so far): annotate it",
                        data.name
                    ),
                );
            }
        }
        for (e, _) in self.body.exprs.iter() {
            let t = self.solver.deep(&self.out.expr_types[index(e)]);
            if t.has_infer() && !reported {
                reported = true;
                self.report(
                    Code::E0405,
                    self.range(e),
                    format!(
                        "the type of this cannot be inferred (it is `{t}` so far): annotate it"
                    ),
                );
            }
        }
        if !reported && let Some(o) = pending.first() {
            let ty = self.solver.deep(&o.ty);
            self.report(
                Code::E0405,
                o.range,
                format!(
                    "which impl of `{}` to use for `{ty}` cannot be inferred",
                    o.trait_ref
                ),
            );
        }
        let errors = |t: &Ty| -> Ty { erase(t) };
        let resolve = |solver: &mut Solver, t: &Ty| errors(&solver.deep(t));
        for t in &mut self.out.expr_types {
            *t = resolve(&mut self.solver, t);
        }
        for t in &mut self.out.pat_types {
            *t = resolve(&mut self.solver, t);
        }
        for t in &mut self.out.local_types {
            *t = resolve(&mut self.solver, t);
        }
        let solver = &mut self.solver;
        for c in self.out.calls.values_mut() {
            *c = c.map(&mut |t| errors(&solver.deep(t)));
        }
        for c in self.out.compare_calls.values_mut() {
            *c = c.map(&mut |t| errors(&solver.deep(t)));
        }
        // Borrows are second-class: a type argument (which a struct could store) cannot be one.
        // `Self` may be, for the built-in `Clone` of a shared borrow.
        let mut found = Vec::new();
        for (&e, c) in &self.out.calls {
            let skip = usize::from(matches!(c, Callee::Trait { .. }));
            if c.args().iter().skip(skip).any(Ty::has_ref_inside) {
                found.push(self.range(e));
            }
        }
        for (e, _) in self.body.exprs.iter() {
            if let Ty::Adt(_, _, args) = &self.out.expr_types[index(e)]
                && args.iter().any(Ty::has_ref_inside)
            {
                found.push(self.range(e));
            }
        }
        found.sort_by_key(|r| (r.start, r.end));
        found.dedup();
        for r in found {
            self.report(
                Code::E0309,
                r,
                "a type argument cannot be a borrow: borrows live only in parameters and locals",
            );
        }
        // Checks on resolved types.
        let deferred = std::mem::take(&mut self.deferred);
        for d in deferred {
            match d {
                Deferred::Match(e, st, arms) => {
                    let st = errors(&self.solver.deep(&st));
                    if st.has_error() {
                        continue;
                    }
                    let ds = exhaust::check_match(
                        self.items,
                        self.body,
                        &self.out,
                        self.range(e),
                        &st,
                        &arms,
                    );
                    self.out.diagnostics.extend(ds);
                }
                Deferred::Irrefutable(pat, what) => {
                    let t = self.out.pat(pat);
                    if t.has_error() {
                        continue;
                    }
                    if let Some(missing) = exhaust::refutable(self.items, self.body, &self.out, pat)
                    {
                        self.report(
                            Code::E0209,
                            self.body.pats[pat].range,
                            format!(
                                "{what} needs a pattern that always matches, but `{missing}` is \
                                 not matched"
                            ),
                        );
                    }
                }
                Deferred::CopyOut(local, at) => {
                    let t = self.out.local(local);
                    if !self.solver.is_copy(&t) {
                        self.report(
                            Code::E0304,
                            at,
                            format!(
                                "`{t}` is not `Copy`, so it cannot be bound out of a borrow: \
                                 match the borrow without `&` to bind a borrow instead"
                            ),
                        );
                    }
                }
            }
        }
    }
}

/// `t` with every variable still unbound replaced by `Error`.
fn erase(t: &Ty) -> Ty {
    match t {
        Ty::Infer(_) => Ty::Error,
        t => t.map(&mut erase),
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

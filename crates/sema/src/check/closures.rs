//! Closures (`expressions/closures.md`): their types, calls of them, and what they capture.
//!
//! A closure is checked in its enclosing body, so it shares its inference: its parameters' types
//! come from what it is written for (the bound on the parameter it is passed to) or from how they
//! are used. Its errors are collected as a `try` block's are, and become its type's `Fails`; its
//! effects are met where it is written. Once the body's types are known, each closure's captures
//! are found from how its body uses the enclosing locals, and so whether it is `Fn`, `FnMut` or
//! only `FnOnce`.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use crate::hir::{ExprId, ExprKind, LocalId, PatId, Stmt};
use crate::ty::{AssocTy, ClosureKind, ClosureTy, TraitRef};
use crate::{Callee, Code, Ty};

use super::errors::FailCx;
use super::needs::Evidence;
use super::{Checker, index};

/// How a closure captures a local of the enclosing body.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CaptureMode {
    /// By a shared borrow: it only reads it.
    Shared,
    /// By an `&mut` borrow: it changes it.
    Mut,
    /// By value: it moves it out.
    Move,
}

/// What a closure captures.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Capture {
    /// A local of the enclosing body, and how.
    Local(LocalId, CaptureMode),
    /// A handler its body's calls use: the enclosing function's own, or one a `with` around the
    /// closure installs (a copy of the borrow).
    Evidence(Evidence),
}

/// How an expression's value is used, for finding captures.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Use {
    /// Its value is taken.
    Value,
    /// It is a place that is read, or borrowed with `&`.
    Read,
    /// It is a place that is changed, or borrowed with `&mut`.
    Write,
}

/// A closure being checked: the expected type it is written for, if one is known.
pub(super) struct Hint(pub(super) Ty);

impl Checker<'_> {
    /// The closure traits, by index: `FnOnce`, `FnMut` and `Fn`.
    pub(super) fn closure_trait(&self, t: u32) -> Option<ClosureKind> {
        let lang = |n| self.items.lang(n);
        if lang("Fn") == Some(t) {
            Some(ClosureKind::Fn)
        } else if lang("FnMut") == Some(t) {
            Some(ClosureKind::FnMut)
        } else if lang("FnOnce") == Some(t) {
            Some(ClosureKind::FnOnce)
        } else {
            None
        }
    }

    /// `<ty as FnOnce<args>>::name`: a closure's `Output` (index 0) or `Fails` (index 1).
    pub(super) fn closure_assoc(&self, ty: &Ty, args: &Ty, index: u32) -> Ty {
        let once = self.items.lang("FnOnce").expect("the prelude's `FnOnce`");
        Ty::Assoc(Arc::new(AssocTy {
            self_ty: ty.clone(),
            trait_ref: TraitRef {
                trait_: once,
                name: "FnOnce".into(),
                args: vec![args.clone()].into(),
            },
            index,
            name: if index == 0 { "Output" } else { "Fails" }.into(),
        }))
    }

    /// The parameters' and result types the expected type `hint` gives a closure, from the bound
    /// on the type parameter it is passed for.
    fn closure_hint(&mut self, hint: Option<&Ty>, n: usize) -> (Vec<Ty>, Option<Ty>) {
        let Some(h) = hint.map(|h| self.norm(h)) else {
            return (Vec::new(), None);
        };
        let mut params = Vec::new();
        if let Some(args) = self.obligations.closure_args(&mut self.solver, &h, |t| {
            self.items.lang("Fn") == Some(t)
                || self.items.lang("FnMut") == Some(t)
                || self.items.lang("FnOnce") == Some(t)
        }) {
            params = match self.norm(&args) {
                Ty::Tuple(ts) => ts.to_vec(),
                _ => Vec::new(),
            };
        }
        let mut ret = None;
        for (a, b, _) in &self.assoc_eqs {
            if let Ty::Assoc(x) = a
                && x.name.as_ref() == "Output"
                && self.solver.can_unify(&x.self_ty, &h)
                && matches!(self.solver.norm(&x.self_ty), Ty::Infer(_))
            {
                ret = Some(b.clone());
            }
        }
        if params.len() != n {
            params = Vec::new();
        }
        (params, ret)
    }

    /// `|p, …| body` (`expr.closure.1`).
    pub(super) fn closure_expr(
        &mut self,
        e: ExprId,
        params: &[(PatId, Option<Ty>)],
        ret: Option<&Ty>,
        body: ExprId,
    ) -> Ty {
        let hint = self.hint.take().map(|h| h.0);
        let (hint_params, hint_ret) = match self.sig_hint.take() {
            Some(sig) => sig,
            None => self.closure_hint(hint.as_ref(), params.len()),
        };
        let mut ptys = Vec::new();
        for (i, (pat, ty)) in params.iter().enumerate() {
            let t = match ty {
                Some(t) => {
                    let t = self.instantiate(t);
                    if let Some(h) = hint_params.get(i) {
                        self.solver.unify(&t, h);
                    }
                    t
                }
                None => hint_params.get(i).cloned().unwrap_or_else(|| self.fresh()),
            };
            self.check_pat(*pat, &t, None);
            self.deferred.push(super::finish::Deferred::Irrefutable(
                *pat,
                "a closure's parameter",
            ));
            ptys.push(t);
        }
        let rt = match ret {
            Some(t) => {
                let t = self.instantiate(t);
                if let Some(h) = &hint_ret {
                    self.solver.unify(&t, h);
                }
                t
            }
            None => hint_ret.unwrap_or_else(|| self.fresh()),
        };
        let option =
            matches!(self.norm(&rt), Ty::Adt(a, ..) if Some(a) == self.items.lang("Option"));
        let saved_ret = std::mem::replace(&mut self.ret, rt.clone());
        let saved_range = self.ret_range.take();
        let saved_loops = std::mem::take(&mut self.loops);
        self.fail_cx.push(FailCx::Closure {
            errors: Vec::new(),
            option,
        });
        self.check(body, &rt);
        let Some(FailCx::Closure { errors, .. }) = self.fail_cx.pop() else {
            unreachable!("the closure's context")
        };
        self.ret = saved_ret;
        self.ret_range = saved_range;
        self.loops = saved_loops;
        self.closures.push(e);
        Ty::Closure(Arc::new(ClosureTy {
            owner: self.owner,
            expr: index(e) as u32,
            args: self.owner_args.clone().into(),
            params: ptys.into(),
            ret: rt,
            fails: Ty::errors(errors),
            kind: ClosureKind::Fn,
            captures: Arc::from([]),
        }))
    }

    /// `impl Trait { fn … }` (`expr.anon-impl.1`): each method is checked as a closure for the
    /// trait's method's signature; its errors must be ones the trait's method lists.
    pub(super) fn anon_impl(
        &mut self,
        e: ExprId,
        trait_ref: &TraitRef,
        methods: &[(u32, ExprId)],
    ) -> Ty {
        let mut args = vec![Ty::Error];
        args.extend(trait_ref.args.iter().cloned());
        let mut out = Vec::new();
        for &(func, closure) in methods {
            let sig = &self.items.fns[func as usize];
            let params: Vec<Ty> = sig
                .params
                .iter()
                .skip(1)
                .map(|p| p.ty.subst(&args))
                .collect();
            let ret = sig.ret.subst(&args);
            let fails = sig.fails.subst(&args);
            self.sig_hint = Some((params, Some(ret)));
            let t = self.infer(closure);
            self.anon_fails.push((closure, fails));
            out.push((func, t));
        }
        Ty::Anon(Arc::new(crate::ty::AnonTy {
            owner: self.owner,
            expr: index(e) as u32,
            trait_ref: trait_ref.clone(),
            methods: out.into(),
        }))
    }

    /// A call of a closure, or of a value of a type parameter bounded by a closure trait
    /// (`expr.closure.call.1`): its result type, or `None` if `callee` is neither.
    pub(super) fn closure_call(
        &mut self,
        e: ExprId,
        callee: ExprId,
        args: &[ExprId],
    ) -> Option<Ty> {
        let range = self.range(e);
        let t = self.infer(callee);
        let t = self.norm(&t);
        let (params, ret, fails, tuple) = match &t {
            Ty::Closure(c) => {
                let tuple = Ty::tuple(c.params.to_vec());
                (c.params.to_vec(), c.ret.clone(), c.fails.clone(), tuple)
            }
            Ty::Param(..) => {
                let bound = self
                    .solver
                    .env
                    .iter()
                    .filter(|p| p.ty == t)
                    .filter(|p| self.closure_trait(p.trait_ref.trait_).is_some())
                    .map(|p| p.trait_ref.args.first().cloned().unwrap_or(Ty::Unit))
                    .next()?;
                let params = match &bound {
                    Ty::Tuple(ts) => ts.to_vec(),
                    _ => Vec::new(),
                };
                let ret = self.closure_assoc(&t, &bound, 0);
                let fails = self.closure_assoc(&t, &bound, 1);
                (params, ret, fails, bound)
            }
            _ => return None,
        };
        if args.len() != params.len() {
            self.report(
                Code::E0202,
                range,
                format!(
                    "this closure takes {} argument{}, but {} {} given",
                    params.len(),
                    if params.len() == 1 { "" } else { "s" },
                    args.len(),
                    if args.len() == 1 { "was" } else { "were" },
                ),
            );
        }
        for (i, &a) in args.iter().enumerate() {
            match params.get(i) {
                Some(p) => {
                    self.check(a, p);
                }
                None => {
                    self.infer(a);
                }
            }
        }
        let fails = self.solver.deep(&fails);
        // A closure parameter's errors are not the function's to handle: an unmarked call of one
        // passes them on, and its caller's call is marked (`expr.closure.fails.1`).
        let passes_on =
            matches!(t, Ty::Param(..)) && fails.members().iter().all(|m| matches!(m, Ty::Assoc(_)));
        if fails != Ty::Never {
            if self.marked == Some(e) {
                self.call_fails = Some(fails);
            } else if passes_on {
                self.out.passes_on.insert(e);
                self.deliver(&fails.members(), range);
            } else {
                let shown = self.show(&fails);
                self.report(
                    Code::E0501,
                    range,
                    format!(
                        "this call can fail with `{shown}`: propagate the error with `?`, handle \
                         it with `try` and `catch`, or say it cannot happen with `?!`"
                    ),
                );
            }
        }
        self.closure_calls.push((e, callee, t, tuple));
        Some(self.norm(&ret))
    }

    /// Settles the bounds' fixed associated types (`F: FnOnce<…, Output = R>` makes the
    /// closure's result `R`) once their types are known.
    pub(super) fn settle_assoc_eqs(&mut self) {
        let eqs = std::mem::take(&mut self.assoc_eqs);
        let mut keep = Vec::new();
        for (a, b, range) in eqs {
            let n = self.norm(&a);
            if let Ty::Assoc(x) = &n
                && matches!(self.norm(&x.self_ty), Ty::Infer(_))
            {
                keep.push((a, b, range));
                continue;
            }
            if !self.solver.unify(&n, &b) {
                let (n, b) = (self.show(&n), self.show(&b));
                self.report(
                    Code::E0201,
                    range,
                    format!("expected `{b}`, as the bound says, found `{n}`"),
                );
            }
        }
        self.assoc_eqs.extend(keep);
    }

    /// Finds what each closure captures and so its kind, completes the closures' types, chooses
    /// the method each closure call runs, and checks the kinds the bounds need.
    pub(super) fn finish_closures(&mut self) {
        if self.closures.is_empty() && self.closure_calls.is_empty() && self.kind_reqs.is_empty() {
            return;
        }
        let parents: HashMap<ExprId, Option<ExprId>> = self
            .body
            .exprs
            .iter()
            .filter_map(|(id, x)| match &x.kind {
                ExprKind::Closure { parent, .. } => Some((id, *parent)),
                _ => None,
            })
            .collect();
        let mut finals: HashMap<u32, Arc<ClosureTy>> = HashMap::new();
        let mut kinds: HashMap<ExprId, ClosureKind> = HashMap::new();
        for &c in &self.closures.clone() {
            let ExprKind::Closure { body, .. } = &self.body.exprs[c].kind else {
                continue;
            };
            let mut walk = Walk {
                cx: self,
                closure: c,
                parents: &parents,
                kinds: &kinds,
                locals: BTreeMap::new(),
                visited: BTreeSet::new(),
            };
            walk.expr(*body, Use::Value);
            let (locals, visited) = (walk.locals, walk.visited);
            let mut captures: Vec<Capture> =
                locals.iter().map(|(&l, &m)| Capture::Local(l, m)).collect();
            let mut evidence = BTreeSet::new();
            for x in &visited {
                for ev in self.out.evidence(*x) {
                    match *ev {
                        Evidence::Param(i) if i != usize::MAX => {
                            evidence.insert((0, i, 0));
                        }
                        Evidence::Install(w, k) if !visited.contains(&w) => {
                            evidence.insert((1, index(w), k));
                        }
                        _ => {}
                    }
                }
            }
            let installs: HashMap<usize, ExprId> = self
                .body
                .exprs
                .iter()
                .map(|(id, _)| (index(id), id))
                .collect();
            for (kind, a, b) in evidence {
                captures.push(Capture::Evidence(if kind == 0 {
                    Evidence::Param(a)
                } else {
                    Evidence::Install(installs[&a], b)
                }));
            }
            let kind = if locals.values().any(|m| *m == CaptureMode::Move) {
                ClosureKind::FnOnce
            } else if locals.values().any(|m| *m == CaptureMode::Mut) {
                ClosureKind::FnMut
            } else {
                ClosureKind::Fn
            };
            kinds.insert(c, kind);
            let types: Vec<Ty> = captures
                .iter()
                .map(|cap| match *cap {
                    Capture::Local(l, m) => {
                        let t = finalize(&self.out.local_types[index(l)], &finals);
                        match m {
                            CaptureMode::Shared => Ty::reference(false, t),
                            CaptureMode::Mut => Ty::reference(true, t),
                            CaptureMode::Move => t,
                        }
                    }
                    Capture::Evidence(Evidence::Param(i)) => {
                        Ty::reference(false, self.own.get(i).cloned().unwrap_or(Ty::Error))
                    }
                    Capture::Evidence(Evidence::Install(w, k)) => match &self.body.exprs[w].kind {
                        ExprKind::With { handlers, .. } => {
                            Ty::reference(false, handlers[k].effect.clone())
                        }
                        _ => Ty::Error,
                    },
                })
                .collect();
            let Ty::Closure(old) = &self.out.expr_types[index(c)] else {
                continue;
            };
            let mut new = old.map(&mut |t| finalize(t, &finals));
            new.kind = kind;
            new.captures = types.into();
            finals.insert(index(c) as u32, Arc::new(new));
            self.out.captures.insert(c, captures);
        }
        // Every type the body recorded, with the closures' completed.
        let f = |t: &Ty| finalize(t, &finals);
        for t in &mut self.out.expr_types {
            *t = f(t);
        }
        for t in &mut self.out.pat_types {
            *t = f(t);
        }
        for t in &mut self.out.local_types {
            *t = f(t);
        }
        for c in self.out.calls.values_mut() {
            *c = c.map(&mut |t| f(t));
        }
        // The method each closure call runs: by the closure's kind, or the strongest bound.
        for (e, _callee, t, tuple) in std::mem::take(&mut self.closure_calls) {
            let t = f(&self.solver.deep(&t));
            let tuple = f(&super::finish::erase(&self.solver.deep(&tuple)));
            let kind = match &t {
                Ty::Closure(c) => c.kind,
                _ => self
                    .solver
                    .env
                    .iter()
                    .filter(|p| p.ty == t)
                    .filter_map(|p| self.closure_trait(p.trait_ref.trait_))
                    .min()
                    .unwrap_or(ClosureKind::FnOnce),
            };
            let name = match kind {
                ClosureKind::Fn => "Fn",
                ClosureKind::FnMut => "FnMut",
                ClosureKind::FnOnce => "FnOnce",
            };
            let Some(tr) = self.items.lang(name) else {
                continue;
            };
            let func = self.items.traits[tr as usize].methods[0];
            self.out.calls.insert(
                e,
                Callee::Trait {
                    func,
                    args: vec![t, tuple],
                },
            );
        }
        // An anonymous impl's methods fail only as the trait's do.
        for (closure, allowed) in std::mem::take(&mut self.anon_fails) {
            let Ty::Closure(c) = &self.out.expr_types[index(closure)] else {
                continue;
            };
            let allowed = allowed.members();
            let extra: Vec<String> = c
                .fails
                .members()
                .iter()
                .filter(|m| !allowed.contains(m))
                .map(|m| format!("`{m}`"))
                .collect();
            if !extra.is_empty() {
                self.report(
                    Code::E0502,
                    self.body.exprs[closure].range,
                    format!(
                        "this method fails with {}, which the trait's method does not list",
                        extra.join(", ")
                    ),
                );
            }
        }
        // The kinds the bounds need.
        for (t, tr, range) in std::mem::take(&mut self.kind_reqs) {
            let t = f(&self.solver.deep(&t));
            let (Ty::Closure(c), Some(need)) = (&t, self.closure_trait(tr)) else {
                continue;
            };
            if c.kind > need {
                let (has, why) = match c.kind {
                    ClosureKind::FnMut => ("`FnMut`", "it changes what it captures"),
                    _ => ("only `FnOnce`", "it moves out what it captures"),
                };
                let need = match need {
                    ClosureKind::Fn => "`Fn`",
                    _ => "`FnMut`",
                };
                self.report(
                    Code::E0801,
                    range,
                    format!("this closure is {has}, since {why}, but {need} is needed here"),
                );
            }
        }
    }
}

/// `t` with each closure type in `finals` completed.
fn finalize(t: &Ty, finals: &HashMap<u32, Arc<ClosureTy>>) -> Ty {
    match t {
        Ty::Closure(c) => match finals.get(&c.expr) {
            Some(f) => Ty::Closure(f.clone()),
            None => Ty::Closure(Arc::new(c.map(&mut |x| finalize(x, finals)))),
        },
        t => t.map(&mut |x| finalize(x, finals)),
    }
}

/// A walk over a closure's body, finding the enclosing locals it uses and how.
struct Walk<'w, 'a> {
    cx: &'w Checker<'a>,
    closure: ExprId,
    parents: &'w HashMap<ExprId, Option<ExprId>>,
    kinds: &'w HashMap<ExprId, ClosureKind>,
    locals: BTreeMap<LocalId, CaptureMode>,
    visited: BTreeSet<ExprId>,
}

impl Walk<'_, '_> {
    /// Whether local `l` is declared outside the closure.
    fn outside(&self, l: LocalId) -> bool {
        let mut at = self.cx.body.locals[l].closure;
        while let Some(c) = at {
            if c == self.closure {
                return false;
            }
            at = self.parents.get(&c).copied().flatten();
        }
        true
    }

    fn is_copy(&self, t: &Ty) -> bool {
        crate::traits::Solver::new(self.cx.items, &self.cx.solver.env).is_copy(t)
    }

    fn capture(&mut self, l: LocalId, u: Use) {
        if !self.outside(l) {
            return;
        }
        let t = &self.cx.out.local_types[index(l)];
        let mode = match u {
            Use::Write => CaptureMode::Mut,
            Use::Read => CaptureMode::Shared,
            Use::Value if matches!(t, Ty::Ref(true, _)) => CaptureMode::Mut,
            Use::Value if self.is_copy(t) => CaptureMode::Shared,
            Use::Value => CaptureMode::Move,
        };
        let e = self.locals.entry(l).or_insert(mode);
        *e = (*e).max(mode);
    }

    fn stmt(&mut self, s: &Stmt) {
        match *s {
            Stmt::Let { init, else_, .. } => {
                self.expr(init, Use::Value);
                if let Some(x) = else_ {
                    self.expr(x, Use::Value);
                }
            }
            Stmt::Expr { expr, .. } => self.expr(expr, Use::Read),
            Stmt::Defer(x) | Stmt::ErrDefer(x) => self.expr(x, Use::Value),
        }
    }

    fn expr(&mut self, e: ExprId, u: Use) {
        self.visited.insert(e);
        let kind = self.cx.body.exprs[e].kind.clone();
        match kind {
            ExprKind::Local(l) => self.capture(l, u),
            ExprKind::Ref { mutable, expr } => {
                self.expr(expr, if mutable { Use::Write } else { Use::Read });
            }
            ExprKind::Index { base, index: i } => {
                let ft = &self.cx.out.expr_types[index(e)];
                let inner = match u {
                    Use::Value if !self.is_copy(ft) => Use::Value,
                    Use::Write => Use::Write,
                    _ => Use::Read,
                };
                self.expr(base, inner);
                self.expr(i, Use::Value);
            }
            ExprKind::Field { base, .. } => {
                let ft = &self.cx.out.expr_types[index(e)];
                let inner = match u {
                    Use::Value if !self.is_copy(ft) => Use::Value,
                    Use::Write => Use::Write,
                    _ => Use::Read,
                };
                self.expr(base, inner);
            }
            ExprKind::Deref(x) => self.expr(
                x,
                if u == Use::Write {
                    Use::Write
                } else {
                    Use::Read
                },
            ),
            ExprKind::Assign { target, value, .. } => {
                self.expr(target, Use::Write);
                self.expr(value, Use::Value);
            }
            ExprKind::MethodCall { receiver, args, .. } => {
                let adjust = self.cx.out.adjust(e);
                let r = match adjust.autoref {
                    Some(true) => Use::Write,
                    Some(false) => Use::Read,
                    None if adjust.deref => Use::Read,
                    None => Use::Value,
                };
                self.expr(receiver, r);
                for a in args {
                    self.expr(a, Use::Value);
                }
            }
            ExprKind::Call { callee, args } => {
                // Calling a closure the body can name borrows it, or moves it, as its kind says.
                let u = match &self.cx.out.expr_types[index(callee)] {
                    Ty::Closure(c) => match self
                        .kinds
                        .iter()
                        .find(|(x, _)| index(**x) as u32 == c.expr)
                        .map(|(_, k)| *k)
                    {
                        Some(ClosureKind::Fn) => Use::Read,
                        Some(ClosureKind::FnMut) => Use::Write,
                        _ => Use::Value,
                    },
                    Ty::Param(..) => Use::Write,
                    _ => Use::Read,
                };
                self.expr(callee, u);
                for a in args {
                    self.expr(a, Use::Value);
                }
            }
            ExprKind::Block { stmts, tail, .. } => {
                for s in &stmts {
                    self.stmt(s);
                }
                if let Some(t) = tail {
                    self.expr(t, u);
                }
            }
            ExprKind::Match { scrutinee, arms } => {
                self.expr(scrutinee, Use::Read);
                for a in arms {
                    if let Some(g) = a.guard {
                        self.expr(g, Use::Value);
                    }
                    self.expr(a.body, u);
                }
            }
            ExprKind::Let { init, .. } => self.expr(init, Use::Read),
            // A hole is written through `Display::fmt(&self, …)`: it is borrowed.
            ExprKind::Fmt(pieces) => {
                for p in pieces {
                    if let crate::hir::FmtPiece::Hole(h) | crate::hir::FmtPiece::Debug(h) = p {
                        self.expr(h, Use::Read);
                    }
                }
            }
            ExprKind::Closure { body, .. } => self.expr(body, Use::Value),
            ExprKind::AnonImpl { methods, .. } => {
                for (_, c) in methods {
                    self.expr(c, Use::Value);
                }
            }
            ExprKind::With { handlers, body } => {
                for h in handlers {
                    self.expr(h.value, Use::Value);
                }
                self.expr(body, u);
            }
            ExprKind::Try { body, catch } => {
                self.expr(body, u);
                for a in catch.iter().flatten() {
                    if let Some(g) = a.guard {
                        self.expr(g, Use::Value);
                    }
                    self.expr(a.body, u);
                }
            }
            other => {
                for x in children(&other) {
                    self.expr(x, Use::Value);
                }
            }
        }
    }
}

/// The sub-expressions of an expression kind not handled on its own above, all used by value.
fn children(k: &ExprKind) -> Vec<ExprId> {
    match k {
        ExprKind::Tuple(xs) | ExprKind::Array(xs) => xs.clone(),
        ExprKind::RepeatN(x, n) => vec![*x, *n],
        ExprKind::Adt { fields, .. } => fields.iter().map(|&(_, x)| x).collect(),
        ExprKind::If { cond, then, else_ } => {
            let mut v = vec![*cond, *then];
            v.extend(*else_);
            v
        }
        ExprKind::Loop { body } => vec![*body],
        ExprKind::While { cond, body } => vec![*cond, *body],
        ExprKind::Break { value, .. } | ExprKind::Return(value) => value.iter().copied().collect(),
        ExprKind::Assert(x)
        | ExprKind::Question(x)
        | ExprKind::Pass(x)
        | ExprKind::Bang(x, _)
        | ExprKind::Fail(x)
        | ExprKind::Discard(x)
        | ExprKind::Repeat(x, _)
        | ExprKind::ConstBlock(x)
        | ExprKind::Unary(_, x) => vec![*x],
        ExprKind::Binary(_, a, b) | ExprKind::Logic(_, a, b) => vec![*a, *b],
        ExprKind::Compare { first, rest } => std::iter::once(*first)
            .chain(rest.iter().map(|&(_, x)| x))
            .collect(),
        _ => Vec::new(),
    }
}

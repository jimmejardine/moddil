//! Expressions: the type of each kind, checked against what is expected where it is known.

use moddil_syntax::TextRange;

use crate::hir::{Arm, ExprId, ExprKind};
use crate::{Callee, Code, Diagnostic, Ty};

use super::finish::Deferred;
use super::{Checker, LoopCx, index};

impl Checker<'_> {
    /// `fmt"…"` (`expr.fmt.1`): each hole's type (or what a borrow in it points to) is
    /// `Display`.
    pub(super) fn fmt(&mut self, pieces: &[crate::hir::FmtPiece]) {
        for p in pieces {
            let (h, tr) = match p {
                crate::hir::FmtPiece::Hole(h) => (h, "Display"),
                crate::hir::FmtPiece::Debug(h) => (h, "Debug"),
                crate::hir::FmtPiece::Lit(_) => continue,
            };
            {
                let t = self.infer(*h);
                let t = match self.norm(&t) {
                    Ty::Ref(_, x) => (*x).clone(),
                    t => t,
                };
                if let Some(d) = &self.items.lang_ref(tr) {
                    self.require(t.clone(), d.clone(), self.range(*h));
                    if let Some((_, func)) = self.op_trait(tr, "fmt") {
                        self.out.fmt_calls.insert(
                            *h,
                            Callee::Trait {
                                func,
                                args: vec![t],
                            },
                        );
                    }
                }
            }
        }
    }

    pub(super) fn infer_inner(&mut self, e: ExprId) -> Ty {
        let range = self.range(e);
        match &self.body.exprs[e].kind {
            ExprKind::Missing => Ty::Error,
            // An exact number until its type is known (`num.lit.1`): checked against it then.
            &ExprKind::Int(v) => {
                self.literals.push((e, v));
                self.solver.table.fresh_lit(crate::Lit::Int)
            }
            ExprKind::Float(_) => self.solver.table.fresh_lit(crate::Lit::Float),
            ExprKind::Bool(_) => Ty::Bool,
            ExprKind::Unit => Ty::Unit,
            ExprKind::Char(_) => Ty::Char,
            ExprKind::Str(_) => Ty::reference(false, Ty::Str),
            ExprKind::Fmt(pieces) => {
                let pieces = pieces.clone();
                self.fmt(&pieces);
                self.items.string_ty()
            }
            ExprKind::Array(elements) => {
                let elements = elements.clone();
                let elem = self.solver.fresh_n(1).remove(0);
                for &x in &elements {
                    self.check(x, &elem);
                }
                let elem = self.norm(&elem);
                if elem.has_ref() {
                    self.report(
                        Code::E0309,
                        range,
                        "an array cannot hold a borrow: borrows live only in parameters and locals",
                    );
                    return Ty::Error;
                }
                Ty::array(elem, Ty::Len(elements.len() as i64))
            }
            ExprKind::Repeat(value, n) => {
                let (value, n) = (*value, n.clone());
                let t = self.infer(value);
                let t = self.norm(&t);
                // Each element is a copy of the one value (`expr.array.repeat.1`).
                if let Some(copy) = self.items.lang_ref("Copy") {
                    self.require(t.clone(), copy, self.range(value));
                }
                Ty::array(t, n)
            }
            &ExprKind::Index { base, index } => self.index(e, base, index),
            ExprKind::RepeatN(..) => {
                self.report(
                    Code::E0201,
                    range,
                    "the length of an array is known when it is compiled: a literal or a const \
                     parameter (a `Vec` can be made with any length: `let v: Vec<T> = [x; n];`)",
                );
                Ty::Error
            }
            ExprKind::ConstParam(_) => Ty::Int,
            &ExprKind::ConstBlock(x) => self.infer(x),
            &ExprKind::Refine(x, t) => {
                let base = self.items.types[t as usize].ty.clone();
                self.check(x, &base);
                Ty::Refined(t, self.items.types[t as usize].name.as_str().into())
            }
            &ExprKind::Local(l) => self.out.local_types[index(l)].clone(),
            ExprKind::Fn { .. } | ExprKind::AssocFn { .. } | ExprKind::TraitFn { .. } => {
                self.out
                    .diagnostics
                    .push(Diagnostic::unsupported(range, "a function used as a value"));
                Ty::Error
            }
            ExprKind::Tuple(elements) => {
                let ts: Vec<Ty> = elements.iter().map(|&x| self.infer(x)).collect();
                // A tuple holding a borrow is a view (`view.infect.1`).
                let ts: Vec<Ty> = ts.iter().map(|t| self.norm(t)).collect();
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
                self.solver.table.default_literal(&t);
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
                    if !self.items.adts[adt as usize].is_enum {
                        self.field_visible(adt, i, self.range(value));
                    }
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
            ExprKind::Select(arms) => {
                let arms = arms.clone();
                self.select_expr(&arms)
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
            &ExprKind::Question(x) => self.question(e, x),
            &ExprKind::Pass(x) => self.pass(e, x),
            &ExprKind::Bang(x, _) => self.bang(e, x),
            &ExprKind::Fail(x) => self.fail(e, x),
            &ExprKind::Discard(x) => self.discard(x),
            ExprKind::Try { body, catch } => {
                let (body, catch) = (*body, catch.clone());
                self.try_expr(e, body, catch.as_deref())
            }
            ExprKind::With { handlers, body } => {
                let (handlers, body) = (handlers.clone(), *body);
                self.with_expr(e, &handlers, body)
            }
            ExprKind::AnonImpl { trait_ref, methods } => {
                let (trait_ref, methods) = (trait_ref.clone(), methods.clone());
                self.anon_impl(e, &trait_ref, &methods)
            }
            ExprKind::Closure {
                params, ret, body, ..
            } => {
                let (params, ret, body) = (params.clone(), ret.clone(), *body);
                self.closure_expr(e, &params, ret.as_ref(), body)
            }
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
                            Ty::Int | Ty::Num(_) | Ty::Bool | Ty::Error | Ty::Never => {
                                self.binary(e, range, op, &lt, value);
                            }
                            Ty::Infer(_) if self.solver.table.lit_kind(&lt).is_some() => {
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

    pub(super) fn break_expr(
        &mut self,
        range: TextRange,
        target: ExprId,
        value: Option<ExprId>,
    ) -> Ty {
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
    pub(super) fn check_mutable(&mut self, e: ExprId, what: &str) {
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
            // Changing an element through `IndexMut`: the indexed value is borrowed `&mut`.
            ExprKind::Index { base, .. } if self.out.index_calls.contains_key(&e) => {
                let bt = self.out.expr(base);
                let bt = match self.norm(&bt) {
                    Ty::Ref(_, t) => (*t).clone(),
                    t => t,
                };
                if let Some((tr, func)) = self.op_trait("IndexMut", "index_mut") {
                    if self.solver.implements(&bt, &tr) == crate::traits::Answer::No {
                        let s = self.show(&bt);
                        self.report(
                            Code::E0205,
                            range,
                            format!(
                                "`{s}` cannot be {what} by index: it does not implement `IndexMut`"
                            ),
                        );
                        return;
                    }
                    self.require(bt.clone(), tr, range);
                    self.out.index_calls.insert(
                        e,
                        Callee::Trait {
                            func,
                            args: vec![bt],
                        },
                    );
                }
                if !self.out.field_derefs.contains(&e) {
                    self.check_mutable(base, what);
                } else if let Ty::Ref(false, _) = self.norm(&self.out.expr(base)) {
                    self.report(
                        Code::E0302,
                        range,
                        format!("this is behind a shared borrow (`&`), so it cannot be {what}"),
                    );
                }
            }
            ExprKind::Field { base, .. } | ExprKind::Index { base, .. }
                if !self.out.field_derefs.contains(&e) =>
            {
                self.check_mutable(base, what);
            }
            ExprKind::Field { base: x, .. }
            | ExprKind::Index { base: x, .. }
            | ExprKind::Deref(x) => {
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

    /// An array literal where another type is expected (`expr.array.from.1`): made by the
    /// expected type's `FromArrayLiteral`, `from_array` for `[a, b]` and `from_repeat` for
    /// `[x; n]`. `None` when it is not one, so it is checked as an array.
    pub(super) fn array_literal(&mut self, e: ExprId, expected: &Ty) -> Option<Ty> {
        let target = self.norm(expected);
        let is_literal = matches!(
            self.body.exprs[e].kind,
            ExprKind::Array(_) | ExprKind::Repeat(..) | ExprKind::RepeatN(..)
        );
        if !is_literal || !matches!(target, Ty::Adt(..) | Ty::Param(..)) {
            return None;
        }
        let elem = self.solver.fresh_n(1).remove(0);
        let tr = self.items.lang_ref("FromArrayLiteral")?;
        let tr = crate::ty::TraitRef {
            args: std::sync::Arc::from([elem.clone()]),
            ..tr
        };
        let range = self.range(e);
        match self.body.exprs[e].kind.clone() {
            ExprKind::Array(xs) => {
                for &x in &xs {
                    self.check(x, &elem);
                }
                let (_, f) = self.op_trait("FromArrayLiteral", "from_array")?;
                self.out.literal_calls.insert(
                    e,
                    Callee::Trait {
                        func: f,
                        args: vec![target.clone(), elem.clone(), Ty::Len(xs.len() as i64)],
                    },
                );
            }
            ExprKind::Repeat(x, _) | ExprKind::RepeatN(x, _) => {
                self.check(x, &elem);
                if let ExprKind::RepeatN(_, n) = self.body.exprs[e].kind {
                    self.check(n, &Ty::Int);
                }
                if let Some(clone) = self.items.lang_ref("Clone") {
                    self.require(elem.clone(), clone, range);
                }
                let (_, f) = self.op_trait("FromArrayLiteral", "from_repeat")?;
                self.out.literal_calls.insert(
                    e,
                    Callee::Trait {
                        func: f,
                        args: vec![target.clone(), elem.clone()],
                    },
                );
            }
            _ => return None,
        }
        self.require(target.clone(), tr, range);
        self.out.expr_types[index(e)] = target.clone();
        Some(target)
    }

    /// A literal where a refined type is expected (`refine.lit.1`): a value of the refinement,
    /// which compiling checks.
    pub(super) fn refined_literal(&mut self, e: ExprId, expected: &Ty) -> Option<Ty> {
        let target = self.norm(expected);
        let Ty::Refined(t, _) = &target else {
            return None;
        };
        let literal = match &self.body.exprs[e].kind {
            ExprKind::Int(_)
            | ExprKind::Float(_)
            | ExprKind::Bool(_)
            | ExprKind::Char(_)
            | ExprKind::Str(_) => true,
            ExprKind::Unary(crate::hir::UnaryOp::Neg, x) => {
                matches!(
                    self.body.exprs[*x].kind,
                    ExprKind::Int(_) | ExprKind::Float(_)
                )
            }
            _ => false,
        };
        if !literal {
            return None;
        }
        let t = *t;
        let base = self.items.types[t as usize].ty.clone();
        let found = self.infer(e);
        if !self.coerce(&found, &base) {
            return None;
        }
        self.out.refined_literals.push((e, t));
        self.out.expr_types[index(e)] = target.clone();
        Some(target)
    }

    /// `base[index]` on an array or a slice (`expr.index.1`), looking through a borrow.
    pub(super) fn index(&mut self, e: ExprId, base: ExprId, index: ExprId) -> Ty {
        let t = self.infer(base);
        let mut bt = self.norm(&t);
        if let Ty::Ref(_, inner) = &bt {
            self.out.field_derefs.insert(e);
            bt = self.norm(inner);
        }
        self.check(index, &Ty::Int);
        match &bt {
            Ty::Error => Ty::Error,
            Ty::Infer(_) => self.unknown(self.range(base), "to index it"),
            Ty::Array(t, _) | Ty::Slice(t) => (**t).clone(),
            // Through `Index` (`expr.index.trait.1`); `IndexMut` where it is changed.
            _ => match self.operator(e, self.range(e), "[]", &bt, ("Index", "index")) {
                Some(t) => {
                    if let Some(c) = self.out.calls.remove(&e) {
                        self.out.index_calls.insert(e, c);
                    }
                    t
                }
                None => Ty::Error,
            },
        }
    }

    pub(super) fn field(
        &mut self,
        e: ExprId,
        base: ExprId,
        name: &str,
        name_range: TextRange,
    ) -> Ty {
        let t = self.infer(base);
        self.solver.table.default_literal(&t);
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
                if let Ty::Adt(a, ..) = &bt {
                    self.field_visible(*a, i, name_range);
                }
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

    pub(super) fn if_expr(&mut self, cond: ExprId, then: ExprId, else_: Option<ExprId>) -> Ty {
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

    pub(super) fn match_expr(&mut self, e: ExprId, scrutinee: ExprId, arms: &[Arm]) -> Ty {
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
}

impl Checker<'_> {
    /// `select { p = op => body, … }` (`task.select.1`): each operation is one a `select` can
    /// wait on (E0217), each pattern matches every value of its result (E0209), and the arms'
    /// values have one type.
    fn select_expr(&mut self, arms: &[crate::hir::SelectArm]) -> Ty {
        let mut result: Option<Ty> = None;
        for arm in arms {
            let t = self.infer(arm.op);
            let call = match self.body.exprs[arm.op].kind {
                ExprKind::Question(x) => x,
                _ => arm.op,
            };
            if !self.selectable(call) {
                self.report(
                    Code::E0217,
                    self.range(arm.op),
                    "a `select` arm waits on `rx.recv()`, `tx.send(v)`, `t.join()`, \
                     `stop_requested()` or a `#[selectable]` function",
                );
            }
            self.check_pat(arm.pat, &t, None);
            self.deferred
                .push(Deferred::Irrefutable(arm.pat, "a `select` arm"));
            let b = self.infer(arm.body);
            result = Some(match result {
                None => b,
                Some(prev) => match self.join(&prev, &b) {
                    Some(j) => j,
                    None => {
                        let (sp, sb) = (self.show(&prev), self.show(&b));
                        self.report(
                            Code::E0201,
                            self.range(arm.body),
                            format!(
                                "the arms of a `select` must have the same type: expected \
                                 `{sp}`, found `{sb}`"
                            ),
                        );
                        prev
                    }
                },
            });
        }
        result.unwrap_or(Ty::Never)
    }

    /// Whether call `e` is of one of the operations a `select` can wait on.
    fn selectable(&self, e: ExprId) -> bool {
        let Some(callee) = self.out.calls.get(&e) else {
            return false;
        };
        let sig = &self.items.fns[callee.func() as usize];
        if sig.selectable {
            return true;
        }
        if sig.src != crate::items::Src::Prelude || !sig.intrinsic {
            return false;
        }
        let owner = match sig.container {
            crate::items::Container::Impl(k) => match &self.items.impls[k as usize].self_ty {
                Ty::Adt(a, ..) => self.items.adts[*a as usize].name.as_str(),
                _ => return false,
            },
            _ => "",
        };
        matches!(
            (owner, sig.name.as_str()),
            ("Receiver", "recv") | ("Sender", "send") | ("Task", "join") | ("", "stop_requested")
        )
    }
}

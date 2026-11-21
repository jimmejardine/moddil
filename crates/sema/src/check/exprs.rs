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
        let display = self.items.lang_ref("Display");
        for p in pieces {
            if let crate::hir::FmtPiece::Hole(h) = p {
                let t = self.infer(*h);
                let t = match self.norm(&t) {
                    Ty::Ref(_, x) => (*x).clone(),
                    t => t,
                };
                if let Some(d) = &display {
                    self.require(t.clone(), d.clone(), self.range(*h));
                    if let Some((_, func)) = self.op_trait("Display", "fmt") {
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
            ExprKind::Char(_) => Ty::Char,
            ExprKind::Str(_) => Ty::reference(false, Ty::Str),
            ExprKind::Fmt(pieces) => {
                let pieces = pieces.clone();
                self.fmt(&pieces);
                Ty::String
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
            &ExprKind::Question(x) => self.question(e, x),
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

    pub(super) fn field(
        &mut self,
        e: ExprId,
        base: ExprId,
        name: &str,
        name_range: TextRange,
    ) -> Ty {
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

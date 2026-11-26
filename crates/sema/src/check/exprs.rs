//! Expressions: the type of each kind, checked against what is expected where it is known.

use moddil_syntax::TextRange;

use crate::hir::{Arm, ExprId, ExprKind};
use crate::{Callee, Code, Diagnostic, Ty};

use super::finish::Deferred;
use super::{Checker, LoopCx, index};

impl Checker<'_> {
    /// `fmt"…"` (`expr.fmt.1`): each hole's type (or what a borrow in it points to) is
    /// `Display`, or `Debug` for `{e:?}`; and its spec fits it (`expr.fmt.spec.2`,
    /// `expr.fmt.spec.3`).
    pub(super) fn fmt(&mut self, pieces: &[crate::hir::FmtPiece]) {
        for p in pieces {
            let crate::hir::FmtPiece::Hole(h, spec) = p else {
                continue;
            };
            let tr = if spec.debug { "Debug" } else { "Display" };
            let t = self.infer(*h);
            let t = match self.norm(&t) {
                Ty::Ref(_, x) => (*x).clone(),
                t => t,
            };
            // `{x:#?}`: written across lines by the prelude's `debug_pretty`
            // (`expr.fmt.debug.2`).
            let pretty = spec.debug && spec.prefix;
            if let Some(d) = &self.items.lang_ref(tr) {
                self.require(t.clone(), d.clone(), self.range(*h));
                if pretty && let Some(func) = self.items.prelude_fn("debug_pretty") {
                    self.out.fmt_calls.insert(
                        *h,
                        Callee::Fn {
                            func,
                            args: vec![t.clone()],
                        },
                    );
                } else if let Some((_, func)) = self.op_trait(tr, "fmt") {
                    self.out.fmt_calls.insert(
                        *h,
                        Callee::Trait {
                            func,
                            args: vec![t.clone()],
                        },
                    );
                }
            }
            let base = self.items.base(&t);
            let number = matches!(base, Ty::Int | Ty::Num(_));
            let float = matches!(base, Ty::Num(n) if n.is_float());
            let text = base == Ty::Str || base == self.items.string_ty();
            // A width or precision named `name$` is an `int`.
            for x in [spec.width_arg, spec.precision_arg].into_iter().flatten() {
                self.check(x, &Ty::Int);
            }
            let integer = matches!(base, Ty::Int) || (number && !float);
            let wrong = if spec.zero && !number {
                Some("`0` pads a number with zeros")
            } else if spec.plus && !number {
                Some("`+` writes a number's sign")
            } else if spec.radix.is_some() && !integer {
                Some("`x`, `X`, `b` and `o` write an integer's digits in base 16, 2 or 8")
            } else if spec.exponent.is_some() && !float {
                Some("`e` and `E` write a float in exponent form")
            } else if spec.has_precision() && !(float || text) {
                Some("a precision is a float's digits or how much of a text is written")
            } else if spec.has_precision() && text && spec.debug {
                Some("a precision cuts text written by `Display`, not by `Debug`")
            } else {
                None
            };
            if let Some(why) = wrong
                && !matches!(base, Ty::Error | Ty::Infer(_))
            {
                self.out
                    .diagnostics
                    .push(Diagnostic::error(Code::E0902, self.range(*h), why));
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
                // An array of borrows is a view (`view.infect.1`), as a tuple of them is.
                let elem = self.norm(&elem);
                Ty::array(elem, Ty::Len(elements.len() as i64))
            }
            ExprKind::Repeat(value, n) => {
                let (value, n) = (*value, n.clone());
                let t = self.infer(value);
                let t = self.norm(&t);
                // Each element is a copy of the one value, or a clone of it (`expr.array.repeat.1`).
                if let Some(clone) = self.items.lang_ref("Clone") {
                    self.require(t.clone(), clone, self.range(value));
                }
                Ty::array(t, n)
            }
            &ExprKind::Index { base, index } => self.index(e, base, index),
            &ExprKind::Slice {
                base,
                start,
                end,
                inclusive,
            } => {
                let t = self.slice(e, base, start, end);
                // A position is between characters, so text is sliced up to one (`text.pos.1`).
                if inclusive && t == Ty::Str {
                    self.report(
                        Code::E0201,
                        range,
                        "text is sliced up to a position, not through one: write `a..b`",
                    );
                }
                t
            }
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
                    // A shared borrow may point to a borrow (`borrow.type.1`); a `&mut` may
                    // not, so that nothing writes a borrow that lives less in its place.
                    Ty::Ref(..) if mutable => {
                        self.report(
                            Code::E0309,
                            range,
                            "a `&mut` cannot point to a borrow, only a shared `&` can: to borrow \
                             what it points to, write `&mut *r`",
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
                    // A raw pointer, only in an `unsafe` block (`expr.unsafe.1`).
                    Ty::Ptr(_, t) => {
                        if !self.body.unsafe_derefs.contains(&e) {
                            self.report(
                                Code::E0218,
                                range,
                                "a raw pointer is dereferenced only in an `unsafe(\"why\") { … }`                                  block",
                            );
                        }
                        (*t).clone()
                    }
                    Ty::Error => Ty::Error,
                    Ty::Infer(_) => self.unknown(range, "to dereference it"),
                    // Through `Deref` (`type.box.1`); `DerefMut` where it is changed.
                    t @ Ty::Adt(..)
                        if self.op_trait("Deref", "deref").is_some_and(|(tr, _)| {
                            self.solver.implements(&t, &tr) != crate::traits::Answer::No
                        }) =>
                    {
                        match self.operator(e, range, "*", &t, ("Deref", "deref")) {
                            Some(t) => {
                                if let Some(c) = self.out.calls.remove(&e) {
                                    self.out.index_calls.insert(e, c);
                                }
                                t
                            }
                            None => Ty::Error,
                        }
                    }
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
            ExprKind::Continue { .. } | ExprKind::Halt(..) => Ty::Never,
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
                let lt = self.read_through(l, &lt, true);
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
                // A local declared without a value is given one by `=` (`stmt.let.uninit.1`).
                let late = op.is_none()
                    && matches!(self.body.exprs[target].kind, ExprKind::Local(l) if self.late.contains(&l));
                // `r += 1` through a borrow is refused below, for its missing `*`.
                let through = op.is_some() && matches!(self.norm(&lt), Ty::Ref(..));
                if !late && !through {
                    self.check_mutable(target, "assigned to");
                }
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
                            // Through its operator trait (`expr.assign.compound.2`): `x = x + y`.
                            Ty::Adt(..)
                                if matches!(
                                    op,
                                    crate::hir::BinaryOp::Add
                                        | crate::hir::BinaryOp::Sub
                                        | crate::hir::BinaryOp::Mul
                                        | crate::hir::BinaryOp::Div
                                        | crate::hir::BinaryOp::Rem
                                ) =>
                            {
                                let t = self.binary(e, range, op, &lt, value);
                                if t != Ty::Error && !self.coerce(&t, &lt) {
                                    let (want, got) = (self.show(&lt), self.show(&t));
                                    self.report(
                                        Code::E0201,
                                        range,
                                        format!(
                                            "`{}=` gives a `{got}`, which cannot be stored in a \
                                             `{want}`",
                                            op.symbol()
                                        ),
                                    );
                                }
                            }
                            Ty::Infer(_) if self.solver.table.lit_kind(&lt).is_some() => {
                                self.binary(e, range, op, &lt, value);
                            }
                            // A write through a borrow is written with `*` (`borrow.deref.2`).
                            Ty::Ref(..) => {
                                self.infer(value);
                                self.report(
                                    Code::E0205,
                                    range,
                                    format!(
                                        "`{}=` writes through a borrow only with `*`: `*r {}= …`",
                                        op.symbol(),
                                        op.symbol()
                                    ),
                                );
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
            // Changing an element through `IndexMut`, or a box's value through `DerefMut`: the
            // indexed or dereferenced value is borrowed `&mut`; and a range of a type sliced
            // through `Index<Range>`.
            ExprKind::Index { base, .. } | ExprKind::Deref(base) | ExprKind::Slice { base, .. }
                if matches!(self.out.index_calls.get(&e), Some(Callee::Trait { .. })) =>
            {
                let bt = self.out.expr(base);
                let bt = match self.norm(&bt) {
                    Ty::Ref(_, t) => (*t).clone(),
                    t => t,
                };
                let (name, method, how) = match self.body.exprs[e].kind {
                    ExprKind::Deref(_) => ("DerefMut", "deref_mut", "through `*`"),
                    ExprKind::Slice { .. } => ("IndexMut", "index_mut", "by a range"),
                    _ => ("IndexMut", "index_mut", "by index"),
                };
                // `IndexMut` takes the index type `Index` did (`Index<Key>`).
                let idx: Vec<Ty> = self.out.index_calls[&e]
                    .args()
                    .iter()
                    .skip(1)
                    .cloned()
                    .collect();
                if let Some((mut tr, func)) = self.op_trait(name, method) {
                    if !idx.is_empty() {
                        tr.args = idx.clone().into();
                    }
                    if self.solver.implements(&bt, &tr) == crate::traits::Answer::No {
                        let s = self.show(&bt);
                        self.report(
                            Code::E0205,
                            range,
                            format!("`{s}` cannot be {what} {how}: it does not implement `{name}`"),
                        );
                        return;
                    }
                    self.require(bt.clone(), tr, range);
                    let mut args = vec![bt];
                    args.extend(idx);
                    self.out.index_calls.insert(e, Callee::Trait { func, args });
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
            // A field changed through `DerefMut` (`expr.field.deref.1`): a `Box`'s, not an
            // `Rc`'s.
            ExprKind::Field { base, .. } if self.out.field_through.contains_key(&e) => {
                let (callee, target) = self.out.field_through[&e].clone();
                let bt = callee.args()[0].clone();
                if let Some((tr, func)) = self.op_trait("DerefMut", "deref_mut") {
                    if self.solver.implements(&bt, &tr) == crate::traits::Answer::No {
                        let s = self.show(&bt);
                        self.report(
                            Code::E0205,
                            range,
                            format!(
                                "a field of `{s}`'s value cannot be {what}: `{s}` does not                                  implement `DerefMut`"
                            ),
                        );
                        return;
                    }
                    self.require(bt.clone(), tr, range);
                    let callee = Callee::Trait {
                        func,
                        args: vec![bt],
                    };
                    self.out.field_through.insert(e, (callee, target));
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
            // Part of a `Vec` changed: through its `as_mut_slice` (`expr.slice.1`).
            ExprKind::Slice { base, .. } if self.out.index_calls.contains_key(&e) => {
                let bt = self.out.expr(base);
                let bt = match self.norm(&bt) {
                    Ty::Ref(_, t) => (*t).clone(),
                    t => t,
                };
                if let Some((func, args)) = self.items.inherent_method(&bt, "as_mut_slice") {
                    self.out.index_calls.insert(e, Callee::Fn { func, args });
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
            ExprKind::Field { base, .. }
            | ExprKind::Index { base, .. }
            | ExprKind::Slice { base, .. }
                if !self.out.field_derefs.contains(&e) =>
            {
                self.check_mutable(base, what);
            }
            ExprKind::Field { base: x, .. }
            | ExprKind::Index { base: x, .. }
            | ExprKind::Slice { base: x, .. }
            | ExprKind::Deref(x) => {
                let t = self.out.expr(x);
                match self.norm(&t) {
                    Ty::Ref(false, _) => self.report(
                        Code::E0302,
                        range,
                        format!("this is behind a shared borrow (`&`), so it cannot be {what}"),
                    ),
                    Ty::Ptr(false, _) => self.report(
                        Code::E0302,
                        range,
                        format!("this is behind a `*const` pointer, so it cannot be {what}"),
                    ),
                    _ => {}
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
        // The element type the expected type's impl gives, if one impl applies: so `[1, 2]` as a
        // `Vec<u8>` is of `u8`s, not of `int`s first.
        let _ = self.solver.implements(&target, &tr);
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
    /// An integer literal where a `Wrapping<T>` is expected (`num.wrap.3`): a `T`, wrapped.
    pub(super) fn wrapped_literal(&mut self, e: ExprId, expected: &Ty) -> Option<Ty> {
        let target = self.norm(expected);
        let Ty::Adt(a, _, args) = &target else {
            return None;
        };
        let def = &self.items.adts[*a as usize];
        if def.name != "Wrapping" || def.src != crate::Src::Prelude {
            return None;
        }
        let literal = match &self.body.exprs[e].kind {
            ExprKind::Int(_) => true,
            ExprKind::Unary(crate::hir::UnaryOp::Neg, x) => {
                matches!(self.body.exprs[*x].kind, ExprKind::Int(_))
            }
            _ => false,
        };
        if !literal {
            return None;
        }
        let inner = args.first()?.clone();
        self.check(e, &inner);
        self.out.wrapped.insert(e, target.clone());
        Some(target)
    }

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

    /// `base[a..b]` (`expr.slice.1`), looking through a borrow: a `[T]` place of an array, a
    /// slice, or what a type's `as_slice` gives (a `Vec`'s), its ends `int`s; or, with `StrPos`
    /// ends, the text between them (`text.pos.1`).
    pub(super) fn slice(
        &mut self,
        e: ExprId,
        base: ExprId,
        start: Option<ExprId>,
        end: Option<ExprId>,
    ) -> Ty {
        let t = self.infer(base);
        let mut bt = self.norm(&t);
        if let Ty::Ref(_, inner) = &bt {
            self.out.field_derefs.insert(e);
            bt = self.norm(inner);
        }
        let ends = |cx: &mut Self, t: &Ty| {
            for x in [start, end].into_iter().flatten() {
                cx.check(x, t);
            }
        };
        let text = self.items.string_ty();
        match &bt {
            Ty::Error => Ty::Error,
            Ty::Infer(_) => self.unknown(self.range(base), "to slice it"),
            Ty::Array(t, _) | Ty::Slice(t) => {
                ends(self, &Ty::Int);
                Ty::Slice(t.clone())
            }
            Ty::Str => {
                let pos = self.strpos_ty();
                ends(self, &pos);
                Ty::Str
            }
            t if *t == text => {
                let pos = self.strpos_ty();
                ends(self, &pos);
                Ty::Str
            }
            // Through `Index<Range>` and the like, for a type with an impl taking the range
            // (`expr.index.trait.1`).
            t if self.range_indexed(t, e, start, end) => {
                ends(self, &Ty::Int);
                self.range_index(e, t, start, end).unwrap_or(Ty::Error)
            }
            t => match self.items.inherent_method(t, "as_slice") {
                // Through its `as_slice`, or `as_mut_slice` where it is changed.
                Some((func, args)) => {
                    ends(self, &Ty::Int);
                    let ret = self.items.fns[func as usize].ret.subst(&args);
                    self.out.index_calls.insert(e, Callee::Fn { func, args });
                    match ret {
                        Ty::Ref(_, inner) => (*inner).clone(),
                        _ => Ty::Error,
                    }
                }
                None => {
                    let t = self.show(t);
                    self.report(
                        Code::E0205,
                        self.range(e),
                        format!(
                            "`{t}` cannot be sliced: only arrays, slices, a `Vec` and text can"
                        ),
                    );
                    Ty::Error
                }
            },
        }
    }

    /// `base[a..b]` on a type `t` with an impl of `Index` for the range's type (`Range`,
    /// `RangeInclusive`, `RangeFrom`): the element type, the call recorded; `None` if it has
    /// none, or the range has no such type (`..b`, `..`).
    fn range_index(
        &mut self,
        e: ExprId,
        t: &Ty,
        start: Option<ExprId>,
        end: Option<ExprId>,
    ) -> Option<Ty> {
        let range_ty = self.range_ty(e, start, end)?;
        let out = self.operator_with(
            e,
            self.range(e),
            "[]",
            t,
            Some(&range_ty),
            ("Index", "index"),
        )?;
        if let Some(c) = self.out.calls.remove(&e) {
            self.out.index_calls.insert(e, c);
        }
        Some(out)
    }

    /// The type of the range in `base[a..b]` at `e`: `Range`, `RangeInclusive` or `RangeFrom`
    /// (`..b` and `..` have none).
    fn range_ty(&self, e: ExprId, start: Option<ExprId>, end: Option<ExprId>) -> Option<Ty> {
        let ExprKind::Slice { inclusive, .. } = self.body.exprs[e].kind else {
            return None;
        };
        let name = match (start.is_some(), end.is_some(), inclusive) {
            (true, true, false) => "Range",
            (true, true, true) => "RangeInclusive",
            (true, false, _) => "RangeFrom",
            _ => return None,
        };
        let adt = self.items.lang(name)?;
        Some(Ty::Adt(adt, name.into(), Vec::new().into()))
    }

    /// Whether `t` has an impl of `Index` for the range in `base[a..b]` at `e`.
    fn range_indexed(
        &mut self,
        t: &Ty,
        e: ExprId,
        start: Option<ExprId>,
        end: Option<ExprId>,
    ) -> bool {
        let Some(range_ty) = self.range_ty(e, start, end) else {
            return false;
        };
        let Some((mut tr, _)) = self.op_trait("Index", "index") else {
            return false;
        };
        tr.args = vec![range_ty].into();
        self.solver.implements(t, &tr) != crate::traits::Answer::No
    }

    /// The prelude's `StrPos`, a position in text (`text.pos.1`).
    pub(super) fn strpos_ty(&self) -> Ty {
        match self.items.lang("StrPos") {
            Some(a) => Ty::Adt(a, "StrPos".into(), Vec::new().into()),
            None => Ty::Error,
        }
    }

    /// `base[index]` on an array or a slice (`expr.index.1`), looking through a borrow.
    pub(super) fn index(&mut self, e: ExprId, base: ExprId, index: ExprId) -> Ty {
        let t = self.infer(base);
        let mut bt = self.norm(&t);
        if let Ty::Ref(_, inner) = &bt {
            self.out.field_derefs.insert(e);
            bt = self.norm(inner);
        }
        match &bt {
            Ty::Error | Ty::Infer(_) | Ty::Array(..) | Ty::Slice(_) => {
                self.check(index, &Ty::Int);
            }
            _ => {}
        }
        match &bt {
            Ty::Error => Ty::Error,
            Ty::Infer(_) => self.unknown(self.range(base), "to index it"),
            Ty::Array(t, _) | Ty::Slice(t) => (**t).clone(),
            // Through `Index` (`expr.index.trait.1`); `IndexMut` where it is changed. The index
            // is an `int` unless it is of a type the impl takes (`Index<Key>`).
            _ => {
                let it = self.index_type(index);
                let r = self.range(e);
                match self.operator_with(e, r, "[]", &bt, Some(&it), ("Index", "index")) {
                    Some(t) => {
                        if let Some(c) = self.out.calls.remove(&e) {
                            self.out.index_calls.insert(e, c);
                        }
                        t
                    }
                    None => Ty::Error,
                }
            }
        }
    }

    /// The type of `index`, indexing a type through `Index`: an integer literal's, and one whose
    /// type is still to be inferred, is `int`.
    fn index_type(&mut self, index: ExprId) -> Ty {
        if matches!(self.body.exprs[index].kind, ExprKind::Int(_)) {
            self.check(index, &Ty::Int);
            return Ty::Int;
        }
        let it = self.infer(index);
        let it = self.norm(&it);
        if matches!(it, Ty::Infer(_)) {
            if !self.coerce(&it, &Ty::Int) {
                self.mismatch(index, &Ty::Int, &it);
            }
            return Ty::Int;
        }
        it
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
        let mut found = match &bt {
            Ty::Error => return Ty::Error,
            Ty::Infer(_) => return self.unknown(self.range(base), "to read a field of it"),
            _ => self.field_of(&bt, name),
        };
        // A field of what the base dereferences to, through `Deref`, if it has none of its own
        // (`expr.field.deref.1`): a `Box`'s or an `Rc`'s value.
        if found.is_none()
            && let Some((through, target)) = self.deref_target(&bt)
            && let Some(f) = self.field_of(&target, name)
            && let Some((trait_ref, func)) = self.op_trait("Deref", "deref")
        {
            self.require(through.clone(), trait_ref, self.range(base));
            let callee = Callee::Trait {
                func,
                args: vec![through],
            };
            self.out.field_through.insert(e, (callee, target.clone()));
            bt = target;
            found = Some(f);
        }
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

    /// Field `name` of a struct or tuple of type `t`: its index and type.
    fn field_of(&self, t: &Ty, name: &str) -> Option<(u32, Ty)> {
        match t {
            Ty::Tuple(ts) => name
                .parse::<usize>()
                .ok()
                .filter(|&i| i < ts.len())
                .map(|i| (i as u32, ts[i].clone())),
            Ty::Adt(a, ..) if !self.items.adts[*a as usize].is_enum => {
                let def = self.items.variant(*a, 0);
                def.field(name)
                    .map(|i| (i, self.items.field_types(t, 0)[i as usize].clone()))
            }
            _ => None,
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

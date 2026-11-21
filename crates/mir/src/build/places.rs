//! Places and operands: what an expression denotes, and how its value is used (copied,
//! moved or reborrowed), keeping each operand's value if a later operand assigns.

use moddil_sema::Ty;
use moddil_sema::hir::{ExprId, ExprKind, FmtPiece, Stmt};

use super::Builder;
use crate::{Const, Operand, Place, Rvalue};

impl Builder<'_> {
    /// Whether evaluating `e` may assign to a local.
    pub(super) fn assigns(&self, e: ExprId) -> bool {
        let any = |xs: &[ExprId]| xs.iter().any(|&x| self.assigns(x));
        match &self.hir.exprs[e].kind {
            ExprKind::Assign { .. } => true,
            ExprKind::Missing
            | ExprKind::Int(_)
            | ExprKind::Bool(_)
            | ExprKind::Unit
            | ExprKind::Char(_)
            | ExprKind::Str(_)
            | ExprKind::ConstParam(_)
            | ExprKind::Local(_)
            | ExprKind::Fn { .. }
            | ExprKind::AssocFn { .. }
            | ExprKind::TraitFn { .. }
            | ExprKind::Closure { .. }
            | ExprKind::AnonImpl { .. }
            | ExprKind::Continue { .. } => false,
            ExprKind::Assert(c)
            | ExprKind::Deref(c)
            | ExprKind::Ref { expr: c, .. }
            | ExprKind::Question(c)
            | ExprKind::Bang(c, _)
            | ExprKind::Fail(c)
            | ExprKind::Discard(c)
            | ExprKind::Repeat(c, _)
            | ExprKind::ConstBlock(c)
            | ExprKind::Refine(c, _) => self.assigns(*c),
            ExprKind::With { handlers, body } => {
                handlers.iter().any(|h| self.assigns(h.value)) || self.assigns(*body)
            }
            ExprKind::Try { body, catch } => {
                self.assigns(*body)
                    || catch
                        .iter()
                        .flatten()
                        .any(|a| self.assigns(a.body) || a.guard.is_some_and(|g| self.assigns(g)))
            }
            ExprKind::Tuple(xs) | ExprKind::Array(xs) => any(xs),
            ExprKind::RepeatN(x, n) => self.assigns(*x) || self.assigns(*n),
            ExprKind::Index { base, index } => self.assigns(*base) || self.assigns(*index),
            ExprKind::Fmt(pieces) => pieces
                .iter()
                .any(|p| matches!(p, FmtPiece::Hole(h) | FmtPiece::Debug(h) if self.assigns(*h))),
            ExprKind::Adt { fields, .. } => fields.iter().any(|&(_, x)| self.assigns(x)),
            ExprKind::Field { base, .. } => self.assigns(*base),
            ExprKind::Call { args, .. } => any(args),
            ExprKind::MethodCall { receiver, args, .. } => self.assigns(*receiver) || any(args),
            ExprKind::Block { stmts, tail, .. } => {
                stmts.iter().any(|s| match *s {
                    Stmt::Let { init, else_, .. } => {
                        self.assigns(init) || else_.is_some_and(|x| self.assigns(x))
                    }
                    Stmt::Expr { expr, .. } | Stmt::Defer(expr) | Stmt::ErrDefer(expr) => {
                        self.assigns(expr)
                    }
                }) || tail.is_some_and(|t| self.assigns(t))
            }
            ExprKind::If { cond, then, else_ } => {
                self.assigns(*cond) || self.assigns(*then) || else_.is_some_and(|x| self.assigns(x))
            }
            // A `let` condition assigns its bindings, but they are new locals no operand reads.
            ExprKind::Let { init, .. } => self.assigns(*init),
            ExprKind::Match { scrutinee, arms } => {
                self.assigns(*scrutinee)
                    || arms
                        .iter()
                        .any(|a| self.assigns(a.body) || a.guard.is_some_and(|g| self.assigns(g)))
            }
            ExprKind::Loop { body } => self.assigns(*body),
            ExprKind::While { cond, body } => self.assigns(*cond) || self.assigns(*body),
            ExprKind::Break { value, .. } | ExprKind::Return(value) => {
                value.is_some_and(|v| self.assigns(v))
            }
            ExprKind::Unary(_, x) => self.assigns(*x),
            ExprKind::Binary(_, l, r) | ExprKind::Logic(_, l, r) => {
                self.assigns(*l) || self.assigns(*r)
            }
            ExprKind::Compare { first, rest } => {
                self.assigns(*first) || rest.iter().any(|&(_, x)| self.assigns(x))
            }
        }
    }

    /// The place `e` denotes without evaluating anything, if it is a local, or fields and
    /// dereferences of one.
    pub(super) fn place_of(&mut self, e: ExprId) -> Option<Place> {
        if self.coercion(e).is_some() {
            return None;
        }
        match self.hir.exprs[e].kind {
            ExprKind::Local(l) => Some(self.local_place(l)),
            ExprKind::Field { base, .. } => {
                let i = self.types.field_index(e)?;
                let p = self.place_of(base)?;
                let p = if self.types.field_through_borrow(e) {
                    p.deref()
                } else {
                    p
                };
                Some(p.field(i))
            }
            ExprKind::Deref(x) => Some(self.place_of(x)?.deref()),
            _ => None,
        }
    }

    /// The place `e` denotes, evaluating what is not a place (a call returning a borrow, a
    /// temporary to borrow) into a temporary first.
    pub(super) fn place(&mut self, e: ExprId) -> Place {
        if let Some(p) = self.place_of(e) {
            return p;
        }
        match self.hir.exprs[e].kind {
            ExprKind::Field { base, .. } => {
                let i = self.types.field_index(e).expect("a checked field");
                let p = self.place(base);
                let p = if self.types.field_through_borrow(e) {
                    p.deref()
                } else {
                    p
                };
                p.field(i)
            }
            ExprKind::Deref(x) => self.place(x).deref(),
            ExprKind::Index { base, index } if self.types.index_callee(e).is_some() => {
                // `*Index::index(&base, i)`, or `index_mut` with `&mut base`.
                let callee = self.types.index_callee(e).expect("checked").clone();
                let mutable = self
                    .items
                    .fns
                    .get(callee.func() as usize)
                    .is_some_and(|f| f.name == "index_mut");
                let p = self.place(base);
                let p = if self.types.field_through_borrow(e) {
                    p.deref()
                } else {
                    p
                };
                let base_ty = callee.args()[0].clone();
                let b = self.temp(Ty::reference(mutable, base_ty));
                self.cfg.assign(b, Rvalue::Ref(mutable, p));
                let i = self.temp(Ty::Int);
                self.eval_into(i, index);
                let r = self.temp(Ty::reference(mutable, self.ty(e)));
                let range = self.range(e);
                self.call_into(
                    None,
                    callee,
                    Place::local(r),
                    vec![
                        Operand::Move(Place::local(b)),
                        Operand::Copy(Place::local(i)),
                    ],
                    range,
                );
                Place::local(r).deref()
            }
            ExprKind::Index { base, index } => {
                let p = self.place(base);
                let p = if self.types.field_through_borrow(e) {
                    p.deref()
                } else {
                    p
                };
                let i = self.temp(Ty::Int);
                self.eval_into(i, index);
                let ok = self.temp(Ty::Bool);
                self.cfg.assign(
                    ok,
                    Rvalue::InBounds(Operand::Copy(Place::local(i)), p.clone()),
                );
                let range = self.range(e);
                self.cfg.push(crate::Statement::Assert {
                    cond: Operand::Copy(Place::local(ok)),
                    expected: true,
                    trap: crate::TrapClass::IndexOutOfBounds,
                    range,
                    parts: Vec::new(),
                });
                p.index(i)
            }
            _ => {
                let t = self.temp(self.value_ty(e));
                self.eval_into(t, e);
                Place::local(t)
            }
        }
    }

    /// The operand that uses the value at `place` of type `ty`: a copy for `Copy` types, a fresh
    /// reborrow for `&mut` borrows (so using one does not use it up), and otherwise a move.
    pub(super) fn use_place(&mut self, place: Place, ty: &Ty) -> Operand {
        match &self.solver.norm(ty) {
            Ty::Ref(true, inner) => {
                let t = self.temp(Ty::Ref(true, inner.clone()));
                self.cfg.assign(t, Rvalue::Ref(true, place.deref()));
                Operand::Move(Place::local(t))
            }
            t if self.is_copy(t) => Operand::Copy(place),
            _ => Operand::Move(place),
        }
    }

    /// `e` as an operand. If an expression evaluated later (`later`) may assign to a local, a
    /// place's value is taken into a temporary first, so the operand keeps the value it had here.
    pub(super) fn operand(&mut self, e: ExprId, later: &[ExprId]) -> Operand {
        if let Some(target) = self.coercion(e) {
            let shared = matches!(target, Ty::Ref(false, _));
            let t = self.temp(target);
            self.eval_into(t, e);
            return if shared {
                Operand::Copy(Place::local(t))
            } else {
                Operand::Move(Place::local(t))
            };
        }
        if let Some(&(v, _)) = self.recording.get(&e) {
            self.eval_into(v, e);
            return Operand::Copy(Place::local(v));
        }
        match self.hir.exprs[e].kind {
            ExprKind::Int(v) => return Operand::Const(Const::Int(v as i64)),
            ExprKind::Bool(v) => return Operand::Const(Const::Bool(v)),
            ExprKind::Unit => return Operand::Const(Const::Unit),
            ExprKind::Char(c) => return Operand::Const(Const::Char(c)),
            ExprKind::Str(ref s) => {
                let s = s.clone();
                return Operand::Const(self.cfg.str_const(&s));
            }
            _ => {}
        }
        if !later.iter().any(|&x| self.assigns(x))
            && let Some(place) = self.place_of(e)
        {
            let ty = self.ty(e);
            return self.use_place(place, &ty);
        }
        let ty = self.ty(e);
        let t = self.temp(ty.clone());
        self.eval_into(t, e);
        if self.is_copy(&ty) {
            Operand::Copy(Place::local(t))
        } else {
            Operand::Move(Place::local(t))
        }
    }

    /// The operands of `xs`, evaluated left to right.
    pub(super) fn operands(&mut self, xs: &[ExprId]) -> Vec<Operand> {
        (0..xs.len())
            .map(|i| self.operand(xs[i], &xs[i + 1..]))
            .collect()
    }

    /// The place holding the value `e` matched: `e`'s own place if it is one (matching does not
    /// move it; bindings move or borrow its parts), otherwise a temporary.
    pub(super) fn scrutinee(&mut self, e: ExprId) -> Place {
        if let Some(p) = self.place_of(e) {
            return p;
        }
        let t = self.temp(self.value_ty(e));
        self.eval_into(t, e);
        Place::local(t)
    }
}

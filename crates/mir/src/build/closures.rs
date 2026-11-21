//! Closures: the value a closure expression makes (its captures), calls of closures, and the MIR
//! of a closure's own body.
//!
//! A closure's body is a body of its own, with two parameters: its captures, as a tuple (each a
//! borrow of a place the enclosing body has, or a value moved in, then the handlers it uses), and
//! its arguments, as a tuple. Inside it, a captured local is a place in the first: through the
//! borrow, or the value itself.

use moddil_sema::hir::{ExprId, ExprKind};
use moddil_sema::{Callee, Capture, CaptureMode, Evidence, SelfKind, Ty};

use super::Builder;
use crate::{AggregateKind, Const, Local, Operand, Place, Rvalue};

impl Builder<'_> {
    /// The place a local is: its own, or, in a closure's body, where its capture is.
    pub(super) fn local_place(&mut self, l: moddil_sema::hir::LocalId) -> Place {
        match self.captured.get(&l) {
            Some(p) => p.clone(),
            None => Place::local(self.local(l)),
        }
    }

    /// `|p, …| body`: the closure's value, built from what it captures.
    pub(super) fn closure_value(&mut self, dest: Local, e: ExprId) {
        let ty = self.ty(e);
        let mut ops = Vec::new();
        for cap in self.types.captures(e).to_vec() {
            let op = match cap {
                Capture::Local(l, CaptureMode::Move) => {
                    let place = self.local_place(l);
                    let t = self.types.local(l);
                    self.use_place(place, &t)
                }
                Capture::Local(l, mode) => {
                    let m = mode == CaptureMode::Mut;
                    let place = self.local_place(l);
                    let t = self.temp(Ty::reference(m, self.types.local(l)));
                    self.cfg.assign(t, Rvalue::Ref(m, place));
                    Operand::Move(Place::local(t))
                }
                Capture::Evidence(Evidence::Param(i)) => Operand::Copy(self.evidence[&i].clone()),
                Capture::Evidence(Evidence::Install(w, k)) => {
                    Operand::Copy(self.installed[&(w, k)].clone())
                }
            };
            ops.push(op);
        }
        self.cfg
            .assign(dest, Rvalue::Aggregate(AggregateKind::Closure(ty), ops));
    }

    /// Whether call `e` calls a closure (or a value of a type bounded by a closure trait): its
    /// callee is a closure trait's method, and it is not named by a path.
    pub(super) fn closure_call(&self, e: ExprId, callee: ExprId) -> Option<u32> {
        let Some(Callee::Trait { func, .. }) = self.types.callee(e) else {
            return None;
        };
        let t = self.items.trait_of(*func)?;
        let closure = ["Fn", "FnMut", "FnOnce"]
            .iter()
            .any(|n| self.items.lang(n) == Some(t));
        let named = matches!(
            self.hir.exprs[callee].kind,
            ExprKind::Fn { .. } | ExprKind::AssocFn { .. } | ExprKind::TraitFn { .. }
        );
        (closure && !named).then_some(*func)
    }

    /// `f(args)` for a closure `f`: through its trait's method, with `f` borrowed (or moved, for
    /// `call_once`) and the arguments as a tuple.
    pub(super) fn call_closure(
        &mut self,
        e: ExprId,
        func: u32,
        dest: Local,
        callee: ExprId,
        args: &[ExprId],
    ) {
        let recv = match self.items.fns[func as usize].self_kind {
            Some(SelfKind::Ref | SelfKind::RefMut) => {
                let m = self.items.fns[func as usize].self_kind == Some(SelfKind::RefMut);
                let place = self.place(callee);
                let t = self.temp(Ty::reference(m, self.ty(callee)));
                self.cfg.assign(t, Rvalue::Ref(m, place));
                Operand::Move(Place::local(t))
            }
            _ => self.operand(callee, args),
        };
        let tuple = if args.is_empty() {
            Operand::Const(Const::Unit)
        } else {
            let ops = self.operands(args);
            let tys: Vec<Ty> = args.iter().map(|&a| self.ty(a)).collect();
            let t = self.temp(Ty::tuple(tys));
            self.cfg
                .assign(t, Rvalue::Aggregate(AggregateKind::Tuple, ops));
            Operand::Move(Place::local(t))
        };
        self.call(e, dest, vec![recv, tuple]);
    }
}

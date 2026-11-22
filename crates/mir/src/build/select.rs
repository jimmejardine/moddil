//! `select { p = op => body, … }` (`task.select.1`): each arm's operation's operands are
//! evaluated first, in order; then the prelude's `__select` waits until one of the operations
//! is ready, does it, and gives its arm's index, having written through borrows the operation's
//! result, or its error and that it failed. The chosen arm's pattern then binds the result, an
//! error passes on as `?` passes it, and the arm's body gives the value.

use moddil_sema::hir::{ExprId, ExprKind, SelectArm};
use moddil_sema::{Callee, Container, Src, Ty};

use super::Builder;
use crate::{BinOp, Const, Local, Operand, Place, Rvalue, Terminator};

/// What `__select` is told of one arm: what kind of operation, on which handle, with which
/// value to send, and where its result, error and failure go.
struct ArmOp {
    result: Local,
    error: Local,
    error_ty: Ty,
    failed: Local,
    /// Whether the operation is marked with `?`.
    marked: bool,
}

impl Builder<'_> {
    pub(super) fn select(&mut self, e: ExprId, dest: Local, arms: &[SelectArm]) {
        let mut descs = Vec::new();
        let mut desc_tys = Vec::new();
        let mut ops = Vec::new();
        for arm in arms {
            let (call, marked) = match self.hir.exprs[arm.op].kind {
                ExprKind::Question(x) => (x, true),
                _ => (arm.op, false),
            };
            let callee = self
                .types
                .callee(call)
                .expect("a checked operation")
                .clone();
            let func = callee.func();
            let sig = &self.items.fns[func as usize];
            let kind = match (owner_name(self.items, func), sig.name.as_str()) {
                ("Receiver", "recv") => 0,
                ("Sender", "send") => 1,
                ("Task", "join") => 2,
                _ => 3,
            };
            // The handle, borrowed (it is taken only by the arm that runs), and what to send.
            let (handle, handle_ty) = match self.hir.exprs[call].kind.clone() {
                ExprKind::MethodCall { receiver, .. } => {
                    let place = self.scrutinee(receiver);
                    let t = self.value_ty(receiver);
                    let r = self.temp(Ty::reference(false, t.clone()));
                    self.cfg.assign(r, Rvalue::Ref(false, place));
                    (Operand::Move(Place::local(r)), Ty::reference(false, t))
                }
                _ => (Operand::Const(Const::Unit), Ty::Unit),
            };
            let (payload, payload_ty) = match self.hir.exprs[call].kind.clone() {
                ExprKind::MethodCall { args, .. } if kind == 1 => {
                    let t = self.value_ty(args[0]);
                    let held = self.temp(t.clone());
                    self.eval_into(held, args[0]);
                    let r = self.temp(Ty::reference(true, t.clone()));
                    self.cfg.assign(r, Rvalue::Ref(true, Place::local(held)));
                    (Operand::Move(Place::local(r)), Ty::reference(true, t))
                }
                _ => (Operand::Const(Const::Unit), Ty::Unit),
            };
            let result_ty = self.ty(call);
            let fails = self.fx.fns[func as usize].fails.subst(callee.args());
            let error_ty = if fails == Ty::Never { Ty::Unit } else { fails };
            let result = self.temp(result_ty.clone());
            let error = self.temp(error_ty.clone());
            let failed = self.temp(Ty::Bool);
            self.cfg
                .assign(failed, Rvalue::Use(Operand::Const(Const::Bool(false))));
            let mut parts = vec![Operand::Const(Const::Int(kind)), handle, payload];
            let mut tys = vec![Ty::Int, handle_ty, payload_ty];
            for (l, t) in [
                (result, result_ty),
                (error, error_ty.clone()),
                (failed, Ty::Bool),
            ] {
                let r = self.temp(Ty::reference(true, t.clone()));
                self.cfg.assign(r, Rvalue::Ref(true, Place::local(l)));
                parts.push(Operand::Move(Place::local(r)));
                tys.push(Ty::reference(true, t));
            }
            let desc_ty = Ty::tuple(tys);
            let desc = self.temp(desc_ty.clone());
            self.cfg
                .assign(desc, Rvalue::Aggregate(crate::AggregateKind::Tuple, parts));
            descs.push(Operand::Move(Place::local(desc)));
            desc_tys.push(desc_ty);
            ops.push(ArmOp {
                result,
                error,
                error_ty,
                failed,
                marked,
            });
        }
        let all_ty = Ty::tuple(desc_tys);
        let all = self.temp(all_ty.clone());
        self.cfg
            .assign(all, Rvalue::Aggregate(crate::AggregateKind::Tuple, descs));
        let chosen = self.temp(Ty::Int);
        let select = Callee::Fn {
            func: self.prelude_fn("__select"),
            args: vec![all_ty],
        };
        let range = self.range(e);
        self.call_into(
            None,
            select,
            Place::local(chosen),
            vec![Operand::Move(Place::local(all))],
            range,
        );
        // The chosen arm.
        let join = self.cfg.new_block();
        for (i, (arm, op)) in arms.iter().zip(&ops).enumerate() {
            let is = self.temp(Ty::Bool);
            self.cfg.assign(
                is,
                Rvalue::Binary(
                    BinOp::Eq,
                    Operand::Copy(Place::local(chosen)),
                    Operand::Const(Const::Int(i as i64)),
                ),
            );
            let next = self.cfg.new_block();
            self.cfg.branch(Operand::Copy(Place::local(is)), next);
            if op.marked && op.error_ty != Ty::Unit {
                let ok = self.cfg.new_block();
                let bad = self.cfg.new_block();
                self.cfg.terminate(Terminator::SwitchBool {
                    cond: Operand::Copy(Place::local(op.failed)),
                    then: bad,
                    else_: ok,
                });
                self.cfg.continue_in(bad);
                self.raise(Operand::Move(Place::local(op.error)), &op.error_ty.clone());
                self.cfg.continue_in(ok);
            }
            let fail = self.cfg.new_block();
            self.match_pat(arm.pat, &Place::local(op.result), fail);
            self.eval_into(dest, arm.body);
            self.cfg.terminate_into(Terminator::Goto(join), next);
        }
        self.cfg.terminate_into(Terminator::Unreachable, join);
    }

    /// The prelude's free function `name`.
    fn prelude_fn(&self, name: &str) -> u32 {
        self.items
            .fns
            .iter()
            .position(|f| {
                f.src == Src::Prelude && f.name == name && matches!(f.container, Container::Free)
            })
            .expect("a prelude function") as u32
    }
}

/// The name of the type whose impl function `f` is in, or `""` for a free function.
fn owner_name(items: &moddil_sema::ItemTree, f: u32) -> &str {
    match items.fns[f as usize].container {
        Container::Impl(k) => match &items.impls[k as usize].self_ty {
            Ty::Adt(a, ..) => items.adts[*a as usize].name.as_str(),
            _ => "",
        },
        _ => "",
    }
}

//! Errors: where a raised error goes (the enclosing `try` block, a `catch`, an `errdefer`'s
//! suppression, or out of the function), and `?`, `?!`, `fail`, `try` and `catch` as branches
//! on `Result`s, `Option`s and the error edges of calls.

use moddil_sema::hir::{Arm, ExprId, PatKind};
use moddil_sema::{Propagated, Ty};

use super::Builder;
use super::scopes::ScopeItem;
use crate::{
    AggregateKind, BinOp, BlockId, Const, Local, Operand, Place, Rvalue, Terminator, TrapClass,
};

/// Where an error raised in the code being built goes.
#[derive(Clone, Copy)]
pub(super) enum Target {
    /// Out of the function or test.
    Out,
    /// Into a `try` block's result: an `Err` (or `None`) in `dest`, then to `join`.
    Try {
        dest: Local,
        join: BlockId,
        depth: usize,
    },
    /// To a `catch`: the error in `err`, then to `dispatch`.
    Catch {
        err: Local,
        dispatch: BlockId,
        depth: usize,
    },
    /// Suppressed: an `errdefer` block's own errors end it, and are dropped.
    Suppress { join: BlockId, depth: usize },
}

/// What happens to the error of a call a `?` or `?!` is around.
#[derive(Clone)]
pub(super) enum OnError {
    Propagate,
    Trap(String),
}

impl Builder<'_> {
    /// The prelude's `Result` or `Option` with arguments `args`, variant `variant` built from
    /// `value`.
    fn wrap(&mut self, dest: Local, name: &str, variant: u32, value: Vec<Operand>) {
        let adt = self.items.lang(name).expect("a prelude type");
        let path = self.items.adts[adt as usize].path(variant);
        self.cfg.assign(
            dest,
            Rvalue::Aggregate(AggregateKind::Adt { adt, variant, path }, value),
        );
    }

    /// Raises the error `err` where it goes (`err.question.1`): leaving the scopes in between
    /// runs their `errdefer` blocks.
    pub(super) fn raise(&mut self, err: Operand, ty: &Ty) {
        // The error is kept outside every scope, so dropping their values leaves it.
        let e = self.cfg.new_local(ty.clone(), None);
        self.cfg.assign(e, Rvalue::Use(err));
        match *self.targets.last().expect("a target") {
            Target::Out => {
                self.leave_scopes_as(0, true);
                self.cfg
                    .terminate(Terminator::Fail(Operand::Move(Place::local(e))));
            }
            Target::Try { dest, join, depth } => {
                self.leave_scopes_as(depth, true);
                self.wrap(dest, "Result", 1, vec![Operand::Move(Place::local(e))]);
                self.cfg.terminate(Terminator::Goto(join));
            }
            Target::Catch {
                err,
                dispatch,
                depth,
            } => {
                self.leave_scopes_as(depth, true);
                self.cfg
                    .assign(err, Rvalue::Use(Operand::Move(Place::local(e))));
                self.cfg.terminate(Terminator::Goto(dispatch));
            }
            Target::Suppress { join, depth } => {
                self.leave_scopes_as(depth, true);
                self.cfg.terminate(Terminator::Goto(join));
            }
        }
    }

    /// `?` on `None`: the function returns `None`, or the `try` block is `None`. Absence is not
    /// an error, so `errdefer` blocks do not run.
    fn raise_none(&mut self) {
        match *self.targets.last().expect("a target") {
            Target::Try { dest, join, depth } => {
                self.leave_scopes_as(depth, false);
                self.wrap(dest, "Option", 0, Vec::new());
                self.cfg.terminate(Terminator::Goto(join));
            }
            _ => {
                self.wrap(Local::RETURN, "Option", 0, Vec::new());
                self.leave_scopes_as(0, false);
                self.cfg.terminate(Terminator::Return);
            }
        }
    }

    /// `x?` (`trap` is `None`) or `x ?! "reason"`, into `dest`.
    pub(super) fn question(&mut self, e: ExprId, x: ExprId, dest: Local, trap: Option<&str>) {
        let action = match trap {
            Some(r) => OnError::Trap(r.to_string()),
            None => OnError::Propagate,
        };
        match self.types.propagated(e) {
            Some(Propagated::Call) => {
                self.on_error.insert(x, action);
                self.eval_into(dest, x);
            }
            Some(kind @ (Propagated::Result | Propagated::Option)) => {
                let place = self.scrutinee(x);
                let d = self.temp(Ty::Int);
                self.cfg.assign(d, Rvalue::Discriminant(place.clone()));
                // `Ok` is variant 0 of `Result`; `Some` is variant 1 of `Option`.
                let (good, bad) = if kind == Propagated::Result {
                    (0, 1)
                } else {
                    (1, 0)
                };
                let is_good = self.temp(Ty::Bool);
                self.cfg.assign(
                    is_good,
                    Rvalue::Binary(
                        BinOp::Eq,
                        Operand::Copy(Place::local(d)),
                        Operand::Const(Const::Int(good)),
                    ),
                );
                let bad_bb = self.cfg.new_block();
                self.cfg
                    .branch(Operand::Copy(Place::local(is_good)), bad_bb);
                let value = place.downcast(good as u32).field(0);
                let ty = self.ty(e);
                let op = self.use_place(value, &ty);
                self.cfg.assign(dest, Rvalue::Use(op));
                let join = self.cfg.new_block();
                self.cfg.terminate_into(Terminator::Goto(join), bad_bb);
                match (kind, action) {
                    (_, OnError::Trap(r)) => {
                        self.cfg
                            .terminate(Terminator::Trap(TrapClass::Unexpected, r));
                    }
                    (Propagated::Option, OnError::Propagate) => self.raise_none(),
                    (_, OnError::Propagate) => {
                        let err_ty = match self.ty(x) {
                            Ty::Adt(_, _, args) => args[1].clone(),
                            _ => Ty::Error,
                        };
                        let err = place.downcast(bad as u32).field(0);
                        let op = self.use_place(err, &err_ty);
                        self.raise(op, &err_ty);
                    }
                }
                self.cfg.continue_in(join);
            }
            None => unreachable!("a checked `?`"),
        }
    }

    /// The error edge of a call of a function that can fail with `fails`: where its error
    /// goes, if the call is marked with `?` or `?!` (`on_error` was recorded for it).
    pub(super) fn error_edge(&mut self, e: Option<ExprId>, fails: &Ty) -> Option<(Local, OnError)> {
        if *fails == Ty::Never {
            return None;
        }
        let action = e.and_then(|e| self.on_error.remove(&e))?;
        let err = self.cfg.new_local(fails.clone(), None);
        Some((err, action))
    }

    /// Builds what happens on a call's error edge: the error in `err` propagates or traps.
    pub(super) fn on_error_block(&mut self, err: Local, action: OnError) {
        let ty = self.cfg.local_ty(err);
        match action {
            OnError::Propagate => self.raise(Operand::Move(Place::local(err)), &ty),
            OnError::Trap(r) => self
                .cfg
                .terminate(Terminator::Trap(TrapClass::Unexpected, r)),
        }
    }

    /// `try { body }` into `dest`: `Ok` or `Some` of its value, or the `Err` or `None` its `?`s
    /// give (`err.try.1`).
    pub(super) fn try_block(&mut self, dest: Local, body: ExprId, option: bool) {
        let join = self.cfg.new_block();
        self.targets.push(Target::Try {
            dest,
            join,
            depth: self.scopes.depth(),
        });
        let t = self.temp(self.ty(body));
        self.eval_into(t, body);
        self.targets.pop();
        let ty = self.ty(body);
        let op = self.use_place(Place::local(t), &ty);
        if option {
            self.wrap(dest, "Option", 1, vec![op]);
        } else {
            self.wrap(dest, "Result", 0, vec![op]);
        }
        self.cfg.terminate_into(Terminator::Goto(join), join);
    }

    /// `try { body } catch { arms }` into `dest` (`err.catch.1`): an error the arms do not
    /// match keeps propagating.
    pub(super) fn try_catch(&mut self, e: ExprId, dest: Local, body: ExprId, arms: &[Arm]) {
        let set = self.types.try_errors(e).unwrap_or(Ty::Error);
        let err = self.cfg.new_local(set.clone(), None);
        let dispatch = self.cfg.new_block();
        let join = self.cfg.new_block();
        self.targets.push(Target::Catch {
            err,
            dispatch,
            depth: self.scopes.depth(),
        });
        self.eval_into(dest, body);
        self.targets.pop();
        self.cfg.terminate_into(Terminator::Goto(join), dispatch);
        let place = Place::local(err);
        for arm in arms {
            let next = self.cfg.new_block();
            let tested = match &self.hir.pats[arm.pat].kind {
                PatKind::OfType { adt, .. } | PatKind::Adt { adt, .. } => Some(*adt),
                _ => None,
            };
            if let Some(adt) = tested
                && matches!(set, Ty::Errors(_))
            {
                let is = self.temp(Ty::Bool);
                self.cfg.assign(is, Rvalue::ErrorIs(place.clone(), adt));
                self.cfg.branch(Operand::Copy(Place::local(is)), next);
            }
            let moved = self.match_pat(arm.pat, &place, next);
            if let Some(g) = arm.guard {
                let c = self.operand(g, &[]);
                self.guard_branch(c, &moved, next);
            }
            self.eval_into(dest, arm.body);
            self.cfg.terminate_into(Terminator::Goto(join), next);
        }
        // No arm matched: the error keeps propagating.
        self.raise(Operand::Move(place), &set);
        self.cfg.continue_in(join);
    }

    /// An `errdefer` block's body, run while its scope is left with an error: its own errors
    /// are suppressed (`stmt.errdefer.1`).
    pub(super) fn run_errdefer(&mut self, body: ExprId) {
        let join = self.cfg.new_block();
        self.targets.push(Target::Suppress {
            join,
            depth: self.scopes.depth(),
        });
        let t = self.temp(Ty::Unit);
        self.eval_into(t, body);
        self.targets.pop();
        self.cfg.terminate_into(Terminator::Goto(join), join);
    }

    /// Registers an `errdefer` block in the innermost scope.
    pub(super) fn errdefer(&mut self, body: ExprId) {
        self.scopes.push(ScopeItem::ErrDefer(body));
    }
}

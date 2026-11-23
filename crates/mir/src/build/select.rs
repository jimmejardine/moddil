//! `select { p = op => body, … }` (`task.select.1`): each arm's operation's operands are
//! evaluated first, in order; then the prelude's `__select` waits until one of the operations
//! is ready, does it, and gives its arm's index, having written through borrows the operation's
//! result, or its error and that it failed. The chosen arm's pattern then binds the result, an
//! error passes on as `?` passes it, and the arm's body gives the value.
//!
//! An arm on a `#[selectable]` function (`task.select.4`) evaluates the call's arguments once,
//! calls the function's first part for the receiver to wait on, and, if chosen, its second
//! part with what was received: the call's result, or its error.

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
    /// For a `#[selectable]` function: the call, the type of its second part, and its
    /// arguments.
    rest: Option<(ExprId, Ty, Vec<Local>)>,
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
            // A selectable function with no body (a stub mod's) has no parts: it traps.
            let kind = match (owner_name(self.items, func), sig.name.as_str()) {
                _ if sig.selectable && self.selectables.contains_key(&func) => 5,
                _ if sig.selectable => 4,
                ("Receiver", "recv") => 0,
                ("Sender", "send") => 1,
                ("Task", "join") => 2,
                _ => 3,
            };
            // A `#[selectable]` function: its receiver, from its first part.
            let mut rest = None;
            let (handle, handle_ty) = if kind == 5 {
                let (chan, done) = self.selectables[&func].clone();
                let (chan, done) = (chan.subst(callee.args()), done.subst(callee.args()));
                let args = self.selectable_args(call, &chan);
                let rx_ty = match &chan {
                    Ty::Closure(c) => c.ret.clone(),
                    _ => Ty::Error,
                };
                let rx = self.temp(rx_ty.clone());
                self.call_part(None, &chan, &args, None, rx);
                rest = Some((call, done, args));
                (Operand::Copy(Place::local(rx)), rx_ty)
            } else {
                match self.hir.exprs[call].kind.clone() {
                    ExprKind::MethodCall { receiver, .. } => {
                        let place = self.scrutinee(receiver);
                        let t = self.value_ty(receiver);
                        let r = self.temp(Ty::reference(false, t.clone()));
                        self.cfg.assign(r, Rvalue::Ref(false, place));
                        (Operand::Move(Place::local(r)), Ty::reference(false, t))
                    }
                    _ => (Operand::Const(Const::Unit), Ty::Unit),
                }
            };
            let kind = if kind == 5 { 0 } else { kind };
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
            // What is received, for a selectable function: an `Option` of what its receiver
            // carries, which its second part takes last.
            let result_ty = match &rest {
                Some((_, Ty::Closure(c), _)) => c.params.last().cloned().unwrap_or(Ty::Error),
                Some(_) => Ty::Error,
                None => self.ty(call),
            };
            let fails = match &rest {
                Some(_) => Ty::Never,
                None => self.fx.fns[func as usize].fails.subst(callee.args()),
            };
            let error_ty = if fails == Ty::Never { Ty::Unit } else { fails };
            let result = self.temp(result_ty.clone());
            let error = self.temp(error_ty.clone());
            let failed = self.temp(Ty::Bool);
            // Fresh each time the `select` runs: in a loop, the last time's result was moved
            // out by its arm's pattern.
            for l in [result, error] {
                self.cfg.push(crate::Statement::StorageDead(l));
            }
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
                rest,
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
            // A selectable function's second part, with what was received.
            let mut result = op.result;
            if let Some((call, done, args)) = &op.rest {
                let out = self.temp(self.ty(*call));
                self.call_part(Some(*call), done, args, Some(op.result), out);
                result = out;
            }
            let fail = self.cfg.new_block();
            self.match_pat(arm.pat, &Place::local(result), fail);
            self.eval_into(dest, arm.body);
            self.cfg.terminate_into(Terminator::Goto(join), next);
        }
        self.cfg.terminate_into(Terminator::Unreachable, join);
    }

    /// The arguments of selectable call `call`, each evaluated once into a local of the type
    /// part `part` takes it as.
    fn selectable_args(&mut self, call: ExprId, part: &Ty) -> Vec<Local> {
        let params: Vec<Ty> = match part {
            Ty::Closure(c) => c.params.to_vec(),
            _ => Vec::new(),
        };
        let mut ops = Vec::new();
        match self.hir.exprs[call].kind.clone() {
            ExprKind::MethodCall { receiver, args, .. } => {
                let adjust = self.types.adjust(call);
                let recv = match adjust.autoref {
                    Some(m) => {
                        let mut place = self.place(receiver);
                        let inner = match self.ty(receiver) {
                            Ty::Ref(_, t) if adjust.deref => {
                                place = place.deref();
                                (*t).clone()
                            }
                            t => t,
                        };
                        let t = self.temp(Ty::reference(m, inner));
                        self.cfg.assign(t, Rvalue::Ref(m, place));
                        Operand::Move(Place::local(t))
                    }
                    None => self.operand(receiver, &args),
                };
                ops.push(recv);
                ops.extend(self.operands(&args));
            }
            ExprKind::Call { args, .. } => ops.extend(self.operands(&args)),
            _ => {}
        }
        ops.into_iter()
            .enumerate()
            .map(|(i, op)| {
                let t = self.temp(params.get(i).cloned().unwrap_or(Ty::Error));
                self.cfg.assign(t, Rvalue::Use(op));
                t
            })
            .collect()
    }

    /// Calls part `part` of a selectable function, a closure capturing nothing, with `args`
    /// (copied: borrows and `Copy` values) and, for the second part, what was received
    /// (moved), into `dest`; its error goes where call `call`'s would.
    fn call_part(
        &mut self,
        call: Option<ExprId>,
        part: &Ty,
        args: &[Local],
        received: Option<Local>,
        dest: Local,
    ) {
        let range = call.map(|c| self.range(c)).unwrap_or_default();
        let value = self.temp(part.clone());
        self.cfg.assign(
            value,
            Rvalue::Aggregate(crate::AggregateKind::Closure(part.clone()), Vec::new()),
        );
        let r = self.temp(Ty::reference(false, part.clone()));
        self.cfg.assign(r, Rvalue::Ref(false, Place::local(value)));
        let mut ops: Vec<Operand> = args
            .iter()
            .map(|&a| Operand::Copy(Place::local(a)))
            .collect();
        let mut tys: Vec<Ty> = args.iter().map(|&a| self.cfg.local_ty(a)).collect();
        if let Some(got) = received {
            ops.push(Operand::Move(Place::local(got)));
            tys.push(self.cfg.local_ty(got));
        }
        let tuple_ty = Ty::tuple(tys);
        let tuple = self.temp(tuple_ty.clone());
        self.cfg
            .assign(tuple, Rvalue::Aggregate(crate::AggregateKind::Tuple, ops));
        // An error of the rest passes on, as the arm's `?` says (a call that can fail is
        // marked, or is an error).
        if let Some(c) = call {
            self.on_error.insert(c, super::errors::OnError::Propagate);
        }
        let fn_call = self.trait_method("Fn", "call");
        self.call_into(
            call,
            Callee::Trait {
                func: fn_call,
                args: vec![part.clone(), tuple_ty],
            },
            Place::local(dest),
            vec![
                Operand::Move(Place::local(r)),
                Operand::Move(Place::local(tuple)),
            ],
            range,
        );
    }

    /// Method `name` of the prelude's trait `tr`.
    fn trait_method(&self, tr: &str, name: &str) -> u32 {
        let t = self.items.lang(tr).expect("a prelude trait");
        self.items
            .fns
            .iter()
            .position(|f| matches!(f.container, Container::Trait(k) if k == t) && f.name == name)
            .expect("the trait's method") as u32
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

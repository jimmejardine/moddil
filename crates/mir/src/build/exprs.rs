//! Expressions and statements: each HIR expression kind as blocks and statements.

use moddil_sema::hir::{ExprId, ExprKind, FmtPiece, Handler, LogicOp, PatKind, Stmt, UnaryOp};
use moddil_sema::{Callee, Evidence, Ty};
use moddil_syntax::TextRange;

use super::scopes::{LoopCx, ScopeItem};
use super::{Builder, fn_name};
use crate::{
    AggregateKind, AssertPart, Const, Local, Operand, Place, Rvalue, Statement, Terminator,
    TrapClass, UnOp,
};

impl Builder<'_> {
    /// Evaluates `e` and stores its value in `dest`. A part of an `assert`'s condition is
    /// evaluated into its own local first, and marked as evaluated.
    pub(super) fn eval_into(&mut self, dest: Local, e: ExprId) {
        // A borrow used as a `dyn` borrow is converted.
        if let Some(target) = self.coercion(e) {
            let ty = self.ty(e);
            let t = self.cfg.new_local(ty.clone(), None);
            let outer = self.uncoerced.replace(e);
            self.eval_dyn_part(t, e);
            self.uncoerced = outer;
            let op = self.use_place(Place::local(t), &ty);
            let from = match ty {
                Ty::Ref(_, inner) => (*inner).clone(),
                t => t,
            };
            debug_assert!(matches!(target, Ty::Ref(..)));
            self.cfg.assign(dest, Rvalue::Unsize(op, from));
            return;
        }
        self.eval_dyn_part(dest, e);
    }

    /// [`Builder::eval_into`], once any conversion to a `dyn` borrow is done.
    fn eval_dyn_part(&mut self, dest: Local, e: ExprId) {
        // Each part is built once, so it leaves the table as it is built.
        if let Some((v, flag)) = self.recording.remove(&e) {
            self.eval_plain(v, e);
            self.cfg
                .assign(flag, Rvalue::Use(Operand::Const(Const::Bool(true))));
            if dest != v {
                let ty = self.ty(e);
                let op = self.use_place(Place::local(v), &ty);
                self.cfg.assign(dest, Rvalue::Use(op));
            }
            return;
        }
        self.eval_plain(dest, e);
    }

    /// The parts of an `assert`'s condition whose values a failure shows: every sub-expression
    /// that is not a literal, through operators, calls, fields, tuples and struct expressions.
    pub(super) fn assert_parts(&self, e: ExprId, out: &mut Vec<ExprId>) {
        let kind = &self.hir.exprs[e].kind;
        if !matches!(kind, ExprKind::Int(_) | ExprKind::Bool(_) | ExprKind::Unit) {
            out.push(e);
        }
        let children: Vec<ExprId> = match kind {
            ExprKind::Unary(_, x) | ExprKind::Field { base: x, .. } | ExprKind::Deref(x) => {
                vec![*x]
            }
            ExprKind::Binary(_, l, r) | ExprKind::Logic(_, l, r) => vec![*l, *r],
            ExprKind::Compare { first, rest } => std::iter::once(*first)
                .chain(rest.iter().map(|&(_, x)| x))
                .collect(),
            ExprKind::Call { args, .. } | ExprKind::Tuple(args) => args.clone(),
            ExprKind::MethodCall { receiver, args, .. } => std::iter::once(*receiver)
                .chain(args.iter().copied())
                .collect(),
            ExprKind::Adt { fields, .. } => fields.iter().map(|&(_, x)| x).collect(),
            _ => Vec::new(),
        };
        for c in children {
            self.assert_parts(c, out);
        }
    }

    pub(super) fn eval_plain(&mut self, dest: Local, e: ExprId) {
        let span = self.range(e);
        let outer = self.cfg.set_span(span);
        self.eval_kind(dest, e);
        self.cfg.set_span(outer);
    }

    pub(super) fn eval_kind(&mut self, dest: Local, e: ExprId) {
        let range = self.range(e);
        match self.hir.exprs[e].kind.clone() {
            ExprKind::Missing
            | ExprKind::Fn { .. }
            | ExprKind::AssocFn { .. }
            | ExprKind::TraitFn { .. } => {
                unreachable!("MIR is only built for bodies without errors")
            }
            ExprKind::Int(_)
            | ExprKind::Bool(_)
            | ExprKind::Unit
            | ExprKind::Char(_)
            | ExprKind::Str(_)
            | ExprKind::Local(_)
            | ExprKind::Field { .. }
            | ExprKind::Deref(_) => {
                let place = match self.hir.exprs[e].kind {
                    ExprKind::Field { .. } | ExprKind::Deref(_) => Some(self.place(e)),
                    _ => None,
                };
                let op = match place {
                    Some(p) => {
                        let ty = self.ty(e);
                        self.use_place(p, &ty)
                    }
                    None => self.operand(e, &[]),
                };
                self.cfg.assign(dest, Rvalue::Use(op));
            }
            ExprKind::Ref { mutable, expr } => {
                let place = self.place(expr);
                self.cfg.assign(dest, Rvalue::Ref(mutable, place));
            }
            ExprKind::Fmt(pieces) => self.fmt(dest, &pieces, range),
            ExprKind::Tuple(xs) => {
                let ops = self.operands(&xs);
                self.cfg
                    .assign(dest, Rvalue::Aggregate(AggregateKind::Tuple, ops));
            }
            ExprKind::Adt {
                adt,
                variant,
                fields,
                ..
            } => {
                // Evaluated in source order, stored in field order.
                let values: Vec<ExprId> = fields.iter().map(|&(_, x)| x).collect();
                let ops = self.operands(&values);
                let mut ordered = vec![Operand::Const(Const::Unit); fields.len()];
                for (&(i, _), op) in fields.iter().zip(ops) {
                    ordered[i as usize] = op;
                }
                let path = self.items.adts[adt as usize].path(variant);
                self.cfg.assign(
                    dest,
                    Rvalue::Aggregate(AggregateKind::Adt { adt, variant, path }, ordered),
                );
            }
            ExprKind::Block {
                stmts,
                tail,
                observe_tail,
            } => {
                self.scopes.open();
                for s in &stmts {
                    let mark = self.scopes.mark();
                    self.stmt(s);
                    self.drop_temps(mark);
                }
                match tail {
                    Some(t) if observe_tail => {
                        let place = self.place(t);
                        self.cfg.push(Statement::Observe {
                            range: self.range(t),
                            value: Operand::Copy(place.clone()),
                        });
                        let ty = self.ty(t);
                        let op = self.use_place(place, &ty);
                        self.cfg.assign(dest, Rvalue::Use(op));
                    }
                    Some(t) => self.eval_into(dest, t),
                    None => self
                        .cfg
                        .assign(dest, Rvalue::Use(Operand::Const(Const::Unit))),
                }
                let depth = self.scopes.depth() - 1;
                self.leave_scopes(depth);
                self.scopes.close();
            }
            ExprKind::If { cond, then, else_ } => {
                let c = self.operand(cond, &[]);
                let then_bb = self.cfg.new_block();
                let join = self.cfg.new_block();
                let else_bb = match else_ {
                    Some(_) => self.cfg.new_block(),
                    None => {
                        self.cfg
                            .assign(dest, Rvalue::Use(Operand::Const(Const::Unit)));
                        join
                    }
                };
                self.cfg.terminate_into(
                    Terminator::SwitchBool {
                        cond: c,
                        then: then_bb,
                        else_: else_bb,
                    },
                    then_bb,
                );
                self.eval_into(dest, then);
                self.cfg.terminate_into(Terminator::Goto(join), else_bb);
                if let Some(x) = else_ {
                    self.eval_into(dest, x);
                    self.cfg.terminate_into(Terminator::Goto(join), join);
                }
                self.cfg.continue_in(join);
            }
            ExprKind::Let { pat, init } => {
                let place = self.scrutinee(init);
                let fail = self.cfg.new_block();
                let join = self.cfg.new_block();
                self.match_pat(pat, &place, fail);
                self.cfg
                    .assign(dest, Rvalue::Use(Operand::Const(Const::Bool(true))));
                self.cfg.terminate_into(Terminator::Goto(join), fail);
                self.cfg
                    .assign(dest, Rvalue::Use(Operand::Const(Const::Bool(false))));
                self.cfg.terminate_into(Terminator::Goto(join), join);
            }
            ExprKind::Match { scrutinee, arms } => {
                let place = self.scrutinee(scrutinee);
                // While guards run, the matched value is borrowed, so a guard cannot change it.
                let guarded = arms.iter().any(|a| a.guard.is_some());
                let fake = guarded.then(|| {
                    let t = self.temp(Ty::reference(false, self.ty(scrutinee)));
                    self.cfg.assign(t, Rvalue::Ref(false, place.clone()));
                    t
                });
                let join = self.cfg.new_block();
                for arm in &arms {
                    let next = self.cfg.new_block();
                    let moved = self.match_pat(arm.pat, &place, next);
                    if let Some(g) = arm.guard {
                        let c = self.operand(g, &[]);
                        if let Some(f) = fake {
                            self.cfg.push(Statement::Read(Place::local(f)));
                        }
                        self.guard_branch(c, &moved, next);
                    }
                    self.eval_into(dest, arm.body);
                    self.cfg.terminate_into(Terminator::Goto(join), next);
                }
                // The arms are exhaustive, so control never gets past the last one.
                self.cfg.terminate_into(Terminator::Unreachable, join);
            }
            ExprKind::Loop { body } => {
                let header = self.cfg.new_block();
                let exit = self.cfg.new_block();
                self.cfg.terminate_into(Terminator::Goto(header), header);
                self.scopes.enter_loop(LoopCx {
                    id: e,
                    exit,
                    header,
                    dest,
                    depth: self.scopes.depth(),
                });
                let t = self.temp(Ty::Unit);
                self.eval_into(t, body);
                self.scopes.exit_loop();
                self.cfg.terminate_into(Terminator::Goto(header), exit);
            }
            ExprKind::While { cond, body } => {
                let header = self.cfg.new_block();
                let exit = self.cfg.new_block();
                self.cfg.terminate_into(Terminator::Goto(header), header);
                // The condition's `let` bindings live for one iteration.
                self.scopes.open();
                let c = self.operand(cond, &[]);
                let body_bb = self.cfg.new_block();
                let leave = self.cfg.new_block();
                self.cfg.terminate_into(
                    Terminator::SwitchBool {
                        cond: c,
                        then: body_bb,
                        else_: leave,
                    },
                    body_bb,
                );
                self.scopes.enter_loop(LoopCx {
                    id: e,
                    exit,
                    header,
                    dest,
                    depth: self.scopes.depth() - 1,
                });
                let t = self.temp(Ty::Unit);
                self.eval_into(t, body);
                self.scopes.exit_loop();
                let depth = self.scopes.depth() - 1;
                self.leave_scopes(depth);
                self.cfg.terminate_into(Terminator::Goto(header), leave);
                self.leave_scopes(depth);
                self.scopes.close();
                self.cfg.terminate_into(Terminator::Goto(exit), exit);
                self.cfg
                    .assign(dest, Rvalue::Use(Operand::Const(Const::Unit)));
            }
            ExprKind::Break { target, value } => {
                let cx = self.scopes.loop_cx(target);
                let (exit, loop_dest, depth) = (cx.exit, cx.dest, cx.depth);
                let is_while = matches!(self.hir.exprs[target].kind, ExprKind::While { .. });
                match value {
                    Some(v) => self.eval_into(loop_dest, v),
                    None if !is_while => {
                        self.cfg
                            .assign(loop_dest, Rvalue::Use(Operand::Const(Const::Unit)));
                    }
                    None => {}
                }
                self.leave_scopes(depth);
                self.cfg.terminate(Terminator::Goto(exit));
            }
            ExprKind::Continue { target } => {
                let cx = self.scopes.loop_cx(target);
                let (header, depth) = (cx.header, cx.depth);
                self.leave_scopes(depth);
                self.cfg.terminate(Terminator::Goto(header));
            }
            ExprKind::Question(x) => self.question(e, x, dest, None),
            ExprKind::Bang(x, reason) => self.question(e, x, dest, Some(&reason)),
            ExprKind::Fail(x) => {
                let ty = self.ty(x);
                let op = self.operand(x, &[]);
                self.raise(op, &ty);
            }
            ExprKind::Discard(x) => {
                let t = self.temp(self.ty(x));
                self.eval_into(t, x);
                self.cfg
                    .assign(dest, Rvalue::Use(Operand::Const(Const::Unit)));
            }
            ExprKind::Try { body, catch: None } => {
                let option = matches!(
                    self.ty(e),
                    Ty::Adt(a, ..) if Some(a) == self.items.lang("Option")
                );
                self.try_block(dest, body, option);
            }
            ExprKind::Try {
                body,
                catch: Some(arms),
            } => self.try_catch(e, dest, body, &arms),
            ExprKind::With { handlers, body } => self.with_into(e, dest, &handlers, body),
            ExprKind::Return(v) => {
                match v {
                    Some(v) => self.eval_into(Local::RETURN, v),
                    None => self
                        .cfg
                        .assign(Local::RETURN, Rvalue::Use(Operand::Const(Const::Unit))),
                }
                self.leave_scopes(0);
                self.cfg.terminate(Terminator::Return);
            }
            ExprKind::Assert(cond) => {
                let mut exprs = Vec::new();
                self.assert_parts(cond, &mut exprs);
                let mut parts = Vec::new();
                for &x in &exprs {
                    let value = self.temp(self.ty(x));
                    let evaluated = self.temp(Ty::Bool);
                    self.cfg
                        .assign(evaluated, Rvalue::Use(Operand::Const(Const::Bool(false))));
                    self.recording.insert(x, (value, evaluated));
                    parts.push(AssertPart {
                        range: self.range(x),
                        value,
                        evaluated,
                    });
                }
                let c = self.operand(cond, &[]);
                for x in &exprs {
                    self.recording.remove(x);
                }
                self.cfg.push(Statement::Assert {
                    cond: c,
                    expected: true,
                    trap: TrapClass::Assertion,
                    range,
                    parts,
                });
                self.cfg
                    .assign(dest, Rvalue::Use(Operand::Const(Const::Unit)));
            }
            ExprKind::Unary(_, x) if self.types.callee(e).is_some() => {
                let a = self.operand(x, &[]);
                self.call(e, dest, vec![a]);
            }
            ExprKind::Binary(_, l, r) if self.types.callee(e).is_some() => {
                let a = self.operand(l, &[r]);
                let b = self.operand(r, &[]);
                self.call(e, dest, vec![a, b]);
            }
            ExprKind::Unary(op, x) => {
                let a = self.operand(x, &[]);
                if op == UnaryOp::Neg {
                    let t = self.temp(Ty::Bool);
                    self.cfg.assign(t, Rvalue::NegOverflows(a.clone()));
                    self.check(t, TrapClass::Overflow, range);
                }
                let op = match op {
                    UnaryOp::Neg => UnOp::Neg,
                    UnaryOp::Not => UnOp::Not,
                };
                self.cfg.assign(dest, Rvalue::Unary(op, a));
            }
            ExprKind::Binary(op, l, r) => {
                let a = self.operand(l, &[r]);
                let b = self.operand(r, &[]);
                self.arith(Place::local(dest), op, &self.ty(l), a, b, range);
            }
            ExprKind::Logic(op, l, r) => {
                let a = self.operand(l, &[]);
                let rhs = self.cfg.new_block();
                let short = self.cfg.new_block();
                let join = self.cfg.new_block();
                let (then, else_, value) = match op {
                    LogicOp::And => (rhs, short, false),
                    LogicOp::Or => (short, rhs, true),
                };
                self.cfg.terminate_into(
                    Terminator::SwitchBool {
                        cond: a,
                        then,
                        else_,
                    },
                    short,
                );
                self.cfg
                    .assign(dest, Rvalue::Use(Operand::Const(Const::Bool(value))));
                self.cfg.terminate_into(Terminator::Goto(join), rhs);
                self.eval_into(dest, r);
                self.cfg.terminate_into(Terminator::Goto(join), join);
            }
            ExprKind::Compare { first, rest } => {
                self.compare_expr = e;
                self.compare(dest, first, &rest);
            }
            ExprKind::Assign { target, op, value } => {
                let ty = self.ty(target);
                match op {
                    None => {
                        let t = self.temp(self.ty(value));
                        self.eval_into(t, value);
                        let place = self.place(target);
                        // The old value is dropped before the new one is stored.
                        self.drop_place(place.clone(), &ty, false);
                        let op = self.use_place(Place::local(t), &ty);
                        self.cfg.assign(place, Rvalue::Use(op));
                    }
                    Some(op) => {
                        let place = self.place(target);
                        let a = if self.assigns(value) {
                            let t = self.temp(ty.clone());
                            self.cfg
                                .assign(t, Rvalue::Use(Operand::Copy(place.clone())));
                            Operand::Copy(Place::local(t))
                        } else {
                            Operand::Copy(place.clone())
                        };
                        let b = self.operand(value, &[]);
                        self.arith(place, op, &ty, a, b, range);
                    }
                }
                self.cfg
                    .assign(dest, Rvalue::Use(Operand::Const(Const::Unit)));
            }
            ExprKind::Call { callee, args } => {
                if let Some(func) = self.closure_call(e, callee) {
                    self.call_closure(e, func, dest, callee, &args);
                    return;
                }
                let ops = self.operands(&args);
                self.call(e, dest, ops);
            }
            ExprKind::Closure { .. } => self.closure_value(dest, e),
            ExprKind::AnonImpl { methods, .. } => {
                // Its methods' closures, together.
                let mut ops = Vec::new();
                for (_, c) in methods {
                    let t = self.temp(self.ty(c));
                    self.closure_value(t, c);
                    ops.push(Operand::Move(Place::local(t)));
                }
                self.cfg
                    .assign(dest, Rvalue::Aggregate(AggregateKind::Tuple, ops));
            }
            ExprKind::MethodCall { receiver, args, .. } => {
                let adjust = self.types.adjust(e);
                let recv = match adjust.autoref {
                    Some(m) => {
                        let mut place = self.place(receiver);
                        if adjust.deref {
                            place = place.deref();
                        }
                        let inner = match self.ty(receiver) {
                            Ty::Ref(_, t) if adjust.deref => (*t).clone(),
                            t => t,
                        };
                        let t = self.temp(Ty::reference(m, inner));
                        self.cfg.assign(t, Rvalue::Ref(m, place));
                        Operand::Move(Place::local(t))
                    }
                    None if adjust.deref => {
                        let place = self.place(receiver).deref();
                        let inner = match self.ty(receiver) {
                            Ty::Ref(_, t) => (*t).clone(),
                            t => t,
                        };
                        self.use_place(place, &inner)
                    }
                    None => self.operand(receiver, &args),
                };
                let mut ops = vec![recv];
                ops.extend(self.operands(&args));
                self.call(e, dest, ops);
            }
        }
    }

    /// Calls what `e` (a call, method call or operator) runs, with `args`, into `dest`.
    pub(super) fn call(&mut self, e: ExprId, dest: Local, mut args: Vec<Operand>) {
        let callee = self.types.callee(e).expect("a checked call").clone();
        // The handlers it needs, after its arguments.
        for ev in self.types.evidence(e) {
            let local = match *ev {
                Evidence::Param(i) => self.evidence[&i].clone(),
                Evidence::Install(w, k) => self.installed[&(w, k)].clone(),
            };
            args.push(Operand::Copy(local));
        }
        self.call_into(Some(e), callee, Place::local(dest), args, self.range(e));
    }

    /// `with E = value, … { body }`: each handler is evaluated, then the block runs with them
    /// installed. A handler that is a value lives until the block ends, then is dropped; each
    /// installed handler is a borrow (as `dyn`, for a trait).
    fn with_into(&mut self, e: ExprId, dest: Local, handlers: &[Handler], body: ExprId) {
        self.scopes.open();
        for (k, h) in handlers.iter().enumerate() {
            let ty = self.ty(h.value);
            let borrow = match self.solver.norm(&ty) {
                Ty::Ref(_, inner) => {
                    let op = self.operand(h.value, &[]);
                    (op, (*inner).clone())
                }
                t => {
                    let held = self.cfg.new_local(t.clone(), None);
                    self.scopes.push(ScopeItem::Local(held));
                    self.eval_into(held, h.value);
                    let r = self.temp(Ty::reference(false, t.clone()));
                    self.cfg.assign(r, Rvalue::Ref(false, Place::local(held)));
                    (Operand::Move(Place::local(r)), t)
                }
            };
            let installed = self
                .cfg
                .new_local(Ty::reference(false, h.effect.clone()), None);
            self.scopes.push(ScopeItem::Local(installed));
            let value = match &h.effect {
                Ty::Dyn(_) => Rvalue::Unsize(borrow.0, borrow.1),
                _ => Rvalue::Use(borrow.0),
            };
            self.cfg.assign(installed, value);
            self.installed.insert((e, k), Place::local(installed));
        }
        self.eval_into(dest, body);
        let depth = self.scopes.depth() - 1;
        self.leave_scopes(depth);
        self.scopes.close();
    }

    /// A call; if its function can fail, its error edge goes where the `?` or `?!` around
    /// call `e` says.
    /// `fmt"…"`: a new `String`, then each piece in order: text is pushed onto it, and each
    /// hole is borrowed and written by its `Display::fmt`.
    fn fmt(&mut self, dest: Local, pieces: &[FmtPiece], range: TextRange) {
        let string = |name: &str| {
            let func = self
                .items
                .builtin_method(&Ty::String, name)
                .expect("the prelude's `String`");
            Callee::Fn {
                func,
                args: Vec::new(),
            }
        };
        self.call_into(None, string("new"), Place::local(dest), Vec::new(), range);
        let push_str = string("push_str");
        for p in pieces {
            let out = self.temp(Ty::reference(true, Ty::String));
            self.cfg.assign(out, Rvalue::Ref(true, Place::local(dest)));
            let unit = self.temp(Ty::Unit);
            match p {
                FmtPiece::Lit(s) => {
                    let c = self.cfg.str_const(s);
                    self.call_into(
                        None,
                        push_str.clone(),
                        Place::local(unit),
                        vec![Operand::Move(Place::local(out)), Operand::Const(c)],
                        range,
                    );
                }
                &FmtPiece::Hole(h) => {
                    let ty = self.ty(h);
                    // A borrow is written as what it points to.
                    let this = match &ty {
                        Ty::Ref(..) => self.operand(h, &[]),
                        t => {
                            let place = self.place(h);
                            let r = self.temp(Ty::reference(false, t.clone()));
                            self.cfg.assign(r, Rvalue::Ref(false, place));
                            Operand::Move(Place::local(r))
                        }
                    };
                    let callee = self
                        .types
                        .fmt_callee(h)
                        .expect("a checked `fmt` hole")
                        .clone();
                    self.call_into(
                        None,
                        callee,
                        Place::local(unit),
                        vec![this, Operand::Move(Place::local(out))],
                        self.range(h),
                    );
                }
            }
        }
    }

    pub(super) fn call_into(
        &mut self,
        e: Option<ExprId>,
        callee: Callee,
        dest: Place,
        args: Vec<Operand>,
        range: TextRange,
    ) {
        let name = fn_name(self.items, callee.func());
        let fails = self.fx.fns[callee.func() as usize]
            .fails
            .subst(callee.args());
        let fails = self.solver.deep(&fails);
        let edge = self.error_edge(e, &fails);
        let target = self.cfg.new_block();
        let error_bb = edge.as_ref().map(|_| self.cfg.new_block());
        let on_error = edge
            .as_ref()
            .zip(error_bb)
            .map(|((err, _), b)| (Place::local(*err), b));
        self.cfg.terminate_into(
            Terminator::Call {
                callee,
                name,
                args,
                dest,
                target,
                on_error,
                range,
            },
            target,
        );
        if let (Some((err, action)), Some(b)) = (edge, error_bb) {
            self.cfg.continue_in(b);
            self.on_error_block(err, action);
            self.cfg.continue_in(target);
        }
    }

    pub(super) fn stmt(&mut self, s: &Stmt) {
        match *s {
            Stmt::Let {
                pat, init, else_, ..
            } => {
                if let PatKind::Bind { local, sub: None } = self.hir.pats[pat].kind {
                    // Declared after its initialiser is evaluated: `let x = x + 1` reads the
                    // outer `x`.
                    let t = self.temp(self.types.local(local));
                    self.eval_into(t, init);
                    let named = self.local(local);
                    let ty = self.types.local(local);
                    let op = self.use_place(Place::local(t), &ty);
                    self.cfg.assign(named, Rvalue::Use(op));
                    return;
                }
                let place = self.scrutinee(init);
                let fail = self.cfg.new_block();
                self.match_pat(pat, &place, fail);
                if let Some(x) = else_ {
                    let matched = self.cfg.current();
                    self.cfg.continue_in(fail);
                    // The `else` block diverges.
                    let ignored = self.temp(Ty::Never);
                    self.eval_into(ignored, x);
                    self.cfg.terminate_into(Terminator::Unreachable, matched);
                }
            }
            Stmt::Defer(body) => {
                self.scopes.push(ScopeItem::Defer(body));
            }
            Stmt::ErrDefer(body) => self.errdefer(body),
            Stmt::Expr {
                expr,
                observe: true,
                ..
            } => {
                // Observing reads the value; a place keeps it.
                let place = self.place(expr);
                self.cfg.push(Statement::Observe {
                    range: self.range(expr),
                    value: Operand::Copy(place),
                });
            }
            Stmt::Expr { expr, .. } => {
                // A place whose value is discarded is read, not moved, as `let _ = p` would.
                if let Some(place) = self.place_of(expr) {
                    let at = self.range(expr);
                    let outer = self.cfg.set_span(at);
                    self.cfg.push(Statement::Read(place));
                    self.cfg.set_span(outer);
                } else {
                    let t = self.temp(self.ty(expr));
                    self.eval_into(t, expr);
                }
            }
        }
    }
}

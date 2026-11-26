//! Expressions and statements: each HIR expression kind as blocks and statements.

use moddil_sema::hir::{
    Align, ExprId, ExprKind, FmtPiece, Halt, Handler, LogicOp, PatKind, Radix, Spec, Stmt, UnaryOp,
};
use moddil_sema::{Callee, Container, Item, Ty};
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
        // An integer literal taken as a `Wrapping<T>` (`num.wrap.3`): the `T`, wrapped.
        if self.uncoerced != Some(e)
            && let Some(Ty::Adt(adt, ..)) = self.types.wrapped(e).cloned()
        {
            let t = self.temp(self.ty(e));
            let outer = self.uncoerced.replace(e);
            self.eval_into(t, e);
            self.uncoerced = outer;
            let path = self.items.adts[adt as usize].path(0);
            self.cfg.assign(
                dest,
                Rvalue::Aggregate(
                    AggregateKind::Adt {
                        adt,
                        variant: 0,
                        path,
                    },
                    vec![Operand::Move(Place::local(t))],
                ),
            );
            return;
        }
        // A function named where a C function pointer is wanted: its address, not a closure.
        if let Some(f) = self.types.fn_ptr(e) {
            self.cfg.assign(dest, Rvalue::FnPtr(f));
            return;
        }
        // A borrow used as a borrow of what it dereferences to: through `deref` or `deref_mut`
        // (`type.deref.coerce.1`).
        if self.uncoerced != Some(e)
            && let Some((callee, _)) = self.types.deref_coercion(e).cloned()
        {
            let ty = self.ty(e);
            let t = self.cfg.new_local(ty.clone(), None);
            let outer = self.uncoerced.replace(e);
            self.eval_dyn_part(t, e);
            self.uncoerced = outer;
            let mutable = self
                .items
                .fns
                .get(callee.func() as usize)
                .is_some_and(|f| f.name == "deref_mut");
            let pointee = match &ty {
                Ty::Ref(_, inner) => (**inner).clone(),
                other => other.clone(),
            };
            let b = self.temp(Ty::reference(mutable, pointee));
            self.cfg
                .assign(b, Rvalue::Ref(mutable, Place::local(t).deref()));
            let range = self.range(e);
            self.call_into(
                None,
                callee,
                Place::local(dest),
                vec![Operand::Move(Place::local(b))],
                range,
            );
            return;
        }
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
            // A number widened (`num.widen.1`).
            if matches!(target, Ty::Int | Ty::Num(_)) {
                self.cfg.assign(dest, Rvalue::Widen(op, target));
                return;
            }
            // A `&String` as a `&str`: its `as_str` (`text.coerce.1`). As a `dyn` borrow it is
            // converted as any other borrow is.
            if from == self.items.string_ty() && matches!(&target, Ty::Ref(_, t) if **t == Ty::Str)
            {
                let func = self
                    .items
                    .builtin_method(&from, "as_str")
                    .expect("the prelude's `String::as_str`");
                let callee = Callee::Fn {
                    func,
                    args: Vec::new(),
                };
                let range = self.range(e);
                self.call_into(None, callee, Place::local(dest), vec![op], range);
                return;
            }
            debug_assert!(matches!(target, Ty::Ref(..) | Ty::Adt(..)));
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
                let op = self.use_part(e, v, &ty);
                self.cfg.assign(dest, Rvalue::Use(op));
            }
            return;
        }
        self.eval_plain(dest, e);
    }

    /// If `e` is a part of the `assert` being built and a place, borrowed where it lives (a
    /// method's receiver), it is shown from there, as are the places it is a field of: they are
    /// not computed, so not copied, and stay where they are.
    pub(super) fn part_in_place(&mut self, e: ExprId) {
        let Some(&(_, evaluated)) = self.recording.get(&e) else {
            return;
        };
        let Some(place) = self.place_of(e) else {
            return;
        };
        self.recording.remove(&e);
        self.cfg
            .assign(evaluated, Rvalue::Use(Operand::Const(Const::Bool(true))));
        let mark = self.cfg.mark();
        self.parts_in_place.insert(e, (place, mark));
        if let ExprKind::Field { base, .. } = self.hir.exprs[e].kind {
            self.part_in_place(base);
        }
    }

    /// The parts of an `assert`'s condition whose values a failure shows: every sub-expression
    /// that is not a literal, through operators, calls, fields, tuples and struct expressions.
    pub(super) fn assert_parts(&self, e: ExprId, out: &mut Vec<ExprId>) {
        let kind = &self.hir.exprs[e].kind;
        if !matches!(
            kind,
            ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Bool(_) | ExprKind::Unit
        ) {
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
            | ExprKind::Float(_)
            | ExprKind::Bool(_)
            | ExprKind::Unit
            | ExprKind::Char(_)
            | ExprKind::Str(_)
            | ExprKind::Local(_)
            | ExprKind::Field { .. }
            | ExprKind::Index { .. }
            | ExprKind::Slice { .. }
            | ExprKind::Deref(_) => {
                let place = match self.hir.exprs[e].kind {
                    ExprKind::Field { .. }
                    | ExprKind::Index { .. }
                    | ExprKind::Slice { .. }
                    | ExprKind::Deref(_) => Some(self.place(e)),
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
            // `&N` of a const item borrows its static value.
            ExprKind::Ref {
                mutable: false,
                expr,
            } if self.static_const(expr).is_some() => {
                let f = self.static_const(expr).expect("a const");
                self.cfg.assign(dest, Rvalue::StaticRef(f));
            }
            // `&s[a..b]` of text: the prelude's `between` (`text.pos.1`).
            ExprKind::Ref { expr, .. }
                if matches!(self.hir.exprs[expr].kind, ExprKind::Slice { .. })
                    && self.ty(expr) == Ty::Str =>
            {
                self.text_slice(dest, expr);
            }
            ExprKind::Ref { mutable, expr } => {
                let place = self.place(expr);
                self.cfg.assign(dest, Rvalue::Ref(mutable, place));
            }
            // A `const { }` block is evaluated where it is, for now; a refined value is its
            // base's, as it is.
            // Computed when compiling, if its value can be copied out of its static data
            // (`comptime.block.2`); else where it is.
            ExprKind::ConstBlock(_)
                if self.frozen_block(e).is_some() && self.is_copy(&self.ty(e)) =>
            {
                let p = self.place(e);
                let ty = self.ty(e);
                let op = self.use_place(p, &ty);
                self.cfg.assign(dest, Rvalue::Use(op));
            }
            // One whose value is `Clone`: each use a clone of its static data, so it is
            // computed once, when compiling (`comptime.block.3`).
            ExprKind::ConstBlock(_)
                if self.frozen_block(e).is_some() && self.clone_fn(&self.ty(e)).is_some() =>
            {
                let ty = self.ty(e);
                let func = self.clone_fn(&ty).expect("a clone");
                let p = self.place(e);
                let r = self.temp(Ty::reference(false, ty.clone()));
                self.cfg.assign(r, Rvalue::Ref(false, p));
                self.call_into(
                    None,
                    Callee::Trait {
                        func,
                        args: vec![ty],
                    },
                    Place::local(dest),
                    vec![Operand::Move(Place::local(r))],
                    range,
                );
            }
            ExprKind::ConstBlock(x) | ExprKind::Refine(x, _) => self.eval_into(dest, x),
            ExprKind::Fmt(pieces) => self.fmt(dest, &pieces, range),
            ExprKind::Array(_) | ExprKind::Repeat(..) | ExprKind::RepeatN(..)
                if self.types.literal_callee(e).is_some() =>
            {
                self.literal_into(e, dest);
            }
            ExprKind::Array(xs) => {
                let ops = self.operands(&xs);
                self.cfg
                    .assign(dest, Rvalue::Aggregate(AggregateKind::Array, ops));
            }
            ExprKind::Repeat(x, n) => {
                let t = self.ty(x);
                let op = self.operand(x, &[]);
                if self.is_copy(&t) {
                    self.cfg.assign(dest, Rvalue::Repeat(op, n));
                } else {
                    // Clones of a value that is not `Copy` (`expr.array.repeat.1`).
                    let callee = Callee::Fn {
                        func: self.prelude_fn("repeat_array"),
                        args: vec![t, n],
                    };
                    self.call_into(Some(e), callee, Place::local(dest), vec![op], range);
                }
            }
            ExprKind::RepeatN(..) => unreachable!("a checked `[x; n]` makes a value of a type"),
            ExprKind::ConstParam(i) => {
                // Substitution goes by index, so the name is only for printing.
                self.cfg
                    .assign(dest, Rvalue::Length(Ty::Param(i, "const".into())));
            }
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
                self.cfg.node = self.scopes.node();
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
            ExprKind::Select(arms) => self.select(e, dest, &arms),
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
                self.cfg.node = self.scopes.node();
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
            // A wrapper's call (`attr.around.1`): its errors pass on, if it has any.
            ExprKind::Pass(x) => {
                if self.types.propagated(e).is_some() {
                    self.question(e, x, dest, None);
                } else {
                    self.eval_into(dest, x);
                }
            }
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
                self.ensures();
                self.leave_scopes(0);
                self.cfg.terminate(Terminator::Return);
            }
            ExprKind::Halt(halt, reason) => {
                let class = match halt {
                    Halt::Unreachable => TrapClass::Unreachable,
                    Halt::Todo => TrapClass::Unimplemented,
                };
                self.cfg.terminate(Terminator::Trap(class, reason));
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
                        value: Place::local(value),
                        evaluated,
                    });
                }
                let c = self.operand(cond, &[]);
                for x in &exprs {
                    self.recording.remove(x);
                }
                // A part moved away has no value left to show (and reading it would read what
                // its new owner may have freed).
                let mut kept = Vec::with_capacity(parts.len());
                for (x, mut part) in exprs.iter().zip(parts) {
                    let mut moved = self.moved_parts.remove(x);
                    if let Some((p, mark)) = self.parts_in_place.remove(x) {
                        // Shown where it lives, unless the rest of the condition moved it away.
                        moved |= self.cfg.moved_since(&mark, p.local);
                        part.value = p;
                    }
                    if !moved {
                        kept.push(part);
                    }
                }
                let parts = kept;
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
                let a = self.read_operand(x, &[]);
                self.call(e, dest, vec![a]);
            }
            ExprKind::Binary(_, l, r) if self.types.callee(e).is_some() => {
                let a = self.read_operand(l, &[r]);
                let b = self.read_operand(r, &[]);
                self.call(e, dest, vec![a, b]);
            }
            ExprKind::Unary(op, x) => {
                let a = self.read_operand(x, &[]);
                let float = matches!(self.ty(e), Ty::Num(n) if n.is_float());
                if op == UnaryOp::Neg && !float {
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
                let a = self.read_operand(l, &[r]);
                let b = self.read_operand(r, &[]);
                let ty = self.read_ty(l);
                self.arith(Place::local(dest), op, &ty, a, b, range);
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
                    // Through its operator trait (`expr.assign.compound.2`): `x = x + y`.
                    Some(_) if self.types.callee(e).is_some() => {
                        let place = self.place(target);
                        let b = self.read_operand(value, &[]);
                        let a = self.use_place(place.clone(), &ty);
                        let t = self.temp(ty.clone());
                        self.call(e, t, vec![a, b]);
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
                        let b = self.read_operand(value, &[]);
                        self.arith(place, op, &ty, a, b, range);
                    }
                }
                self.cfg
                    .assign(dest, Rvalue::Use(Operand::Const(Const::Unit)));
            }
            // A const item computed when compiling (`comptime.static.1`): read from its static
            // data, copied if it is `Copy` (a move of it is refused, E1009).
            ExprKind::Call { .. } if self.frozen_const(e).is_some() => {
                let p = self.place(e);
                let ty = self.ty(e);
                let op = self.use_place(p, &ty);
                self.cfg.assign(dest, Rvalue::Use(op));
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
            // Through `Deref` (`expr.method.deref.1`): the receiver is what `deref` (or
            // `deref_mut`) gives.
            ExprKind::MethodCall { receiver, args, .. } if self.types.method_deref(e).is_some() => {
                let adjust = self.types.adjust(e);
                let callee = self.types.method_deref(e).expect("checked").clone();
                let mutable = self
                    .items
                    .fns
                    .get(callee.func() as usize)
                    .is_some_and(|f| f.name == "deref_mut");
                self.part_in_place(receiver);
                let mut place = self.place(receiver);
                let mut base = self.ty(receiver);
                while let Ty::Ref(_, inner) = base.clone() {
                    place = place.deref();
                    base = (*inner).clone();
                }
                let target = match self.types.callee(e) {
                    Some(c) => {
                        let f = &self.items.fns[c.func() as usize];
                        match f.params.first().map(|p| p.ty.subst(c.args())) {
                            Some(Ty::Ref(_, t)) => (*t).clone(),
                            Some(t) => t,
                            None => base.clone(),
                        }
                    }
                    None => base.clone(),
                };
                let b = self.temp(Ty::reference(mutable, base));
                self.cfg.assign(b, Rvalue::Ref(mutable, place));
                let r = self.temp(Ty::reference(mutable, target.clone()));
                let range = self.range(e);
                self.call_into(
                    None,
                    callee,
                    Place::local(r),
                    vec![Operand::Move(Place::local(b))],
                    range,
                );
                let inner = Place::local(r).deref();
                let recv = match adjust.autoref {
                    Some(m) => {
                        let t = self.temp(Ty::reference(m, target));
                        self.cfg.assign(t, Rvalue::Ref(m, inner));
                        Operand::Move(Place::local(t))
                    }
                    None => self.use_place(inner, &target),
                };
                let mut ops = vec![recv];
                ops.extend(self.operands(&args));
                self.call(e, dest, ops);
            }
            ExprKind::MethodCall { receiver, .. } if self.fmt_sink(e).is_some() => {
                let pieces = self.fmt_sink(e).expect("a `fmt` literal");
                let sink = self.text_sink(receiver);
                self.fmt_into(Sink::Dyn(sink), &pieces, range);
                self.cfg
                    .assign(dest, Rvalue::Use(Operand::Const(Const::Unit)));
            }
            ExprKind::MethodCall { receiver, args, .. } => {
                let adjust = self.types.adjust(e);
                let recv = match adjust.autoref {
                    Some(m) => {
                        self.part_in_place(receiver);
                        let mut place = self.place(receiver);
                        if adjust.deref {
                            place = place.deref();
                        }
                        let inner = match self.ty(receiver) {
                            Ty::Ref(_, t) if adjust.deref => (*t).clone(),
                            t => t,
                        };
                        let t = self.temp(Ty::reference(m, inner));
                        if m {
                            // Two-phase (`expr.method.twophase.1`): the receiver's place first,
                            // its `&mut` borrow once the arguments are evaluated, so they may
                            // read it (`v.push(v.len())`).
                            let rest = self.operands(&args);
                            self.cfg.assign(t, Rvalue::Ref(m, place));
                            let mut ops = vec![Operand::Move(Place::local(t))];
                            ops.extend(rest);
                            self.call(e, dest, ops);
                            return;
                        }
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
        // Named and left-out arguments (`expr.call.named.1`): the written ones, evaluated in
        // the order written, are passed in the parameters' order, and a default fills each gap.
        if let Some(order) = self.types.arg_order(e).map(<[_]>::to_vec) {
            let written = match &self.hir.exprs[e].kind {
                ExprKind::Call { args, .. } | ExprKind::MethodCall { args, .. } => args.len(),
                _ => 0,
            };
            let offset = args.len().saturating_sub(written);
            let mut taken: Vec<Option<Operand>> =
                args.split_off(offset).into_iter().map(Some).collect();
            let range = self.range(e);
            for slot in order {
                match slot {
                    moddil_sema::ArgSlot::Arg(i) => {
                        args.push(taken[i].take().expect("an argument passed once"));
                    }
                    moddil_sema::ArgSlot::Default(d) => {
                        let t = self.temp(self.items.fns[d as usize].ret.clone());
                        let callee = Callee::Fn {
                            func: d,
                            args: Vec::new(),
                        };
                        self.call_into(None, callee, Place::local(t), Vec::new(), range);
                        args.push(Operand::Move(Place::local(t)));
                    }
                }
            }
        }
        // The handlers it needs, after its arguments.
        for ev in self.types.evidence(e).to_vec() {
            let local = self.evidence_place(ev);
            args.push(Operand::Copy(local));
        }
        self.call_into(Some(e), callee, Place::local(dest), args, self.range(e));
    }

    /// `&s[a..b]` of text, `s` a `str` or a `String` (through borrows): `s.between(a, b)`, with
    /// `s.start()` or `s.end()` for an end not written (`text.pos.1`).
    fn text_slice(&mut self, dest: Local, e: ExprId) {
        let ExprKind::Slice {
            base, start, end, ..
        } = self.hir.exprs[e].kind.clone()
        else {
            unreachable!("a slice")
        };
        let range = self.range(e);
        let method = |b: &Self, name: &str| {
            b.items
                .builtin_method(&Ty::Str, name)
                .expect("the prelude's text positions")
        };
        // The text, as a `&str`.
        let string = self.items.string_ty();
        let mut bt = self.ty(base);
        let mut p = self.place(base);
        while let Ty::Ref(_, inner) = bt.clone() {
            if *inner == Ty::Str {
                break;
            }
            p = p.deref();
            bt = (*inner).clone();
        }
        let text = self.temp(Ty::reference(false, Ty::Str));
        if bt == string {
            let func = self
                .items
                .builtin_method(&string, "as_str")
                .expect("the prelude's `String::as_str`");
            let b = self.temp(Ty::reference(false, string));
            self.cfg.assign(b, Rvalue::Ref(false, p));
            self.call_into(
                None,
                Callee::Fn {
                    func,
                    args: Vec::new(),
                },
                Place::local(text),
                vec![Operand::Move(Place::local(b))],
                range,
            );
        } else if bt == Ty::Str {
            self.cfg.assign(text, Rvalue::Ref(false, p));
        } else {
            let op = self.use_place(p, &bt);
            self.cfg.assign(text, Rvalue::Use(op));
        }
        let mut ends = Vec::new();
        for (x, name) in [(start, "start"), (end, "end")] {
            let t = self.temp(self.items.fns[method(self, name) as usize].ret.clone());
            match x {
                Some(x) => self.eval_into(t, x),
                None => {
                    let func = method(self, name);
                    self.call_into(
                        None,
                        Callee::Fn {
                            func,
                            args: Vec::new(),
                        },
                        Place::local(t),
                        vec![Operand::Copy(Place::local(text))],
                        range,
                    );
                }
            }
            ends.push(Operand::Move(Place::local(t)));
        }
        let func = method(self, "between");
        let mut args = vec![Operand::Copy(Place::local(text))];
        args.extend(ends);
        self.call_into(
            None,
            Callee::Fn {
                func,
                args: Vec::new(),
            },
            Place::local(dest),
            args,
            range,
        );
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
                    // Named for what it is, in errors about what borrows it.
                    let what = match &h.effect {
                        Ty::Dyn(tr, _) => tr.name.to_string(),
                        e => format!("{e:?}"),
                    };
                    let held = self
                        .cfg
                        .new_local(t.clone(), Some(format!("the `{what}` handler")));
                    self.scopes.push(ScopeItem::Local(held));
                    self.cfg.node = self.scopes.node();
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
            self.cfg.node = self.scopes.node();
            let value = match &h.effect {
                Ty::Dyn(..) => Rvalue::Unsize(borrow.0, borrow.1),
                _ => Rvalue::Use(borrow.0),
            };
            self.cfg.assign(installed, value);
            // A runtime takes over before the body runs (`task.runtime.1`).
            if matches!(&h.effect, Ty::Adt(a, ..) if Some(*a) == self.items.lang("Runtime")) {
                let unit = self.temp(Ty::Unit);
                let callee = Callee::Fn {
                    func: self.prelude_fn("__runtime_install"),
                    args: Vec::new(),
                };
                let rt = Operand::Copy(Place::local(installed));
                self.call_into(None, callee, Place::local(unit), vec![rt], h.range);
            }
            self.installed.insert((e, k), Place::local(installed));
        }
        self.eval_into(dest, body);
        let depth = self.scopes.depth() - 1;
        self.leave_scopes(depth);
        self.scopes.close();
        self.cfg.node = self.scopes.node();
    }

    /// A call; if its function can fail, its error edge goes where the `?` or `?!` around
    /// call `e` says.
    /// The const item whose value `e` is, if it is a use of one.
    /// The item of `const { }` block `e`, if it is computed when compiling (`comptime.block.2`).
    pub(super) fn frozen_block(&self, e: ExprId) -> Option<Item> {
        let ExprKind::ConstBlock(_) = self.hir.exprs[e].kind else {
            return None;
        };
        let item = match self.base {
            Item::Fn(i) => Item::Closure(i, false, super::idx(e) as u32),
            Item::Test(i) => Item::Closure(i, true, super::idx(e) as u32),
            Item::Closure(..) => return None,
        };
        self.items.freezes_blocks(self.base).then_some(item)
    }

    /// The const item `e` names, if it is one frozen when compiling (`comptime.static.1`):
    /// without type arguments.
    pub(super) fn frozen_const(&self, e: ExprId) -> Option<u32> {
        let f = self.static_const(e)?;
        let plain =
            matches!(self.types.callee(e), Some(Callee::Fn { args, .. }) if args.is_empty());
        (plain && self.items.is_frozen_const(f)).then_some(f)
    }

    fn static_const(&self, e: ExprId) -> Option<u32> {
        let ExprKind::Call { args, .. } = &self.hir.exprs[e].kind else {
            return None;
        };
        match self.types.callee(e) {
            Some(Callee::Fn { func, .. })
                if args.is_empty() && self.items.fns[*func as usize].const_item =>
            {
                Some(*func)
            }
            _ => None,
        }
    }

    /// An array literal made into the type its context expects (`expr.array.from.1`):
    /// `FromArrayLiteral::from_array([a, b])`, or `from_repeat(x, n)` for `[x; n]`.
    fn literal_into(&mut self, e: ExprId, dest: Local) {
        let callee = self.types.literal_callee(e).expect("checked").clone();
        let range = self.range(e);
        let args = match self.hir.exprs[e].kind.clone() {
            ExprKind::Array(xs) => {
                let elem = callee.args()[1].clone();
                let n = callee.args()[2].clone();
                let ops = self.operands(&xs);
                let a = self.temp(Ty::array(elem, n));
                self.cfg
                    .assign(a, Rvalue::Aggregate(AggregateKind::Array, ops));
                vec![Operand::Move(Place::local(a))]
            }
            ExprKind::Repeat(x, n) => {
                let op = self.operand(x, &[]);
                let k = self.temp(Ty::Int);
                self.cfg.assign(k, Rvalue::Length(n));
                vec![op, Operand::Copy(Place::local(k))]
            }
            ExprKind::RepeatN(x, n) => self.operands(&[x, n]),
            _ => unreachable!("an array literal"),
        };
        self.call_into(None, callee, Place::local(dest), args, range);
    }

    /// `fmt"…"`: a new `String`, then each piece in order: text is pushed onto it, and each
    /// hole is borrowed and written by its `Display::fmt` (or `Debug::fmt`). A hole with a width
    /// or a precision is written into a `String` of its own first, which is then cut and padded
    /// onto it (`expr.fmt.spec.1`).
    fn fmt(&mut self, dest: Local, pieces: &[FmtPiece], range: TextRange) {
        let new = self.string_method("new");
        self.call_into(None, new, Place::local(dest), Vec::new(), range);
        self.fmt_into(Sink::String(dest), pieces, range);
    }

    /// `w.write_fmt(fmt"…")`'s literal (`text.write.2`), if `e` is that call: its pieces, which
    /// are written straight onto `w`.
    fn fmt_sink(&self, e: ExprId) -> Option<Vec<FmtPiece>> {
        let ExprKind::MethodCall { args, .. } = &self.hir.exprs[e].kind else {
            return None;
        };
        let [arg] = args.as_slice() else {
            return None;
        };
        let ExprKind::Fmt(pieces) = &self.hir.exprs[*arg].kind else {
            return None;
        };
        let sig = &self.items.fns[self.types.callee(e)?.func() as usize];
        let tw = self.items.lang("TextWrite")?;
        let of_text_write = match sig.container {
            Container::Trait(t) => t == tw,
            Container::Impl(k) => self.items.impls[k as usize]
                .trait_ref
                .as_ref()
                .is_some_and(|t| t.trait_ == tw),
            _ => false,
        };
        (sig.name == "write_fmt" && of_text_write).then(|| pieces.clone())
    }

    /// The receiver of `w.write_fmt(…)`, `w` or a `&mut` borrow of one, as a local holding a
    /// `&mut dyn TextWrite`.
    fn text_sink(&mut self, receiver: ExprId) -> Local {
        let (place, inner) = match self.ty(receiver) {
            Ty::Ref(true, t) => (self.place(receiver).deref(), (*t).clone()),
            t => (self.place(receiver), t),
        };
        let tw = self
            .items
            .lang_ref("TextWrite")
            .expect("the prelude's `TextWrite`");
        let r = self.temp(Ty::reference(true, inner.clone()));
        self.cfg.assign(r, Rvalue::Ref(true, place));
        if matches!(inner, Ty::Dyn(..)) {
            return r;
        }
        let out = self.temp(Ty::reference(true, Ty::dyn_of(tw)));
        self.cfg
            .assign(out, Rvalue::Unsize(Operand::Move(Place::local(r)), inner));
        out
    }

    /// A `&mut` borrow of what `sink` writes to, for one call: the `String`, or a reborrow
    /// of the `&mut dyn TextWrite`.
    fn sink_ref(&mut self, sink: Sink) -> Operand {
        match sink {
            Sink::String(s) => {
                let string = self.items.string_ty();
                let borrow = self.temp(Ty::reference(true, string.clone()));
                self.cfg.assign(borrow, Rvalue::Ref(true, Place::local(s)));
                let tw = self
                    .items
                    .lang_ref("TextWrite")
                    .expect("the prelude's `TextWrite`");
                let out = self.temp(Ty::reference(true, Ty::dyn_of(tw)));
                self.cfg.assign(
                    out,
                    Rvalue::Unsize(Operand::Move(Place::local(borrow)), string),
                );
                Operand::Move(Place::local(out))
            }
            Sink::Dyn(d) => {
                let tw = self
                    .items
                    .lang_ref("TextWrite")
                    .expect("the prelude's `TextWrite`");
                let out = self.temp(Ty::reference(true, Ty::dyn_of(tw)));
                self.cfg
                    .assign(out, Rvalue::Ref(true, Place::local(d).deref()));
                Operand::Move(Place::local(out))
            }
        }
    }

    /// Text `s`, a `&str`, written onto `sink`: pushed onto the `String`, or through the
    /// sink's `write_str`.
    fn sink_str(&mut self, sink: Sink, s: Operand, range: TextRange) {
        match sink {
            Sink::String(dest) => self.push_str(dest, s, range),
            Sink::Dyn(_) => {
                let tw = self
                    .items
                    .lang_ref("TextWrite")
                    .expect("the prelude's `TextWrite`");
                let func = self.items.traits[tw.trait_ as usize]
                    .methods
                    .iter()
                    .copied()
                    .find(|&m| self.items.fns[m as usize].name == "write_str")
                    .expect("`TextWrite::write_str`");
                let out = self.sink_ref(sink);
                let unit = self.temp(Ty::Unit);
                let callee = Callee::Trait {
                    func,
                    args: vec![Ty::dyn_of(tw)],
                };
                self.call_into(None, callee, Place::local(unit), vec![out, s], range);
            }
        }
    }

    /// `fmt"…"`'s pieces written onto `sink` (`expr.fmt.1`, `text.write.2`).
    fn fmt_into(&mut self, sink: Sink, pieces: &[FmtPiece], range: TextRange) {
        let new = self.string_method("new");
        for p in pieces {
            match p {
                FmtPiece::Lit(s) => {
                    let c = self.cfg.str_const(s);
                    self.sink_str(sink, Operand::Const(c), range);
                }
                FmtPiece::Hole(h, spec) if spec.is_plain() => self.write_hole(*h, sink),
                FmtPiece::Hole(h, spec) if matches!(sink, Sink::Dyn(_)) => {
                    self.padded_hole(*h, spec, sink, range);
                }
                FmtPiece::Hole(h, spec) => {
                    let h = *h;
                    let piece = self.temp(self.items.string_ty());
                    self.call_into(None, new.clone(), Place::local(piece), Vec::new(), range);
                    let base = match self.ty(h) {
                        Ty::Ref(_, t) => (*t).clone(),
                        t => t,
                    };
                    let base = self.items.base(&base);
                    let number = matches!(base, Ty::Int | Ty::Num(_));
                    let text = base == Ty::Str || base == self.items.string_ty();
                    // The precision, written or named (`.name$`), as an operand; -1 for none.
                    let precision = match (spec.precision, spec.precision_arg) {
                        (_, Some(x)) => Some(self.operand(x, &[])),
                        (Some(k), None) => Some(Operand::Const(Const::Int(i64::from(k)))),
                        (None, None) => None,
                    };
                    let float = matches!(base, Ty::Num(n) if n.is_float());
                    // A float's fixed digits (`expr.fmt.spec.3`), or its exponent form
                    // (`expr.fmt.spec.5`): the float's `fixed_text` or `exp_text`.
                    let fixed = match (&precision, spec.exponent) {
                        (_, Some(upper)) if float => {
                            self.items.builtin_method(&base, "exp_text").map(|f| {
                                let digits =
                                    precision.clone().unwrap_or(Operand::Const(Const::Int(-1)));
                                (f, vec![digits, Operand::Const(Const::Bool(upper))])
                            })
                        }
                        (Some(digits), None) if float => self
                            .items
                            .builtin_method(&base, "fixed_text")
                            .map(|f| (f, vec![digits.clone()])),
                        _ => None,
                    };
                    let radix = spec.radix.and_then(|r| {
                        let tr = self.items.lang("RadixText")?;
                        let func = self.items.traits[tr as usize]
                            .methods
                            .iter()
                            .copied()
                            .find(|&m| self.items.fns[m as usize].name == "write_radix")?;
                        Some((r, func))
                    });
                    match fixed {
                        _ if radix.is_some() => {
                            // An integer's digits in base 2, 8 or 16 (`expr.fmt.spec.4`).
                            let (r, func) = radix.expect("a base");
                            let this = self.hole_ref(h);
                            let out = self.sink_ref(Sink::String(piece));
                            let unit = self.temp(Ty::Unit);
                            let callee = Callee::Trait {
                                func,
                                args: vec![base.clone()],
                            };
                            let args = vec![
                                this,
                                out,
                                Operand::Const(Const::Int(r.bits())),
                                Operand::Const(Const::Bool(r == Radix::UpperHex)),
                                Operand::Const(Const::Bool(spec.prefix)),
                            ];
                            self.call_into(None, callee, Place::local(unit), args, self.range(h));
                        }
                        Some((func, rest)) => {
                            let this = self.hole_ref(h);
                            let s = self.temp(Ty::reference(false, Ty::Str));
                            let callee = Callee::Fn {
                                func,
                                args: Vec::new(),
                            };
                            let mut args = vec![this];
                            args.extend(rest);
                            self.call_into(None, callee, Place::local(s), args, self.range(h));
                            self.push_str(piece, Operand::Move(Place::local(s)), range);
                        }
                        None => self.write_hole(h, Sink::String(piece)),
                    }
                    if spec.plus {
                        let out = self.temp(Ty::reference(true, self.items.string_ty()));
                        self.cfg.assign(out, Rvalue::Ref(true, Place::local(piece)));
                        let unit = self.temp(Ty::Unit);
                        let plus = self.string_method("sign_plus");
                        let args = vec![Operand::Move(Place::local(out))];
                        self.call_into(None, plus, Place::local(unit), args, range);
                    }
                    let align = match spec.align {
                        Some(Align::Center) => 1,
                        Some(Align::Right) => 2,
                        None if number => 2,
                        Some(Align::Left) | None => 0,
                    };
                    let keep = match precision {
                        Some(k) if text && !spec.debug => k,
                        _ => Operand::Const(Const::Int(-1)),
                    };
                    // The width, written or named (`name$`).
                    let width = match spec.width_arg {
                        Some(x) => self.operand(x, &[]),
                        None => Operand::Const(Const::Int(spec.width.map_or(0, i64::from))),
                    };
                    // Padded onto the `String`, or into one of its own that is then written
                    // onto the sink.
                    let (dest, padded_alone) = match sink {
                        Sink::String(dest) => (dest, None),
                        Sink::Dyn(_) => {
                            let alone = self.temp(self.items.string_ty());
                            self.call_into(
                                None,
                                new.clone(),
                                Place::local(alone),
                                Vec::new(),
                                range,
                            );
                            (alone, Some(alone))
                        }
                    };
                    let out = self.temp(Ty::reference(true, self.items.string_ty()));
                    self.cfg.assign(out, Rvalue::Ref(true, Place::local(dest)));
                    let unit = self.temp(Ty::Unit);
                    let args = vec![
                        Operand::Move(Place::local(out)),
                        Operand::Move(Place::local(piece)),
                        Operand::Const(Const::Char(spec.fill.unwrap_or(' '))),
                        Operand::Const(Const::Int(align)),
                        width,
                        Operand::Const(Const::Bool(spec.zero)),
                        keep,
                    ];
                    let padded = self.string_method("push_padded");
                    self.call_into(None, padded, Place::local(unit), args, range);
                    if let Some(alone) = padded_alone {
                        let text = self.temp(Ty::reference(false, Ty::Str));
                        let as_str = self.string_method("as_str");
                        let s = self.temp(Ty::reference(false, self.items.string_ty()));
                        self.cfg.assign(s, Rvalue::Ref(false, Place::local(alone)));
                        self.call_into(
                            None,
                            as_str,
                            Place::local(text),
                            vec![Operand::Move(Place::local(s))],
                            range,
                        );
                        self.sink_str(sink, Operand::Move(Place::local(text)), range);
                    }
                }
            }
        }
    }

    /// A hole with a spec written onto the `&mut dyn TextWrite` sink of `w.write_fmt(…)`
    /// with no `String` made for it (`text.write.2`): its text written once into a `Measure`
    /// to count it, then once more onto the sink through a `Padded`, which adds the fill or the
    /// zeros, the `+`, and cuts it to a precision. The hole's value is taken once.
    fn padded_hole(&mut self, h: ExprId, spec: &Spec, sink: Sink, range: TextRange) {
        let base = match self.ty(h) {
            Ty::Ref(_, t) => (*t).clone(),
            t => t,
        };
        let base = self.items.base(&base);
        let number = matches!(base, Ty::Int | Ty::Num(_));
        let text = base == Ty::Str || base == self.items.string_ty();
        let float = matches!(base, Ty::Num(n) if n.is_float());
        // The value's borrow, the precision and the width, each in a local, used twice.
        let this_ty = match self.ty(h) {
            t @ Ty::Ref(..) => t,
            t => Ty::reference(false, t),
        };
        let this_op = self.hole_ref(h);
        let this = self.temp(this_ty);
        self.cfg.assign(this, Rvalue::Use(this_op));
        let int_local = |b: &mut Self, op: Operand| {
            let t = b.temp(Ty::Int);
            b.cfg.assign(t, Rvalue::Use(op));
            t
        };
        let precision = match (spec.precision, spec.precision_arg) {
            (_, Some(x)) => {
                let op = self.operand(x, &[]);
                Some(int_local(self, op))
            }
            (Some(k), None) => Some(int_local(self, Operand::Const(Const::Int(i64::from(k))))),
            (None, None) => None,
        };
        let width = match spec.width_arg {
            Some(x) => self.operand(x, &[]),
            None => Operand::Const(Const::Int(spec.width.map_or(0, i64::from))),
        };
        let width = int_local(self, width);
        let fixed = match (precision, spec.exponent) {
            (_, Some(upper)) if float => self.items.builtin_method(&base, "exp_text").map(|f| {
                let digits = precision.map_or(Operand::Const(Const::Int(-1)), |p| {
                    Operand::Copy(Place::local(p))
                });
                (f, vec![digits, Operand::Const(Const::Bool(upper))])
            }),
            (Some(digits), None) if float => self
                .items
                .builtin_method(&base, "fixed_text")
                .map(|f| (f, vec![Operand::Copy(Place::local(digits))])),
            _ => None,
        };
        let radix = spec.radix.and_then(|r| {
            let tr = self.items.lang("RadixText")?;
            let func = self.items.traits[tr as usize]
                .methods
                .iter()
                .copied()
                .find(|&m| self.items.fns[m as usize].name == "write_radix")?;
            Some((r, func))
        });
        // Writes the hole's text onto the sink in `out` (a local holding a `&mut dyn
        // TextWrite`).
        let produce = |b: &mut Self, out: Local| {
            let this = Operand::Copy(Place::local(this));
            match (&radix, &fixed) {
                (Some((r, func)), _) => {
                    let to = b.sink_ref(Sink::Dyn(out));
                    let unit = b.temp(Ty::Unit);
                    let callee = Callee::Trait {
                        func: *func,
                        args: vec![base.clone()],
                    };
                    let args = vec![
                        this,
                        to,
                        Operand::Const(Const::Int(r.bits())),
                        Operand::Const(Const::Bool(*r == Radix::UpperHex)),
                        Operand::Const(Const::Bool(spec.prefix)),
                    ];
                    b.call_into(None, callee, Place::local(unit), args, b.range(h));
                }
                (None, Some((func, rest))) => {
                    let s = b.temp(Ty::reference(false, Ty::Str));
                    let callee = Callee::Fn {
                        func: *func,
                        args: Vec::new(),
                    };
                    let mut args = vec![this];
                    args.extend(rest.iter().cloned());
                    b.call_into(None, callee, Place::local(s), args, b.range(h));
                    b.sink_str(Sink::Dyn(out), Operand::Move(Place::local(s)), range);
                }
                (None, None) => b.write_hole_of(h, this, Sink::Dyn(out)),
            }
        };
        // First counted.
        let measure_new = self.prelude_fn("measure_new");
        let measure_ty = self.items.fns[measure_new as usize].ret.clone();
        let m = self.temp(measure_ty.clone());
        let callee = Callee::Fn {
            func: measure_new,
            args: Vec::new(),
        };
        self.call_into(None, callee, Place::local(m), Vec::new(), range);
        let counting = self.dyn_sink(m, &measure_ty);
        produce(self, counting);
        // Then written, padded.
        let align = match spec.align {
            Some(Align::Center) => 1,
            Some(Align::Right) => 2,
            None if number => 2,
            Some(Align::Left) | None => 0,
        };
        let keep = match precision {
            Some(k) if text && !spec.debug => Operand::Copy(Place::local(k)),
            _ => Operand::Const(Const::Int(-1)),
        };
        let start = self.prelude_fn("padded_start");
        let padded_ty = self.items.fns[start as usize].ret.clone();
        let p = self.temp(padded_ty.clone());
        let out = self.sink_ref(sink);
        let m_ref = self.temp(Ty::reference(false, measure_ty));
        self.cfg.assign(m_ref, Rvalue::Ref(false, Place::local(m)));
        let args = vec![
            out,
            Operand::Move(Place::local(m_ref)),
            Operand::Const(Const::Char(spec.fill.unwrap_or(' '))),
            Operand::Const(Const::Int(align)),
            Operand::Copy(Place::local(width)),
            Operand::Const(Const::Bool(spec.zero)),
            keep,
            Operand::Const(Const::Bool(spec.plus)),
        ];
        let callee = Callee::Fn {
            func: start,
            args: Vec::new(),
        };
        self.call_into(None, callee, Place::local(p), args, range);
        let writing = self.dyn_sink(p, &padded_ty);
        produce(self, writing);
        let finish = self
            .items
            .builtin_method(&padded_ty, "finish")
            .expect("the prelude's `Padded::finish`");
        let r = self.temp(Ty::reference(true, padded_ty));
        self.cfg.assign(r, Rvalue::Ref(true, Place::local(p)));
        let unit = self.temp(Ty::Unit);
        let callee = Callee::Fn {
            func: finish,
            args: Vec::new(),
        };
        self.call_into(
            None,
            callee,
            Place::local(unit),
            vec![Operand::Move(Place::local(r))],
            range,
        );
    }

    /// A local holding a `&mut dyn TextWrite` of the value in `local`, of type `ty`.
    fn dyn_sink(&mut self, local: Local, ty: &Ty) -> Local {
        let tw = self
            .items
            .lang_ref("TextWrite")
            .expect("the prelude's `TextWrite`");
        let r = self.temp(Ty::reference(true, ty.clone()));
        self.cfg.assign(r, Rvalue::Ref(true, Place::local(local)));
        let out = self.temp(Ty::reference(true, Ty::dyn_of(tw)));
        self.cfg.assign(
            out,
            Rvalue::Unsize(Operand::Move(Place::local(r)), ty.clone()),
        );
        out
    }

    /// The prelude's `String` method `name`.
    fn string_method(&self, name: &str) -> Callee {
        let func = self
            .items
            .builtin_method(&self.items.string_ty(), name)
            .expect("the prelude's `String`");
        Callee::Fn {
            func,
            args: Vec::new(),
        }
    }

    /// `s`, a `&str`, pushed onto the `String` in `dest`.
    fn push_str(&mut self, dest: Local, s: Operand, range: TextRange) {
        let out = self.temp(Ty::reference(true, self.items.string_ty()));
        self.cfg.assign(out, Rvalue::Ref(true, Place::local(dest)));
        let unit = self.temp(Ty::Unit);
        let push_str = self.string_method("push_str");
        self.call_into(
            None,
            push_str,
            Place::local(unit),
            vec![Operand::Move(Place::local(out)), s],
            range,
        );
    }

    /// A borrow of hole `h`'s value: the value itself if it is a borrow, which is written as what
    /// it points to.
    fn hole_ref(&mut self, h: ExprId) -> Operand {
        match self.ty(h) {
            Ty::Ref(..) => self.operand(h, &[]),
            t => {
                let place = self.place(h);
                let r = self.temp(Ty::reference(false, t));
                self.cfg.assign(r, Rvalue::Ref(false, place));
                Operand::Move(Place::local(r))
            }
        }
    }

    /// Hole `h` written onto `sink` by the `fmt` its check chose, which writes to it as a
    /// `&mut dyn TextWrite` (`text.write.1`).
    fn write_hole(&mut self, h: ExprId, sink: Sink) {
        let this = self.hole_ref(h);
        self.write_hole_of(h, this, sink);
    }

    /// Hole `h`, whose value's borrow is `this`, written onto `sink` by its checked `fmt`.
    fn write_hole_of(&mut self, h: ExprId, this: Operand, sink: Sink) {
        let out = self.sink_ref(sink);
        let unit = self.temp(Ty::Unit);
        let callee = self
            .types
            .fmt_callee(h)
            .expect("a checked `fmt` hole")
            .clone();
        self.call_into(
            None,
            callee,
            Place::local(unit),
            vec![this, out],
            self.range(h),
        );
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
                let Some(init) = init else {
                    // `let x;`: declared, and given its value by an assignment
                    // (`stmt.let.uninit.1`).
                    if let PatKind::Bind { local, sub: None } = self.hir.pats[pat].kind {
                        let named = self.local(local);
                        let mutable = self.hir.locals[local].mutable;
                        self.cfg.late(named, mutable);
                    }
                    return;
                };
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
                self.cfg.node = self.scopes.node();
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

/// Where `fmt"…"` writes (`text.write.2`): the `String` it makes, or a local holding the
/// `&mut dyn TextWrite` of `w.write_fmt(…)`.
#[derive(Clone, Copy)]
enum Sink {
    String(Local),
    Dyn(Local),
}

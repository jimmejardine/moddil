//! Building MIR from the typed HIR: the `mir` query.

use moddil_sema::hir::{
    self, BinaryOp, CompareOp, ExprId, ExprKind, LogicOp, PatId, PatKind, Stmt, UnaryOp,
};
use moddil_sema::{Db, Inference, Item, ItemTree, Owner, Ty, tuple_positions};
use moddil_syntax::TextRange;

use std::collections::HashMap;

use crate::{
    AggregateKind, AssertPart, BasicBlock, BinOp, BlockId, Body, Const, Local, LocalDecl, Operand,
    Place, Rvalue, Statement, Terminator, TrapClass, UnOp,
};

/// The MIR of `owner`, or `None` if its file has errors.
#[salsa::tracked(returns(ref))]
pub fn mir<'db>(db: &'db dyn Db, owner: Owner<'db>) -> Option<Body> {
    let file = owner.file(db);
    if moddil_sema::has_errors(db, file) {
        return None;
    }
    let items = moddil_sema::item_tree(db, file);
    let hir = &moddil_sema::body(db, owner).body;
    let types = moddil_sema::infer(db, owner);
    let (name, ret) = match owner.item(db) {
        Item::Fn(i) => {
            let sig = &items.fns[i as usize];
            (sig.name.clone(), sig.ret.clone())
        }
        Item::Test(i) => (format!("{:?}", items.tests[i as usize].name), Ty::Unit),
    };
    let mut b = Builder {
        hir,
        types,
        items,
        body: Body {
            name,
            locals: vec![LocalDecl {
                ty: ret,
                name: None,
            }],
            arg_count: hir.params.len(),
            blocks: Vec::new(),
        },
        current: BlockId(0),
        locals: vec![None; hir.locals.len()],
        loops: Vec::new(),
        recording: HashMap::new(),
    };
    b.current = b.new_block();
    for &p in &hir.params {
        b.local(p);
    }
    for &(local, pat) in &hir.param_pats {
        let place = Place::local(b.local(local));
        let fail = b.new_block();
        b.test_pat(pat, &place, fail);
    }
    if let Some(root) = hir.root {
        b.eval_into(Local::RETURN, root);
    }
    b.terminate(Terminator::Return);
    remove_unreachable(&mut b.body);
    Some(b.body)
}

fn idx<T>(id: la_arena::Idx<T>) -> usize {
    id.into_raw().into_u32() as usize
}

/// An enclosing loop while its body is built.
struct LoopCx {
    /// The HIR `loop` or `while`.
    id: ExprId,
    /// Where `break` goes.
    exit: BlockId,
    /// Where `continue` goes.
    header: BlockId,
    /// Where a `break` value goes.
    dest: Local,
}

struct Builder<'a> {
    hir: &'a hir::Body,
    types: &'a Inference,
    items: &'a ItemTree,
    body: Body,
    current: BlockId,
    /// The MIR local of each HIR local, once declared.
    locals: Vec<Option<Local>>,
    loops: Vec<LoopCx>,
    /// While an `assert`'s condition is built: for each of its parts, the locals holding its
    /// value and whether it was evaluated.
    recording: HashMap<ExprId, (Local, Local)>,
}

impl Builder<'_> {
    fn new_block(&mut self) -> BlockId {
        self.body.blocks.push(BasicBlock {
            statements: Vec::new(),
            terminator: Terminator::Unreachable,
        });
        BlockId(self.body.blocks.len() as u32 - 1)
    }

    fn push(&mut self, s: Statement) {
        self.body.blocks[self.current.index()].statements.push(s);
    }

    fn assign(&mut self, dest: impl Into<Place>, r: Rvalue) {
        self.push(Statement::Assign(dest.into(), r));
    }

    /// Ends the current block with `t` and continues in a new block (unreachable unless
    /// something jumps to it).
    fn terminate(&mut self, t: Terminator) {
        self.body.blocks[self.current.index()].terminator = t;
        self.current = self.new_block();
    }

    /// Ends the current block with `t` and continues in `next`.
    fn terminate_into(&mut self, t: Terminator, next: BlockId) {
        self.body.blocks[self.current.index()].terminator = t;
        self.current = next;
    }

    /// Continues in a new block if `cond` holds, and goes to `fail` if not.
    fn branch(&mut self, cond: Operand, fail: BlockId) {
        let next = self.new_block();
        self.terminate_into(
            Terminator::SwitchBool {
                cond,
                then: next,
                else_: fail,
            },
            next,
        );
    }

    fn temp(&mut self, ty: Ty) -> Local {
        self.body.locals.push(LocalDecl { ty, name: None });
        Local(self.body.locals.len() as u32 - 1)
    }

    /// The MIR local of HIR local `l`, declared on first use.
    fn local(&mut self, l: hir::LocalId) -> Local {
        if let Some(local) = self.locals[idx(l)] {
            return local;
        }
        let ty = self.types.local(l);
        self.body.locals.push(LocalDecl {
            ty,
            name: Some(self.hir.locals[l].name.clone()),
        });
        let local = Local(self.body.locals.len() as u32 - 1);
        self.locals[idx(l)] = Some(local);
        local
    }

    fn ty(&self, e: ExprId) -> Ty {
        self.types.expr(e)
    }

    fn range(&self, e: ExprId) -> TextRange {
        self.hir.exprs[e].range
    }

    /// Whether evaluating `e` may assign to a local.
    fn assigns(&self, e: ExprId) -> bool {
        let any = |xs: &[ExprId]| xs.iter().any(|&x| self.assigns(x));
        match &self.hir.exprs[e].kind {
            ExprKind::Assign { .. } => true,
            ExprKind::Missing
            | ExprKind::Int(_)
            | ExprKind::Bool(_)
            | ExprKind::Unit
            | ExprKind::Local(_)
            | ExprKind::Fn(_)
            | ExprKind::Continue { .. } => false,
            ExprKind::Assert(c) => self.assigns(*c),
            ExprKind::Tuple(xs) => any(xs),
            ExprKind::Adt { fields, .. } => fields.iter().any(|&(_, x)| self.assigns(x)),
            ExprKind::Field { base, .. } => self.assigns(*base),
            ExprKind::Call { args, .. } => any(args),
            ExprKind::Block { stmts, tail, .. } => {
                stmts.iter().any(|s| match *s {
                    Stmt::Let { init, else_, .. } => {
                        self.assigns(init) || else_.is_some_and(|x| self.assigns(x))
                    }
                    Stmt::Expr { expr, .. } => self.assigns(expr),
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

    /// The place `e` denotes, if it is a local or fields of one.
    fn place_of(&mut self, e: ExprId) -> Option<Place> {
        match self.hir.exprs[e].kind {
            ExprKind::Local(l) => Some(Place::local(self.local(l))),
            ExprKind::Field { base, .. } => {
                let i = self.types.field_index(e)?;
                Some(self.place_of(base)?.field(i))
            }
            _ => None,
        }
    }

    /// `e` as an operand. If an expression evaluated later (`later`) may assign to a local, a
    /// place's value is copied first, so the operand keeps the value it had here.
    fn operand(&mut self, e: ExprId, later: &[ExprId]) -> Operand {
        if let Some(&(v, _)) = self.recording.get(&e) {
            self.eval_into(v, e);
            return Operand::Copy(Place::local(v));
        }
        match self.hir.exprs[e].kind {
            ExprKind::Int(v) => return Operand::Const(Const::Int(v as i64)),
            ExprKind::Bool(v) => return Operand::Const(Const::Bool(v)),
            ExprKind::Unit => return Operand::Const(Const::Unit),
            _ => {}
        }
        if !later.iter().any(|&x| self.assigns(x))
            && let Some(place) = self.place_of(e)
        {
            return Operand::Copy(place);
        }
        let t = self.temp(self.ty(e));
        self.eval_into(t, e);
        Operand::Copy(Place::local(t))
    }

    /// The operands of `xs`, evaluated left to right.
    fn operands(&mut self, xs: &[ExprId]) -> Vec<Operand> {
        (0..xs.len())
            .map(|i| self.operand(xs[i], &xs[i + 1..]))
            .collect()
    }

    /// Evaluates `e` and stores its value in `dest`. A part of an `assert`'s condition is
    /// evaluated into its own local first, and marked as evaluated.
    fn eval_into(&mut self, dest: Local, e: ExprId) {
        // Each part is built once, so it leaves the table as it is built.
        if let Some((v, flag)) = self.recording.remove(&e) {
            self.eval_plain(v, e);
            self.assign(flag, Rvalue::Use(Operand::Const(Const::Bool(true))));
            if dest != v {
                self.assign(dest, Rvalue::Use(Operand::Copy(Place::local(v))));
            }
            return;
        }
        self.eval_plain(dest, e);
    }

    /// The parts of an `assert`'s condition whose values a failure shows: every sub-expression
    /// that is not a literal, through operators, calls, fields, tuples and struct expressions.
    fn assert_parts(&self, e: ExprId, out: &mut Vec<ExprId>) {
        let kind = &self.hir.exprs[e].kind;
        if !matches!(kind, ExprKind::Int(_) | ExprKind::Bool(_) | ExprKind::Unit) {
            out.push(e);
        }
        let children: Vec<ExprId> = match kind {
            ExprKind::Unary(_, x) | ExprKind::Field { base: x, .. } => vec![*x],
            ExprKind::Binary(_, l, r) | ExprKind::Logic(_, l, r) => vec![*l, *r],
            ExprKind::Compare { first, rest } => std::iter::once(*first)
                .chain(rest.iter().map(|&(_, x)| x))
                .collect(),
            ExprKind::Call { args, .. } | ExprKind::Tuple(args) => args.clone(),
            ExprKind::Adt { fields, .. } => fields.iter().map(|&(_, x)| x).collect(),
            _ => Vec::new(),
        };
        for c in children {
            self.assert_parts(c, out);
        }
    }

    fn eval_plain(&mut self, dest: Local, e: ExprId) {
        let range = self.range(e);
        match self.hir.exprs[e].kind.clone() {
            ExprKind::Missing | ExprKind::Fn(_) => {
                unreachable!("MIR is only built for bodies without errors")
            }
            ExprKind::Int(_) | ExprKind::Bool(_) | ExprKind::Unit | ExprKind::Local(_) => {
                let op = self.operand(e, &[]);
                self.assign(dest, Rvalue::Use(op));
            }
            ExprKind::Field { base, .. } => {
                let place = match self.place_of(e) {
                    Some(p) => p,
                    None => {
                        // A field of a value that is not in a local: put it in one first.
                        let i = self.types.field_index(e).expect("a checked field");
                        let t = self.temp(self.ty(base));
                        self.eval_into(t, base);
                        Place::local(t).field(i)
                    }
                };
                self.assign(dest, Rvalue::Use(Operand::Copy(place)));
            }
            ExprKind::Tuple(xs) => {
                let ops = self.operands(&xs);
                self.assign(dest, Rvalue::Aggregate(AggregateKind::Tuple, ops));
            }
            ExprKind::Adt {
                adt,
                variant,
                fields,
            } => {
                // Evaluated in source order, stored in field order.
                let values: Vec<ExprId> = fields.iter().map(|&(_, x)| x).collect();
                let ops = self.operands(&values);
                let mut ordered = vec![Operand::Const(Const::Unit); fields.len()];
                for (&(i, _), op) in fields.iter().zip(ops) {
                    ordered[i as usize] = op;
                }
                let path = self.items.adts[adt as usize].path(variant);
                self.assign(
                    dest,
                    Rvalue::Aggregate(AggregateKind::Adt { adt, variant, path }, ordered),
                );
            }
            ExprKind::Block {
                stmts,
                tail,
                observe_tail,
            } => {
                for s in &stmts {
                    self.stmt(s);
                }
                match tail {
                    Some(t) if observe_tail => {
                        let value = self.operand(t, &[]);
                        self.push(Statement::Observe {
                            range: self.range(t),
                            value: value.clone(),
                        });
                        self.assign(dest, Rvalue::Use(value));
                    }
                    Some(t) => self.eval_into(dest, t),
                    None => self.assign(dest, Rvalue::Use(Operand::Const(Const::Unit))),
                }
            }
            ExprKind::If { cond, then, else_ } => {
                let c = self.operand(cond, &[]);
                let then_bb = self.new_block();
                let join = self.new_block();
                let else_bb = match else_ {
                    Some(_) => self.new_block(),
                    None => {
                        self.assign(dest, Rvalue::Use(Operand::Const(Const::Unit)));
                        join
                    }
                };
                self.terminate_into(
                    Terminator::SwitchBool {
                        cond: c,
                        then: then_bb,
                        else_: else_bb,
                    },
                    then_bb,
                );
                self.eval_into(dest, then);
                self.terminate_into(Terminator::Goto(join), else_bb);
                if let Some(x) = else_ {
                    self.eval_into(dest, x);
                    self.terminate_into(Terminator::Goto(join), join);
                }
                self.current = join;
            }
            ExprKind::Let { pat, init } => {
                let place = self.scrutinee(init);
                let fail = self.new_block();
                let join = self.new_block();
                self.test_pat(pat, &place, fail);
                self.assign(dest, Rvalue::Use(Operand::Const(Const::Bool(true))));
                self.terminate_into(Terminator::Goto(join), fail);
                self.assign(dest, Rvalue::Use(Operand::Const(Const::Bool(false))));
                self.terminate_into(Terminator::Goto(join), join);
            }
            ExprKind::Match { scrutinee, arms } => {
                let place = self.scrutinee(scrutinee);
                let join = self.new_block();
                for arm in &arms {
                    let next = self.new_block();
                    self.test_pat(arm.pat, &place, next);
                    if let Some(g) = arm.guard {
                        let c = self.operand(g, &[]);
                        self.branch(c, next);
                    }
                    self.eval_into(dest, arm.body);
                    self.terminate_into(Terminator::Goto(join), next);
                }
                // The arms are exhaustive, so control never gets past the last one.
                self.terminate_into(Terminator::Unreachable, join);
            }
            ExprKind::Loop { body } => {
                let header = self.new_block();
                let exit = self.new_block();
                self.terminate_into(Terminator::Goto(header), header);
                self.loops.push(LoopCx {
                    id: e,
                    exit,
                    header,
                    dest,
                });
                let t = self.temp(Ty::Unit);
                self.eval_into(t, body);
                self.loops.pop();
                self.terminate_into(Terminator::Goto(header), exit);
            }
            ExprKind::While { cond, body } => {
                let header = self.new_block();
                let exit = self.new_block();
                self.terminate_into(Terminator::Goto(header), header);
                let c = self.operand(cond, &[]);
                self.branch(c, exit);
                self.loops.push(LoopCx {
                    id: e,
                    exit,
                    header,
                    dest,
                });
                let t = self.temp(Ty::Unit);
                self.eval_into(t, body);
                self.loops.pop();
                self.terminate_into(Terminator::Goto(header), exit);
                self.assign(dest, Rvalue::Use(Operand::Const(Const::Unit)));
            }
            ExprKind::Break { target, value } => {
                let cx = self
                    .loops
                    .iter()
                    .rev()
                    .find(|l| l.id == target)
                    .expect("a resolved loop");
                let (exit, loop_dest) = (cx.exit, cx.dest);
                let is_while = matches!(self.hir.exprs[target].kind, ExprKind::While { .. });
                match value {
                    Some(v) => self.eval_into(loop_dest, v),
                    None if !is_while => {
                        self.assign(loop_dest, Rvalue::Use(Operand::Const(Const::Unit)));
                    }
                    None => {}
                }
                self.terminate(Terminator::Goto(exit));
            }
            ExprKind::Continue { target } => {
                let header = self
                    .loops
                    .iter()
                    .rev()
                    .find(|l| l.id == target)
                    .expect("a resolved loop")
                    .header;
                self.terminate(Terminator::Goto(header));
            }
            ExprKind::Assert(cond) => {
                let mut exprs = Vec::new();
                self.assert_parts(cond, &mut exprs);
                let mut parts = Vec::new();
                for &x in &exprs {
                    let value = self.temp(self.ty(x));
                    let evaluated = self.temp(Ty::Bool);
                    self.assign(evaluated, Rvalue::Use(Operand::Const(Const::Bool(false))));
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
                self.push(Statement::Assert {
                    cond: c,
                    expected: true,
                    trap: TrapClass::Assertion,
                    range,
                    parts,
                });
                self.assign(dest, Rvalue::Use(Operand::Const(Const::Unit)));
            }
            ExprKind::Return(v) => {
                match v {
                    Some(v) => self.eval_into(Local::RETURN, v),
                    None => self.assign(Local::RETURN, Rvalue::Use(Operand::Const(Const::Unit))),
                }
                self.terminate(Terminator::Return);
            }
            ExprKind::Unary(op, x) => {
                let a = self.operand(x, &[]);
                if op == UnaryOp::Neg {
                    let t = self.temp(Ty::Bool);
                    self.assign(t, Rvalue::NegOverflows(a.clone()));
                    self.check(t, TrapClass::Overflow, range);
                }
                let op = match op {
                    UnaryOp::Neg => UnOp::Neg,
                    UnaryOp::Not => UnOp::Not,
                };
                self.assign(dest, Rvalue::Unary(op, a));
            }
            ExprKind::Binary(op, l, r) => {
                let a = self.operand(l, &[r]);
                let b = self.operand(r, &[]);
                self.arith(Place::local(dest), op, &self.ty(l), a, b, range);
            }
            ExprKind::Logic(op, l, r) => {
                let a = self.operand(l, &[]);
                let rhs = self.new_block();
                let short = self.new_block();
                let join = self.new_block();
                let (then, else_, value) = match op {
                    LogicOp::And => (rhs, short, false),
                    LogicOp::Or => (short, rhs, true),
                };
                self.terminate_into(
                    Terminator::SwitchBool {
                        cond: a,
                        then,
                        else_,
                    },
                    short,
                );
                self.assign(dest, Rvalue::Use(Operand::Const(Const::Bool(value))));
                self.terminate_into(Terminator::Goto(join), rhs);
                self.eval_into(dest, r);
                self.terminate_into(Terminator::Goto(join), join);
            }
            ExprKind::Compare { first, rest } => self.compare(dest, first, &rest),
            ExprKind::Assign { target, op, value } => {
                let place = self.place_of(target).expect("a checked place");
                let ty = self.ty(target);
                match op {
                    None => {
                        let t = self.temp(self.ty(value));
                        self.eval_into(t, value);
                        self.assign(place, Rvalue::Use(Operand::Copy(Place::local(t))));
                    }
                    Some(op) => {
                        let a = if self.assigns(value) {
                            let t = self.temp(ty.clone());
                            self.assign(t, Rvalue::Use(Operand::Copy(place.clone())));
                            Operand::Copy(Place::local(t))
                        } else {
                            Operand::Copy(place.clone())
                        };
                        let b = self.operand(value, &[]);
                        self.arith(place, op, &ty, a, b, range);
                    }
                }
                self.assign(dest, Rvalue::Use(Operand::Const(Const::Unit)));
            }
            ExprKind::Call { callee, args } => {
                let ExprKind::Fn(func) = self.hir.exprs[callee].kind else {
                    unreachable!("only functions are called")
                };
                let ops = self.operands(&args);
                let target = self.new_block();
                self.terminate_into(
                    Terminator::Call {
                        func,
                        args: ops,
                        dest: Place::local(dest),
                        target,
                        range,
                    },
                    target,
                );
            }
        }
    }

    /// A temporary holding the value of `e` for matching, so that a guard or arm assigning to
    /// the scrutinee's local cannot change what later arms test.
    fn scrutinee(&mut self, e: ExprId) -> Place {
        let t = self.temp(self.ty(e));
        let op = self.operand(e, &[]);
        self.assign(t, Rvalue::Use(op));
        Place::local(t)
    }

    /// Tests whether the value at `place` matches `pat`, binding its names as it goes; continues
    /// in the current block if it does and goes to `fail` if not.
    fn test_pat(&mut self, pat: PatId, place: &Place, fail: BlockId) {
        match self.hir.pats[pat].kind.clone() {
            PatKind::Missing | PatKind::Wild | PatKind::Unit => {}
            PatKind::Bind { local, sub } => {
                if let Some(s) = sub {
                    self.test_pat(s, place, fail);
                }
                let l = self.local(local);
                self.assign(l, Rvalue::Use(Operand::Copy(place.clone())));
            }
            PatKind::Int(v) => self.test_cmp(BinOp::Eq, place, Const::Int(v as i64), fail),
            PatKind::Bool(v) => self.test_cmp(BinOp::Eq, place, Const::Bool(v), fail),
            PatKind::Range { lo, hi, inclusive } => {
                self.test_cmp(BinOp::Ge, place, Const::Int(lo as i64), fail);
                if let Some(hi) = hi {
                    let op = if inclusive { BinOp::Le } else { BinOp::Lt };
                    self.test_cmp(op, place, Const::Int(hi as i64), fail);
                }
            }
            PatKind::Tuple { pats, rest } => {
                let arity = match self.types.pat(pat) {
                    Ty::Tuple(ts) => ts.len(),
                    _ => pats.len(),
                };
                for (&s, i) in pats.iter().zip(tuple_positions(pats.len(), rest, arity)) {
                    self.test_pat(s, &place.field(i as u32), fail);
                }
            }
            PatKind::Adt {
                adt,
                variant,
                fields,
            } => {
                let inner = if self.items.adts[adt as usize].is_enum {
                    let d = self.temp(Ty::Int);
                    self.assign(d, Rvalue::Discriminant(place.clone()));
                    self.test_cmp(
                        BinOp::Eq,
                        &Place::local(d),
                        Const::Int(i64::from(variant)),
                        fail,
                    );
                    place.downcast(variant)
                } else {
                    place.clone()
                };
                for (i, s) in fields {
                    self.test_pat(s, &inner.field(i), fail);
                }
            }
            PatKind::Or(alts) => {
                let matched = self.new_block();
                for (k, &alt) in alts.iter().enumerate() {
                    let next = if k + 1 == alts.len() {
                        fail
                    } else {
                        self.new_block()
                    };
                    self.test_pat(alt, place, next);
                    self.terminate_into(Terminator::Goto(matched), next);
                }
                self.current = matched;
            }
        }
    }

    /// Continues if `place op c` holds, else goes to `fail`.
    fn test_cmp(&mut self, op: BinOp, place: &Place, c: Const, fail: BlockId) {
        let t = self.temp(Ty::Bool);
        self.assign(
            t,
            Rvalue::Binary(op, Operand::Copy(place.clone()), Operand::Const(c)),
        );
        self.branch(Operand::Copy(Place::local(t)), fail);
    }

    fn stmt(&mut self, s: &Stmt) {
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
                    self.assign(named, Rvalue::Use(Operand::Copy(Place::local(t))));
                    return;
                }
                let t = self.temp(self.ty(init));
                self.eval_into(t, init);
                let fail = self.new_block();
                self.test_pat(pat, &Place::local(t), fail);
                if let Some(x) = else_ {
                    let matched = self.current;
                    self.current = fail;
                    // The `else` block diverges.
                    let ignored = self.temp(Ty::Never);
                    self.eval_into(ignored, x);
                    self.terminate_into(Terminator::Unreachable, matched);
                }
            }
            Stmt::Expr {
                expr,
                observe: true,
                ..
            } => {
                let value = self.operand(expr, &[]);
                self.push(Statement::Observe {
                    range: self.range(expr),
                    value,
                });
            }
            Stmt::Expr { expr, .. } => {
                let t = self.temp(self.ty(expr));
                self.eval_into(t, expr);
            }
        }
    }

    /// Traps with `trap` if `flag` holds.
    fn check(&mut self, flag: Local, trap: TrapClass, range: TextRange) {
        self.push(Statement::Assert {
            cond: Operand::Copy(Place::local(flag)),
            expected: false,
            trap,
            range,
            parts: Vec::new(),
        });
    }

    /// `dest = a op b`, with the checks the operator needs.
    fn arith(
        &mut self,
        dest: Place,
        op: BinaryOp,
        ty: &Ty,
        a: Operand,
        b: Operand,
        range: TextRange,
    ) {
        let op = match op {
            BinaryOp::Add => BinOp::Add,
            BinaryOp::Sub => BinOp::Sub,
            BinaryOp::Mul => BinOp::Mul,
            BinaryOp::Div => BinOp::Div,
            BinaryOp::Rem => BinOp::Rem,
            BinaryOp::BitAnd => BinOp::BitAnd,
            BinaryOp::BitOr => BinOp::BitOr,
            BinaryOp::BitXor => BinOp::BitXor,
            BinaryOp::Shl => BinOp::Shl,
            BinaryOp::Shr => BinOp::Shr,
        };
        if *ty == Ty::Int {
            if matches!(op, BinOp::Div | BinOp::Rem) {
                let t = self.temp(Ty::Bool);
                self.assign(
                    t,
                    Rvalue::Binary(BinOp::Eq, b.clone(), Operand::Const(Const::Int(0))),
                );
                self.check(t, TrapClass::DivisionByZero, range);
            }
            if matches!(
                op,
                BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Shl | BinOp::Shr
            ) {
                let t = self.temp(Ty::Bool);
                self.assign(t, Rvalue::Overflows(op, a.clone(), b.clone()));
                self.check(t, TrapClass::Overflow, range);
            }
        }
        self.assign(dest, Rvalue::Binary(op, a, b));
    }

    /// `a < b <= c`: each operand once, left to right, stopping at the first comparison that
    /// fails.
    fn compare(&mut self, dest: Local, first: ExprId, rest: &[(CompareOp, ExprId)]) {
        let operands: Vec<ExprId> = rest.iter().map(|&(_, x)| x).collect();
        let mut prev = self.operand(first, &operands);
        if let [(op, x)] = rest {
            let cur = self.operand(*x, &[]);
            self.assign(dest, Rvalue::Binary(compare_op(*op), prev, cur));
            return;
        }
        let join = self.new_block();
        for (i, &(op, x)) in rest.iter().enumerate() {
            let cur = self.operand(x, &operands[i + 1..]);
            self.assign(dest, Rvalue::Binary(compare_op(op), prev, cur.clone()));
            if i + 1 < rest.len() {
                let next = self.new_block();
                self.terminate_into(
                    Terminator::SwitchBool {
                        cond: Operand::Copy(Place::local(dest)),
                        then: next,
                        else_: join,
                    },
                    next,
                );
            }
            prev = cur;
        }
        self.terminate_into(Terminator::Goto(join), join);
    }
}

fn compare_op(op: CompareOp) -> BinOp {
    match op {
        CompareOp::Eq => BinOp::Eq,
        CompareOp::Ne => BinOp::Ne,
        CompareOp::Lt => BinOp::Lt,
        CompareOp::Le => BinOp::Le,
        CompareOp::Gt => BinOp::Gt,
        CompareOp::Ge => BinOp::Ge,
    }
}

/// Removes the blocks control never reaches (those after a `return`, say) and renumbers the
/// rest in order.
fn remove_unreachable(body: &mut Body) {
    let mut reachable = vec![false; body.blocks.len()];
    let mut work = vec![BlockId(0)];
    while let Some(b) = work.pop() {
        if std::mem::replace(&mut reachable[b.index()], true) {
            continue;
        }
        work.extend(successors(&body.blocks[b.index()].terminator));
    }
    let mut renumber = vec![BlockId(0); body.blocks.len()];
    let mut next = 0;
    for (i, &r) in reachable.iter().enumerate() {
        if r {
            renumber[i] = BlockId(next);
            next += 1;
        }
    }
    let blocks = std::mem::take(&mut body.blocks);
    for (i, mut block) in blocks.into_iter().enumerate() {
        if !reachable[i] {
            continue;
        }
        match &mut block.terminator {
            Terminator::Goto(t) | Terminator::Call { target: t, .. } => *t = renumber[t.index()],
            Terminator::SwitchBool { then, else_, .. } => {
                *then = renumber[then.index()];
                *else_ = renumber[else_.index()];
            }
            Terminator::Return | Terminator::Unreachable => {}
        }
        body.blocks.push(block);
    }
}

/// The blocks a terminator can go to.
pub(crate) fn successors(t: &Terminator) -> Vec<BlockId> {
    match t {
        Terminator::Goto(t) | Terminator::Call { target: t, .. } => vec![*t],
        Terminator::SwitchBool { then, else_, .. } => vec![*then, *else_],
        Terminator::Return | Terminator::Unreachable => vec![],
    }
}

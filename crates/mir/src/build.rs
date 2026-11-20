//! Building MIR from the typed HIR: the `mir` query.

use moddil_sema::hir::{self, BinaryOp, CompareOp, ExprId, ExprKind, LogicOp, Stmt, UnaryOp};
use moddil_sema::{Db, Inference, Item, Owner, Ty};
use moddil_syntax::TextRange;

use crate::{
    BasicBlock, BinOp, BlockId, Body, Const, Local, LocalDecl, Operand, Rvalue, Statement,
    Terminator, TrapClass, UnOp,
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
            (sig.name.clone(), sig.ret)
        }
        Item::Test(i) => (format!("{:?}", items.tests[i as usize].name), Ty::Unit),
    };
    let mut b = Builder {
        hir,
        types,
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
    };
    b.current = b.new_block();
    for &p in &hir.params {
        let local = b.named(p);
        b.locals[idx(p)] = Some(local);
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

struct Builder<'a> {
    hir: &'a hir::Body,
    types: &'a Inference,
    body: Body,
    current: BlockId,
    /// The MIR local of each HIR local, once declared.
    locals: Vec<Option<Local>>,
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

    fn assign(&mut self, dest: Local, r: Rvalue) {
        self.push(Statement::Assign(dest, r));
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

    fn temp(&mut self, ty: Ty) -> Local {
        self.body.locals.push(LocalDecl { ty, name: None });
        Local(self.body.locals.len() as u32 - 1)
    }

    /// A new MIR local for HIR local `l`.
    fn named(&mut self, l: hir::LocalId) -> Local {
        let ty = self.types.local(l);
        self.body.locals.push(LocalDecl {
            ty,
            name: Some(self.hir.locals[l].name.clone()),
        });
        Local(self.body.locals.len() as u32 - 1)
    }

    fn ty(&self, e: ExprId) -> Ty {
        self.types.expr(e)
    }

    fn range(&self, e: ExprId) -> TextRange {
        self.hir.exprs[e].range
    }

    /// Whether evaluating `e` may assign to a local.
    fn assigns(&self, e: ExprId) -> bool {
        match &self.hir.exprs[e].kind {
            ExprKind::Assign { .. } => true,
            ExprKind::Missing
            | ExprKind::Int(_)
            | ExprKind::Bool(_)
            | ExprKind::Unit
            | ExprKind::Local(_)
            | ExprKind::Fn(_) => false,
            ExprKind::Call { args, .. } => args.iter().any(|&x| self.assigns(x)),
            ExprKind::Block { stmts, tail, .. } => {
                stmts.iter().any(|s| match *s {
                    Stmt::Let { init, .. } => self.assigns(init),
                    Stmt::Expr { expr, .. } => self.assigns(expr),
                }) || tail.is_some_and(|t| self.assigns(t))
            }
            ExprKind::If { cond, then, else_ } => {
                self.assigns(*cond) || self.assigns(*then) || else_.is_some_and(|x| self.assigns(x))
            }
            ExprKind::Return(v) => v.is_some_and(|v| self.assigns(v)),
            ExprKind::Unary(_, x) => self.assigns(*x),
            ExprKind::Binary(_, l, r) | ExprKind::Logic(_, l, r) => {
                self.assigns(*l) || self.assigns(*r)
            }
            ExprKind::Compare { first, rest } => {
                self.assigns(*first) || rest.iter().any(|&(_, x)| self.assigns(x))
            }
        }
    }

    /// `e` as an operand. If an expression evaluated later (`later`) may assign to a local, a
    /// local's value is copied first, so the operand keeps the value it had here.
    fn operand(&mut self, e: ExprId, later: &[ExprId]) -> Operand {
        match self.hir.exprs[e].kind {
            ExprKind::Int(v) => Operand::Const(Const::Int(v as i64)),
            ExprKind::Bool(v) => Operand::Const(Const::Bool(v)),
            ExprKind::Unit => Operand::Const(Const::Unit),
            ExprKind::Local(l) if !later.iter().any(|&x| self.assigns(x)) => {
                Operand::Copy(self.local(l))
            }
            _ => {
                let t = self.temp(self.ty(e));
                self.eval_into(t, e);
                Operand::Copy(t)
            }
        }
    }

    fn local(&self, l: hir::LocalId) -> Local {
        self.locals[idx(l)].expect("locals are declared before use")
    }

    /// Evaluates `e` and stores its value in `dest`.
    fn eval_into(&mut self, dest: Local, e: ExprId) {
        let range = self.range(e);
        match self.hir.exprs[e].kind.clone() {
            ExprKind::Missing | ExprKind::Fn(_) => {
                unreachable!("MIR is only built for bodies without errors")
            }
            ExprKind::Int(_) | ExprKind::Bool(_) | ExprKind::Unit | ExprKind::Local(_) => {
                let op = self.operand(e, &[]);
                self.assign(dest, Rvalue::Use(op));
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
                            value,
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
                    self.assign(t, Rvalue::NegOverflows(a));
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
                self.arith(dest, op, self.ty(l), a, b, range);
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
            ExprKind::Assign { local, op, value } => {
                let l = self.local(local);
                match op {
                    None => {
                        let t = self.temp(self.ty(value));
                        self.eval_into(t, value);
                        self.assign(l, Rvalue::Use(Operand::Copy(t)));
                    }
                    Some(op) => {
                        let a = if self.assigns(value) {
                            let t = self.temp(self.types.local(local));
                            self.assign(t, Rvalue::Use(Operand::Copy(l)));
                            Operand::Copy(t)
                        } else {
                            Operand::Copy(l)
                        };
                        let b = self.operand(value, &[]);
                        self.arith(l, op, self.types.local(local), a, b, range);
                    }
                }
                self.assign(dest, Rvalue::Use(Operand::Const(Const::Unit)));
            }
            ExprKind::Call { callee, args } => {
                let ExprKind::Fn(func) = self.hir.exprs[callee].kind else {
                    unreachable!("only functions are called")
                };
                let ops: Vec<Operand> = (0..args.len())
                    .map(|i| self.operand(args[i], &args[i + 1..]))
                    .collect();
                let target = self.new_block();
                self.terminate_into(
                    Terminator::Call {
                        func,
                        args: ops,
                        dest,
                        target,
                        range,
                    },
                    target,
                );
            }
        }
    }

    fn stmt(&mut self, s: &Stmt) {
        match *s {
            Stmt::Let { local, init, .. } => match local {
                Some(l) => {
                    // Declared after its initialiser is evaluated: `let x = x + 1` reads the
                    // outer `x`.
                    let t = self.temp(self.types.local(l));
                    self.eval_into(t, init);
                    let named = self.named(l);
                    self.locals[idx(l)] = Some(named);
                    self.assign(named, Rvalue::Use(Operand::Copy(t)));
                }
                None => {
                    let t = self.temp(self.ty(init));
                    self.eval_into(t, init);
                }
            },
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
            cond: Operand::Copy(flag),
            expected: false,
            trap,
            range,
        });
    }

    /// `dest = a op b`, with the checks the operator needs.
    fn arith(
        &mut self,
        dest: Local,
        op: BinaryOp,
        ty: Ty,
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
        if ty == Ty::Int {
            if matches!(op, BinOp::Div | BinOp::Rem) {
                let t = self.temp(Ty::Bool);
                self.assign(
                    t,
                    Rvalue::Binary(BinOp::Eq, b, Operand::Const(Const::Int(0))),
                );
                self.check(t, TrapClass::DivisionByZero, range);
            }
            if matches!(
                op,
                BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Shl | BinOp::Shr
            ) {
                let t = self.temp(Ty::Bool);
                self.assign(t, Rvalue::Overflows(op, a, b));
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
            self.assign(dest, Rvalue::Binary(compare_op(op), prev, cur));
            if i + 1 < rest.len() {
                let next = self.new_block();
                self.terminate_into(
                    Terminator::SwitchBool {
                        cond: Operand::Copy(dest),
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

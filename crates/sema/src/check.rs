//! Type checking a body: the `infer` query.

use la_arena::RawIdx;
use moddil_syntax::TextRange;

use crate::hir::{Body, ExprId, ExprKind, LocalId, Stmt, UnaryOp};
use crate::items::item_tree;
use crate::lower::body;
use crate::{Code, Db, Diagnostic, Item, ItemTree, Owner, Ty};

/// The types of a body's expressions and locals, with the type errors found.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Inference {
    expr_types: Vec<Ty>,
    local_types: Vec<Ty>,
    /// Type errors.
    pub diagnostics: Vec<Diagnostic>,
}

impl Inference {
    /// The type of expression `e`.
    pub fn expr(&self, e: ExprId) -> Ty {
        self.expr_types
            .get(e.into_raw().into_u32() as usize)
            .copied()
            .unwrap_or(Ty::Error)
    }

    /// The type of local `l`.
    pub fn local(&self, l: LocalId) -> Ty {
        self.local_types
            .get(l.into_raw().into_u32() as usize)
            .copied()
            .unwrap_or(Ty::Error)
    }
}

fn index<T>(id: la_arena::Idx<T>) -> usize {
    let raw: RawIdx = id.into_raw();
    raw.into_u32() as usize
}

/// Type checks the body of `owner`.
#[salsa::tracked(returns(ref))]
pub fn infer<'db>(db: &'db dyn Db, owner: Owner<'db>) -> Inference {
    let lowered = body(db, owner);
    let items = item_tree(db, owner.file(db));
    let body = &lowered.body;
    let (ret, ret_range, is_test) = match owner.item(db) {
        Item::Fn(i) => {
            let sig = &items.fns[i as usize];
            (sig.ret, sig.ret_range, false)
        }
        Item::Test(_) => (Ty::Unit, None, true),
    };
    let mut cx = Checker {
        body,
        items,
        ret,
        ret_range,
        out: Inference {
            expr_types: vec![Ty::Error; body.exprs.len()],
            local_types: vec![Ty::Error; body.locals.len()],
            diagnostics: Vec::new(),
        },
    };
    if let Item::Fn(i) = owner.item(db) {
        for (&local, p) in body.params.iter().zip(&items.fns[i as usize].params) {
            cx.out.local_types[index(local)] = p.ty;
        }
    }
    if let Some(root) = body.root {
        let t = cx.infer(root);
        // A test's final value is observed, not returned, so it may have any type.
        if !is_test && !t.fits(ret) {
            let at = match &body.exprs[root].kind {
                ExprKind::Block {
                    tail: Some(tail), ..
                } => body.exprs[*tail].range,
                _ => body.exprs[root].range,
            };
            let mut d = Diagnostic::error(
                Code::E0201,
                at,
                format!("expected `{ret}` because of the return type, found `{t}`"),
            );
            if let Some(r) = ret_range {
                d = d.with_label(r, "the return type");
            }
            cx.out.diagnostics.push(d);
        }
    }
    cx.out
}

struct Checker<'a> {
    body: &'a Body,
    items: &'a ItemTree,
    ret: Ty,
    ret_range: Option<TextRange>,
    out: Inference,
}

/// The type of an expression whose branches have types `a` and `b`, if they agree.
fn join(a: Ty, b: Ty) -> Option<Ty> {
    match (a, b) {
        (Ty::Error, _) | (_, Ty::Error) => Some(Ty::Error),
        (Ty::Never, t) | (t, Ty::Never) => Some(t),
        (a, b) if a == b => Some(a),
        _ => None,
    }
}

impl Checker<'_> {
    fn range(&self, e: ExprId) -> TextRange {
        self.body.exprs[e].range
    }

    fn mismatch(&mut self, e: ExprId, expected: Ty, found: Ty) {
        let d = Diagnostic::error(
            Code::E0201,
            self.range(e),
            format!("expected `{expected}`, found `{found}`"),
        );
        self.out.diagnostics.push(d);
    }

    fn check(&mut self, e: ExprId, expected: Ty) -> Ty {
        let t = self.infer(e);
        if !t.fits(expected) {
            self.mismatch(e, expected, t);
        }
        t
    }

    fn infer(&mut self, e: ExprId) -> Ty {
        let t = self.infer_inner(e);
        self.out.expr_types[index(e)] = t;
        t
    }

    fn infer_inner(&mut self, e: ExprId) -> Ty {
        let range = self.range(e);
        match &self.body.exprs[e].kind {
            ExprKind::Missing => Ty::Error,
            &ExprKind::Int(v) => {
                if i64::try_from(v).is_err() {
                    self.out.diagnostics.push(Diagnostic::error(
                        Code::E0204,
                        range,
                        format!("`{v}` does not fit in `int` (a signed 64-bit integer)"),
                    ));
                }
                Ty::Int
            }
            ExprKind::Bool(_) => Ty::Bool,
            ExprKind::Unit => Ty::Unit,
            &ExprKind::Local(l) => self.out.local_types[index(l)],
            ExprKind::Fn(_) => {
                self.out
                    .diagnostics
                    .push(Diagnostic::unsupported(range, "a function used as a value"));
                Ty::Error
            }
            ExprKind::Call { callee, args } => self.call(e, *callee, args),
            ExprKind::Block {
                stmts,
                tail,
                observe_tail: _,
            } => self.block(stmts, *tail),
            &ExprKind::If { cond, then, else_ } => {
                self.check(cond, Ty::Bool);
                let t1 = self.infer(then);
                match else_ {
                    None => {
                        if !t1.fits(Ty::Unit) {
                            self.out.diagnostics.push(Diagnostic::error(
                                Code::E0201,
                                self.range(then),
                                format!(
                                    "an `if` without `else` has type `()`, but this branch has \
                                     type `{t1}`"
                                ),
                            ));
                        }
                        Ty::Unit
                    }
                    Some(else_) => {
                        let t2 = self.infer(else_);
                        join(t1, t2).unwrap_or_else(|| {
                            let d = Diagnostic::error(
                                Code::E0201,
                                self.range(else_),
                                format!(
                                    "the branches of an `if` must have the same type: expected \
                                     `{t1}`, found `{t2}`"
                                ),
                            )
                            .with_label(self.range(then), format!("this branch has type `{t1}`"));
                            self.out.diagnostics.push(d);
                            t1
                        })
                    }
                }
            }
            &ExprKind::Return(value) => {
                let ret = self.ret;
                match value {
                    Some(v) => {
                        self.check(v, ret);
                    }
                    None if !Ty::Unit.fits(ret) => {
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
            &ExprKind::Unary(op, x) => {
                let t = self.infer(x);
                match (op, t) {
                    (_, Ty::Error | Ty::Never) => Ty::Error,
                    (UnaryOp::Neg | UnaryOp::Not, Ty::Int) => Ty::Int,
                    (UnaryOp::Not, Ty::Bool) => Ty::Bool,
                    _ => {
                        let sym = if op == UnaryOp::Neg { "-" } else { "!" };
                        self.out.diagnostics.push(Diagnostic::error(
                            Code::E0205,
                            range,
                            format!("`{sym}` is not defined for `{t}`"),
                        ));
                        Ty::Error
                    }
                }
            }
            &ExprKind::Binary(op, l, r) => {
                let lt = self.infer(l);
                self.binary(range, op, lt, r)
            }
            &ExprKind::Logic(_, l, r) => {
                self.check(l, Ty::Bool);
                self.check(r, Ty::Bool);
                Ty::Bool
            }
            ExprKind::Compare { first, rest } => {
                let mut prev = (*first, self.infer(*first));
                for &(op, x) in rest {
                    let t = self.infer(x);
                    let (p, pt) = prev;
                    if op.is_ordering() {
                        for (side, st) in [(p, pt), (x, t)] {
                            if !st.fits(Ty::Int) {
                                self.out.diagnostics.push(Diagnostic::error(
                                    Code::E0205,
                                    self.range(side),
                                    format!("`{}` is not defined for `{st}`", op.symbol()),
                                ));
                            }
                        }
                    } else if join(pt, t).is_none() {
                        self.mismatch(x, pt, t);
                    }
                    prev = (x, t);
                }
                Ty::Bool
            }
            &ExprKind::Assign { local, op, value } => {
                let lt = self.out.local_types[index(local)];
                match op {
                    None => {
                        self.check(value, lt);
                    }
                    Some(op) => {
                        self.binary(range, op, lt, value);
                    }
                }
                Ty::Unit
            }
        }
    }

    /// `l op r`, where `l` has type `lt`.
    fn binary(&mut self, range: TextRange, op: crate::hir::BinaryOp, lt: Ty, r: ExprId) -> Ty {
        match lt {
            Ty::Int => {
                self.check(r, Ty::Int);
                Ty::Int
            }
            Ty::Bool if op.applies_to_bool() => {
                self.check(r, Ty::Bool);
                Ty::Bool
            }
            Ty::Error | Ty::Never => {
                self.infer(r);
                Ty::Error
            }
            _ => {
                self.infer(r);
                self.out.diagnostics.push(Diagnostic::error(
                    Code::E0205,
                    range,
                    format!("`{}` is not defined for `{lt}`", op.symbol()),
                ));
                Ty::Error
            }
        }
    }

    fn call(&mut self, e: ExprId, callee: ExprId, args: &[ExprId]) -> Ty {
        let range = self.range(e);
        let ExprKind::Fn(f) = self.body.exprs[callee].kind else {
            let t = self.infer(callee);
            if t != Ty::Error {
                self.out.diagnostics.push(Diagnostic::error(
                    Code::E0203,
                    self.range(callee),
                    format!("this is a `{t}`, not a function"),
                ));
            }
            for &a in args {
                self.infer(a);
            }
            return Ty::Error;
        };
        let sig = &self.items.fns[f as usize];
        self.out.expr_types[index(callee)] = Ty::Unit;
        if args.len() != sig.params.len() {
            let d = Diagnostic::error(
                Code::E0202,
                range,
                format!(
                    "`{}` takes {} argument{}, but {} {} given",
                    sig.name,
                    sig.params.len(),
                    if sig.params.len() == 1 { "" } else { "s" },
                    args.len(),
                    if args.len() == 1 { "was" } else { "were" },
                ),
            )
            .with_label(sig.name_range, "defined here");
            self.out.diagnostics.push(d);
        }
        let params: Vec<Ty> = sig.params.iter().map(|p| p.ty).collect();
        let ret = sig.ret;
        for (i, &a) in args.iter().enumerate() {
            match params.get(i) {
                Some(&t) => {
                    self.check(a, t);
                }
                None => {
                    self.infer(a);
                }
            }
        }
        ret
    }

    fn block(&mut self, stmts: &[Stmt], tail: Option<ExprId>) -> Ty {
        let mut diverges = false;
        for s in stmts {
            match *s {
                Stmt::Let { local, ty, init } => {
                    let t = match ty {
                        Some(ty) => {
                            self.check(init, ty);
                            ty
                        }
                        None => self.infer(init),
                    };
                    diverges |= t == Ty::Never;
                    if let Some(l) = local {
                        self.out.local_types[index(l)] = t;
                    }
                }
                Stmt::Expr {
                    expr,
                    semi,
                    observe,
                } => {
                    let t = self.infer(expr);
                    diverges |= t == Ty::Never;
                    // A value followed by a `// =>` expectation is used by it.
                    if !semi && !observe && !t.fits(Ty::Unit) {
                        self.out.diagnostics.push(Diagnostic::error(
                            Code::E0201,
                            self.range(expr),
                            format!(
                                "expected `()`, found `{t}`: a value in the middle of a block \
                                 needs a `;` after it"
                            ),
                        ));
                    }
                }
            }
        }
        match tail {
            Some(t) => self.infer(t),
            None if diverges => Ty::Never,
            None => Ty::Unit,
        }
    }
}

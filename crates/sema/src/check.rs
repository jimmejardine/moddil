//! Type checking a body: the `infer` query. It also checks that patterns are exhaustive where
//! they must be ([`crate::exhaust`]).

use std::collections::HashMap;

use la_arena::RawIdx;
use moddil_syntax::TextRange;

use crate::exhaust;
use crate::hir::{Arm, Body, ExprId, ExprKind, LocalId, PatId, PatKind, Stmt, UnaryOp};
use crate::items::item_tree;
use crate::lower::body;
use crate::{Code, Db, Diagnostic, Item, ItemTree, Owner, Ty};

/// The types of a body's expressions, patterns and locals, with the type errors found.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Inference {
    expr_types: Vec<Ty>,
    pat_types: Vec<Ty>,
    local_types: Vec<Ty>,
    field_indices: HashMap<ExprId, u32>,
    /// Type errors, and exhaustiveness errors and warnings.
    pub diagnostics: Vec<Diagnostic>,
}

impl Inference {
    /// The type of expression `e`.
    pub fn expr(&self, e: ExprId) -> Ty {
        self.expr_types.get(index(e)).cloned().unwrap_or(Ty::Error)
    }

    /// The type of the value pattern `p` matches.
    pub fn pat(&self, p: PatId) -> Ty {
        self.pat_types.get(index(p)).cloned().unwrap_or(Ty::Error)
    }

    /// The type of local `l`.
    pub fn local(&self, l: LocalId) -> Ty {
        self.local_types.get(index(l)).cloned().unwrap_or(Ty::Error)
    }

    /// For a field expression `base.name`: the field's index in its struct or tuple.
    pub fn field_index(&self, e: ExprId) -> Option<u32> {
        self.field_indices.get(&e).copied()
    }
}

pub(crate) fn index<T>(id: la_arena::Idx<T>) -> usize {
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
            (sig.ret.clone(), sig.ret_range, false)
        }
        Item::Test(_) => (Ty::Unit, None, true),
    };
    let mut cx = Checker {
        body,
        items,
        ret: ret.clone(),
        ret_range,
        loops: Vec::new(),
        out: Inference {
            expr_types: vec![Ty::Error; body.exprs.len()],
            pat_types: vec![Ty::Error; body.pats.len()],
            local_types: vec![Ty::Error; body.locals.len()],
            field_indices: HashMap::new(),
            diagnostics: Vec::new(),
        },
    };
    if let Item::Fn(i) = owner.item(db) {
        for (&local, p) in body.params.iter().zip(&items.fns[i as usize].params) {
            cx.out.local_types[index(local)] = p.ty.clone();
        }
        for &(local, pat) in &body.param_pats {
            let t = cx.out.local_types[index(local)].clone();
            cx.check_pat(pat, &t);
            cx.irrefutable(pat, "a parameter");
        }
    }
    if let Some(root) = body.root {
        let t = cx.infer(root);
        // A test's final value is observed, not returned, so it may have any type.
        if !is_test && !t.fits(&ret) {
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

/// An enclosing loop while its body is checked.
struct LoopCx {
    /// The `loop` or `while` expression.
    id: ExprId,
    /// Whether it is a `while`, which cannot `break` with a value.
    is_while: bool,
    /// The type of the values `break` gives it so far.
    ty: Option<Ty>,
}

struct Checker<'a> {
    body: &'a Body,
    items: &'a ItemTree,
    ret: Ty,
    ret_range: Option<TextRange>,
    loops: Vec<LoopCx>,
    out: Inference,
}

/// The type of an expression whose branches have types `a` and `b`, if they agree.
fn join(a: &Ty, b: &Ty) -> Option<Ty> {
    match (a, b) {
        (Ty::Error, _) | (_, Ty::Error) => Some(Ty::Error),
        (Ty::Never, t) | (t, Ty::Never) => Some(t.clone()),
        (a, b) if a == b => Some(a.clone()),
        _ => None,
    }
}

impl Checker<'_> {
    fn range(&self, e: ExprId) -> TextRange {
        self.body.exprs[e].range
    }

    fn report(&mut self, code: Code, range: TextRange, message: impl Into<String>) {
        self.out
            .diagnostics
            .push(Diagnostic::error(code, range, message));
    }

    fn mismatch(&mut self, e: ExprId, expected: &Ty, found: &Ty) {
        self.report(
            Code::E0201,
            self.range(e),
            format!("expected `{expected}`, found `{found}`"),
        );
    }

    fn check(&mut self, e: ExprId, expected: &Ty) -> Ty {
        let t = self.infer(e);
        if !t.fits(expected) {
            self.mismatch(e, expected, &t);
        }
        t
    }

    fn infer(&mut self, e: ExprId) -> Ty {
        let t = self.infer_inner(e);
        self.out.expr_types[index(e)] = t.clone();
        t
    }

    fn infer_inner(&mut self, e: ExprId) -> Ty {
        let range = self.range(e);
        match &self.body.exprs[e].kind {
            ExprKind::Missing => Ty::Error,
            &ExprKind::Int(v) => {
                if i64::try_from(v).is_err() {
                    self.report(
                        Code::E0204,
                        range,
                        format!("`{v}` does not fit in `int` (a signed 64-bit integer)"),
                    );
                }
                Ty::Int
            }
            ExprKind::Bool(_) => Ty::Bool,
            ExprKind::Unit => Ty::Unit,
            &ExprKind::Local(l) => self.out.local_types[index(l)].clone(),
            ExprKind::Fn(_) => {
                self.out
                    .diagnostics
                    .push(Diagnostic::unsupported(range, "a function used as a value"));
                Ty::Error
            }
            ExprKind::Tuple(elements) => {
                Ty::tuple(elements.iter().map(|&x| self.infer(x)).collect())
            }
            ExprKind::Adt {
                adt,
                variant,
                fields,
            } => {
                let def = self.items.variant(*adt, *variant);
                for &(i, value) in fields {
                    let t = def.fields[i as usize].ty.clone();
                    self.check(value, &t);
                }
                self.items.adts[*adt as usize].ty(*adt)
            }
            ExprKind::Field {
                base,
                name,
                name_range,
            } => self.field(e, *base, name, *name_range),
            ExprKind::Call { callee, args } => self.call(e, *callee, args),
            ExprKind::Block { stmts, tail, .. } => self.block(stmts, *tail),
            &ExprKind::If { cond, then, else_ } => self.if_expr(cond, then, else_),
            &ExprKind::Let { pat, init } => {
                let t = self.infer(init);
                self.check_pat(pat, &t);
                Ty::Bool
            }
            ExprKind::Match { scrutinee, arms } => self.match_expr(e, *scrutinee, arms),
            &ExprKind::Loop { body } => {
                self.loops.push(LoopCx {
                    id: e,
                    is_while: false,
                    ty: None,
                });
                self.check(body, &Ty::Unit);
                let cx = self.loops.pop().expect("the loop");
                // A loop without `break` never finishes.
                cx.ty.unwrap_or(Ty::Never)
            }
            &ExprKind::While { cond, body } => {
                self.check(cond, &Ty::Bool);
                self.loops.push(LoopCx {
                    id: e,
                    is_while: true,
                    ty: None,
                });
                self.check(body, &Ty::Unit);
                self.loops.pop();
                Ty::Unit
            }
            &ExprKind::Break { target, value } => {
                let t = match value {
                    Some(v) => self.infer(v),
                    None => Ty::Unit,
                };
                let Some(at) = self.loops.iter().position(|l| l.id == target) else {
                    return Ty::Never;
                };
                if self.loops[at].is_while {
                    if value.is_some() {
                        self.report(
                            Code::E0212,
                            range,
                            "only `loop` can be left with a value; `while` has type `()`",
                        );
                    }
                    return Ty::Never;
                }
                match self.loops[at].ty.clone() {
                    None => self.loops[at].ty = Some(t),
                    Some(prev) => match join(&prev, &t) {
                        Some(j) => self.loops[at].ty = Some(j),
                        None => self.report(
                            Code::E0201,
                            value.map_or(range, |v| self.range(v)),
                            format!(
                                "this loop's earlier `break` gives a `{prev}`, but this one \
                                 gives a `{t}`"
                            ),
                        ),
                    },
                }
                Ty::Never
            }
            ExprKind::Continue { .. } => Ty::Never,
            &ExprKind::Assert(cond) => {
                self.check(cond, &Ty::Bool);
                Ty::Unit
            }
            &ExprKind::Return(value) => {
                let ret = self.ret.clone();
                match value {
                    Some(v) => {
                        self.check(v, &ret);
                    }
                    None if !Ty::Unit.fits(&ret) => {
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
                match (op, &t) {
                    (_, Ty::Error | Ty::Never) => Ty::Error,
                    (UnaryOp::Neg | UnaryOp::Not, Ty::Int) => Ty::Int,
                    (UnaryOp::Not, Ty::Bool) => Ty::Bool,
                    _ => {
                        let sym = if op == UnaryOp::Neg { "-" } else { "!" };
                        self.report(
                            Code::E0205,
                            range,
                            format!("`{sym}` is not defined for `{t}`"),
                        );
                        Ty::Error
                    }
                }
            }
            &ExprKind::Binary(op, l, r) => {
                let lt = self.infer(l);
                self.binary(range, op, &lt, r)
            }
            &ExprKind::Logic(_, l, r) => {
                self.check(l, &Ty::Bool);
                self.check(r, &Ty::Bool);
                Ty::Bool
            }
            ExprKind::Compare { first, rest } => {
                let mut prev = (*first, self.infer(*first));
                for &(op, x) in rest {
                    let t = self.infer(x);
                    let (p, pt) = prev;
                    if op.is_ordering() {
                        for (side, st) in [(p, &pt), (x, &t)] {
                            if !st.fits(&Ty::Int) {
                                self.report(
                                    Code::E0205,
                                    self.range(side),
                                    format!("`{}` is not defined for `{st}`", op.symbol()),
                                );
                            }
                        }
                    } else {
                        match join(&pt, &t) {
                            None => self.mismatch(x, &pt, &t),
                            Some(Ty::Tuple(_) | Ty::Adt(..)) => self.report(
                                Code::E0205,
                                range,
                                format!(
                                    "`{}` is not defined for `{pt}`: comparing tuples, structs \
                                     and enums needs traits, which come later; match them instead",
                                    op.symbol()
                                ),
                            ),
                            Some(_) => {}
                        }
                    }
                    prev = (x, t);
                }
                Ty::Bool
            }
            &ExprKind::Assign { target, op, value } => {
                // Assigning to a function is already reported.
                let lt = match self.body.exprs[target].kind {
                    ExprKind::Fn(_) => Ty::Error,
                    _ => self.infer(target),
                };
                match op {
                    None => {
                        self.check(value, &lt);
                    }
                    Some(op) => {
                        self.binary(range, op, &lt, value);
                    }
                }
                Ty::Unit
            }
        }
    }

    fn field(&mut self, e: ExprId, base: ExprId, name: &str, name_range: TextRange) -> Ty {
        let bt = self.infer(base);
        let found = match &bt {
            Ty::Error => return Ty::Error,
            Ty::Tuple(ts) => name
                .parse::<usize>()
                .ok()
                .filter(|&i| i < ts.len())
                .map(|i| (i as u32, ts[i].clone())),
            Ty::Adt(a, _) if !self.items.adts[*a as usize].is_enum => {
                let def = self.items.variant(*a, 0);
                def.field(name)
                    .map(|i| (i, def.fields[i as usize].ty.clone()))
            }
            _ => None,
        };
        match found {
            Some((i, t)) => {
                self.out.field_indices.insert(e, i);
                t
            }
            None => {
                let hint = match &bt {
                    Ty::Adt(..) => ": an enum's fields are reached by matching it",
                    _ => "",
                };
                self.report(
                    Code::E0107,
                    name_range,
                    format!("`{bt}` has no field called `{name}`{hint}"),
                );
                Ty::Error
            }
        }
    }

    fn if_expr(&mut self, cond: ExprId, then: ExprId, else_: Option<ExprId>) -> Ty {
        self.check(cond, &Ty::Bool);
        let t1 = self.infer(then);
        let Some(else_) = else_ else {
            if !t1.fits(&Ty::Unit) {
                self.report(
                    Code::E0201,
                    self.range(then),
                    format!(
                        "an `if` without `else` has type `()`, but this branch has type `{t1}`"
                    ),
                );
            }
            return Ty::Unit;
        };
        let t2 = self.infer(else_);
        join(&t1, &t2).unwrap_or_else(|| {
            let d = Diagnostic::error(
                Code::E0201,
                self.range(else_),
                format!(
                    "the branches of an `if` must have the same type: expected `{t1}`, found \
                     `{t2}`"
                ),
            )
            .with_label(self.range(then), format!("this branch has type `{t1}`"));
            self.out.diagnostics.push(d);
            t1
        })
    }

    fn match_expr(&mut self, e: ExprId, scrutinee: ExprId, arms: &[Arm]) -> Ty {
        let st = self.infer(scrutinee);
        let mut result: Option<(Ty, ExprId)> = None;
        let errors_before = self.out.diagnostics.len();
        let mut broken = false;
        for arm in arms {
            broken |= self.has_missing(arm.pat);
            self.check_pat(arm.pat, &st);
            broken |= self.out.diagnostics.len() > errors_before;
            if let Some(g) = arm.guard {
                self.check(g, &Ty::Bool);
            }
            let t = self.infer(arm.body);
            result = match result {
                None => Some((t, arm.body)),
                Some((prev, first)) => match join(&prev, &t) {
                    Some(j) => Some((j, first)),
                    None => {
                        let d = Diagnostic::error(
                            Code::E0201,
                            self.range(arm.body),
                            format!(
                                "the arms of a `match` must have the same type: expected \
                                 `{prev}`, found `{t}`"
                            ),
                        )
                        .with_label(self.range(first), format!("this arm has type `{prev}`"));
                        self.out.diagnostics.push(d);
                        Some((prev, first))
                    }
                },
            };
        }
        // Patterns with errors would make the analysis report noise.
        if !st.has_error() && !broken {
            let ds =
                exhaust::check_match(self.items, self.body, &self.out, self.range(e), &st, arms);
            self.out.diagnostics.extend(ds);
        }
        result.map_or(Ty::Never, |(t, _)| t)
    }

    /// Whether `p` contains a pattern already reported as an error while lowering.
    fn has_missing(&self, p: PatId) -> bool {
        match &self.body.pats[p].kind {
            PatKind::Missing => true,
            PatKind::Bind { sub, .. } => sub.is_some_and(|s| self.has_missing(s)),
            PatKind::Tuple { pats, .. } | PatKind::Or(pats) => {
                pats.iter().any(|&s| self.has_missing(s))
            }
            PatKind::Adt { fields, .. } => fields.iter().any(|&(_, s)| self.has_missing(s)),
            _ => false,
        }
    }

    /// Reports E0209 if `pat` can fail to match a value of its type.
    fn irrefutable(&mut self, pat: PatId, what: &str) {
        let t = self.out.pat(pat);
        if t.has_error() {
            return;
        }
        if let Some(missing) = exhaust::refutable(self.items, self.body, &self.out, pat) {
            self.report(
                Code::E0209,
                self.body.pats[pat].range,
                format!(
                    "{what} needs a pattern that always matches, but `{missing}` is not \
                     matched"
                ),
            );
        }
    }

    /// Checks that `p` can match a value of type `expected`, and gives its bindings their types.
    fn check_pat(&mut self, p: PatId, expected: &Ty) {
        self.out.pat_types[index(p)] = expected.clone();
        let range = self.body.pats[p].range;
        let wrong = |found: &str| -> String {
            format!("expected `{expected}`, but this pattern matches {found}")
        };
        match &self.body.pats[p].kind {
            PatKind::Missing | PatKind::Wild => {}
            &PatKind::Bind { local, sub } => {
                self.out.local_types[index(local)] = expected.clone();
                if let Some(s) = sub {
                    self.check_pat(s, expected);
                }
            }
            &PatKind::Int(v) => {
                if !Ty::Int.fits(expected) {
                    self.report(Code::E0201, range, wrong("an `int`"));
                } else if i64::try_from(v).is_err() {
                    self.report(
                        Code::E0204,
                        range,
                        format!("`{v}` does not fit in `int` (a signed 64-bit integer)"),
                    );
                }
            }
            PatKind::Bool(_) => {
                if !Ty::Bool.fits(expected) {
                    self.report(Code::E0201, range, wrong("a `bool`"));
                }
            }
            &PatKind::Range { lo, hi, inclusive } => {
                if !Ty::Int.fits(expected) {
                    self.report(Code::E0201, range, wrong("an `int`"));
                    return;
                }
                for v in [Some(lo), hi].into_iter().flatten() {
                    if i64::try_from(v).is_err() {
                        self.report(
                            Code::E0204,
                            range,
                            format!("`{v}` does not fit in `int` (a signed 64-bit integer)"),
                        );
                        return;
                    }
                }
                if let Some(hi) = hi
                    && (hi < lo || (hi == lo && !inclusive))
                {
                    self.report(
                        Code::E0214,
                        range,
                        "this range is empty, so the pattern matches nothing",
                    );
                }
            }
            PatKind::Unit => {
                if !Ty::Unit.fits(expected) {
                    self.report(Code::E0201, range, wrong("`()`"));
                }
            }
            PatKind::Tuple { pats, rest } => {
                let pats = pats.clone();
                let rest = *rest;
                let elements: Vec<Ty> = match expected {
                    Ty::Tuple(ts) => ts.to_vec(),
                    Ty::Error => vec![Ty::Error; pats.len()],
                    _ => {
                        self.report(Code::E0201, range, wrong("a tuple"));
                        vec![Ty::Error; pats.len()]
                    }
                };
                let fits = match rest {
                    Some(_) => pats.len() <= elements.len(),
                    None => pats.len() == elements.len(),
                };
                if !fits {
                    self.report(
                        Code::E0213,
                        range,
                        format!(
                            "`{expected}` has {} elements, but this pattern has {}",
                            elements.len(),
                            pats.len()
                        ),
                    );
                    for &s in &pats {
                        self.check_pat(s, &Ty::Error);
                    }
                    return;
                }
                let positions = tuple_positions(pats.len(), rest, elements.len());
                for (&s, i) in pats.iter().zip(positions) {
                    self.check_pat(s, &elements[i]);
                }
            }
            PatKind::Adt {
                adt,
                variant,
                fields,
            } => {
                let (adt, variant, fields) = (*adt, *variant, fields.clone());
                let def = &self.items.adts[adt as usize];
                if !def.ty(adt).fits(expected) {
                    let path = def.path(variant);
                    self.report(Code::E0201, range, wrong(&format!("`{path}`")));
                    for &(_, s) in &fields {
                        self.check_pat(s, &Ty::Error);
                    }
                    return;
                }
                for (i, s) in fields {
                    let t = self.items.variant(adt, variant).fields[i as usize]
                        .ty
                        .clone();
                    self.check_pat(s, &t);
                }
            }
            PatKind::Or(alts) => {
                for &a in &alts.clone() {
                    self.check_pat(a, expected);
                }
            }
        }
    }

    /// `l op r`, where `l` has type `lt`.
    fn binary(&mut self, range: TextRange, op: crate::hir::BinaryOp, lt: &Ty, r: ExprId) -> Ty {
        match lt {
            Ty::Int => {
                self.check(r, &Ty::Int);
                Ty::Int
            }
            Ty::Bool if op.applies_to_bool() => {
                self.check(r, &Ty::Bool);
                Ty::Bool
            }
            Ty::Error | Ty::Never => {
                self.infer(r);
                Ty::Error
            }
            _ => {
                self.infer(r);
                self.report(
                    Code::E0205,
                    range,
                    format!("`{}` is not defined for `{lt}`", op.symbol()),
                );
                Ty::Error
            }
        }
    }

    fn call(&mut self, e: ExprId, callee: ExprId, args: &[ExprId]) -> Ty {
        let range = self.range(e);
        let ExprKind::Fn(f) = self.body.exprs[callee].kind else {
            let t = self.infer(callee);
            if t != Ty::Error {
                self.report(
                    Code::E0203,
                    self.range(callee),
                    format!("this is a `{t}`, not a function"),
                );
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
        let params: Vec<Ty> = sig.params.iter().map(|p| p.ty.clone()).collect();
        let ret = sig.ret.clone();
        for (i, &a) in args.iter().enumerate() {
            match params.get(i) {
                Some(t) => {
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
                Stmt::Let {
                    pat,
                    ref ty,
                    init,
                    else_,
                } => {
                    let t = match ty {
                        Some(ty) => {
                            self.check(init, ty);
                            ty.clone()
                        }
                        None => self.infer(init),
                    };
                    diverges |= t == Ty::Never;
                    self.check_pat(pat, &t);
                    match else_ {
                        None => self.irrefutable(pat, "`let` without `else`"),
                        Some(x) => {
                            let et = self.infer(x);
                            if !matches!(et, Ty::Never | Ty::Error) {
                                self.report(
                                    Code::E0210,
                                    self.range(x),
                                    format!(
                                        "the `else` block of `let … else` must not finish \
                                         normally (end it with `return`, `break` or \
                                         `continue`), but it has type `{et}`"
                                    ),
                                );
                            }
                        }
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
                    if !semi && !observe && !t.fits(&Ty::Unit) {
                        self.report(
                            Code::E0201,
                            self.range(expr),
                            format!(
                                "expected `()`, found `{t}`: a value in the middle of a block \
                                 needs a `;` after it"
                            ),
                        );
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

/// The element positions the patterns of a tuple pattern match: `n` patterns, with `..` before
/// pattern `rest` if any, against a tuple of `arity` elements.
pub fn tuple_positions(n: usize, rest: Option<usize>, arity: usize) -> Vec<usize> {
    match rest {
        None => (0..n).collect(),
        Some(at) => (0..n)
            .map(|i| if i < at { i } else { arity - (n - i) })
            .collect(),
    }
}

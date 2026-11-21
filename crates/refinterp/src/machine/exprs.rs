//! Expressions: literals, struct literals, `if`, `match`, loops and operators.

use moddil_syntax::ast::{self, AstNode};
use moddil_syntax::{SyntaxKind as K, SyntaxNode};

use crate::program::VariantInfo;
use crate::value::{Head, Value};
use crate::{Eval, Flow, TrapClass, trap};

use super::patterns::Bind;
use super::{Machine, literal, member_name, segments};

use super::ops::{arith, compare, compound};

impl Machine<'_> {
    pub(super) fn eval(&mut self, e: &ast::Expr) -> Eval {
        let node = e.syntax();
        let range = node.text_range();
        match e {
            ast::Expr::Literal(_) => Ok(literal(node, false)),
            ast::Expr::Path(_) => {
                if let Some((p, indirect)) = self.place_of(e)? {
                    return Ok(self.take(&p, indirect));
                }
                let segments = segments(node);
                let (ty, info) = self.ctor(&segments).expect("a checked path");
                Ok(Value::Adt {
                    ty,
                    variant: info.name.clone(),
                    fields: Vec::new(),
                })
            }
            ast::Expr::Paren(_) => self.eval(&e.sub_exprs()[0]),
            ast::Expr::Tuple(_) => {
                let xs = self.eval_all(&e.sub_exprs())?;
                Ok(if xs.is_empty() {
                    Value::Unit
                } else {
                    Value::Tuple(xs)
                })
            }
            ast::Expr::StructLit(_) => self.struct_lit(node),
            ast::Expr::Field(_) => {
                // Field access looks through a borrow (`expr.field.1`).
                let (p, indirect) = self.place_of(e)?.expect("a field is a place");
                Ok(self.take(&p, indirect))
            }
            ast::Expr::Ref(_) => {
                let mutable = node.token(K::MUT_KW).is_some();
                let p = self.place(&e.sub_exprs()[0])?;
                Ok(Value::Ref(p, mutable))
            }
            ast::Expr::Block(_) => self.block(&ast::Block::cast(node.clone()).expect("a block")),
            ast::Expr::If(_) => self.if_expr(e),
            ast::Expr::LetCond(_) => {
                let init = node.children().find_map(ast::Expr::cast).expect("a value");
                let src = self.scrutinee(&init)?;
                let pat = node.children().find_map(ast::Pat::cast).expect("a pattern");
                let mut binds = Vec::new();
                let matched = self.matches(&pat, &src, &mut binds);
                if matched {
                    // In scope for the rest of the condition and the block it guards.
                    self.bind(binds);
                }
                Ok(Value::Bool(matched))
            }
            ast::Expr::Match(_) => self.match_expr(node),
            ast::Expr::Loop(_) | ast::Expr::While(_) => self.loop_expr(node, None),
            ast::Expr::Labelled(_) => {
                let label = node.token(K::LABEL).map(|t| t.text().to_string());
                let inner = node.children().find_map(ast::Expr::cast).expect("a loop");
                self.loop_expr(inner.syntax(), label.as_deref())
            }
            ast::Expr::Break(_) => {
                let label = node.token(K::LABEL).map(|t| t.text().to_string());
                let v = match e.sub_exprs().first() {
                    Some(x) => self.eval(x)?,
                    None => Value::Unit,
                };
                Err(Flow::Break(label, v))
            }
            ast::Expr::Continue(_) => {
                let label = node.token(K::LABEL).map(|t| t.text().to_string());
                Err(Flow::Continue(label))
            }
            ast::Expr::Return(_) => {
                let v = match e.sub_exprs().first() {
                    Some(x) => self.eval(x)?,
                    None => Value::Unit,
                };
                Err(Flow::Return(v))
            }
            ast::Expr::Prefix(_) => self.prefix(node, e),
            ast::Expr::Bin(_) => self.binary(node, e),
            ast::Expr::CompareChain(_) => self.chain(node, e),
            ast::Expr::Assign(_) => self.assign(node, e),
            ast::Expr::Call(_) => self.call(node, e, range),
            ast::Expr::MethodCall(_) => self.method_call(node, e, range),
            ast::Expr::Propagate(_) => self.question(node, e, false),
            ast::Expr::Unwrap(_) => self.question(node, e, true),
            ast::Expr::Fail(_) => self.fail(node, e),
            ast::Expr::Try(_) => self.try_expr(node),
            ast::Expr::With(_) => self.with_expr(node),
            _ => unreachable!("a checked program of slices S0 to S5"),
        }
    }

    pub(super) fn eval_all(&mut self, xs: &[ast::Expr]) -> Eval<Vec<Value>> {
        xs.iter().map(|x| self.eval(x)).collect()
    }

    /// The struct or variant a path names, with `Self` for the impl's type.
    pub(super) fn ctor(&self, segments: &[String]) -> Option<(String, &VariantInfo)> {
        let self_ty = match self.memory.type_of("Self") {
            Some(Head::Adt(t)) => Some(t.clone()),
            _ => None,
        };
        let resolved: Vec<String> = segments
            .iter()
            .map(|s| match (s.as_str(), &self_ty) {
                ("Self", Some(t)) => t.clone(),
                _ => s.clone(),
            })
            .collect();
        self.program.ctor(&resolved)
    }

    /// `Point { x: 1, y }`: fields evaluated in the order written (`expr.struct.order.1`).
    pub(super) fn struct_lit(&mut self, node: &SyntaxNode) -> Eval {
        let segments = segments(node);
        let (ty, info) = self.ctor(&segments).expect("a checked struct");
        let info = info.clone();
        let mut fields = vec![Value::Unit; info.fields.len()];
        for init in node.children().filter(|c| c.kind() == K::FIELD_INIT) {
            let name = member_name(&init);
            let v = match init.children().find_map(ast::Expr::cast) {
                Some(x) => self.eval(&x)?,
                None => {
                    let s = self.memory.slot(&name).expect("a checked shorthand");
                    let p = self.memory.here(s);
                    self.take(&p, false)
                }
            };
            let i = info
                .fields
                .iter()
                .position(|f| *f == name)
                .expect("a checked field");
            fields[i] = v;
        }
        Ok(Value::Adt {
            ty,
            variant: info.name,
            fields,
        })
    }

    pub(super) fn if_expr(&mut self, e: &ast::Expr) -> Eval {
        let range = e.syntax().text_range();
        let subs = e.sub_exprs();
        let mark = self.memory.mark();
        let taken = match self.eval(&subs[0]) {
            Ok(Value::Bool(b)) => b,
            Ok(_) => unreachable!("a checked condition"),
            Err(f) => return self.exit(mark, Err(f), range),
        };
        if taken {
            let result = self.eval(&subs[1]);
            self.exit(mark, result, range)
        } else {
            self.exit(mark, Ok(()), range)?;
            match subs.get(2) {
                Some(x) => self.eval(x),
                None => Ok(Value::Unit),
            }
        }
    }

    /// The arms are tried in order; the first whose pattern matches and whose guard holds runs
    /// (`expr.match.1`). Its names move or borrow parts of the value only once it is chosen.
    pub(super) fn match_expr(&mut self, node: &SyntaxNode) -> Eval {
        let range = node.text_range();
        let scrutinee = node.children().find_map(ast::Expr::cast).expect("a value");
        let src = self.scrutinee(&scrutinee)?;
        let list = node.child(K::MATCH_ARM_LIST).expect("arms");
        for arm in list.children().filter(|c| c.kind() == K::MATCH_ARM) {
            let pat = arm.children().find_map(ast::Pat::cast).expect("a pattern");
            let mut binds = Vec::new();
            if !self.matches(&pat, &src, &mut binds) {
                continue;
            }
            let mark = self.memory.mark();
            let guard = arm
                .child(K::MATCH_GUARD)
                .and_then(|g| g.children().find_map(ast::Expr::cast));
            if let Some(g) = guard {
                // While the guard runs, the names hold copies of what they would bind.
                let copies: Vec<Bind> = binds
                    .iter()
                    .map(|(n, v, _)| (n.clone(), v.clone(), None))
                    .collect();
                self.bind(copies);
                let go = match self.eval(&g) {
                    Ok(Value::Bool(b)) => b,
                    Ok(_) => unreachable!("a checked guard"),
                    Err(f) => {
                        self.memory.truncate(mark);
                        return Err(f);
                    }
                };
                self.memory.truncate(mark);
                if !go {
                    continue;
                }
            }
            self.bind(binds);
            let body = arm.children().filter_map(ast::Expr::cast).last();
            let result = self.eval(&body.expect("an arm's value"));
            return self.exit(mark, result, range);
        }
        unreachable!("a checked `match` is exhaustive")
    }

    /// `loop` and `while`, maybe labelled (`expr.loop.1`, `expr.while.1`).
    pub(super) fn loop_expr(&mut self, node: &SyntaxNode, label: Option<&str>) -> Eval {
        let range = node.text_range();
        let exprs: Vec<ast::Expr> = node.children().filter_map(ast::Expr::cast).collect();
        let is_while = node.kind() == K::WHILE_EXPR;
        let body =
            ast::Block::cast(exprs.last().expect("a body").syntax().clone()).expect("a block");
        let ours = |l: &Option<String>| l.is_none() || l.as_deref() == label;
        loop {
            let mark = self.memory.mark();
            if is_while {
                match self.eval(&exprs[0]) {
                    Ok(Value::Bool(true)) => {}
                    Ok(_) => return self.exit(mark, Ok(Value::Unit), range),
                    Err(f) => return self.exit(mark, Err(f), range),
                }
            }
            let result = self.block(&body);
            let result = self.exit(mark, result, range);
            match result {
                Ok(_) => {}
                Err(Flow::Break(l, v)) if ours(&l) => {
                    return Ok(if is_while { Value::Unit } else { v });
                }
                Err(Flow::Continue(l)) if ours(&l) => {}
                Err(f) => return Err(f),
            }
        }
    }

    pub(super) fn prefix(&mut self, node: &SyntaxNode, e: &ast::Expr) -> Eval {
        let range = node.text_range();
        let op = node
            .child_tokens()
            .find(|t| !t.kind().is_trivia())
            .expect("an operator");
        let operand = &e.sub_exprs()[0];
        if op.kind() == K::MINUS
            && let ast::Expr::Literal(lit) = operand
            && lit.token(K::INTEGER).is_some()
        {
            // `-9223372036854775808` is one literal (`expr.lit.int.1`).
            return Ok(literal(lit, true));
        }
        if op.kind() == K::STAR {
            // What a borrow points to (`expr.deref.1`).
            let v = self.eval(operand)?;
            return Ok(self.target(v));
        }
        match (op.kind(), self.eval(operand)?) {
            (K::MINUS, Value::Int(v)) => v
                .checked_neg()
                .map(Value::Int)
                .ok_or_else(|| trap(TrapClass::Overflow, range)),
            (K::BANG, Value::Int(v)) => Ok(Value::Int(!v)),
            (K::BANG, Value::Bool(v)) => Ok(Value::Bool(!v)),
            // Other types, through the operator's trait (`expr.op.trait.1`).
            (K::MINUS, v) => self.call_trait("Neg", "neg", vec![v], range),
            (K::BANG, v) => self.call_trait("Not", "not", vec![v], range),
            _ => unreachable!("a checked prefix operator"),
        }
    }

    pub(super) fn binary(&mut self, node: &SyntaxNode, e: &ast::Expr) -> Eval {
        let range = node.text_range();
        let op = node
            .child_tokens()
            .find(|t| !t.kind().is_trivia())
            .expect("an operator");
        let (l, r) = e.operands();
        let (l, r) = (l.expect("operands"), r.expect("operands"));
        match op.kind() {
            K::AMP_AMP | K::PIPE_PIPE => {
                let short = op.kind() == K::PIPE_PIPE;
                if self.eval(&l)? == Value::Bool(short) {
                    return Ok(Value::Bool(short));
                }
                self.eval(&r)
            }
            k if compare(k).is_some() => {
                let pairs = [(k, r)];
                self.compare_all(&l, &pairs)
            }
            k => {
                let a = self.eval(&l)?;
                let b = self.eval(&r)?;
                if let (Value::Int(_) | Value::Bool(_), _) = (&a, &b) {
                    return arith(k, a, b, range);
                }
                let (tr, method) = match k {
                    K::PLUS => ("Add", "add"),
                    K::MINUS => ("Sub", "sub"),
                    K::STAR => ("Mul", "mul"),
                    K::SLASH => ("Div", "div"),
                    K::PERCENT => ("Rem", "rem"),
                    _ => unreachable!("a checked operator"),
                };
                self.call_trait(tr, method, vec![a, b], range)
            }
        }
    }

    /// `a < b <= c`: each operand once, left to right, stopping at the first comparison that is
    /// false (`expr.cmp.chain.1`).
    pub(super) fn chain(&mut self, node: &SyntaxNode, e: &ast::Expr) -> Eval {
        let ops: Vec<K> = node
            .child_tokens()
            .map(|t| t.kind())
            .filter(|&k| compare(k).is_some())
            .collect();
        let operands = e.sub_exprs();
        let pairs: Vec<(K, ast::Expr)> =
            ops.into_iter().zip(operands[1..].iter().cloned()).collect();
        self.compare_all(&operands[0], &pairs)
    }

    /// Comparisons: on `int`, `bool` and `()` directly; on other types through `PartialEq` and
    /// `PartialOrd`, with borrows of the operands (`expr.cmp.trait.1`).
    pub(super) fn compare_all(&mut self, first: &ast::Expr, rest: &[(K, ast::Expr)]) -> Eval {
        let range = first
            .syntax()
            .parent()
            .map_or_else(|| first.syntax().text_range(), |p| p.text_range());
        let mut prev = self.place(first)?;
        for (op, x) in rest {
            let cur = self.place(x)?;
            let (a, b) = (self.memory.load(&prev), self.memory.load(&cur));
            let holds = match (&a, &b) {
                (Value::Int(_) | Value::Bool(_) | Value::Unit, _) => {
                    compare(*op).expect("a comparison")(&a, &b)
                }
                _ => {
                    let (tr, method) = match *op {
                        K::EQ_EQ => ("PartialEq", "eq"),
                        K::BANG_EQ => ("PartialEq", "ne"),
                        K::LT => ("PartialOrd", "lt"),
                        K::LT_EQ => ("PartialOrd", "le"),
                        K::GT => ("PartialOrd", "gt"),
                        _ => ("PartialOrd", "ge"),
                    };
                    let args = vec![
                        Value::Ref(prev.clone(), false),
                        Value::Ref(cur.clone(), false),
                    ];
                    self.call_trait(tr, method, args, range)? == Value::Bool(true)
                }
            };
            if !holds {
                return Ok(Value::Bool(false));
            }
            prev = cur;
        }
        Ok(Value::Bool(true))
    }

    /// `place = e` and `place op= e`: the place is a variable or fields of one
    /// (`expr.assign.1`, `expr.field.assign.1`). The old value is dropped first (`own.drop.1`).
    pub(super) fn assign(&mut self, node: &SyntaxNode, e: &ast::Expr) -> Eval {
        let range = node.text_range();
        let op = node
            .child_tokens()
            .find(|t| !t.kind().is_trivia())
            .expect("an operator");
        let (target, value) = e.operands();
        let (target, value) = (target.expect("a place"), value.expect("a value"));
        // `x = e` evaluates `e` first; `x op= e` reads `x` first (`expr.assign.1`).
        if op.kind() == K::EQ {
            let v = self.eval(&value)?;
            let p = self.place(&target)?;
            let old = self.memory.load(&p);
            self.memory.store(&p, Value::Moved);
            self.drop_value(old, true, range)?;
            self.memory.store(&p, v);
        } else {
            let p = self.place(&target)?;
            let old = self.memory.load(&p);
            let v = self.eval(&value)?;
            let new = arith(compound(op.kind()), old, v, range)?;
            self.memory.store(&p, new);
        }
        Ok(Value::Unit)
    }
}

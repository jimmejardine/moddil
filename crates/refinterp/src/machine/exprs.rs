//! Expressions: literals, struct literals, `if`, `match`, loops and operators.

use moddil_syntax::ast::{self, AstNode};
use moddil_syntax::{SyntaxKind as K, SyntaxNode};

use crate::program::VariantInfo;
use crate::value::{Head, Value};
use crate::{Eval, Flow, TrapClass, trap};

use super::patterns::Bind;
use super::{Machine, literal, member_name, segments};

use super::ops::{arith, compare, compound};
use super::text;

impl Machine<'_> {
    pub(super) fn eval(&mut self, e: &ast::Expr) -> Eval {
        let node = e.syntax();
        let range = node.text_range();
        match e {
            ast::Expr::Literal(_) => {
                match node.child_tokens().find(|t| t.kind() == K::TAGGED_STRING) {
                    Some(tok) => self.fmt(&tok),
                    None => Ok(literal(node, false)),
                }
            }
            ast::Expr::Path(_) => {
                if let Some((p, indirect)) = self.place_of(e)? {
                    return Ok(self.take(&p, indirect));
                }
                // A const parameter's value (`generic.const.1`).
                if let [n] = segments(node).as_slice()
                    && let Some(Head::Len(v)) = self.memory.type_of(n)
                {
                    return Ok(Value::Int(v));
                }
                // A const item's value (`comptime.const.1`).
                if let Some(p) = self.const_place(node)? {
                    return Ok(self.memory.load(&p));
                }
                let segments = segments(node);
                // A function used as a value (`expr.fn-value.1`): a closure calling it.
                if self.fn_path(&segments) {
                    return Ok(self.closure_value(node));
                }
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
            // A literal of a type its context names (`expr.array.from.1`).
            ast::Expr::Array(_) if self.literal_target(node).is_some() => {
                let head = self.literal_target(node).expect("a target");
                let parts = e.sub_exprs();
                let (name, args) = if node.token(K::SEMI).is_some() {
                    ("from_repeat", self.eval_all(&parts)?)
                } else {
                    ("from_array", vec![Value::Array(self.eval_all(&parts)?)])
                };
                let m = self
                    .program
                    .trait_method("FromArrayLiteral", &head, name)
                    .expect("a checked literal");
                self.run_method(m, &head, args, range)
            }
            // `[a, b]` and `[x; n]` (`expr.array.1`, `expr.array.repeat.1`).
            ast::Expr::Array(_) if node.token(K::SEMI).is_some() => {
                let parts = e.sub_exprs();
                let x = self.eval(&parts[0])?;
                let Value::Int(n) = self.eval(&parts[1])? else {
                    unreachable!("a checked length")
                };
                Ok(Value::Array(vec![x; usize::try_from(n).unwrap_or(0)]))
            }
            ast::Expr::Array(_) => Ok(Value::Array(self.eval_all(&e.sub_exprs())?)),
            ast::Expr::Index(_) => {
                let (p, indirect) = self.place_of(e)?.expect("an element is a place");
                Ok(self.take(&p, indirect))
            }
            ast::Expr::Field(_) => {
                // Field access looks through a borrow (`expr.field.1`).
                let (p, indirect) = self.place_of(e)?.expect("a field is a place");
                Ok(self.take(&p, indirect))
            }
            ast::Expr::Ref(_) => {
                let mutable = node.token(K::MUT_KW).is_some();
                let inner = &e.sub_exprs()[0];
                // A borrow of a const is of its static value (`borrow.static.1`).
                if !mutable
                    && let ast::Expr::Path(_) = inner
                    && let Some(p) = self.const_place(inner.syntax())?
                {
                    return Ok(Value::Ref(p, false));
                }
                let p = self.place(inner)?;
                Ok(Value::Ref(p, mutable))
            }
            // `const { e }` (`comptime.block.1`): evaluated where it is.
            ast::Expr::ConstBlock(_) => {
                let b = node.children().find_map(ast::Block::cast).expect("a block");
                self.block(&b)
            }
            ast::Expr::Block(_) => self.block(&ast::Block::cast(node.clone()).expect("a block")),
            ast::Expr::If(_) => self.if_expr(e),
            ast::Expr::LetCond(_) => {
                let init = node.children().find_map(ast::Expr::cast).expect("a value");
                let src = self.scrutinee(&init)?;
                let pat = node.children().find_map(ast::Pat::cast).expect("a pattern");
                let mut binds = Vec::new();
                let matched = self.matches(&pat, &src, &mut binds);
                self.matched(range)?;
                if matched {
                    // In scope for the rest of the condition and the block it guards.
                    self.bind(binds);
                }
                Ok(Value::Bool(matched))
            }
            ast::Expr::Match(_) => self.match_expr(node),
            ast::Expr::Loop(_) | ast::Expr::While(_) => self.loop_expr(node, None),
            ast::Expr::For(_) => self.for_expr(node, None),
            ast::Expr::Range(_) => self.range_value(node, e),
            ast::Expr::Labelled(_) => {
                let label = node.token(K::LABEL).map(|t| t.text().to_string());
                let inner = node.children().find_map(ast::Expr::cast).expect("a loop");
                if inner.syntax().kind() == K::FOR_EXPR {
                    return self.for_expr(inner.syntax(), label.as_deref());
                }
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
            ast::Expr::Select(_) => self.select_expr(node),
            ast::Expr::With(_) => self.with_expr(node),
            ast::Expr::Closure(_) => Ok(self.closure_value(node)),
            ast::Expr::Impl(_) => Ok(self.anon_value(node)),
            _ => unreachable!("a checked program of slices S0 to S7a"),
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
        self.program.ctor(self.memory.module(), &resolved)
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
            fields[i] = crate::num::typed(v, info.types.get(i).and_then(Option::as_ref));
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
            let matched = self.matches(&pat, &src, &mut binds);
            self.matched(pat.syntax().text_range())?;
            if !matched {
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

    /// `for p in e { body }` (`expr.for.1`): `e` itself if it is an iterator, or else what its
    /// `IntoIterator::into_iter` gives; then the body for each item `next` gives, until `None`.
    pub(super) fn for_expr(&mut self, node: &SyntaxNode, label: Option<&str>) -> Eval {
        let range = node.text_range();
        let exprs: Vec<ast::Expr> = node.children().filter_map(ast::Expr::cast).collect();
        let pat = node.children().find_map(ast::Pat::cast).expect("a pattern");
        let body = ast::Block::cast(
            node.children()
                .filter(|n| n.kind() == K::BLOCK)
                .last()
                .expect("a body"),
        )
        .expect("a block");
        let v = self.eval(&exprs[0])?;
        let head = v.head();
        let it = if self.program.implements(&head, "Iterator") {
            v
        } else {
            let m = self
                .program
                .trait_method("IntoIterator", &head, "into_iter")
                .expect("a checked `for`");
            self.run_method(m, &head, vec![v], range)?
        };
        let outer = self.memory.mark();
        self.memory.push("{iterator}".to_string(), it);
        let slot = self.memory.slot("{iterator}").expect("the iterator");
        let place = self.memory.here(slot);
        let ours = |l: &Option<String>| l.is_none() || l.as_deref() == label;
        loop {
            let it_head = self.memory.load(&place).head();
            let m = self
                .program
                .trait_method("Iterator", &it_head, "next")
                .expect("a checked iterator");
            let next =
                match self.run_method(m, &it_head, vec![Value::Ref(place.clone(), true)], range) {
                    Ok(v) => v,
                    Err(f) => return self.exit(outer, Err(f), range),
                };
            let Value::Adt {
                variant, fields, ..
            } = next
            else {
                unreachable!("an `Option`")
            };
            if variant == "None" {
                return self.exit(outer, Ok(Value::Unit), range);
            }
            let mark = self.memory.mark();
            let mut binds = Vec::new();
            let item = fields.into_iter().next().expect("the item");
            let matched = self.matches(&pat, &super::patterns::Src::Val(item), &mut binds);
            debug_assert!(matched, "a `for` pattern is irrefutable");
            self.bind(binds);
            let result = self.block(&body);
            let result = self.exit(mark, result, range);
            match result {
                Ok(_) => {}
                Err(Flow::Break(l, _)) if ours(&l) => {
                    return self.exit(outer, Ok(Value::Unit), range);
                }
                Err(Flow::Continue(l)) if ours(&l) => {}
                Err(f) => return self.exit(outer, Err(f), range),
            }
        }
    }

    /// The kind of value an array literal's context makes of it, if that is not an array: the
    /// type written where it is used (a `let`'s type, a function's parameter, or the result of the
    /// function it is the value of), if it implements `FromArrayLiteral`.
    pub(super) fn literal_target(&self, node: &SyntaxNode) -> Option<Head> {
        let head = self.context_head(node)?;
        (matches!(head, Head::Adt(_)) && self.program.implements(&head, "FromArrayLiteral"))
            .then_some(head)
    }

    /// The kind of value the type written where `node` is used names, if one is: a `let`'s
    /// type, a called function's parameter, or the result of the function it is the value of.
    pub(super) fn context_head(&self, node: &SyntaxNode) -> Option<Head> {
        let parent = node.parent()?;
        let (ty, module) = match parent.kind() {
            K::LET_STMT => (
                parent.children().find_map(ast::Type::cast)?,
                self.memory.module(),
            ),
            K::ARG => {
                let list = parent.parent()?;
                let call = list.parent()?;
                if call.kind() != K::CALL_EXPR {
                    return None;
                }
                let at = list
                    .children()
                    .filter(|c| c.kind() == K::ARG)
                    .position(|c| c == parent)?;
                let callee = call.children().find_map(ast::Expr::cast)?;
                let Some((crate::program::Def::Item(k), _)) =
                    self.resolve(&segments(callee.syntax()))
                else {
                    return None;
                };
                let f = self.program.fns.get(&k)?;
                let p = f.node.params()?.params().nth(at)?;
                (p.ty()?, f.module as usize)
            }
            _ => {
                // The value of a function's body, or what it returns.
                let fn_node = if parent.kind() == K::RETURN_EXPR {
                    parent.ancestors().find(|a| a.kind() == K::FN_ITEM)?
                } else if parent.kind() == K::BLOCK
                    && parent.parent()?.kind() == K::FN_ITEM
                    && parent
                        .children()
                        .filter_map(ast::Expr::cast)
                        .last()?
                        .syntax()
                        == node
                    && !parent.child_tokens().any(|t| {
                        t.kind() == K::SEMI && t.text_range().start > node.text_range().end
                    })
                {
                    parent.parent()?
                } else {
                    return None;
                };
                let f = ast::FnItem::cast(fn_node)?;
                (f.ret_type()?.ty()?, self.memory.module())
            }
        };
        Some(self.program.head_in(module, &ty))
    }

    /// Where the value of the const item a path names is, computing it the first time: its
    /// initialiser, run in a call of its own in the const's module.
    pub(super) fn const_place(&mut self, node: &SyntaxNode) -> Eval<Option<crate::value::Pointer>> {
        let segs = segments(node);
        if let [n] = segs.as_slice()
            && self.memory.slot(n).is_some()
        {
            return Ok(None);
        }
        let Some((crate::program::Def::Item(k), used)) = self.resolve(&segs) else {
            return Ok(None);
        };
        if used != segs.len() {
            return Ok(None);
        }
        let Some((init, module)) = self.program.consts.get(&k).cloned() else {
            return Ok(None);
        };
        if let Some(p) = self.memory.static_of(&k) {
            return Ok(Some(p));
        }
        let file = self.memory.file();
        self.memory
            .push_frame(std::collections::HashMap::new(), file, module, false);
        let v = self.eval(&init);
        self.memory.pop_frame();
        Ok(Some(self.memory.add_static(k, v?)))
    }

    /// `a..b`, `a..=b` and `a..` (`expr.range.1`): values of the prelude's range structs.
    fn range_value(&mut self, node: &SyntaxNode, e: &ast::Expr) -> Eval {
        let parts = e.sub_exprs();
        let mut fields = self.eval_all(&parts)?;
        let name = if node.token(K::DOT_DOT_EQ).is_some() {
            fields.push(Value::Bool(false));
            "RangeInclusive"
        } else if parts.len() == 2 {
            "Range"
        } else {
            "RangeFrom"
        };
        Ok(Value::Adt {
            ty: name.to_string(),
            variant: name.to_string(),
            fields,
        })
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
            && (lit.token(K::INTEGER).is_some() || lit.token(K::FLOAT).is_some())
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
            (k, v @ (Value::Num(..) | Value::FloatLit(_) | Value::BigLit(_))) => {
                crate::num::unary(k == K::MINUS, v, range)
            }
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
                // Numbers of a sized type, and the literals that meet them (`num.ops.1`).
                if crate::num::is_num(&a, &b) {
                    return crate::num::arith(op.text(), a, b, range);
                }
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
            let texts = (text::text_of(self, &a), text::text_of(self, &b));
            let holds = match (&a, &b) {
                // Numbers by their type's order; a float is unordered with NaN.
                _ if crate::num::is_num(&a, &b) => {
                    use std::cmp::Ordering::{Equal, Greater, Less};
                    let c = crate::num::compare(&a, &b);
                    match *op {
                        K::EQ_EQ => c == Some(Equal),
                        K::BANG_EQ => c != Some(Equal),
                        K::LT => c == Some(Less),
                        K::LT_EQ => matches!(c, Some(Less | Equal)),
                        K::GT => c == Some(Greater),
                        _ => matches!(c, Some(Greater | Equal)),
                    }
                }
                (Value::Int(_) | Value::Bool(_) | Value::Unit, _) => {
                    compare(*op).expect("a comparison")(&a, &b)
                }
                (Value::Char(x), Value::Char(y)) => text::ordered(*op, x, y),
                // `&str`s compare their text (`text.cmp.1`).
                _ if matches!(texts, (Some(_), Some(_))) => {
                    let (Some(x), Some(y)) = texts else {
                        unreachable!()
                    };
                    text::ordered(*op, &x, &y)
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
                    // A slice borrow is passed as it is; other operands are borrowed.
                    let arg = |m: &Self, v: &Value, p: &crate::value::Pointer| match v {
                        Value::Ref(q, _)
                            if matches!(m.memory.load(q), Value::Array(_) | Value::Buf(..)) =>
                        {
                            v.clone()
                        }
                        _ => Value::Ref(p.clone(), false),
                    };
                    let args = vec![arg(self, &a, &prev), arg(self, &b, &cur)];
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
            // A literal stored where a number of a sized type was takes its type.
            let v = match &old {
                Value::Num(k, _) => crate::num::as_kind(v, *k),
                _ => v,
            };
            self.memory.store(&p, Value::Moved);
            self.drop_value(old, true, range)?;
            self.memory.store(&p, v);
        } else {
            let p = self.place(&target)?;
            let old = self.memory.load(&p);
            let v = self.eval(&value)?;
            let new = if crate::num::is_num(&old, &v) {
                let sym = op.text();
                crate::num::arith(&sym[..sym.len() - 1], old, v, range)?
            } else {
                arith(compound(op.kind()), old, v, range)?
            };
            self.memory.store(&p, new);
        }
        Ok(Value::Unit)
    }
}

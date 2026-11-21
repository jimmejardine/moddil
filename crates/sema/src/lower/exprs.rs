//! Expressions: each kind of syntax tree expression as HIR.

use moddil_syntax::ast::{self, AstNode};
use moddil_syntax::{SyntaxKind as K, SyntaxNode, SyntaxToken, TextRange};

use super::names::JumpError;
use super::{Binds, Lowerer, Resolved};
use crate::hir::{Arm, BinaryOp, CompareOp, ExprId, ExprKind, Handler, LogicOp, PatKind, UnaryOp};
use crate::items::Shape;
use crate::resolve::{generic_args, path_segments};
use crate::{Code, Diagnostic, Ty};

impl Lowerer<'_> {
    pub(super) fn expr(&mut self, e: &ast::Expr) -> ExprId {
        let node = e.syntax().clone();
        let range = node.text_range();
        match e {
            ast::Expr::Literal(_) => self.literal(&node, false),
            ast::Expr::Path(_) => match self.resolve(&node) {
                // An enum's name followed by something that is not one of its variants.
                Some(Resolved::AssocFn(Ty::Adt(a, ..), name, at, _))
                    if self.items.adts[a as usize].is_enum =>
                {
                    let ty = &self.items.adts[a as usize].name;
                    self.report(
                        Code::E0101,
                        at,
                        format!("`{ty}` has no variant called `{name}`"),
                    );
                    self.missing(range)
                }
                Some(r) => self.resolved(r, range),
                None => self.missing(range),
            },
            ast::Expr::Paren(_) => {
                let inner = e.sub_exprs().into_iter().next();
                self.expr_or_missing(inner, range)
            }
            ast::Expr::Tuple(_) => {
                let elements: Vec<ExprId> = e.sub_exprs().iter().map(|x| self.expr(x)).collect();
                if elements.is_empty() {
                    self.alloc(ExprKind::Unit, range)
                } else {
                    self.alloc(ExprKind::Tuple(elements), range)
                }
            }
            ast::Expr::StructLit(_) => self.struct_lit(&node),
            ast::Expr::Field(_) => self.field(&node, e),
            ast::Expr::Block(_) => match ast::Block::cast(node) {
                Some(b) => self.block(&b),
                None => self.missing(range),
            },
            ast::Expr::If(_) => self.if_expr(&node, e),
            ast::Expr::LetCond(_) => self.let_cond(&node),
            ast::Expr::Match(_) => self.match_expr(&node),
            ast::Expr::Loop(_) | ast::Expr::While(_) => self.loop_expr(&node, None),
            ast::Expr::Labelled(_) => {
                let label = node.token(K::LABEL).map(|t| t.text().to_string());
                match node.children().find_map(ast::Expr::cast) {
                    Some(inner @ (ast::Expr::Loop(_) | ast::Expr::While(_))) => {
                        self.loop_expr(inner.syntax(), label)
                    }
                    Some(ast::Expr::For(_)) => self.unsupported(range, "`for`"),
                    _ => self.unsupported(range, "a labelled block"),
                }
            }
            ast::Expr::Break(_) | ast::Expr::Continue(_) => self.jump(&node, e),
            ast::Expr::Return(_) => {
                let value = e.sub_exprs().first().map(|v| self.expr(v));
                if self.names.in_defer() {
                    self.report(
                        Code::E0313,
                        range,
                        "a `defer` block cannot `return`: it runs while its scope is being left",
                    );
                    return self.missing(range);
                }
                self.alloc(ExprKind::Return(value), range)
            }
            ast::Expr::Ref(_) => {
                let mutable = node.token(K::MUT_KW).is_some();
                let inner = e.sub_exprs().into_iter().next();
                let expr = self.expr_or_missing(inner, range);
                self.alloc(ExprKind::Ref { mutable, expr }, range)
            }
            ast::Expr::Prefix(_) => self.prefix(&node, e),
            ast::Expr::Bin(_) => self.binary(&node, e),
            ast::Expr::CompareChain(_) => self.compare(&node, e),
            ast::Expr::Assign(_) => self.assign(&node, e),
            ast::Expr::Call(_) => self.call(&node, e),
            ast::Expr::For(_) => self.unsupported(range, "`for`"),
            ast::Expr::MethodCall(_) => self.method_call(&node, e),
            ast::Expr::Propagate(_) => {
                if let Some(note) = node.child(K::ERROR_CONTEXT)
                    && note.token(K::STRING).is_none()
                {
                    self.error(Diagnostic::unsupported(
                        note.text_range(),
                        "a formatted note",
                    ));
                }
                let inner = e.sub_exprs().into_iter().next();
                let x = self.expr_or_missing(inner, range);
                self.alloc(ExprKind::Question(x), range)
            }
            ast::Expr::Unwrap(_) => {
                let reason = node
                    .token(K::STRING)
                    .map(|t| unquote(t.text()))
                    .unwrap_or_default();
                let inner = e.sub_exprs().into_iter().next();
                let x = self.expr_or_missing(inner, range);
                self.alloc(ExprKind::Bang(x, reason), range)
            }
            ast::Expr::Fail(_) => {
                let inner = e.sub_exprs().into_iter().next();
                let x = self.expr_or_missing(inner, range);
                self.alloc(ExprKind::Fail(x), range)
            }
            ast::Expr::Try(_) => self.try_expr(&node),
            ast::Expr::With(_) => self.with_expr(&node),
            _ => self.unsupported(range, "this expression"),
        }
    }

    pub(super) fn literal(&mut self, node: &SyntaxNode, negate: bool) -> ExprId {
        let range = node.text_range();
        if node.child(K::UNIT_SUFFIX).is_some() {
            return self.unsupported(range, "a unit of measure");
        }
        let Some(tok) = node.child_tokens().find(|t| !t.kind().is_trivia()) else {
            return self.missing(range);
        };
        match tok.kind() {
            K::TRUE_KW => self.alloc(ExprKind::Bool(true), range),
            K::FALSE_KW => self.alloc(ExprKind::Bool(false), range),
            K::INTEGER => match ast::int_value(tok.text()) {
                Some(v) => self.alloc(ExprKind::Int(if negate { -v } else { v }), range),
                None => {
                    self.report(
                        Code::E0204,
                        range,
                        format!("`{}` is too large for any integer type", tok.text()),
                    );
                    self.missing(range)
                }
            },
            _ => self.unsupported(range, "this kind of literal"),
        }
    }

    /// `Path { field: value, … }`.
    pub(super) fn struct_lit(&mut self, node: &SyntaxNode) -> ExprId {
        let range = node.text_range();
        let mut ty_args = Vec::new();
        let target = match self.resolve(node) {
            Some(Resolved::Ctor(adt, variant, args)) => {
                let def = self.items.variant(adt, variant);
                ty_args = args;
                if def.shape == Shape::Named {
                    Some((adt, variant))
                } else {
                    let path = self.items.adts[adt as usize].path(variant);
                    self.report(
                        Code::E0111,
                        range,
                        format!(
                            "`{path}` has no named fields, so it is not written with `{{ … }}`"
                        ),
                    );
                    None
                }
            }
            Some(_) => {
                self.report(Code::E0111, range, "this is not a struct or variant");
                None
            }
            None => None,
        };
        let mut fields: Vec<(u32, ExprId)> = Vec::new();
        let mut given: Vec<(u32, TextRange)> = Vec::new();
        for init in node.children().filter(|c| c.kind() == K::FIELD_INIT) {
            let init_range = init.text_range();
            if init.token(K::DOT_DOT).is_some() {
                self.error(Diagnostic::unsupported(init_range, "struct update syntax"));
                continue;
            }
            let Some(name_tok) = ast::member_name(&init) else {
                continue;
            };
            let name = name_tok.text().to_string();
            let value = match init.children().find_map(ast::Expr::cast) {
                Some(v) => self.expr(&v),
                None => match self.lookup(&name) {
                    Some(l) => self.alloc(ExprKind::Local(l), name_tok.text_range()),
                    None => {
                        self.report(
                            Code::E0101,
                            name_tok.text_range(),
                            format!("there is nothing called `{name}` here"),
                        );
                        self.missing(init_range)
                    }
                },
            };
            let Some((adt, variant)) = target else {
                continue;
            };
            let def = self.items.variant(adt, variant);
            match def.field(&name) {
                Some(i) => {
                    if let Some(&(_, first)) = given.iter().find(|(g, _)| *g == i) {
                        let d = Diagnostic::error(
                            Code::E0108,
                            name_tok.text_range(),
                            format!("the field `{name}` is given twice"),
                        )
                        .with_label(first, "first given here");
                        self.error(d);
                    } else {
                        given.push((i, name_tok.text_range()));
                        fields.push((i, value));
                    }
                }
                None => {
                    let path = self.items.adts[adt as usize].path(variant);
                    self.report(
                        Code::E0107,
                        name_tok.text_range(),
                        format!("`{path}` has no field called `{name}`"),
                    );
                }
            }
        }
        let Some((adt, variant)) = target else {
            return self.missing(range);
        };
        let def = self.items.variant(adt, variant);
        let missing: Vec<String> = (0..def.fields.len() as u32)
            .filter(|i| !given.iter().any(|(g, _)| g == i))
            .map(|i| format!("`{}`", def.fields[i as usize].name))
            .collect();
        if !missing.is_empty() {
            let path = self.items.adts[adt as usize].path(variant);
            self.report(
                Code::E0207,
                range,
                format!("`{path}` needs the fields {}", missing.join(", ")),
            );
        }
        self.alloc(
            ExprKind::Adt {
                adt,
                variant,
                fields,
                args: ty_args,
            },
            range,
        )
    }

    /// `receiver.name::<T>(args)`.
    pub(super) fn method_call(&mut self, node: &SyntaxNode, e: &ast::Expr) -> ExprId {
        let range = node.text_range();
        let receiver = e.sub_exprs().into_iter().next();
        let receiver = self.expr_or_missing(receiver, range);
        let generic_args = node.child(K::GENERIC_ARGS).map(|a| {
            let mut ds = Vec::new();
            let (types, _) = generic_args(&a, &self.types, &mut ds);
            self.out.diagnostics.extend(ds);
            types
        });
        let args = self.args(node);
        let Some(name) = node.child_tokens().find(|t| t.kind() == K::IDENT) else {
            return self.missing(range);
        };
        self.alloc(
            ExprKind::MethodCall {
                receiver,
                name: name.text().to_string(),
                name_range: name.text_range(),
                generic_args,
                args,
            },
            range,
        )
    }

    /// The arguments of a call or method call.
    pub(super) fn args(&mut self, node: &SyntaxNode) -> Vec<ExprId> {
        let mut args = Vec::new();
        if let Some(list) = node.child(K::ARG_LIST).and_then(ast::ArgList::cast) {
            for a in list.args() {
                if let Some(label) = a.label() {
                    self.error(Diagnostic::unsupported(
                        label.text_range(),
                        "a named argument",
                    ));
                }
                let x = a.expr();
                args.push(self.expr_or_missing(x, a.syntax().text_range()));
            }
        }
        args
    }

    /// `base.name` or `base.0`.
    pub(super) fn field(&mut self, node: &SyntaxNode, e: &ast::Expr) -> ExprId {
        let range = node.text_range();
        let base = e.sub_exprs().into_iter().next();
        let base = self.expr_or_missing(base, range);
        match ast::member_name(node) {
            Some(tok) => self.alloc(
                ExprKind::Field {
                    base,
                    name: tok.text().to_string(),
                    name_range: tok.text_range(),
                },
                range,
            ),
            None => self.missing(range),
        }
    }

    pub(super) fn if_expr(&mut self, node: &SyntaxNode, e: &ast::Expr) -> ExprId {
        let range = node.text_range();
        if node.token(K::CONST_KW).is_some() {
            return self.unsupported(range, "`if const`");
        }
        let mut subs = e.sub_exprs().into_iter();
        // Names bound by `let` conditions are in scope in the condition after them and in the
        // first block, not in the `else` branch.
        let mark = self.names.mark();
        let cond = self.expr_or_missing(subs.next(), range);
        let then = self.expr_or_missing(subs.next(), range);
        self.names.restore(mark);
        let else_ = subs.next().map(|x| self.expr(&x));
        self.alloc(ExprKind::If { cond, then, else_ }, range)
    }

    /// `let p = e` in an `if` or `while` head. Its names stay in scope until the head's
    /// expression truncates the scopes.
    pub(super) fn let_cond(&mut self, node: &SyntaxNode) -> ExprId {
        let range = node.text_range();
        let init = node.children().find_map(ast::Expr::cast);
        let init = self.expr_or_missing(init, range);
        let Some(p) = node.children().find_map(ast::Pat::cast) else {
            return self.missing(range);
        };
        let mut binds = Binds::default();
        let pat = self.pat(&p, &mut binds);
        self.bring_into_scope(binds);
        self.alloc(ExprKind::Let { pat, init }, range)
    }

    pub(super) fn match_expr(&mut self, node: &SyntaxNode) -> ExprId {
        let range = node.text_range();
        let scrutinee = node.children().find_map(ast::Expr::cast);
        let scrutinee = self.expr_or_missing(scrutinee, range);
        let mut arms = Vec::new();
        let Some(list) = node.child(K::MATCH_ARM_LIST) else {
            return self.missing(range);
        };
        if let Some(attr) = list.child(K::INNER_ATTR) {
            self.error(Diagnostic::unsupported(attr.text_range(), "an attribute"));
        }
        for arm in list.children().filter(|c| c.kind() == K::MATCH_ARM) {
            if let Some(attr) = arm.child(K::OUTER_ATTR) {
                self.error(Diagnostic::unsupported(attr.text_range(), "an attribute"));
            }
            let mark = self.names.mark();
            let mut binds = Binds::default();
            let pat = match arm.children().find_map(ast::Pat::cast) {
                Some(p) => self.pat(&p, &mut binds),
                None => self.alloc_pat(PatKind::Missing, arm.text_range()),
            };
            self.bring_into_scope(binds);
            let guard = arm.child(K::MATCH_GUARD).map(|g| {
                let cond = g.children().find_map(ast::Expr::cast);
                self.expr_or_missing(cond, g.text_range())
            });
            let body = arm.children().filter_map(ast::Expr::cast).last();
            let body = self.expr_or_missing(body, arm.text_range());
            self.names.restore(mark);
            arms.push(Arm { pat, guard, body });
        }
        self.alloc(ExprKind::Match { scrutinee, arms }, range)
    }

    /// `loop { … }` or `while cond { … }`, maybe labelled.
    pub(super) fn loop_expr(&mut self, node: &SyntaxNode, label: Option<String>) -> ExprId {
        let range = node.text_range();
        let id = self.missing(range);
        self.names.enter_loop(label, id);
        let mark = self.names.mark();
        let cond = if node.kind() == K::WHILE_EXPR {
            // The condition, then the body block.
            let exprs: Vec<ast::Expr> = node.children().filter_map(ast::Expr::cast).collect();
            let c = if exprs.len() > 1 {
                exprs.first().cloned()
            } else {
                None
            };
            Some(self.expr_or_missing(c, range))
        } else {
            None
        };
        let body = node
            .children()
            .filter(|n| n.kind() == K::BLOCK)
            .last()
            .and_then(ast::Block::cast);
        let body = match body {
            Some(b) => self.block(&b),
            None => self.missing(range),
        };
        self.names.restore(mark);
        self.names.exit_loop();
        self.out.body.exprs[id].kind = match cond {
            Some(cond) => ExprKind::While { cond, body },
            None => ExprKind::Loop { body },
        };
        id
    }

    /// `break` or `continue`, with an optional label (and a value for `break`).
    pub(super) fn jump(&mut self, node: &SyntaxNode, e: &ast::Expr) -> ExprId {
        let range = node.text_range();
        let is_break = node.kind() == K::BREAK_EXPR;
        let value = if is_break {
            e.sub_exprs().first().map(|v| self.expr(v))
        } else {
            None
        };
        let label = node.token(K::LABEL);
        let what = if is_break { "break" } else { "continue" };
        let target = match self.names.jump_target(label.as_ref().map(|l| l.text())) {
            Ok(target) => target,
            Err(JumpError::NoSuchLabel) => {
                let l = label.expect("a label");
                self.report(
                    Code::E0110,
                    l.text_range(),
                    format!("no enclosing loop is labelled `{}`", l.text()),
                );
                return self.missing(range);
            }
            Err(JumpError::NotInLoop) => {
                self.report(
                    Code::E0109,
                    range,
                    format!("`{what}` is only allowed inside a loop"),
                );
                return self.missing(range);
            }
            Err(JumpError::OutOfDefer) => {
                self.report(
                    Code::E0313,
                    range,
                    format!("a `defer` block cannot `{what}` out of itself"),
                );
                return self.missing(range);
            }
        };
        let kind = if is_break {
            ExprKind::Break { target, value }
        } else {
            ExprKind::Continue { target }
        };
        self.alloc(kind, range)
    }

    pub(super) fn prefix(&mut self, node: &SyntaxNode, e: &ast::Expr) -> ExprId {
        let range = node.text_range();
        let Some(op) = ast::operator(node) else {
            return self.missing(range);
        };
        let operand = e.sub_exprs().into_iter().next();
        if op.kind() == K::STAR {
            let x = self.expr_or_missing(operand, range);
            return self.alloc(ExprKind::Deref(x), range);
        }
        let op = match op.kind() {
            K::MINUS => UnaryOp::Neg,
            K::BANG => UnaryOp::Not,
            _ => return self.unsupported(range, "this operator"),
        };
        match operand {
            Some(ast::Expr::Literal(lit))
                if op == UnaryOp::Neg && lit.token(K::INTEGER).is_some() =>
            {
                // `-9223372036854775808` is one literal: constant arithmetic on literals is exact.
                let id = self.literal(&lit, true);
                self.out.body.exprs[id].range = range;
                id
            }
            Some(x) => {
                let x = self.expr(&x);
                self.alloc(ExprKind::Unary(op, x), range)
            }
            None => self.missing(range),
        }
    }

    pub(super) fn operands(&mut self, e: &ast::Expr, range: TextRange) -> (ExprId, ExprId) {
        let (l, r) = e.operands();
        let l = self.expr_or_missing(l, range);
        let r = self.expr_or_missing(r, range);
        (l, r)
    }

    pub(super) fn binary(&mut self, node: &SyntaxNode, e: &ast::Expr) -> ExprId {
        let range = node.text_range();
        let Some(op) = ast::operator(node) else {
            return self.missing(range);
        };
        if let Some(logic) = match op.kind() {
            K::AMP_AMP => Some(LogicOp::And),
            K::PIPE_PIPE => Some(LogicOp::Or),
            _ => None,
        } {
            let (l, r) = self.operands(e, range);
            return self.alloc(ExprKind::Logic(logic, l, r), range);
        }
        if let Some(cmp) = compare_op(&op) {
            let (first, x) = self.operands(e, range);
            let rest = vec![(cmp, x)];
            return self.alloc(ExprKind::Compare { first, rest }, range);
        }
        let Some(op) = binary_op(&op) else {
            return self.unsupported(op.text_range(), &format!("`{}`", op.text()));
        };
        let (l, r) = self.operands(e, range);
        self.alloc(ExprKind::Binary(op, l, r), range)
    }

    pub(super) fn compare(&mut self, node: &SyntaxNode, e: &ast::Expr) -> ExprId {
        let range = node.text_range();
        let ops: Vec<CompareOp> = node.child_tokens().filter_map(|t| compare_op(&t)).collect();
        let ascending = ops
            .iter()
            .all(|o| matches!(o, CompareOp::Lt | CompareOp::Le));
        let descending = ops
            .iter()
            .all(|o| matches!(o, CompareOp::Gt | CompareOp::Ge));
        if ops.len() > 1 && !ascending && !descending {
            self.report(
                Code::E0002,
                range,
                "a comparison chain must run in one direction (all `<` and `<=`, or all `>` and \
                 `>=`), and `==` and `!=` do not chain",
            );
        }
        let mut operands = e.sub_exprs().into_iter();
        let first = self.expr_or_missing(operands.next(), range);
        let mut rest = Vec::new();
        for op in ops {
            let x = self.expr_or_missing(operands.next(), range);
            rest.push((op, x));
        }
        self.alloc(ExprKind::Compare { first, rest }, range)
    }

    pub(super) fn assign(&mut self, node: &SyntaxNode, e: &ast::Expr) -> ExprId {
        let range = node.text_range();
        let Some(op_tok) = ast::operator(node) else {
            return self.missing(range);
        };
        let op = match op_tok.kind() {
            K::EQ => None,
            K::PLUS_EQ => Some(BinaryOp::Add),
            K::MINUS_EQ => Some(BinaryOp::Sub),
            K::STAR_EQ => Some(BinaryOp::Mul),
            K::SLASH_EQ => Some(BinaryOp::Div),
            K::PERCENT_EQ => Some(BinaryOp::Rem),
            K::AMP_EQ => Some(BinaryOp::BitAnd),
            K::PIPE_EQ => Some(BinaryOp::BitOr),
            K::CARET_EQ => Some(BinaryOp::BitXor),
            K::LT_LT_EQ => Some(BinaryOp::Shl),
            K::GT_GT_EQ => Some(BinaryOp::Shr),
            _ => {
                return self.unsupported(op_tok.text_range(), &format!("`{}`", op_tok.text()));
            }
        };
        let (target, value) = e.operands();
        let value = self.expr_or_missing(value, range);
        let Some(target) = target else {
            return self.missing(range);
        };
        let target_range = target.syntax().text_range();
        if matches!(target, ast::Expr::Index(_)) {
            return self.unsupported(target_range, "assigning to this place");
        }
        let target = self.expr(&target);
        // The place must be a local or a dereferenced borrow, or fields of one. Whether it may
        // be changed needs types, so the checker decides that.
        let mut root = target;
        while let ExprKind::Field { base, .. } = self.out.body.exprs[root].kind {
            root = base;
        }
        match self.out.body.exprs[root].kind {
            ExprKind::Local(_) | ExprKind::Deref(_) | ExprKind::Missing => {}
            ExprKind::Fn { .. } | ExprKind::AssocFn { .. } | ExprKind::TraitFn { .. } => {
                self.report(
                    Code::E0206,
                    target_range,
                    "a function cannot be assigned to",
                );
            }
            _ => {
                self.report(
                    Code::E0206,
                    target_range,
                    "only a variable or a field of one can be assigned to",
                );
            }
        }
        self.alloc(ExprKind::Assign { target, op, value }, range)
    }

    pub(super) fn call(&mut self, node: &SyntaxNode, e: &ast::Expr) -> ExprId {
        let range = node.text_range();
        let callee_node = e.sub_exprs().into_iter().next();
        if let Some(c @ ast::Expr::Path(_)) = &callee_node {
            let segments = c
                .syntax()
                .child(K::PATH)
                .and_then(|p| path_segments(&p))
                .unwrap_or_default();
            // `discard` is in the prelude too.
            if let [one] = segments.as_slice()
                && one.name == "discard"
                && self.lookup("discard").is_none()
                && self.items.fn_named("discard").is_none()
            {
                return self.discard(node, range);
            }
            // `assert` is in the prelude: a local or item of the same name shadows it.
            if let [one] = segments.as_slice()
                && one.name == "assert"
                && self.lookup("assert").is_none()
                && self.items.fn_named("assert").is_none()
                && self.items.adt_named("assert").is_none()
            {
                let args = self.args(node);
                if args.len() != 1 {
                    self.report(
                        Code::E0202,
                        range,
                        format!(
                            "`assert` takes one argument, the condition, but {} were given",
                            args.len()
                        ),
                    );
                    return self.missing(range);
                }
                return self.alloc(ExprKind::Assert(args[0]), range);
            }
            let resolved = self.resolve(c.syntax());
            let args = self.args(node);
            let callee = match resolved {
                // A tuple struct or variant applied to its fields.
                Some(Resolved::Ctor(adt, variant, ty_args)) => {
                    return self.ctor_call(range, adt, variant, ty_args, args);
                }
                Some(r) => self.resolved(r, c.syntax().text_range()),
                None => self.missing(c.syntax().text_range()),
            };
            return self.alloc(ExprKind::Call { callee, args }, range);
        }
        let callee = self.expr_or_missing(callee_node, range);
        let args = self.args(node);
        self.alloc(ExprKind::Call { callee, args }, range)
    }

    /// `discard(r, "reason")`: the reason is a string literal.
    fn discard(&mut self, node: &SyntaxNode, range: TextRange) -> ExprId {
        let args: Vec<ast::Expr> = node
            .child(K::ARG_LIST)
            .and_then(ast::ArgList::cast)
            .map(|l| l.args().filter_map(|a| a.expr()).collect())
            .unwrap_or_default();
        let reason = args
            .get(1)
            .is_some_and(|r| matches!(r, ast::Expr::Literal(l) if l.token(K::STRING).is_some()));
        if args.len() != 2 || !reason {
            self.report(
                Code::E0202,
                range,
                "`discard` takes the `Result` and a string saying why: `discard(r, \"reason\")`",
            );
            return self.missing(range);
        }
        let r = self.expr(&args[0]);
        self.alloc(ExprKind::Discard(r), range)
    }

    /// `try { … }`, maybe with `catch { arms }`.
    /// `with E = value, … { body }` (`eff.with.1`): each effect named once (E0606), each an
    /// effect (E0601).
    fn with_expr(&mut self, node: &SyntaxNode) -> ExprId {
        let range = node.text_range();
        let mut handlers: Vec<Handler> = Vec::new();
        for h in node.children().filter(|c| c.kind() == K::HANDLER) {
            let effect = match h.children().find_map(ast::Type::cast) {
                Some(t) => {
                    let mut ds = Vec::new();
                    let ty = crate::resolve::effect_type(&t, &self.types, &mut ds);
                    self.out.diagnostics.extend(ds);
                    if let Some(why) = self.items.not_an_effect(&ty) {
                        self.report(Code::E0601, t.syntax().text_range(), why);
                        Ty::Error
                    } else {
                        ty
                    }
                }
                None => Ty::Error,
            };
            if effect != Ty::Error
                && let Some(first) = handlers.iter().find(|x| x.effect == effect)
            {
                let d = Diagnostic::error(
                    Code::E0606,
                    h.text_range(),
                    format!(
                        "`{}` is given two handlers here",
                        crate::check::show_effect(&effect)
                    ),
                )
                .with_label(first.range, "first given here");
                self.error(d);
            }
            let value = h.children().find_map(ast::Expr::cast);
            let value = self.expr_or_missing(value, h.text_range());
            handlers.push(Handler {
                effect,
                value,
                range: h.text_range(),
            });
        }
        let body = match node.child(K::BLOCK).and_then(ast::Block::cast) {
            Some(b) => self.block(&b),
            None => self.missing(range),
        };
        self.alloc(ExprKind::With { handlers, body }, range)
    }

    fn try_expr(&mut self, node: &SyntaxNode) -> ExprId {
        let range = node.text_range();
        let Some(block) = node.child(K::BLOCK).and_then(ast::Block::cast) else {
            return self.missing(range);
        };
        let body = self.block(&block);
        let catch = node
            .child(K::CATCH)
            .and_then(|c| c.child(K::MATCH_ARM_LIST))
            .map(|list| {
                let mut arms = Vec::new();
                for arm in list.children().filter(|c| c.kind() == K::MATCH_ARM) {
                    let mark = self.names.mark();
                    let mut binds = Binds::default();
                    let pat = match arm.children().find_map(ast::Pat::cast) {
                        Some(p) => self.catch_pat(&p, &mut binds),
                        None => self.alloc_pat(PatKind::Missing, arm.text_range()),
                    };
                    self.bring_into_scope(binds);
                    let guard = arm.child(K::MATCH_GUARD).map(|g| {
                        let cond = g.children().find_map(ast::Expr::cast);
                        self.expr_or_missing(cond, g.text_range())
                    });
                    let body = arm.children().filter_map(ast::Expr::cast).last();
                    let body = self.expr_or_missing(body, arm.text_range());
                    self.names.restore(mark);
                    arms.push(Arm { pat, guard, body });
                }
                arms
            });
        self.alloc(ExprKind::Try { body, catch }, range)
    }

    /// A `catch` arm's pattern: `Type(p)` names an error's type (`err.catch.1`); anything else
    /// is an ordinary pattern.
    fn catch_pat(&mut self, p: &ast::Pat, b: &mut Binds) -> crate::hir::PatId {
        let node = p.syntax();
        if let ast::Pat::TupleStruct(_) = p
            && let Some(segs) = node.child(K::PATH).and_then(|x| path_segments(&x))
            && let [one] = segs.as_slice()
            && let Some(adt) = self.items.adt_named(&one.name)
        {
            let subs: Vec<ast::Pat> = node.children().filter_map(ast::Pat::cast).collect();
            if let [sub] = subs.as_slice()
                && !matches!(sub, ast::Pat::Rest(_))
            {
                let pat = self.pat(sub, b);
                return self.alloc_pat(PatKind::OfType { adt, pat }, node.text_range());
            }
        }
        self.pat(p, b)
    }

    pub(super) fn ctor_call(
        &mut self,
        range: TextRange,
        adt: u32,
        variant: u32,
        ty_args: Vec<Ty>,
        args: Vec<ExprId>,
    ) -> ExprId {
        let def = self.items.variant(adt, variant);
        let path = self.items.adts[adt as usize].path(variant);
        if def.shape != Shape::Tuple {
            self.report(
                Code::E0111,
                range,
                format!("`{path}` has no positional fields, so it is not called"),
            );
            return self.missing(range);
        }
        if args.len() != def.fields.len() {
            let n = def.fields.len();
            let d = Diagnostic::error(
                Code::E0202,
                range,
                format!(
                    "`{path}` has {n} field{}, but {} {} given",
                    if n == 1 { "" } else { "s" },
                    args.len(),
                    if args.len() == 1 { "was" } else { "were" },
                ),
            )
            .with_label(def.name_range, "defined here");
            self.error(d);
        }
        let fields = args
            .into_iter()
            .take(def.fields.len())
            .enumerate()
            .map(|(i, a)| (i as u32, a))
            .collect();
        self.alloc(
            ExprKind::Adt {
                adt,
                variant,
                fields,
                args: ty_args,
            },
            range,
        )
    }
}

pub(super) fn compare_op(tok: &SyntaxToken) -> Option<CompareOp> {
    Some(match tok.kind() {
        K::EQ_EQ => CompareOp::Eq,
        K::BANG_EQ => CompareOp::Ne,
        K::LT => CompareOp::Lt,
        K::LT_EQ => CompareOp::Le,
        K::GT => CompareOp::Gt,
        K::GT_EQ => CompareOp::Ge,
        _ => return None,
    })
}

pub(super) fn binary_op(tok: &SyntaxToken) -> Option<BinaryOp> {
    Some(match tok.kind() {
        K::PLUS => BinaryOp::Add,
        K::MINUS => BinaryOp::Sub,
        K::STAR => BinaryOp::Mul,
        K::SLASH => BinaryOp::Div,
        K::PERCENT => BinaryOp::Rem,
        K::AMP => BinaryOp::BitAnd,
        K::PIPE => BinaryOp::BitOr,
        K::CARET => BinaryOp::BitXor,
        K::LT_LT => BinaryOp::Shl,
        K::GT_GT => BinaryOp::Shr,
        _ => return None,
    })
}

/// A string literal's text, without its quotes (escapes are kept as written).
fn unquote(text: &str) -> String {
    text.strip_prefix('"')
        .and_then(|t| t.strip_suffix('"'))
        .unwrap_or(text)
        .to_string()
}

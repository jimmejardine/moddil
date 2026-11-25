//! Expressions: each kind of syntax tree expression as HIR.

use moddil_syntax::ast::{self, AstNode};
use moddil_syntax::{SyntaxKind as K, SyntaxNode, SyntaxToken, TextRange};

use super::names::JumpError;
use super::{Binds, Lowerer, Resolved};
use crate::hir::{
    Arm, BinaryOp, CompareOp, ExprId, ExprKind, Halt, Handler, LogicOp, PatKind, UnaryOp,
};
use crate::items::{Def, Shape};
use crate::resolve::{generic_args, path_segments};
use crate::{Code, Diagnostic, Ty};

impl Lowerer<'_> {
    pub(super) fn expr(&mut self, e: &ast::Expr) -> ExprId {
        let node = e.syntax().clone();
        let range = node.text_range();
        if let Some(got) = self.received_at(&node) {
            return got;
        }
        match e {
            ast::Expr::Literal(_) => self.literal(&node, false),
            ast::Expr::InlineFor(_) => self.inline_for(&node),
            ast::Expr::ReflectedField(_) => self.reflected_field(&node),
            ast::Expr::Path(_) if self.reflected(&node).is_some() => {
                self.report(
                    Code::E1007,
                    range,
                    "a reflected field is used as `f.name`, `x.[f]`, `[f]: e`, `f.is_refined()` or \
                     `f.new(e)`, not as a value",
                );
                self.missing(range)
            }
            ast::Expr::Path(_) if self.reflected_variant(&node).is_some() => {
                self.report(
                    Code::E1007,
                    range,
                    "a reflected variant is used as `v.name`, `v.is(x)` or `v.make()`, not as a \
                     value",
                );
                self.missing(range)
            }
            ast::Expr::Path(_) if self.const_param(&node).is_some() => {
                let i = self.const_param(&node).expect("a const parameter");
                self.alloc(ExprKind::ConstParam(i), range)
            }
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
                Some(Resolved::Fn(func, _)) if self.items.fns[func as usize].deriver.is_some() => {
                    self.report(
                        Code::E1006,
                        range,
                        "a derive function is not called: `#[derive]` instantiates its template",
                    );
                    self.missing(range)
                }
                Some(Resolved::Fn(func, args)) if !self.items.fns[func as usize].const_item => {
                    let callee = self.alloc(ExprKind::Fn { func, args }, range);
                    self.fn_value(callee, func, range)
                }
                // A trait's const (`T::N`, `item.const.assoc.1`): its value, a call of it.
                Some(r @ Resolved::TraitFn(_, func, ..))
                    if self.items.fns[func as usize].const_item =>
                {
                    let callee = self.resolved(r, range);
                    self.const_use(callee, range)
                }
                Some(r @ Resolved::TraitFn(_, func, ..)) => {
                    let callee = self.resolved(r, range);
                    self.fn_value(callee, func, range)
                }
                Some(Resolved::AssocFn(ty, name, at, args))
                    if self
                        .inherent_fn(&ty, &name)
                        .is_some_and(|f| self.items.fns[f as usize].const_item) =>
                {
                    let callee = self.resolved(Resolved::AssocFn(ty, name, at, args), range);
                    self.const_use(callee, range)
                }
                // A type's function as a value (`Vec::new`, `Point::y`): its arity is the
                // function's (`expr.fn-value.1`).
                Some(Resolved::AssocFn(ty, name, at, args))
                    if self.inherent_fn(&ty, &name).is_some() =>
                {
                    let func = self.inherent_fn(&ty, &name).expect("found");
                    let callee = self.resolved(Resolved::AssocFn(ty, name, at, args), range);
                    self.fn_value(callee, func, range)
                }
                // A tuple variant or struct as a value (`Some`): a closure building it.
                Some(Resolved::Ctor(adt, variant, ty_args))
                    if self.items.variant(adt, variant).shape == Shape::Tuple =>
                {
                    let n = self.items.variant(adt, variant).fields.len();
                    self.value_closure(range, n, |this, args| {
                        this.ctor_call(range, adt, variant, ty_args, args)
                    })
                }
                // A type's const from a trait it implements (`Point::N`), known once the
                // checker finds it; a function so named is not a value.
                Some(r @ Resolved::AssocFn(..)) => {
                    let callee = self.resolved(r, range);
                    let call = self.const_use(callee, range);
                    self.out.body.const_paths.insert(call);
                    call
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
            ast::Expr::Unsafe(_) => self.unsafe_block(&node),
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
                    Some(inner @ ast::Expr::For(_)) => self.for_expr(inner.syntax(), label),
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
            ast::Expr::For(_) => self.for_expr(&node, None),
            ast::Expr::Range(_) => self.range_expr(&node, e),
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
            ast::Expr::Select(_) => self.select_expr(&node),
            ast::Expr::With(_) => self.with_expr(&node),
            ast::Expr::Closure(_) => self.closure_expr(&node),
            ast::Expr::Impl(_) => self.anon_impl(&node),
            ast::Expr::Array(_) => self.array(&node),
            // `const { e }`: the body's locals and parameters are out of its reach.
            ast::Expr::ConstBlock(_) => {
                let names = std::mem::take(&mut self.names);
                let params = std::mem::take(&mut self.types.params);
                let inner = node
                    .children()
                    .find_map(ast::Block::cast)
                    .map(|b| self.block(&b));
                self.names = names;
                self.types.params = params;
                let inner = inner.unwrap_or_else(|| self.missing(range));
                let block = self.alloc(ExprKind::ConstBlock(inner), range);
                // Under a `#[fuel(n)]` statement (`comptime.eval.2`).
                if let Some(n) = self.fuel {
                    self.out.body.fuel.insert(block, n);
                }
                block
            }
            ast::Expr::Index(_) => {
                let parts: Vec<ast::Expr> = node.children().filter_map(ast::Expr::cast).collect();
                match parts.as_slice() {
                    // `xs[a..b]` and the other ranges (`expr.slice.1`).
                    [b, r @ ast::Expr::Range(_)] => {
                        let base = self.expr(b);
                        let rn = r.syntax();
                        let inclusive = rn.token(K::DOT_DOT_EQ).is_some();
                        let op = rn
                            .child_tokens()
                            .find(|t| matches!(t.kind(), K::DOT_DOT | K::DOT_DOT_EQ))
                            .map_or(rn.text_range().start, |t| t.text_range().start);
                        let mut start = None;
                        let mut end = None;
                        for p in r.sub_exprs() {
                            let x = self.expr(&p);
                            if p.syntax().text_range().start < op {
                                start = Some(x);
                            } else {
                                end = Some(x);
                            }
                        }
                        self.alloc(
                            ExprKind::Slice {
                                base,
                                start,
                                end,
                                inclusive,
                            },
                            range,
                        )
                    }
                    [b, i] => {
                        let base = self.expr(b);
                        let index = self.expr(i);
                        self.alloc(ExprKind::Index { base, index }, range)
                    }
                    _ => self.unsupported(range, "this kind of index"),
                }
            }
            _ => self.unsupported(range, "this expression"),
        }
    }

    /// A single-segment path naming a const parameter in scope (and no local).
    fn const_param(&self, node: &SyntaxNode) -> Option<u32> {
        let segs = node.child(K::PATH).and_then(|p| path_segments(&p))?;
        let [one] = segs.as_slice() else {
            return None;
        };
        if self.lookup(&one.name).is_some() {
            return None;
        }
        let i = self.types.params.iter().rposition(|p| p.name == one.name)?;
        self.types.params[i].konst.then_some(i as u32)
    }

    /// `[a, b, c]` or `[x; n]` (`expr.array.1`).
    fn array(&mut self, node: &SyntaxNode) -> ExprId {
        let range = node.text_range();
        let parts: Vec<ast::Expr> = node.children().filter_map(ast::Expr::cast).collect();
        if node.token(K::SEMI).is_some() {
            let [value, len] = parts.as_slice() else {
                return self.missing(range);
            };
            let value = self.expr(value);
            // A literal or a const parameter is a length; anything else is a value, for a type
            // made with `from_repeat`.
            let constant = match len {
                ast::Expr::Literal(_) => true,
                ast::Expr::Path(_) => self.const_param(len.syntax()).is_some(),
                _ => false,
            };
            if !constant {
                let n = self.expr(len);
                return self.alloc(ExprKind::RepeatN(value, n), range);
            }
            let mut ds = Vec::new();
            let n = crate::resolve::array_len(len, &self.types, &mut ds);
            self.out.diagnostics.extend(ds);
            return self.alloc(ExprKind::Repeat(value, n), range);
        }
        let elements: Vec<ExprId> = parts.iter().map(|x| self.expr(x)).collect();
        self.alloc(ExprKind::Array(elements), range)
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
            K::INTEGER => match ast::IntValue::parse(tok.text()) {
                Some(v) => self.alloc(ExprKind::Int(if negate { v.negated() } else { v }), range),
                None => {
                    self.report(
                        Code::E0204,
                        range,
                        format!("`{}` is too large for any integer type", tok.text()),
                    );
                    self.missing(range)
                }
            },
            K::FLOAT => {
                let text: String = tok.text().chars().filter(|&c| c != '_').collect();
                let text = if negate { format!("-{text}") } else { text };
                self.alloc(ExprKind::Float(text), range)
            }
            K::CHAR => match super::text::char_value(tok.text()) {
                Ok(c) => self.alloc(ExprKind::Char(c), range),
                Err(why) => {
                    self.report(Code::E0902, range, why);
                    self.missing(range)
                }
            },
            K::STRING | K::RAW_STRING | K::BLOCK_STRING => self.string_lit(&tok, range),
            K::TAGGED_STRING => self.tagged(&tok, range),
            _ => self.unsupported(range, "this kind of literal"),
        }
    }

    /// `Path { field: value, … }`.
    pub(super) fn struct_lit(&mut self, node: &SyntaxNode) -> ExprId {
        let range = node.text_range();
        let mut ty_args = Vec::new();
        // In a template, its parameter names the struct (`t { … }`).
        let names_template = self.template.is_some_and(|t| {
            node.child(K::PATH)
                .and_then(ast::Path::cast)
                .is_some_and(|p| p.segments() == [t.var.clone()])
        });
        // A reflected variant names its own value, of any shape (`v { [f]: e, … }`).
        let variant_lit = self.variant_literal(node);
        let resolved = if names_template || variant_lit.is_some() {
            None
        } else {
            self.resolve(node)
        };
        // A template's tuple, `t { [f]: e, … }`: the tuple of those elements, in order.
        if names_template
            && let Some(t) = self.template
            && t.adt.is_none()
            && let Ty::Tuple(elems) = &t.ty
        {
            let n = elems.len();
            let mut values: Vec<Option<ExprId>> = vec![None; n];
            let mut ok = true;
            for (name, _, value) in self.struct_inits(node) {
                match name.parse::<usize>() {
                    Ok(k) if k < n && values[k].is_none() => values[k] = Some(value),
                    _ => ok = false,
                }
            }
            return match values.into_iter().collect::<Option<Vec<_>>>() {
                Some(elements) if ok => self.alloc(ExprKind::Tuple(elements), range),
                _ => {
                    self.report(
                        Code::E1007,
                        range,
                        "a template's tuple is written with each of its elements once",
                    );
                    self.missing(range)
                }
            };
        }
        let target = match resolved {
            _ if variant_lit.is_some() => variant_lit,
            _ if names_template => match self.template.and_then(|t| t.adt) {
                Some(adt) if self.items.adts[adt as usize].is_enum => {
                    self.report(
                        Code::E1007,
                        range,
                        "an enum's value is one of its variants: `v { … }` for a variant `v` \
                         (`comptime.template.variant.1`)",
                    );
                    None
                }
                Some(adt) => Some((adt, 0)),
                None => {
                    self.report(
                        Code::E1007,
                        range,
                        "a template for a built-in type other than a tuple has no fields to \
                         write (`comptime.derive.for.1`)",
                    );
                    None
                }
            },
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
        for (name, name_range, value) in self.struct_inits(node) {
            let Some((adt, variant)) = target else {
                continue;
            };
            let def = self.items.variant(adt, variant);
            match def.field(&name) {
                Some(i) => {
                    if let Some(&(_, first)) = given.iter().find(|(g, _)| *g == i) {
                        let d = Diagnostic::error(
                            Code::E0108,
                            name_range,
                            format!("the field `{name}` is given twice"),
                        )
                        .with_label(first, "first given here");
                        self.error(d);
                    } else {
                        given.push((i, name_range));
                        fields.push((i, value));
                    }
                }
                None => {
                    let path = self.items.adts[adt as usize].path(variant);
                    self.report(
                        Code::E0107,
                        name_range,
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
        // A reflected variant's `is(x)` and `make()`, in a template.
        if let Some(k) = e
            .sub_exprs()
            .first()
            .and_then(|r| self.reflected_variant(r.syntax()))
        {
            let method = ast::member_name(node).map(|t| t.text().to_string());
            let args: Vec<ast::Expr> = node
                .child(K::ARG_LIST)
                .and_then(ast::ArgList::cast)
                .map(|l| l.args().filter_map(|a| a.expr()).collect())
                .unwrap_or_default();
            return self.variant_call(k, &method.unwrap_or_default(), &args, range);
        }
        // A reflected field's `is_refined()` and `new(e)`, in a template.
        if let Some(k) = e
            .sub_exprs()
            .first()
            .and_then(|r| self.reflected(r.syntax()))
        {
            let method = ast::member_name(node).map(|t| t.text().to_string());
            let args: Vec<ast::Expr> = node
                .child(K::ARG_LIST)
                .and_then(ast::ArgList::cast)
                .map(|l| l.args().filter_map(|a| a.expr()).collect())
                .unwrap_or_default();
            return self.reflected_call(k, &method.unwrap_or_default(), &args, range);
        }
        let receiver = e.sub_exprs().into_iter().next();
        let receiver = self.expr_or_missing(receiver, range);
        let generic_args = node.child(K::GENERIC_ARGS).map(|a| {
            let mut ds = Vec::new();
            let (types, _) = generic_args(&a, &self.types, &mut ds);
            self.out.diagnostics.extend(ds);
            types
        });
        let args = self.args(node);
        // A method's name may be a keyword (`s.spawn(…)`).
        let Some(name) = ast::member_name(node) else {
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
                let x = a.expr();
                let id = self.expr_or_missing(x, a.syntax().text_range());
                // A named argument (`expr.call.named.1`).
                if let Some(label) = a.label() {
                    self.out
                        .body
                        .labels
                        .insert(id, (label.text().to_string(), label.text_range()));
                }
                args.push(id);
            }
        }
        args
    }

    /// `base.name` or `base.0`.
    pub(super) fn field(&mut self, node: &SyntaxNode, e: &ast::Expr) -> ExprId {
        let range = node.text_range();
        if let Some(k) = e
            .sub_exprs()
            .first()
            .and_then(|b| self.reflected(b.syntax()))
        {
            let member = ast::member_name(node).map(|t| t.text().to_string());
            return self.reflected_member(k, &member.unwrap_or_default(), range);
        }
        if let Some(k) = e
            .sub_exprs()
            .first()
            .and_then(|b| self.reflected_variant(b.syntax()))
        {
            let member = ast::member_name(node).map(|t| t.text().to_string());
            return self.variant_member(k, &member.unwrap_or_default(), range);
        }
        // `t.name`: the name of the type a template is instantiated for.
        if ast::member_name(node).is_some_and(|m| m.text() == "name")
            && let Some(name) = e
                .sub_exprs()
                .first()
                .and_then(|b| self.template_name(b.syntax()))
        {
            return self.alloc(ExprKind::Str(name), range);
        }
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
            return self.if_const(node, e);
        }
        // `if v.is(x) { … }` in a template: a match on `x` (`comptime.template.variant.1`).
        if let Some(m) = self.variant_if(node, e) {
            return m;
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
        if let Some(attr) = list
            .children()
            .find(|c| c.kind() == K::INNER_ATTR && !crate::is_doc(c))
        {
            self.error(Diagnostic::unsupported(attr.text_range(), "an attribute"));
        }
        for arm in list.children().filter(|c| c.kind() == K::MATCH_ARM) {
            if let Some(attr) = arm
                .children()
                .find(|c| c.kind() == K::OUTER_ATTR && !crate::is_doc(c))
            {
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

    /// `select { p = op => body, … }` (`task.select.1`): each arm's pattern binds names for its
    /// body.
    fn select_expr(&mut self, node: &SyntaxNode) -> ExprId {
        let range = node.text_range();
        let Some(list) = node.child(K::MATCH_ARM_LIST) else {
            return self.missing(range);
        };
        let mut arms = Vec::new();
        for arm in list.children().filter(|c| c.kind() == K::SELECT_ARM) {
            let exprs: Vec<ast::Expr> = arm.children().filter_map(ast::Expr::cast).collect();
            let op = self.expr_or_missing(exprs.first().cloned(), arm.text_range());
            let mark = self.names.mark();
            let mut binds = Binds::default();
            let pat = match arm.children().find_map(ast::Pat::cast) {
                Some(p) => self.pat(&p, &mut binds),
                None => self.alloc_pat(PatKind::Missing, arm.text_range()),
            };
            self.bring_into_scope(binds);
            let body = self.expr_or_missing(exprs.get(1).cloned(), arm.text_range());
            self.names.restore(mark);
            arms.push(crate::hir::SelectArm { pat, op, body });
        }
        self.alloc(ExprKind::Select(arms), range)
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

    /// `for p in e { body }` (`expr.for.1`), as
    /// `{ let mut it = IntoIterator::into_iter(e); loop { match Iterator::next(&mut it) {
    /// Some(p) => body, None => break } } }`.
    pub(super) fn for_expr(&mut self, node: &SyntaxNode, label: Option<String>) -> ExprId {
        let range = node.text_range();
        let exprs: Vec<ast::Expr> = node.children().filter_map(ast::Expr::cast).collect();
        let (Some(mut iterable), Some(pat_node)) = (
            exprs.first().cloned(),
            node.children().find_map(ast::Pat::cast),
        ) else {
            return self.missing(range);
        };
        // `for x in it? { … }` (`expr.for.3`): the `?` marks each call of `next`.
        let marked = matches!(iterable, ast::Expr::Propagate(_));
        if marked {
            let q = iterable.syntax().clone();
            if let Some(note) = q.child(K::ERROR_CONTEXT) {
                self.error(Diagnostic::unsupported(
                    note.text_range(),
                    "a note after a `for`'s `?`",
                ));
            }
            match iterable.sub_exprs().into_iter().next() {
                Some(inner) => iterable = inner,
                None => return self.missing(range),
            }
        }
        let lang = |cx: &Self, tr: &str, method: &str| -> Option<(u32, u32)> {
            let t = cx.items.lang(tr)?;
            let f = cx.items.traits[t as usize]
                .methods
                .iter()
                .copied()
                .find(|&f| cx.items.fns[f as usize].name == method)?;
            Some((t, f))
        };
        let (Some((into_t, into_f)), Some((iter_t, next_f)), Some(option)) = (
            lang(self, "IntoIterator", "into_iter"),
            lang(self, "Iterator", "next"),
            self.items.lang("Option"),
        ) else {
            return self.unsupported(range, "`for` without the prelude's iterators");
        };
        let variant = |cx: &Self, name: &str| {
            cx.items.adts[option as usize]
                .variants
                .iter()
                .position(|v| v.name == name)
                .unwrap_or(0) as u32
        };
        let (some, none) = (variant(self, "Some"), variant(self, "None"));
        let trait_fn = |cx: &mut Self, trait_: u32, func: u32| {
            cx.alloc(
                ExprKind::TraitFn {
                    trait_,
                    func,
                    trait_args: Vec::new(),
                    args: None,
                },
                range,
            )
        };
        // The iterator, in a local the program cannot name.
        let iterable = self.expr(&iterable);
        let into = trait_fn(self, into_t, into_f);
        let init = self.alloc(
            ExprKind::Call {
                callee: into,
                args: vec![iterable],
            },
            range,
        );
        let it = self.out.body.locals.alloc(crate::hir::Local {
            name: "{iterator}".to_string(),
            mutable: true,
            range,
            closure: self.closure,
        });
        let it_pat = self.alloc_pat(
            PatKind::Bind {
                local: it,
                sub: None,
            },
            range,
        );
        // The loop.
        let id = self.missing(range);
        self.names.enter_loop(label, id);
        let mark = self.names.mark();
        let next = trait_fn(self, iter_t, next_f);
        let it_expr = self.alloc(ExprKind::Local(it), range);
        let borrow = self.alloc(
            ExprKind::Ref {
                mutable: true,
                expr: it_expr,
            },
            range,
        );
        let call = self.alloc(
            ExprKind::Call {
                callee: next,
                args: vec![borrow],
            },
            range,
        );
        self.out.body.for_nexts.insert(call);
        // Marked, the call's errors pass on, and its `Option` is still the loop's to take apart.
        let scrutinee = if marked {
            self.alloc(ExprKind::Pass(call), range)
        } else {
            call
        };
        let mut binds = Binds::default();
        let p = self.pat(&pat_node, &mut binds);
        self.bring_into_scope(binds);
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
        let some_pat = self.alloc_pat(
            PatKind::Adt {
                adt: option,
                variant: some,
                fields: vec![(0, p)],
            },
            range,
        );
        let none_pat = self.alloc_pat(
            PatKind::Adt {
                adt: option,
                variant: none,
                fields: Vec::new(),
            },
            range,
        );
        let leave = self.alloc(
            ExprKind::Break {
                target: id,
                value: None,
            },
            range,
        );
        let arms = vec![
            Arm {
                pat: some_pat,
                guard: None,
                body,
            },
            Arm {
                pat: none_pat,
                guard: None,
                body: leave,
            },
        ];
        let m = self.alloc(ExprKind::Match { scrutinee, arms }, range);
        self.out.body.exprs[id].kind = ExprKind::Loop { body: m };
        self.alloc(
            ExprKind::Block {
                stmts: vec![crate::hir::Stmt::Let {
                    pat: it_pat,
                    ty: None,
                    init: Some(init),
                    else_: None,
                }],
                tail: Some(id),
                observe_tail: false,
            },
            range,
        )
    }

    /// `a..b`, `a..=b` and `a..` (`expr.range.1`): the prelude's `Range`, `RangeInclusive` and
    /// `RangeFrom`.
    fn range_expr(&mut self, node: &SyntaxNode, e: &ast::Expr) -> ExprId {
        let range = node.text_range();
        let parts = e.sub_exprs();
        let inclusive = node.token(K::DOT_DOT_EQ).is_some();
        let starts = parts
            .first()
            .is_some_and(|p| p.syntax().text_range().start == range.start);
        let (name, ends) = match (starts, parts.len(), inclusive) {
            (true, 2, false) => ("Range", true),
            (true, 2, true) => ("RangeInclusive", true),
            (true, 1, false) => ("RangeFrom", false),
            _ => return self.unsupported(range, "this kind of range"),
        };
        let Some(adt) = self.items.lang(name) else {
            return self.missing(range);
        };
        let mut fields = vec![(0, self.expr(&parts[0]))];
        if ends {
            fields.push((1, self.expr(&parts[1])));
        }
        if inclusive {
            let done = self.alloc(ExprKind::Bool(false), range);
            fields.push((2, done));
        }
        self.alloc(
            ExprKind::Adt {
                adt,
                variant: 0,
                fields,
                args: Vec::new(),
            },
            range,
        )
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

    /// `unsafe("why") { … }` (`expr.unsafe.1`): a block in which raw pointers may be
    /// dereferenced. Its mod must declare the `unsafe` capability (`mod.caps.1`), and the
    /// justification must say something.
    fn unsafe_block(&mut self, node: &SyntaxNode) -> ExprId {
        let range = node.text_range();
        let module = &self.items.modules[self.types.module as usize];
        let allowed = self
            .items
            .mods
            .get(module.mod_ as usize)
            .is_none_or(|m| m.unsafe_ok);
        if !allowed {
            self.report(
                Code::E0219,
                range,
                "`unsafe` is used only in a mod whose manifest declares the capability: \
                 `capabilities = [\"unsafe\"]` under `[mod]`",
            );
        }
        let why = node.token(K::STRING);
        if why.is_some_and(|t| t.text().trim_matches('"').trim().is_empty()) {
            self.report(
                Code::E0219,
                range,
                "an `unsafe` block says why it is sound: `unsafe(\"…\") { … }`",
            );
        }
        let Some(block) = node.children().find_map(ast::Block::cast) else {
            return self.missing(range);
        };
        self.unsafe_depth += 1;
        let e = self.block(&block);
        self.unsafe_depth -= 1;
        e
    }

    pub(super) fn prefix(&mut self, node: &SyntaxNode, e: &ast::Expr) -> ExprId {
        let range = node.text_range();
        let Some(op) = ast::operator(node) else {
            return self.missing(range);
        };
        let operand = e.sub_exprs().into_iter().next();
        if op.kind() == K::STAR {
            let x = self.expr_or_missing(operand, range);
            let e = self.alloc(ExprKind::Deref(x), range);
            if self.unsafe_depth > 0 {
                self.out.body.unsafe_derefs.insert(e);
            }
            return e;
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
        // Literals, folded exactly (`num.lit.1`).
        if let (Some(a), Some(b)) = (
            super::fold::Exact::of(&self.out.body.exprs[l].kind),
            super::fold::Exact::of(&self.out.body.exprs[r].kind),
        ) && let Some(v) = super::fold::fold(op, a, b)
        {
            return self.alloc(v.literal(), range);
        }
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
            K::PLUS_PERCENT_EQ => Some(BinaryOp::WrapAdd),
            K::MINUS_PERCENT_EQ => Some(BinaryOp::WrapSub),
            K::STAR_PERCENT_EQ => Some(BinaryOp::WrapMul),
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
        let mut target = self.expr(&target);
        // The place must be a local or a dereferenced borrow, or fields or elements of one.
        // Whether it may be changed needs types, so the checker decides that.
        let mut root = target;
        while let ExprKind::Field { base, .. } | ExprKind::Index { base, .. } =
            self.out.body.exprs[root].kind
        {
            root = base;
        }
        match self.out.body.exprs[root].kind {
            ExprKind::Local(_) | ExprKind::Deref(_) | ExprKind::Missing => {}
            _ if self.is_fn_value(root) => {
                self.report(
                    Code::E0206,
                    target_range,
                    "a function cannot be assigned to",
                );
                target = self.missing(target_range);
            }
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

    /// The value of the const `callee` names: a call of it, with no arguments.
    fn const_use(&mut self, callee: ExprId, range: TextRange) -> ExprId {
        self.alloc(
            ExprKind::Call {
                callee,
                args: Vec::new(),
            },
            range,
        )
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
            // `old(e)` in a postcondition: `e` as it was on entry (`contract.old.1`).
            if let [one] = segments.as_slice()
                && one.name == "old"
                && self.olds.is_some()
                && self.lookup("old").is_none()
                && self.item("old").is_none()
            {
                let args: Vec<ast::Expr> = node
                    .child(K::ARG_LIST)
                    .and_then(ast::ArgList::cast)
                    .map(|l| l.args().filter_map(|a| a.expr()).collect())
                    .unwrap_or_default();
                let [arg] = args.as_slice() else {
                    self.report(Code::E1103, range, "`old` takes one value: `old(e)`");
                    return self.missing(range);
                };
                // Evaluated on entry, where `old` means nothing more.
                let olds = self.olds.take();
                let value = self.expr(arg);
                self.olds = olds;
                let local = self.out.body.locals.alloc(crate::hir::Local {
                    name: "{old}".to_string(),
                    mutable: false,
                    range,
                    closure: self.closure,
                });
                if let Some(o) = &mut self.olds {
                    o.push((local, value));
                }
                return self.alloc(ExprKind::Local(local), range);
            }
            // `discard` is in the prelude too.
            if let [one] = segments.as_slice()
                && one.name == "discard"
                && self.lookup("discard").is_none()
                && self.item("discard").is_none()
            {
                return self.discard(node, range);
            }
            // `assert` is in the prelude: a local or item of the same name shadows it.
            if let [one] = segments.as_slice()
                && one.name == "assert"
                && self.lookup("assert").is_none()
                && self.item("assert").is_none()
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
            // So are `unreachable` and `todo` (`expr.unreachable.1`, `expr.todo.1`).
            if let [one] = segments.as_slice()
                && let Some(halt) = match one.name.as_str() {
                    "unreachable" => Some(Halt::Unreachable),
                    "todo" => Some(Halt::Todo),
                    _ => None,
                }
                && self.lookup(&one.name).is_none()
                && self.item(&one.name).is_none()
            {
                return self.halt(node, halt, &one.name, range);
            }
            let resolved = self.resolve(c.syntax());
            if let Some(Resolved::Fn(func, _)) = &resolved
                && self.items.fns[*func as usize].deriver.is_some()
            {
                self.report(
                    Code::E1006,
                    range,
                    "a derive function is not called: `#[derive]` instantiates its template",
                );
                return self.missing(range);
            }
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

    /// `unreachable("reason")` or `todo("what")`: the reason is a string literal.
    fn halt(&mut self, node: &SyntaxNode, halt: Halt, name: &str, range: TextRange) -> ExprId {
        let args: Vec<ast::Expr> = node
            .child(K::ARG_LIST)
            .and_then(ast::ArgList::cast)
            .map(|l| l.args().filter_map(|a| a.expr()).collect())
            .unwrap_or_default();
        let reason = match args.as_slice() {
            [ast::Expr::Literal(l)] => l.token(K::STRING).map(|t| unquote(t.text())),
            _ => None,
        };
        let Some(reason) = reason else {
            self.report(
                Code::E0202,
                range,
                format!("`{name}` takes a string saying why: `{name}(\"reason\")`"),
            );
            return self.missing(range);
        };
        self.alloc(ExprKind::Halt(halt, reason), range)
    }

    /// `try { … }`, maybe with `catch { arms }`.
    /// `|p, …| body` and `|p: T, …| -> R { … }` (`expr.closure.1`): its parameters are its own
    /// locals; a jump out of it, or a `return` that would leave its enclosing function, cannot be
    /// written.
    fn closure_expr(&mut self, node: &SyntaxNode) -> ExprId {
        let params: Vec<SyntaxNode> = node
            .child(K::CLOSURE_PARAMS)
            .into_iter()
            .flat_map(|ps| ps.children().collect::<Vec<_>>())
            .filter(|c| c.kind() == K::CLOSURE_PARAM)
            .collect();
        let body = node.children().filter_map(ast::Expr::cast).last();
        self.closure_of(node, &params, body)
    }

    /// The path `e`, lowered as a call's callee: a function named, not a closure calling it.
    pub(super) fn callee_path(&mut self, e: &ast::Expr) -> ExprId {
        let range = e.syntax().text_range();
        match self.resolve(e.syntax()) {
            Some(r) => self.resolved(r, range),
            None => self.missing(range),
        }
    }

    /// Whether closure expression `e` is a function used as a value (`expr.fn-value.1`).
    fn is_fn_value(&self, e: ExprId) -> bool {
        let ExprKind::Closure { body, .. } = self.out.body.exprs[e].kind else {
            return false;
        };
        let ExprKind::Pass(call) = self.out.body.exprs[body].kind else {
            return false;
        };
        matches!(
            self.out.body.exprs[call].kind,
            ExprKind::Call { callee, .. }
                if matches!(
                    self.out.body.exprs[callee].kind,
                    ExprKind::Fn { .. } | ExprKind::TraitFn { .. } | ExprKind::AssocFn { .. }
                )
                    && self.out.body.exprs[callee].range == self.out.body.exprs[e].range
        )
    }

    /// Function `func`, named by `callee`, used as a value (`expr.fn-value.1`): a closure taking
    /// its arguments and calling it, so it implements the closure traits with the function's
    /// signature, and the needs of its effects are met where it is named.
    fn fn_value(&mut self, callee: ExprId, func: u32, range: TextRange) -> ExprId {
        let n = self.items.fns[func as usize].params.len();
        self.value_closure(range, n, |this, args| {
            let call = this.alloc(ExprKind::Call { callee, args }, range);
            // Its errors pass on, as the function's do.
            this.alloc(ExprKind::Pass(call), range)
        })
    }

    /// The function of an inherent impl of `ty` called `name`, if there is one: what
    /// `Type::name` names as a value.
    fn inherent_fn(&self, ty: &Ty, name: &str) -> Option<u32> {
        let same = |t: &Ty| match (t, ty) {
            (Ty::Adt(a, ..), Ty::Adt(b, ..)) => a == b,
            (t, u) => t == u,
        };
        let fns: Vec<u32> = self
            .items
            .impls
            .iter()
            .filter(|i| i.trait_ref.is_none() && same(&i.self_ty))
            .flat_map(|i| i.methods.iter().copied())
            .filter(|&f| self.items.fns[f as usize].name == name)
            .collect();
        // The function without `self` first (`item.impl.same-name.1`).
        fns.iter()
            .copied()
            .find(|&f| self.items.fns[f as usize].self_kind.is_none())
            .or_else(|| fns.first().copied())
    }

    /// A closure of `n` parameters whose body `body` makes from them.
    fn value_closure(
        &mut self,
        range: TextRange,
        n: usize,
        body: impl FnOnce(&mut Self, Vec<ExprId>) -> ExprId,
    ) -> ExprId {
        let id = self.alloc(ExprKind::Missing, range);
        let parent = self.closure.replace(id);
        let mut params = Vec::new();
        let mut args = Vec::new();
        for i in 0..n {
            let local = self.out.body.locals.alloc(crate::hir::Local {
                name: format!("{{arg{i}}}"),
                mutable: false,
                range,
                closure: Some(id),
            });
            let pat = self.alloc_pat(PatKind::Bind { local, sub: None }, range);
            params.push((pat, None));
            args.push(self.alloc(ExprKind::Local(local), range));
        }
        let body = body(self, args);
        self.closure = parent;
        self.out.body.exprs[id].kind = ExprKind::Closure {
            params,
            ret: None,
            body,
            parent,
        };
        id
    }

    /// A closure from its parts: the node (for its range and result type), its parameters (each
    /// a pattern and maybe a type) and its body.
    fn closure_of(
        &mut self,
        node: &SyntaxNode,
        param_nodes: &[SyntaxNode],
        body: Option<ast::Expr>,
    ) -> ExprId {
        let range = node.text_range();
        let id = self.alloc(ExprKind::Missing, range);
        let parent = self.closure.replace(id);
        let mark = self.names.mark();
        let outer = self.names.enter_closure();
        let mut binds = Binds::default();
        let mut params = Vec::new();
        for p in param_nodes {
            let pat = match p.children().find_map(ast::Pat::cast) {
                Some(pat) => self.pat(&pat, &mut binds),
                None => self.alloc_pat(PatKind::Missing, p.text_range()),
            };
            let ty = p.children().find_map(ast::Type::cast).map(|t| {
                let mut ds = Vec::new();
                let ty = crate::resolve::resolve_type(&t, &self.types, &mut ds);
                self.out.diagnostics.extend(ds);
                ty
            });
            params.push((pat, ty));
        }
        self.bring_into_scope(binds);
        let ret = node
            .child(K::RET_TYPE)
            .and_then(|r| r.children().find_map(ast::Type::cast))
            .map(|t| {
                let mut ds = Vec::new();
                let ty = crate::resolve::resolve_type(&t, &self.types, &mut ds);
                self.out.diagnostics.extend(ds);
                ty
            });
        let body = self.expr_or_missing(body, range);
        self.names.exit_closure(outer);
        self.names.restore(mark);
        self.closure = parent;
        self.out.body.exprs[id].kind = ExprKind::Closure {
            params,
            ret,
            body,
            parent,
        };
        id
    }

    /// `impl Trait { fn … }` as an expression (`expr.anon-impl.1`): a value of a type of its own
    /// that implements the trait, each method a closure over the enclosing body's locals.
    fn anon_impl(&mut self, node: &SyntaxNode) -> ExprId {
        let range = node.text_range();
        let Some(path) = node.child(K::PATH_TYPE).and_then(|t| t.child(K::PATH)) else {
            return self.unsupported(range, "this `impl` expression");
        };
        let mut ds = Vec::new();
        let found = crate::resolve::trait_path(&path, &self.types, &mut ds);
        self.out.diagnostics.extend(ds);
        let Some((trait_ref, _)) = found else {
            return self.missing(range);
        };
        let def = &self.items.traits[trait_ref.trait_ as usize];
        if let Some(why) = self.items.not_dyn(trait_ref.trait_, false) {
            let name = def.name.clone();
            self.error(Diagnostic::unsupported(
                range,
                &format!("an anonymous impl of `{name}`, which cannot be used as `dyn` ({why}),"),
            ));
            return self.missing(range);
        }
        let mut methods = Vec::new();
        for f in node
            .child(K::IMPL_BODY)
            .into_iter()
            .flat_map(|b| b.children().collect::<Vec<_>>())
            .filter(|c| c.kind() == K::FN_ITEM)
        {
            let name = f
                .child(K::NAME)
                .and_then(ast::Name::cast)
                .map(|n| n.text())
                .unwrap_or_default();
            let def = &self.items.traits[trait_ref.trait_ as usize];
            let Some(&func) = def
                .methods
                .iter()
                .find(|&&m| self.items.fns[m as usize].name == name)
            else {
                let t = def.name.clone();
                self.report(
                    Code::E0408,
                    f.text_range(),
                    format!("`{t}` has no method `{name}`"),
                );
                continue;
            };
            let list = f.child(K::PARAM_LIST);
            let by_ref = list
                .as_ref()
                .and_then(|l| l.child(K::SELF_PARAM))
                .is_some_and(|s| s.token(K::AMP).is_some() && s.token(K::MUT_KW).is_none());
            if !by_ref {
                self.error(Diagnostic::unsupported(
                    f.text_range(),
                    "a method of an anonymous impl that does not take `&self`",
                ));
                continue;
            }
            let params: Vec<SyntaxNode> = list
                .into_iter()
                .flat_map(|l| l.children().collect::<Vec<_>>())
                .filter(|c| c.kind() == K::PARAM)
                .collect();
            let body = f.child(K::BLOCK).and_then(ast::Expr::cast);
            let closure = self.closure_of(&f, &params, body);
            methods.push((func, closure));
        }
        let def = &self.items.traits[trait_ref.trait_ as usize];
        for &m in &def.methods {
            let sig = &self.items.fns[m as usize];
            if !sig.has_body && !methods.iter().any(|&(f, _)| f == m) {
                let (t, n) = (def.name.clone(), sig.name.clone());
                self.report(
                    Code::E0407,
                    range,
                    format!("this impl of `{t}` leaves out its method `{n}`"),
                );
            }
        }
        self.alloc(ExprKind::AnonImpl { trait_ref, methods }, range)
    }

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
            && let Some(Def::Adt(adt)) = self.item(&one.name)
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
        K::PLUS_PERCENT => BinaryOp::WrapAdd,
        K::MINUS_PERCENT => BinaryOp::WrapSub,
        K::STAR_PERCENT => BinaryOp::WrapMul,
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

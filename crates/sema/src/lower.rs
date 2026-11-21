//! Lowering a body from the syntax tree to HIR: the `body` query.

use moddil_syntax::ast::{self, AstNode};
use moddil_syntax::{SyntaxKind as K, SyntaxNode, SyntaxToken, TextRange};

use crate::collect::{item_tree, node_at, source};
use crate::hir::{
    Arm, BinaryOp, Body, CompareOp, Expr, ExprId, ExprKind, Local, LocalId, LogicOp, Pat, PatId,
    PatKind, Stmt, UnaryOp,
};
use crate::items::{Container, GenericParam, Pred, Shape, Src};
use crate::resolve::{Scope, adt_args, generic_args, path_segments, resolve_type};
use crate::ty::TraitRef;
use crate::{Code, Db, Diagnostic, Item, ItemTree, Owner, Ty};

/// A body in HIR, with the problems found while lowering it.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Lowered {
    /// The body.
    pub body: Body,
    /// Unresolved names, unsupported constructs and the like.
    pub diagnostics: Vec<Diagnostic>,
}

/// The HIR of a function's or a test's body.
#[salsa::tracked(returns(ref))]
pub fn body<'db>(db: &'db dyn Db, owner: Owner<'db>) -> Lowered {
    let file = owner.file(db);
    let items = item_tree(db, file);
    let src = match owner.item(db) {
        Item::Fn(i) => items.fns[i as usize].src,
        Item::Test(_) => Src::File,
    };
    let (root, text) = source(db, file, src);
    let mut cx = Lowerer {
        text,
        items,
        out: Lowered::default(),
        scopes: Vec::new(),
        loops: Vec::new(),
        defer_floor: None,
        params: &[],
        preds: &[],
        self_ty: None,
        trait_ctx: None,
        impl_assoc: None,
    };
    let block = match owner.item(db) {
        Item::Fn(i) => {
            let sig = &items.fns[i as usize];
            let node = node_at(&root, K::FN_ITEM, sig.node).and_then(ast::FnItem::cast);
            cx.params = &sig.generics.params;
            cx.preds = &sig.generics.preds;
            match sig.container {
                Container::Impl(k) => {
                    let imp = &items.impls[k as usize];
                    cx.self_ty = Some(imp.self_ty.clone());
                    cx.impl_assoc = imp.trait_ref.clone().map(|t| (t, imp.assoc.clone()));
                }
                Container::Trait(t) => {
                    cx.self_ty = Some(Ty::Param(0, "Self".into()));
                    cx.trait_ctx = Some(t);
                }
                Container::Free => {}
            }
            let param_nodes: Vec<ast::Param> = node
                .as_ref()
                .and_then(ast::FnItem::params)
                .map(|l| l.params().collect())
                .unwrap_or_default();
            // `self` has no `PARAM` node.
            let skip = usize::from(sig.self_kind.is_some());
            let mut binds = Binds::default();
            for (k, p) in sig.params.iter().enumerate() {
                let local = cx.out.body.locals.alloc(Local {
                    name: p.name.clone().unwrap_or_else(|| "_".to_string()),
                    mutable: p.mutable,
                    range: p.range,
                });
                cx.out.body.params.push(local);
                if let Some(name) = &p.name {
                    cx.scopes.push((name.clone(), local));
                } else if p.destructures
                    && let Some(pat) = k
                        .checked_sub(skip)
                        .and_then(|k| param_nodes.get(k))
                        .and_then(ast::Param::pat)
                {
                    let id = cx.pat(&pat, &mut binds);
                    cx.out.body.param_pats.push((local, id));
                }
            }
            cx.scopes
                .extend(binds.names.into_iter().map(|(n, l, _)| (n, l)));
            node.and_then(|f| f.body())
        }
        Item::Test(i) => root
            .children()
            .filter(|c| c.kind() == K::TEST_ITEM)
            .nth(i as usize)
            .and_then(ast::TestItem::cast)
            .and_then(|t| t.body()),
    };
    if let Some(block) = block {
        let id = cx.block(&block);
        cx.out.body.root = Some(id);
    }
    cx.out
}

/// The names a pattern binds as it is lowered.
#[derive(Default)]
struct Binds {
    /// Each name bound so far, with its local and where it is bound.
    names: Vec<(String, LocalId, TextRange)>,
    /// In the second and later alternatives of an or-pattern: the locals the first alternative
    /// bound, so every alternative binds the same ones.
    reuse: Vec<(String, LocalId)>,
}

/// What a value path names.
enum Resolved {
    Local(LocalId),
    /// A function, with the type arguments for its own parameters.
    Fn(u32, Vec<Ty>),
    /// A struct (variant 0) or an enum's variant, with the type's arguments.
    Ctor(u32, u32, Vec<Ty>),
    /// `Type::name`.
    AssocFn(Ty, String, TextRange, Option<Vec<Ty>>),
    /// `Trait::name`: the trait, the method, the trait's arguments and the method's.
    TraitFn(u32, u32, Vec<Ty>, Option<Vec<Ty>>),
}

struct Lowerer<'a> {
    text: &'a str,
    items: &'a ItemTree,
    out: Lowered,
    /// Locals in scope, innermost last.
    scopes: Vec<(String, LocalId)>,
    /// Enclosing loops, innermost last: the label and the loop expression.
    loops: Vec<(Option<String>, ExprId)>,
    /// Inside a `defer` block: how many loops enclose the block, so jumps out of it are errors.
    defer_floor: Option<usize>,
    /// The type parameters in scope.
    params: &'a [GenericParam],
    /// The bounds in scope.
    preds: &'a [Pred],
    /// What `Self` is.
    self_ty: Option<Ty>,
    /// Inside a trait's method: the trait.
    trait_ctx: Option<u32>,
    /// Inside a method of an impl of a trait: the trait and the impl's associated types.
    impl_assoc: Option<(TraitRef, Vec<Option<Ty>>)>,
}

impl Lowerer<'_> {
    /// What type names mean in this body; types left out are inferred.
    fn scope(&self) -> Scope<'_> {
        Scope {
            tree: self.items,
            params: self.params,
            preds: self.preds,
            self_ty: self.self_ty.clone(),
            trait_ctx: self.trait_ctx,
            impl_assoc: self.impl_assoc.clone(),
            holes: true,
        }
    }

    /// The types in a `GENERIC_ARGS` node, for something with `want` parameters.
    fn type_args(&mut self, node: &SyntaxNode, want: usize, what: &str) -> Vec<Ty> {
        let mut ds = Vec::new();
        let (types, assoc) = generic_args(node, &self.scope(), &mut ds);
        for (_, r, _) in assoc {
            ds.push(Diagnostic::error(
                Code::E0413,
                r,
                "associated types are fixed in bounds, not in a call",
            ));
        }
        let ok = crate::resolve::arity(what, want, types.len(), node.text_range(), &mut ds);
        self.out.diagnostics.extend(ds);
        if ok { types } else { vec![Ty::Error; want] }
    }

    fn alloc(&mut self, kind: ExprKind, range: TextRange) -> ExprId {
        self.out.body.exprs.alloc(Expr { kind, range })
    }

    fn alloc_pat(&mut self, kind: PatKind, range: TextRange) -> PatId {
        self.out.body.pats.alloc(Pat { kind, range })
    }

    fn error(&mut self, d: Diagnostic) {
        self.out.diagnostics.push(d);
    }

    fn report(&mut self, code: Code, range: TextRange, message: impl Into<String>) {
        self.error(Diagnostic::error(code, range, message));
    }

    fn unsupported(&mut self, range: TextRange, what: &str) -> ExprId {
        self.error(Diagnostic::unsupported(range, what));
        self.alloc(ExprKind::Missing, range)
    }

    fn missing(&mut self, range: TextRange) -> ExprId {
        self.alloc(ExprKind::Missing, range)
    }

    fn expr_or_missing(&mut self, e: Option<ast::Expr>, range: TextRange) -> ExprId {
        match e {
            Some(e) => self.expr(&e),
            None => self.missing(range),
        }
    }

    /// Whether a `// =>` expectation follows `end` on the same line (after an optional `;`).
    fn observed(&self, end: u32) -> bool {
        let rest = self.text[end as usize..].trim_start_matches([' ', '\t']);
        let rest = rest
            .strip_prefix(';')
            .unwrap_or(rest)
            .trim_start_matches([' ', '\t']);
        rest.starts_with("// =>")
    }

    fn block(&mut self, block: &ast::Block) -> ExprId {
        let node = block.syntax();
        if let Some(attr) = node.child(K::INNER_ATTR) {
            self.error(Diagnostic::unsupported(attr.text_range(), "an attribute"));
        }
        let mark = self.scopes.len();
        let mut stmts = Vec::new();
        for s in block.stmts() {
            match s {
                ast::Stmt::Let(l) => {
                    if let Some(stmt) = self.let_stmt(&l) {
                        stmts.push(stmt);
                    }
                }
                ast::Stmt::Expr(e) => {
                    if let Some(attr) = e.syntax().child(K::OUTER_ATTR) {
                        self.error(Diagnostic::unsupported(attr.text_range(), "an attribute"));
                    }
                    let semi = e.syntax().token(K::SEMI).is_some();
                    let Some(inner) = e.expr() else { continue };
                    let observe = self.observed(inner.syntax().text_range().end);
                    let expr = self.expr(&inner);
                    stmts.push(Stmt::Expr {
                        expr,
                        semi,
                        observe,
                    });
                }
                ast::Stmt::BlockLike(e) => {
                    let observe = self.observed(e.syntax().text_range().end);
                    let expr = self.expr(&e);
                    stmts.push(Stmt::Expr {
                        expr,
                        semi: false,
                        observe,
                    });
                }
                ast::Stmt::Item(n)
                    if n.kind() == K::DEFER_STMT && n.token(K::ERRDEFER_KW).is_none() =>
                {
                    let Some(b) = n.child(K::BLOCK).and_then(ast::Block::cast) else {
                        continue;
                    };
                    let outer = self.defer_floor.replace(self.loops.len());
                    let body = self.block(&b);
                    self.defer_floor = outer;
                    stmts.push(Stmt::Defer(body));
                }
                ast::Stmt::Item(n) => {
                    let what = if n.kind() == K::DEFER_STMT {
                        "`errdefer`"
                    } else {
                        "a local item"
                    };
                    self.error(Diagnostic::unsupported(n.text_range(), what));
                }
            }
        }
        let (tail, observe_tail) = match block.tail_expr() {
            Some(t) => {
                let observe = self.observed(t.syntax().text_range().end);
                (Some(self.expr(&t)), observe)
            }
            None => (None, false),
        };
        self.scopes.truncate(mark);
        self.alloc(
            ExprKind::Block {
                stmts,
                tail,
                observe_tail,
            },
            node.text_range(),
        )
    }

    fn let_stmt(&mut self, l: &ast::LetStmt) -> Option<Stmt> {
        let node = l.syntax();
        if let Some(attr) = node.child(K::OUTER_ATTR) {
            self.error(Diagnostic::unsupported(attr.text_range(), "an attribute"));
        }
        let ty = l.ty().map(|t| {
            let mut ds = Vec::new();
            let ty = resolve_type(&t, &self.scope(), &mut ds);
            self.out.diagnostics.extend(ds);
            ty
        });
        let init = match l.init() {
            Some(e) => self.expr(&e),
            None => self.unsupported(node.text_range(), "`let` without an initialiser"),
        };
        // The `else` block does not see the pattern's names.
        let else_ = node
            .child(K::LET_ELSE)
            .and_then(|e| e.child(K::BLOCK))
            .and_then(ast::Block::cast)
            .map(|b| self.block(&b));
        let p = l.pat()?;
        let mut binds = Binds::default();
        let pat = self.pat(&p, &mut binds);
        self.bring_into_scope(binds);
        Some(Stmt::Let {
            pat,
            ty,
            init,
            else_,
        })
    }

    fn bring_into_scope(&mut self, binds: Binds) {
        self.scopes
            .extend(binds.names.into_iter().map(|(n, l, _)| (n, l)));
    }

    fn lookup(&self, name: &str) -> Option<LocalId> {
        self.scopes
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .map(|&(_, id)| id)
    }

    /// The segments of the `PATH` in `node` (without generic arguments), or `None` after
    /// reporting a path this slice does not support.
    fn segments(&mut self, node: &SyntaxNode) -> Option<Vec<String>> {
        let segs = node.child(K::PATH).and_then(|p| path_segments(&p));
        match segs {
            Some(segs) if !segs.is_empty() => Some(segs.into_iter().map(|s| s.name).collect()),
            _ => {
                self.error(Diagnostic::unsupported(node.text_range(), "this path"));
                None
            }
        }
    }

    /// The type arguments of struct or enum `adt` written on a segment, or holes.
    fn adt_args(&mut self, adt: u32, args: Option<&SyntaxNode>, range: TextRange) -> Vec<Ty> {
        let mut ds = Vec::new();
        let args = adt_args(adt, args, range, &self.scope(), &mut ds);
        self.out.diagnostics.extend(ds);
        args
    }

    /// What the value path in `node` names, or `None` after reporting why it names nothing.
    fn resolve(&mut self, node: &SyntaxNode) -> Option<Resolved> {
        let range = node.text_range();
        let Some(segs) = node.child(K::PATH).and_then(|p| path_segments(&p)) else {
            self.error(Diagnostic::unsupported(range, "this path"));
            return None;
        };
        match segs.as_slice() {
            [seg] => {
                let name = seg.name.as_str();
                if let Some(local) = self.lookup(name) {
                    if seg.args.is_some() {
                        self.report(
                            Code::E0404,
                            range,
                            format!("`{name}` takes no type arguments"),
                        );
                    }
                    return Some(Resolved::Local(local));
                }
                if let Some(f) = self.items.fn_named(name) {
                    let sig = &self.items.fns[f as usize];
                    let want = sig.generics.params.len() - sig.generics.parent as usize;
                    let args = match &seg.args {
                        Some(a) => self.type_args(a, want, &format!("`{name}`")),
                        None => vec![Ty::Infer(Ty::HOLE); want],
                    };
                    return Some(Resolved::Fn(f, args));
                }
                if let Some((a, v)) = self.items.bare_variant(name) {
                    let args = self.adt_args(a, seg.args.as_ref(), range);
                    return Some(Resolved::Ctor(a, v, args));
                }
                if let Some(a) = self.items.adt_named(name) {
                    if !self.items.adts[a as usize].is_enum {
                        let args = self.adt_args(a, seg.args.as_ref(), range);
                        return Some(Resolved::Ctor(a, 0, args));
                    }
                    self.report(
                        Code::E0111,
                        range,
                        format!("`{name}` is an enum; name one of its variants: `{name}::…`"),
                    );
                    return None;
                }
                if name == "Self"
                    && let Some(Ty::Adt(a, _, args)) = self.self_ty.clone()
                    && !self.items.adts[a as usize].is_enum
                {
                    return Some(Resolved::Ctor(a, 0, args.to_vec()));
                }
                if name == "assert" {
                    self.report(
                        Code::E0112,
                        range,
                        "`assert` is a built-in, not a value: call it as `assert(condition)`",
                    );
                    return None;
                }
                self.report(
                    Code::E0101,
                    range,
                    format!("there is nothing called `{name}` here"),
                );
                None
            }
            [ty, item] => {
                let own_args = |cx: &mut Self| -> Option<Vec<Ty>> {
                    item.args.as_ref().map(|a| {
                        let mut ds = Vec::new();
                        let (types, _) = generic_args(a, &cx.scope(), &mut ds);
                        cx.out.diagnostics.extend(ds);
                        types
                    })
                };
                // `Trait::method`.
                if let Some(t) = self.items.trait_named(&ty.name) {
                    let def = &self.items.traits[t as usize];
                    let Some(&func) = def
                        .methods
                        .iter()
                        .find(|&&m| self.items.fns[m as usize].name == item.name)
                    else {
                        self.report(
                            Code::E0413,
                            item.range,
                            format!("`{}` has no method `{}`", ty.name, item.name),
                        );
                        return None;
                    };
                    let want = def.generics.params.len() - 1;
                    let trait_args = match &ty.args {
                        Some(a) => self.type_args(a, want, &format!("`{}`", ty.name)),
                        None => vec![Ty::Infer(Ty::HOLE); want],
                    };
                    let args = own_args(self);
                    return Some(Resolved::TraitFn(t, func, trait_args, args));
                }
                // The type: an enum's variant, or a function of the type.
                let base = match ty.name.as_str() {
                    "int" => Ty::Int,
                    "bool" => Ty::Bool,
                    "Self" => match &self.self_ty {
                        Some(t) => t.clone(),
                        None => {
                            self.report(
                                Code::E0102,
                                ty.range,
                                "`Self` is the type of an impl or trait, and there is none here",
                            );
                            return None;
                        }
                    },
                    name => {
                        if let Some(i) = self.params.iter().rposition(|p| p.name == name) {
                            Ty::Param(i as u32, name.into())
                        } else if let Some(a) = self.items.adt_named(name) {
                            let args = self.adt_args(a, ty.args.as_ref(), ty.range);
                            self.items.adts[a as usize].ty_with(a, args)
                        } else {
                            self.report(
                                Code::E0101,
                                ty.range,
                                format!("there is no type called `{name}`"),
                            );
                            return None;
                        }
                    }
                };
                if let Ty::Adt(a, _, args) = &base {
                    let def = &self.items.adts[*a as usize];
                    if def.is_enum
                        && let Some(v) = def.variant(&item.name)
                    {
                        if item.args.is_some() {
                            self.report(
                                Code::E0404,
                                item.range,
                                "a variant takes no type arguments: write them on the enum",
                            );
                        }
                        return Some(Resolved::Ctor(*a, v, args.to_vec()));
                    }
                }
                let args = own_args(self);
                Some(Resolved::AssocFn(base, item.name.clone(), item.range, args))
            }
            _ => {
                self.error(Diagnostic::unsupported(range, "this path"));
                None
            }
        }
    }

    /// The expression for a resolved path.
    fn resolved(&mut self, r: Resolved, range: TextRange) -> ExprId {
        match r {
            Resolved::Local(l) => self.alloc(ExprKind::Local(l), range),
            Resolved::Fn(func, args) => self.alloc(ExprKind::Fn { func, args }, range),
            Resolved::AssocFn(ty, name, name_range, args) => self.alloc(
                ExprKind::AssocFn {
                    ty,
                    name,
                    name_range,
                    args,
                },
                range,
            ),
            Resolved::TraitFn(trait_, func, trait_args, args) => self.alloc(
                ExprKind::TraitFn {
                    trait_,
                    func,
                    trait_args,
                    args,
                },
                range,
            ),
            Resolved::Ctor(adt, variant, args) => {
                let def = self.items.variant(adt, variant);
                match def.shape {
                    Shape::Unit => self.alloc(
                        ExprKind::Adt {
                            adt,
                            variant,
                            fields: Vec::new(),
                            args,
                        },
                        range,
                    ),
                    Shape::Tuple => self.unsupported(range, "a constructor used as a value"),
                    Shape::Named => {
                        let path = self.items.adts[adt as usize].path(variant);
                        self.report(
                            Code::E0111,
                            range,
                            format!("`{path}` has named fields: write `{path} {{ … }}`"),
                        );
                        self.missing(range)
                    }
                }
            }
        }
    }

    fn expr(&mut self, e: &ast::Expr) -> ExprId {
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
                if self.defer_floor.is_some() {
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
            _ => self.unsupported(range, "this expression"),
        }
    }

    fn literal(&mut self, node: &SyntaxNode, negate: bool) -> ExprId {
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
    fn struct_lit(&mut self, node: &SyntaxNode) -> ExprId {
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
    fn method_call(&mut self, node: &SyntaxNode, e: &ast::Expr) -> ExprId {
        let range = node.text_range();
        let receiver = e.sub_exprs().into_iter().next();
        let receiver = self.expr_or_missing(receiver, range);
        let generic_args = node.child(K::GENERIC_ARGS).map(|a| {
            let mut ds = Vec::new();
            let (types, _) = generic_args(&a, &self.scope(), &mut ds);
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
    fn args(&mut self, node: &SyntaxNode) -> Vec<ExprId> {
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
    fn field(&mut self, node: &SyntaxNode, e: &ast::Expr) -> ExprId {
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

    fn if_expr(&mut self, node: &SyntaxNode, e: &ast::Expr) -> ExprId {
        let range = node.text_range();
        if node.token(K::CONST_KW).is_some() {
            return self.unsupported(range, "`if const`");
        }
        let mut subs = e.sub_exprs().into_iter();
        // Names bound by `let` conditions are in scope in the condition after them and in the
        // first block, not in the `else` branch.
        let mark = self.scopes.len();
        let cond = self.expr_or_missing(subs.next(), range);
        let then = self.expr_or_missing(subs.next(), range);
        self.scopes.truncate(mark);
        let else_ = subs.next().map(|x| self.expr(&x));
        self.alloc(ExprKind::If { cond, then, else_ }, range)
    }

    /// `let p = e` in an `if` or `while` head. Its names stay in scope until the head's
    /// expression truncates the scopes.
    fn let_cond(&mut self, node: &SyntaxNode) -> ExprId {
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

    fn match_expr(&mut self, node: &SyntaxNode) -> ExprId {
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
            let mark = self.scopes.len();
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
            self.scopes.truncate(mark);
            arms.push(Arm { pat, guard, body });
        }
        self.alloc(ExprKind::Match { scrutinee, arms }, range)
    }

    /// `loop { … }` or `while cond { … }`, maybe labelled.
    fn loop_expr(&mut self, node: &SyntaxNode, label: Option<String>) -> ExprId {
        let range = node.text_range();
        let id = self.missing(range);
        self.loops.push((label, id));
        let mark = self.scopes.len();
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
        self.scopes.truncate(mark);
        self.loops.pop();
        self.out.body.exprs[id].kind = match cond {
            Some(cond) => ExprKind::While { cond, body },
            None => ExprKind::Loop { body },
        };
        id
    }

    /// `break` or `continue`, with an optional label (and a value for `break`).
    fn jump(&mut self, node: &SyntaxNode, e: &ast::Expr) -> ExprId {
        let range = node.text_range();
        let is_break = node.kind() == K::BREAK_EXPR;
        let value = if is_break {
            e.sub_exprs().first().map(|v| self.expr(v))
        } else {
            None
        };
        let label = node.token(K::LABEL);
        let target = match &label {
            Some(l) => {
                let found = self
                    .loops
                    .iter()
                    .rev()
                    .find(|(name, _)| name.as_deref() == Some(l.text()))
                    .map(|&(_, id)| id);
                if found.is_none() {
                    self.report(
                        Code::E0110,
                        l.text_range(),
                        format!("no enclosing loop is labelled `{}`", l.text()),
                    );
                }
                found
            }
            None => {
                let found = self.loops.last().map(|&(_, id)| id);
                if found.is_none() {
                    let what = if is_break { "break" } else { "continue" };
                    self.report(
                        Code::E0109,
                        range,
                        format!("`{what}` is only allowed inside a loop"),
                    );
                }
                found
            }
        };
        let Some(target) = target else {
            return self.missing(range);
        };
        if let Some(floor) = self.defer_floor
            && self.loops.iter().position(|&(_, id)| id == target) < Some(floor)
        {
            let what = if is_break { "break" } else { "continue" };
            self.report(
                Code::E0313,
                range,
                format!("a `defer` block cannot `{what}` out of itself"),
            );
            return self.missing(range);
        }
        let kind = if is_break {
            ExprKind::Break { target, value }
        } else {
            ExprKind::Continue { target }
        };
        self.alloc(kind, range)
    }

    fn prefix(&mut self, node: &SyntaxNode, e: &ast::Expr) -> ExprId {
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

    fn operands(&mut self, e: &ast::Expr, range: TextRange) -> (ExprId, ExprId) {
        let (l, r) = e.operands();
        let l = self.expr_or_missing(l, range);
        let r = self.expr_or_missing(r, range);
        (l, r)
    }

    fn binary(&mut self, node: &SyntaxNode, e: &ast::Expr) -> ExprId {
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

    fn compare(&mut self, node: &SyntaxNode, e: &ast::Expr) -> ExprId {
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

    fn assign(&mut self, node: &SyntaxNode, e: &ast::Expr) -> ExprId {
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

    fn call(&mut self, node: &SyntaxNode, e: &ast::Expr) -> ExprId {
        let range = node.text_range();
        let callee_node = e.sub_exprs().into_iter().next();
        if let Some(c @ ast::Expr::Path(_)) = &callee_node {
            let segments = c
                .syntax()
                .child(K::PATH)
                .and_then(|p| path_segments(&p))
                .unwrap_or_default();
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

    fn ctor_call(
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

    // Patterns.

    fn pat(&mut self, p: &ast::Pat, b: &mut Binds) -> PatId {
        let node = p.syntax().clone();
        let range = node.text_range();
        let kind = match p {
            ast::Pat::Wildcard(_) => PatKind::Wild,
            ast::Pat::Literal(_) => self.literal_pat(&node),
            ast::Pat::Range(_) => self.range_pat(&node),
            ast::Pat::Ident(_) => {
                let mutable = node.token(K::MUT_KW).is_some();
                let name = node.child(K::NAME).and_then(ast::Name::cast);
                let sub = node.children().find_map(ast::Pat::cast);
                let sub = sub.map(|s| self.pat(&s, b));
                match name {
                    Some(n) => {
                        let local = self.bind(&n.text(), mutable, n.syntax().text_range(), b);
                        PatKind::Bind { local, sub }
                    }
                    None => PatKind::Missing,
                }
            }
            ast::Pat::At(_) => {
                let sub = node.children().find_map(ast::Pat::cast);
                let sub = sub.map(|s| self.pat(&s, b));
                match self.segments(&node).as_deref() {
                    Some([name]) => {
                        let at = node.child(K::PATH).map_or(range, |n| n.text_range());
                        let local = self.bind(name, false, at, b);
                        PatKind::Bind { local, sub }
                    }
                    _ => PatKind::Missing,
                }
            }
            ast::Pat::Path(_) => self.path_pat(&node, b),
            ast::Pat::TupleStruct(_) => self.tuple_struct_pat(&node, b),
            ast::Pat::Struct(_) => self.struct_pat(&node, b),
            ast::Pat::Tuple(_) => {
                let subs: Vec<ast::Pat> = node.children().filter_map(ast::Pat::cast).collect();
                if subs.is_empty() {
                    PatKind::Unit
                } else if subs.len() == 1
                    && node.token(K::COMMA).is_none()
                    && !matches!(subs[0], ast::Pat::Rest(_))
                {
                    // `(p)` is `p` in parentheses.
                    return self.pat(&subs[0], b);
                } else {
                    let (pats, rest) = self.with_rest(&subs, b);
                    PatKind::Tuple { pats, rest }
                }
            }
            ast::Pat::Or(_) => self.or_pat(&node, b),
            ast::Pat::Rest(_) => {
                self.report(
                    Code::E0001,
                    range,
                    "`..` is only allowed among the fields of a tuple or struct pattern",
                );
                PatKind::Missing
            }
            ast::Pat::Ref(_) => {
                let mutable = node.token(K::MUT_KW).is_some();
                match node.children().find_map(ast::Pat::cast) {
                    Some(inner) => PatKind::Ref {
                        mutable,
                        pat: self.pat(&inner, b),
                    },
                    None => PatKind::Missing,
                }
            }
            ast::Pat::Typed(_) => {
                self.error(Diagnostic::unsupported(range, "a refinement test"));
                PatKind::Missing
            }
            _ => {
                self.error(Diagnostic::unsupported(range, "this pattern"));
                PatKind::Missing
            }
        };
        self.alloc_pat(kind, range)
    }

    /// A new local for `name`, or the one the first alternative of an or-pattern bound.
    fn bind(&mut self, name: &str, mutable: bool, range: TextRange, b: &mut Binds) -> LocalId {
        if let Some(&(_, _, first)) = b.names.iter().find(|(n, _, _)| n == name) {
            let d = Diagnostic::error(
                Code::E0106,
                range,
                format!("`{name}` is bound more than once in this pattern"),
            )
            .with_label(first, "first bound here");
            self.error(d);
        }
        let local = match b.reuse.iter().find(|(n, _)| n == name) {
            Some(&(_, l)) => l,
            None => self.out.body.locals.alloc(Local {
                name: name.to_string(),
                mutable,
                range,
            }),
        };
        b.names.push((name.to_string(), local, range));
        local
    }

    /// The patterns among `subs`, and where `..` is among them.
    fn with_rest(&mut self, subs: &[ast::Pat], b: &mut Binds) -> (Vec<PatId>, Option<usize>) {
        let mut pats = Vec::new();
        let mut rest = None;
        for s in subs {
            if let ast::Pat::Rest(r) = s {
                if rest.is_some() {
                    self.report(
                        Code::E0001,
                        r.text_range(),
                        "`..` may appear only once in a pattern",
                    );
                }
                rest = Some(pats.len());
            } else {
                pats.push(self.pat(s, b));
            }
        }
        (pats, rest)
    }

    fn literal_pat(&mut self, node: &SyntaxNode) -> PatKind {
        let range = node.text_range();
        let negate = node.token(K::MINUS).is_some();
        let Some(tok) = node
            .child_tokens()
            .find(|t| !t.kind().is_trivia() && t.kind() != K::MINUS)
        else {
            return PatKind::Missing;
        };
        match tok.kind() {
            K::TRUE_KW => PatKind::Bool(true),
            K::FALSE_KW => PatKind::Bool(false),
            K::INTEGER => match ast::int_value(tok.text()) {
                Some(v) => PatKind::Int(if negate { -v } else { v }),
                None => {
                    self.report(
                        Code::E0204,
                        range,
                        format!("`{}` is too large for any integer type", tok.text()),
                    );
                    PatKind::Missing
                }
            },
            _ => {
                self.error(Diagnostic::unsupported(range, "this kind of literal"));
                PatKind::Missing
            }
        }
    }

    fn range_pat(&mut self, node: &SyntaxNode) -> PatKind {
        let range = node.text_range();
        let bounds: Vec<SyntaxNode> = node
            .children()
            .filter(|c| c.kind() == K::LITERAL_PAT)
            .collect();
        let inclusive = node.token(K::DOT_DOT_EQ).is_some();
        let mut values = Vec::new();
        for b in &bounds {
            match self.literal_pat(b) {
                PatKind::Int(v) => values.push(v),
                PatKind::Missing => return PatKind::Missing,
                _ => {
                    self.report(
                        Code::E0201,
                        b.text_range(),
                        "the bounds of a range pattern are integers",
                    );
                    return PatKind::Missing;
                }
            }
        }
        match values.as_slice() {
            [lo] => PatKind::Range {
                lo: *lo,
                hi: None,
                inclusive,
            },
            [lo, hi] => PatKind::Range {
                lo: *lo,
                hi: Some(*hi),
                inclusive,
            },
            _ => {
                self.error(Diagnostic::unsupported(range, "this range pattern"));
                PatKind::Missing
            }
        }
    }

    /// `x` (a binding), a unit struct, or a unit variant `E::A`.
    fn path_pat(&mut self, node: &SyntaxNode, b: &mut Binds) -> PatKind {
        let range = node.text_range();
        let Some(segments) = self.segments(node) else {
            return PatKind::Missing;
        };
        if let [name] = segments.as_slice()
            && self.items.adt_named(name).is_none()
            && self.items.bare_variant(name).is_none()
            && name != "Self"
        {
            let local = self.bind(name, false, range, b);
            return PatKind::Bind { local, sub: None };
        }
        let Some((adt, variant)) = self.ctor_pat_target(node) else {
            return PatKind::Missing;
        };
        let def = self.items.variant(adt, variant);
        if def.shape != Shape::Unit {
            let path = self.items.adts[adt as usize].path(variant);
            let form = if def.shape == Shape::Tuple {
                format!("{path}(…)")
            } else {
                format!("{path} {{ … }}")
            };
            self.report(
                Code::E0111,
                range,
                format!("`{path}` has fields: match it as `{form}`"),
            );
            return PatKind::Missing;
        }
        PatKind::Adt {
            adt,
            variant,
            fields: Vec::new(),
        }
    }

    /// The struct or variant a pattern's path names.
    fn ctor_pat_target(&mut self, node: &SyntaxNode) -> Option<(u32, u32)> {
        let range = node.text_range();
        let segments = self.segments(node)?;
        if let [name] = segments.as_slice()
            && let Some(found) = self.items.bare_variant(name)
        {
            return Some(found);
        }
        let self_adt = match &self.self_ty {
            Some(Ty::Adt(a, ..)) => Some(*a),
            _ => None,
        };
        let adt_named = |cx: &Self, name: &str| {
            if name == "Self" {
                self_adt
            } else {
                cx.items.adt_named(name)
            }
        };
        match segments.as_slice() {
            [name] => match adt_named(self, name) {
                Some(a) if !self.items.adts[a as usize].is_enum => Some((a, 0)),
                Some(_) => {
                    self.report(
                        Code::E0111,
                        range,
                        format!("`{name}` is an enum; match one of its variants: `{name}::…`"),
                    );
                    None
                }
                None => {
                    self.report(
                        Code::E0101,
                        range,
                        format!("there is no struct called `{name}`"),
                    );
                    None
                }
            },
            [ty, variant] => {
                let Some(a) = adt_named(self, ty) else {
                    self.report(
                        Code::E0101,
                        range,
                        format!("there is no enum called `{ty}`"),
                    );
                    return None;
                };
                let adt = &self.items.adts[a as usize];
                match adt.variant(variant).filter(|_| adt.is_enum) {
                    Some(v) => Some((a, v)),
                    None => {
                        self.report(
                            Code::E0101,
                            range,
                            format!("`{ty}` has no variant called `{variant}`"),
                        );
                        None
                    }
                }
            }
            _ => {
                self.error(Diagnostic::unsupported(range, "this path"));
                None
            }
        }
    }

    /// `Pair(a, b)` or `Shape::Circle(r)`, maybe with `..`.
    fn tuple_struct_pat(&mut self, node: &SyntaxNode, b: &mut Binds) -> PatKind {
        let range = node.text_range();
        let target = self.ctor_pat_target(node);
        let subs: Vec<ast::Pat> = node.children().filter_map(ast::Pat::cast).collect();
        let (pats, rest) = self.with_rest(&subs, b);
        let Some((adt, variant)) = target else {
            return PatKind::Missing;
        };
        let def = self.items.variant(adt, variant);
        let path = self.items.adts[adt as usize].path(variant);
        if def.shape != Shape::Tuple {
            self.report(
                Code::E0111,
                range,
                format!("`{path}` has no positional fields"),
            );
            return PatKind::Missing;
        }
        let n = def.fields.len();
        let fits = match rest {
            Some(_) => pats.len() <= n,
            None => pats.len() == n,
        };
        if !fits {
            self.report(
                Code::E0213,
                range,
                format!(
                    "`{path}` has {n} field{}, but this pattern has {}",
                    if n == 1 { "" } else { "s" },
                    pats.len()
                ),
            );
            return PatKind::Missing;
        }
        let fields = match rest {
            None => pats
                .into_iter()
                .enumerate()
                .map(|(i, p)| (i as u32, p))
                .collect(),
            Some(at) => {
                let after = pats.len() - at;
                pats.into_iter()
                    .enumerate()
                    .map(|(i, p)| {
                        let index = if i < at { i } else { n - after + (i - at) };
                        (index as u32, p)
                    })
                    .collect()
            }
        };
        PatKind::Adt {
            adt,
            variant,
            fields,
        }
    }

    /// `Point { x, y: 0, .. }`.
    fn struct_pat(&mut self, node: &SyntaxNode, b: &mut Binds) -> PatKind {
        let range = node.text_range();
        let target = self.ctor_pat_target(node);
        let target = target.filter(|&(adt, variant)| {
            let def = self.items.variant(adt, variant);
            if def.shape == Shape::Named {
                return true;
            }
            let path = self.items.adts[adt as usize].path(variant);
            self.report(Code::E0111, range, format!("`{path}` has no named fields"));
            false
        });
        let mut fields: Vec<(u32, PatId)> = Vec::new();
        let mut given: Vec<(u32, TextRange)> = Vec::new();
        let mut rest = false;
        for f in node.children().filter(|c| c.kind() == K::FIELD_PAT) {
            if f.token(K::DOT_DOT).is_some() {
                rest = true;
                continue;
            }
            let Some(name_tok) = ast::member_name(&f) else {
                continue;
            };
            let name = name_tok.text().to_string();
            let pat = match f.children().find_map(ast::Pat::cast) {
                Some(p) => self.pat(&p, b),
                None => {
                    let local = self.bind(&name, false, name_tok.text_range(), b);
                    self.alloc_pat(PatKind::Bind { local, sub: None }, f.text_range())
                }
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
                            format!("the field `{name}` is matched twice"),
                        )
                        .with_label(first, "first matched here");
                        self.error(d);
                    } else {
                        given.push((i, name_tok.text_range()));
                        fields.push((i, pat));
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
            return PatKind::Missing;
        };
        let def = self.items.variant(adt, variant);
        let missing: Vec<String> = (0..def.fields.len() as u32)
            .filter(|i| !given.iter().any(|(g, _)| g == i))
            .map(|i| format!("`{}`", def.fields[i as usize].name))
            .collect();
        if !rest && !missing.is_empty() {
            let path = self.items.adts[adt as usize].path(variant);
            self.report(
                Code::E0207,
                range,
                format!(
                    "this pattern leaves out {} of `{path}`; match them or write `..`",
                    missing.join(", ")
                ),
            );
        }
        PatKind::Adt {
            adt,
            variant,
            fields,
        }
    }

    /// `a | b`: every alternative binds the same names, to the same locals.
    fn or_pat(&mut self, node: &SyntaxNode, b: &mut Binds) -> PatKind {
        let before = b.names.len();
        let saved = b.reuse.clone();
        let mut first: Option<Vec<(String, LocalId, TextRange)>> = None;
        let mut alts = Vec::new();
        for alt in node.children().filter_map(ast::Pat::cast) {
            b.names.truncate(before);
            if let Some(f) = &first {
                b.reuse = saved.clone();
                b.reuse.extend(f.iter().map(|(n, l, _)| (n.clone(), *l)));
            }
            let id = self.pat(&alt, b);
            let bound: Vec<(String, LocalId, TextRange)> = b.names[before..].to_vec();
            match &first {
                None => first = Some(bound),
                Some(f) => {
                    let alt_range = alt.syntax().text_range();
                    for (n, _, _) in f {
                        if !bound.iter().any(|(m, _, _)| m == n) {
                            self.report(
                                Code::E0105,
                                alt_range,
                                format!("`{n}` is bound in the first alternative but not this one"),
                            );
                        }
                    }
                    for (n, _, r) in &bound {
                        if !f.iter().any(|(m, _, _)| m == n) {
                            self.report(
                                Code::E0105,
                                *r,
                                format!("`{n}` is not bound in the first alternative"),
                            );
                        }
                    }
                }
            }
            alts.push(id);
        }
        b.names.truncate(before);
        b.names.extend(first.unwrap_or_default());
        b.reuse = saved;
        PatKind::Or(alts)
    }
}

fn compare_op(tok: &SyntaxToken) -> Option<CompareOp> {
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

fn binary_op(tok: &SyntaxToken) -> Option<BinaryOp> {
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

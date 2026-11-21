//! Lowering a body from the syntax tree to HIR: the `body` query.
//!
//! [`Lowerer`] holds two components: [`names::Names`] (the locals, loops and `defer` blocks in
//! scope) and a [`Scope`] (what the names in types refer to). The lowering is split by topic:
//! [`paths`] (what a value path names), [`exprs`] and [`patterns`].

use moddil_syntax::ast::{self, AstNode};
use moddil_syntax::{SyntaxKind as K, SyntaxNode, TextRange};

use crate::collect::{item_tree, node_at, source};
use crate::hir::{Body, Expr, ExprId, ExprKind, Local, LocalId, Pat, PatId, PatKind, Stmt};
use crate::items::{Container, Src};
use crate::resolve::{Scope, generic_args, resolve_type};
use crate::{Code, Db, Diagnostic, Item, ItemTree, Owner, Ty};

mod exprs;
mod names;
mod paths;
mod patterns;

use names::Names;

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
        names: Names::default(),
        types: Scope {
            holes: true,
            ..Scope::new(items)
        },
    };
    let block = match owner.item(db) {
        Item::Fn(i) => {
            let sig = &items.fns[i as usize];
            let node = node_at(&root, K::FN_ITEM, sig.node).and_then(ast::FnItem::cast);
            cx.types.params = &sig.generics.params;
            cx.types.preds = &sig.generics.preds;
            match sig.container {
                Container::Impl(k) => {
                    let imp = &items.impls[k as usize];
                    cx.types.self_ty = Some(imp.self_ty.clone());
                    cx.types.impl_assoc = imp.trait_ref.clone().map(|t| (t, imp.assoc.clone()));
                }
                Container::Trait(t) => {
                    cx.types.self_ty = Some(Ty::Param(0, "Self".into()));
                    cx.types.trait_ctx = Some(t);
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
                    cx.names.bind(name.clone(), local);
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
            cx.names.bind_all(binds.into_names());
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

impl Binds {
    /// The names bound, with their locals, in order.
    fn into_names(self) -> impl Iterator<Item = (String, LocalId)> {
        self.names.into_iter().map(|(n, l, _)| (n, l))
    }
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
    /// The locals, loops and `defer` blocks in scope.
    names: Names,
    /// What the names in types refer to; types left out are inferred.
    types: Scope<'a>,
}

impl Lowerer<'_> {
    /// The types in a `GENERIC_ARGS` node, for something with `want` parameters.
    fn type_args(&mut self, node: &SyntaxNode, want: usize, what: &str) -> Vec<Ty> {
        let mut ds = Vec::new();
        let (types, assoc) = generic_args(node, &self.types, &mut ds);
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
        let mark = self.names.mark();
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
                ast::Stmt::Item(n) if n.kind() == K::DEFER_STMT => {
                    let Some(b) = n.child(K::BLOCK).and_then(ast::Block::cast) else {
                        self.error(Diagnostic::unsupported(
                            n.text_range(),
                            "a `defer` without a block",
                        ));
                        continue;
                    };
                    let outer = self.names.enter_defer();
                    let body = self.block(&b);
                    self.names.exit_defer(outer);
                    stmts.push(if n.token(K::ERRDEFER_KW).is_some() {
                        Stmt::ErrDefer(body)
                    } else {
                        Stmt::Defer(body)
                    });
                }
                ast::Stmt::Item(n) => {
                    self.error(Diagnostic::unsupported(n.text_range(), "a local item"));
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
        self.names.restore(mark);
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
            let ty = resolve_type(&t, &self.types, &mut ds);
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
        self.names.bind_all(binds.into_names());
    }

    fn lookup(&self, name: &str) -> Option<LocalId> {
        self.names.lookup(name)
    }

    // Patterns.
}

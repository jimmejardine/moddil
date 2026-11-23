//! Lowering a body from the syntax tree to HIR: the `body` query.
//!
//! [`Lowerer`] holds two components: [`names::Names`] (the locals, loops and `defer` blocks in
//! scope) and a [`Scope`] (what the names in types refer to). The lowering is split by topic:
//! [`paths`] (what a value path names), [`exprs`] and [`patterns`].

use moddil_syntax::ast::{self, AstNode};
use moddil_syntax::{SyntaxKind as K, SyntaxNode, TextRange};

use crate::collect::{item_tree, node_at, source};
use crate::hir::{
    Body, Expr, ExprId, ExprKind, FmtPiece, Local, LocalId, Pat, PatId, PatKind, Stmt,
};
use crate::items::{Container, Def, Src};
use crate::resolve::{Scope, generic_args, resolve_type};
use crate::{Code, Db, Diagnostic, Item, ItemTree, Owner, Ty};

mod exprs;
mod fold;
mod names;
mod paths;
mod patterns;
mod selectable;
mod templates;
pub(crate) mod text;
mod wrappers;

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
    let program = owner.program(db);
    let items = item_tree(db, program);
    let (src, module) = match owner.item(db) {
        Item::Fn(i) => (items.fns[i as usize].src, items.fns[i as usize].module),
        Item::Closure(..) => unreachable!("a closure's body is its function's or test's"),
        Item::Test(t) => {
            let t = &items.tests[t as usize];
            (Src::File(t.file), t.module)
        }
    };
    let (root, text) = source(db, program, src);
    let mut cx = Lowerer {
        text,
        items,
        out: Lowered::default(),
        names: Names::default(),
        closure: None,
        olds: None,
        types: Scope {
            holes: true,
            module,
            prelude_first: src == Src::Derived,
            ..Scope::new(items)
        },
        template: None,
        reflected: Vec::new(),
        unsafe_depth: 0,
        received: None,
    };
    let block = match owner.item(db) {
        Item::Fn(i) => {
            let sig = &items.fns[i as usize];
            let node = node_at(&root, K::FN_ITEM, sig.node).and_then(ast::FnItem::cast);
            cx.types.params = &sig.generics.params;
            cx.types.preds = &sig.generics.preds;
            cx.template = sig.template.as_ref();
            cx.types.template = sig.template.as_ref().map(|t| (t.var.as_str(), &t.ty));
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
                    closure: None,
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
            for (k, e) in sig.effects.iter().enumerate() {
                if let Some(name) = &e.name {
                    let local = cx.out.body.locals.alloc(Local {
                        name: name.clone(),
                        mutable: false,
                        range: e.range,
                        closure: None,
                    });
                    cx.out.body.effect_locals.push((local, k));
                    cx.names.bind(name.clone(), local);
                }
            }
            // A refinement's functions (`refine.new.1`): `holds` is `(p, self)`, and `new` is
            // `{ let (ok, v) = holds(self); if ok { v as X } else { fail RefinementError { … } } }`.
            if let Some((t, is_new)) = sig.refine {
                let range = sig.name_range;
                let this = cx.out.body.params[0];
                let id = if is_new {
                    let def = &items.types[t as usize];
                    let r = def.refinement.as_ref().expect("a refinement");
                    let holds = cx.alloc(
                        ExprKind::Fn {
                            func: r.holds,
                            args: Vec::new(),
                        },
                        range,
                    );
                    let arg = cx.alloc(ExprKind::Local(this), range);
                    let call = cx.alloc(
                        ExprKind::Call {
                            callee: holds,
                            args: vec![arg],
                        },
                        range,
                    );
                    let hidden = |cx: &mut Lowerer<'_>, name: &str| {
                        cx.out.body.locals.alloc(Local {
                            name: name.to_string(),
                            mutable: false,
                            range,
                            closure: None,
                        })
                    };
                    let ok = hidden(&mut cx, "{ok}");
                    let value = hidden(&mut cx, "{value}");
                    let ok_pat = cx.alloc_pat(
                        PatKind::Bind {
                            local: ok,
                            sub: None,
                        },
                        range,
                    );
                    let value_pat = cx.alloc_pat(
                        PatKind::Bind {
                            local: value,
                            sub: None,
                        },
                        range,
                    );
                    let pair = cx.alloc_pat(
                        PatKind::Tuple {
                            pats: vec![ok_pat, value_pat],
                            rest: None,
                        },
                        range,
                    );
                    let cond = cx.alloc(ExprKind::Local(ok), range);
                    let v = cx.alloc(ExprKind::Local(value), range);
                    let then = cx.alloc(ExprKind::Refine(v, t), range);
                    let name =
                        cx.alloc(ExprKind::Fmt(vec![FmtPiece::Lit(def.name.clone())]), range);
                    let clause =
                        cx.alloc(ExprKind::Fmt(vec![FmtPiece::Lit(r.clause.clone())]), range);
                    let shown_v = cx.alloc(ExprKind::Local(value), range);
                    let shown = cx.alloc(ExprKind::Fmt(vec![FmtPiece::Debug(shown_v)]), range);
                    let else_ = match items.lang("RefinementError") {
                        Some(adt) => {
                            let err = cx.alloc(
                                ExprKind::Adt {
                                    adt,
                                    variant: 0,
                                    fields: vec![(0, name), (1, clause), (2, shown)],
                                    args: Vec::new(),
                                },
                                range,
                            );
                            cx.alloc(ExprKind::Fail(err), range)
                        }
                        None => cx.missing(range),
                    };
                    let test = cx.alloc(
                        ExprKind::If {
                            cond,
                            then,
                            else_: Some(else_),
                        },
                        range,
                    );
                    cx.alloc(
                        ExprKind::Block {
                            stmts: vec![Stmt::Let {
                                pat: pair,
                                ty: None,
                                init: call,
                                else_: None,
                            }],
                            tail: Some(test),
                            observe_tail: false,
                        },
                        range,
                    )
                } else {
                    let pred = node_at(&root, K::TYPE_ITEM, sig.node)
                        .and_then(|n| n.children().find_map(ast::Expr::cast));
                    let pred = match pred {
                        Some(e) => cx.expr(&e),
                        None => cx.missing(range),
                    };
                    let back = cx.alloc(ExprKind::Local(this), range);
                    cx.alloc(ExprKind::Tuple(vec![pred, back]), range)
                };
                cx.out.body.root = Some(id);
                return cx.out;
            }
            // A const item's body is its initialiser.
            if sig.const_item {
                let init = if sig.default_arg {
                    root.descendants()
                        .into_iter()
                        .find(|n| n.text_range() == sig.node && ast::Expr::can_cast(n.kind()))
                        .and_then(ast::Expr::cast)
                } else {
                    node_at(&root, K::CONST_ITEM, sig.node)
                        .and_then(|n| n.children().find_map(ast::Expr::cast))
                };
                if let Some(e) = init {
                    let id = cx.expr(&e);
                    cx.out.body.root = Some(id);
                }
                return cx.out;
            }
            // Contracts (`contract.1`), on a function that is not a trait's.
            if let Some(f) = &node
                && !matches!(sig.container, Container::Trait(_))
            {
                cx.contracts(f.syntax(), &sig.ret);
            }
            // Wrappers (`attr.around.1`), on a free function that is not `const`, with plain
            // parameters.
            if let Some(f) = &node
                && matches!(sig.container, Container::Free)
                && !sig.is_const
                && sig.src.is_file()
                && crate::wrap::params(f.syntax()).is_some()
            {
                let attrs = crate::wrap::around_attrs(f.syntax());
                if !attrs.is_empty() {
                    let id = cx.wrapped(sig, &attrs, f.body().as_ref());
                    cx.out.body.root = Some(id);
                    return cx.out;
                }
            }
            node.and_then(|f| f.body())
        }
        Item::Closure(..) => None,
        Item::Test(i) => node_at(&root, K::TEST_ITEM, items.tests[i as usize].node)
            .and_then(ast::TestItem::cast)
            .and_then(|t| t.body()),
    };
    if let Some(block) = block {
        let id = cx.block(&block);
        cx.out.body.root = Some(id);
        // A selectable function's parts (`task.select.4`).
        if let Item::Fn(i) = owner.item(db)
            && items.fns[i as usize].selectable
            && items.fns[i as usize].control & crate::items::SUSPEND != 0
        {
            cx.selectable_parts(&items.fns[i as usize], &block);
        }
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
    /// The innermost closure being lowered, if any.
    closure: Option<ExprId>,
    /// While a postcondition is lowered: the `old(e)` values it names so far.
    olds: Option<Vec<(LocalId, ExprId)>>,
    /// What the names in types refer to; types left out are inferred.
    types: Scope<'a>,
    /// In a method instantiated from a derive's template: what the template's parameter is.
    template: Option<&'a crate::items::Template>,
    /// The reflected fields `inline for` names in scope, innermost last: each name and field.
    reflected: Vec<(String, u32)>,
    /// How many `unsafe` blocks the expression being lowered is in (`expr.unsafe.1`).
    unsafe_depth: u32,
    /// While a selectable function's rest is lowered: where its receive is, and the local that
    /// holds what was received in its place (`task.select.4`).
    received: Option<(TextRange, LocalId)>,
}

impl Lowerer<'_> {
    /// What the item name `name` means in the body's module.
    fn item(&self, name: &str) -> Option<Def> {
        self.items
            .lookup(self.types.module, name, self.types.prelude_first)
    }

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
        if let Some(attr) = node
            .children()
            .find(|c| c.kind() == K::INNER_ATTR && !crate::is_doc(c))
        {
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
                    if let Some(attr) = e
                        .syntax()
                        .children()
                        .find(|c| c.kind() == K::OUTER_ATTR && !crate::is_doc(c))
                    {
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
        if let Some(attr) = node
            .children()
            .find(|c| c.kind() == K::OUTER_ATTR && !crate::is_doc(c))
        {
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

impl Lowerer<'_> {
    /// The contracts of the function `node`: its `requires` conditions, and its `ensures`
    /// conditions, in which `result` is the value returned and `old(e)` is `e` on entry.
    fn contracts(&mut self, node: &SyntaxNode, ret: &Ty) {
        let _ = ret;
        for c in node.children().filter(|c| c.kind() == K::CONTRACT) {
            let Some(e) = c.children().find_map(ast::Expr::cast) else {
                continue;
            };
            if c.token(K::REQUIRES_KW).is_some() {
                let id = self.expr(&e);
                self.out.body.requires.push(id);
                continue;
            }
            let result = match self.out.body.result {
                Some(r) => r,
                None => {
                    let r = self.out.body.locals.alloc(Local {
                        name: "result".to_string(),
                        mutable: false,
                        range: c.text_range(),
                        closure: None,
                    });
                    self.out.body.result = Some(r);
                    r
                }
            };
            let mark = self.names.mark();
            self.names.bind("result".to_string(), result);
            self.olds = Some(Vec::new());
            let id = self.expr(&e);
            let olds = self.olds.take().unwrap_or_default();
            self.out.body.olds.extend(olds);
            self.names.restore(mark);
            self.out.body.ensures.push(id);
        }
    }
}

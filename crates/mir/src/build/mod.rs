//! Building MIR from the typed HIR: the `mir` query.
//!
//! [`Builder`] lowers one body. It is made of two components with their own state:
//! - [`cfg::Cfg`]: the blocks and locals under construction and the block being filled;
//! - [`scopes::Scopes`]: the open scopes and loops, and what leaving them undoes.
//!
//! The lowering itself is split by topic: [`places`] (places and operands), [`exprs`]
//! (expressions and statements), [`patterns`] and [`arith`] (operators and comparisons).

use std::collections::HashMap;

use moddil_sema::hir::{self, ExprId};
use moddil_sema::{Container, Db, Inference, Item, ItemTree, Owner, Solver, Src, Ty};
use moddil_syntax::TextRange;

use crate::{Body, Const, Local, LocalDecl, Operand, Place, Rvalue, Statement, Terminator};

mod arith;
mod cfg;
mod errors;
mod exprs;
mod patterns;
mod places;
mod scopes;

use cfg::Cfg;
use errors::{OnError, Target};
use scopes::{ScopeItem, Scopes};

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
    let fx = moddil_sema::effects(db, file);
    // The function's effects, which its callers pass as hidden parameters after the others.
    let needs: Vec<Ty> = match owner.item(db) {
        Item::Fn(i) => fx.fns[i as usize].needs.clone(),
        Item::Test(_) => Vec::new(),
    };
    let (name, ret, src, preds) = match owner.item(db) {
        Item::Fn(i) => {
            let sig = &items.fns[i as usize];
            (
                fn_name(items, i),
                sig.ret.clone(),
                sig.src,
                sig.generics.preds.as_slice(),
            )
        }
        Item::Test(i) => (
            format!("{:?}", items.tests[i as usize].name),
            Ty::Unit,
            Src::File,
            &[][..],
        ),
    };
    // In a destructor, `self` is not dropped again at the end: only its fields are.
    let glue_only = match owner.item(db) {
        Item::Fn(i) => match items.fns[i as usize].container {
            Container::Impl(k) => {
                let drop = items.lang("Drop");
                drop.is_some()
                    && items.impls[k as usize].trait_ref.as_ref().map(|t| t.trait_) == drop
            }
            _ => false,
        },
        Item::Test(_) => false,
    };
    let mut b = Builder {
        hir,
        types,
        items,
        solver: Solver::new(items, preds),
        glue_only: glue_only.then_some(Local(1)),
        cfg: Cfg::new(Body {
            name,
            src,
            locals: vec![LocalDecl {
                ty: ret,
                name: None,
            }],
            arg_count: hir.params.len() + needs.len(),
            blocks: Vec::new(),
        }),
        scopes: Scopes::new(),
        locals: vec![None; hir.locals.len()],
        recording: HashMap::new(),
        compare_expr: ExprId::from_raw(la_arena::RawIdx::from_u32(0)),
        targets: vec![Target::Out],
        on_error: HashMap::new(),
        binds: None,
        fx,
        evidence: Vec::new(),
        installed: HashMap::new(),
        uncoerced: None,
    };
    for &p in &hir.params {
        b.local(p);
    }
    // The handlers of the function's effects, each named as its signature names it, if it does.
    for t in &needs {
        let named = match owner.item(db) {
            Item::Fn(i) => hir
                .effect_locals
                .iter()
                .find(|&&(_, k)| items.fns[i as usize].effects.get(k).map(|e| &e.ty) == Some(t)),
            Item::Test(_) => None,
        };
        let name = named.map(|&(l, _)| hir.locals[l].name.clone());
        let local = b.cfg.new_local(Ty::reference(false, t.clone()), name);
        if let Some(&(l, _)) = named {
            b.locals[idx(l)] = Some(local);
        }
        b.evidence.push(local);
    }
    for &(local, pat) in &hir.param_pats {
        let place = Place::local(b.local(local));
        let fail = b.cfg.new_block();
        b.test_pat(pat, &place, fail);
    }
    if let Some(root) = hir.root {
        if let Item::Test(_) = owner.item(db) {
            // A test's value is observed, not returned.
            let t = b.temp(types.expr(root));
            b.eval_into(t, root);
            b.cfg
                .assign(Local::RETURN, Rvalue::Use(Operand::Const(Const::Unit)));
        } else {
            b.eval_into(Local::RETURN, root);
        }
    }
    b.leave_scopes(0);
    b.cfg.terminate(Terminator::Return);
    Some(b.cfg.finish())
}

fn idx<T>(id: la_arena::Idx<T>) -> usize {
    id.into_raw().into_u32() as usize
}

struct Builder<'a> {
    hir: &'a hir::Body,
    types: &'a Inference,
    items: &'a ItemTree,
    /// For questions about types: which are `Copy`, which need dropping.
    solver: Solver<'a>,
    /// In a destructor: its `self`, of which only the fields are dropped.
    glue_only: Option<Local>,
    cfg: Cfg,
    scopes: Scopes,
    /// The MIR local of each HIR local, once declared.
    locals: Vec<Option<Local>>,
    /// While an `assert`'s condition is built: for each of its parts, the locals holding its
    /// value and whether it was evaluated.
    recording: HashMap<ExprId, (Local, Local)>,
    /// The comparison chain being built.
    compare_expr: ExprId,
    /// Where an error raised now goes, innermost last.
    targets: Vec<Target>,
    /// For a call marked with `?` or `?!`: what happens to its error.
    on_error: HashMap<ExprId, OnError>,
    /// While a pattern is tested with [`Builder::match_pat`]: the bindings to make once it
    /// matches.
    binds: Option<Vec<patterns::Binding>>,
    /// Every function's effects and errors.
    fx: &'a moddil_sema::Effects,
    /// The hidden parameters holding the function's own handlers, in the order of its needs.
    evidence: Vec<Local>,
    /// The locals holding the handlers each `with` installs: by the `with` and the handler's
    /// index.
    installed: HashMap<(ExprId, usize), Local>,
    /// The expression being evaluated before its conversion to a `dyn` borrow, which is not
    /// converted again.
    uncoerced: Option<ExprId>,
}

impl Builder<'_> {
    /// The `dyn` borrow type `e`'s value is converted to, if it is.
    fn coercion(&self, e: ExprId) -> Option<Ty> {
        if self.uncoerced == Some(e) {
            return None;
        }
        self.types.dyn_coercion(e).cloned()
    }

    /// The type of the value `e` gives where it is used: its own, or the `dyn` borrow it is
    /// converted to.
    fn value_ty(&self, e: ExprId) -> Ty {
        self.coercion(e).unwrap_or_else(|| self.types.expr(e))
    }
}

impl Builder<'_> {
    /// A new temporary of type `ty`, dropped at the end of its statement if it needs it.
    fn temp(&mut self, ty: Ty) -> Local {
        let drop = self.needs_drop(&ty);
        let t = self.cfg.new_local(ty, None);
        if drop {
            self.scopes.push(ScopeItem::Temp(t));
        }
        t
    }

    fn needs_drop(&mut self, ty: &Ty) -> bool {
        self.solver.needs_drop(ty)
    }

    /// Drops the value at `place`, if its type may need it.
    fn drop_place(&mut self, place: Place, ty: &Ty, glue_only: bool) {
        if !self.needs_drop(ty) {
            return;
        }
        let next = self.cfg.new_block();
        self.cfg.terminate_into(
            Terminator::Drop {
                place,
                target: next,
                glue_only,
            },
            next,
        );
    }

    /// Drops the temporaries made since the innermost scope had `mark` items: at the end of a
    /// statement.
    fn drop_temps(&mut self, mark: usize) {
        for t in self.scopes.take_temps(mark) {
            let ty = self.cfg.local_ty(t);
            self.drop_place(Place::local(t), &ty, false);
        }
    }

    /// The MIR local of HIR local `l`, declared on first use in the innermost scope.
    fn local(&mut self, l: hir::LocalId) -> Local {
        if let Some(local) = self.locals[idx(l)] {
            return local;
        }
        let ty = self.types.local(l);
        let local = self
            .cfg
            .new_local(ty, Some(self.hir.locals[l].name.clone()));
        self.locals[idx(l)] = Some(local);
        self.scopes.push(ScopeItem::Local(local));
        local
    }

    /// Emits the code for leaving every scope deeper than `depth` normally.
    fn leave_scopes(&mut self, depth: usize) {
        self.leave_scopes_as(depth, false);
    }

    /// Emits the code for leaving every scope deeper than `depth`: runs their `defer` blocks
    /// (and, if they are left with an error, their `errdefer` blocks), drops their values and
    /// ends their locals' storage, in the order [`Scopes::unwind`] gives.
    fn leave_scopes_as(&mut self, depth: usize, error: bool) {
        for item in self.scopes.unwind(depth) {
            match item {
                ScopeItem::Local(l) => {
                    let ty = self.cfg.local_ty(l);
                    let glue = self.glue_only == Some(l);
                    self.drop_place(Place::local(l), &ty, glue);
                    self.cfg.push(Statement::StorageDead(l));
                }
                ScopeItem::Temp(t) => {
                    let ty = self.cfg.local_ty(t);
                    self.drop_place(Place::local(t), &ty, false);
                }
                ScopeItem::Defer(body) => {
                    let t = self.temp(Ty::Unit);
                    self.eval_into(t, body);
                }
                ScopeItem::ErrDefer(body) if error => self.run_errdefer(body),
                ScopeItem::ErrDefer(_) => {}
            }
        }
    }

    fn ty(&self, e: ExprId) -> Ty {
        self.types.expr(e)
    }

    fn range(&self, e: ExprId) -> TextRange {
        self.hir.exprs[e].range
    }

    fn is_copy(&mut self, ty: &Ty) -> bool {
        self.solver.is_copy(ty)
    }
}

/// How function `func` is named in printed MIR: `add`, `PartialEq::eq`, `Point::new`.
fn fn_name(items: &ItemTree, func: u32) -> String {
    let sig = &items.fns[func as usize];
    match sig.container {
        Container::Free => sig.name.clone(),
        Container::Trait(t) => format!("{}::{}", items.traits[t as usize].name, sig.name),
        Container::Impl(i) => {
            let imp = &items.impls[i as usize];
            match &imp.trait_ref {
                Some(t) => format!("<{:?} as {}>::{}", imp.self_ty, t.name, sig.name),
                None => format!("{:?}::{}", imp.self_ty, sig.name),
            }
        }
    }
}

//! Building MIR from the typed HIR: the `mir` query.
//!
//! [`Builder`] lowers one body. It is made of two components with their own state:
//! - [`cfg::Cfg`]: the blocks and locals under construction and the block being filled;
//! - [`scopes::Scopes`]: the open scopes and loops, and what leaving them undoes.
//!
//! The lowering itself is split by topic: [`places`] (places and operands), [`exprs`]
//! (expressions and statements), [`patterns`] and [`arith`] (operators and comparisons).

use std::collections::{HashMap, HashSet};

use moddil_sema::hir::{self, ExprId};
use moddil_sema::{Container, Db, Inference, Item, ItemTree, Owner, Solver, Src, Ty};
use moddil_syntax::TextRange;

use crate::{
    Body, Const, Local, LocalDecl, Operand, Place, Rvalue, Statement, Terminator, TrapClass,
};

mod arith;
mod cfg;
mod closures;
mod errors;
mod exprs;
mod patterns;
mod places;
mod scopes;
mod select;

use cfg::Cfg;
use errors::{OnError, Target};
use scopes::{ScopeItem, Scopes};

/// The MIR of `owner`, or `None` if its file has errors.
#[salsa::tracked(returns(ref))]
pub fn mir<'db>(db: &'db dyn Db, owner: Owner<'db>) -> Option<Body> {
    let file = owner.program(db);
    if moddil_sema::has_errors(db, file) {
        return None;
    }
    if let Item::Closure(_, _, expr) = owner.item(db) {
        return Some(closure_mir(db, owner, expr));
    }
    let items = moddil_sema::item_tree(db, file);
    let hir = &moddil_sema::body(db, owner).body;
    let types = revealed(db, file, moddil_sema::infer(db, owner));
    let types = &*types;
    let fx = moddil_sema::effects(db, file);
    // The function's effects, which its callers pass as hidden parameters after the others.
    let needs: Vec<Ty> = match owner.item(db) {
        Item::Fn(i) => fx.fns[i as usize].needs.clone(),
        _ => Vec::new(),
    };
    let (name, ret, src, preds) = match owner.item(db) {
        Item::Fn(i) => {
            let sig = &items.fns[i as usize];
            (
                fn_name(items, i),
                moddil_sema::reveal(db, file, &sig.ret),
                sig.src,
                sig.generics.preds.as_slice(),
            )
        }
        Item::Test(i) | Item::Closure(i, true, _) => (
            format!("{:?}", items.tests[i as usize].name),
            Ty::Unit,
            Src::File(items.tests[i as usize].file),
            &[][..],
        ),
        Item::Closure(..) => unreachable!("built by `closure_mir`"),
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
        _ => false,
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
            track_caller: match owner.item(db) {
                Item::Fn(i) => items.fns[i as usize].track_caller,
                _ => false,
            },
            locals: vec![LocalDecl {
                ty: ret,
                name: None,
            }],
            arg_count: hir.params.len() + needs.len(),
            blocks: Vec::new(),
            strs: Vec::new(),
            cleanups: Vec::new(),
            cleanup_locals: 0,
        }),
        scopes: Scopes::new(),
        locals: vec![None; hir.locals.len()],
        recording: HashMap::new(),
        moved_parts: HashSet::new(),
        compare_expr: ExprId::from_raw(la_arena::RawIdx::from_u32(0)),
        targets: vec![Target::Out],
        on_error: HashMap::new(),
        selectables: selectables(db, owner.program(db), hir, types),
        binds: None,
        fx,
        evidence: HashMap::new(),
        installed: HashMap::new(),
        uncoerced: None,
        captured: HashMap::new(),
        in_closure: false,
    };
    for &p in &hir.params {
        b.local(p);
    }
    // The handlers of the function's effects, each named as its signature names it, if it does.
    for (n, t) in needs.iter().enumerate() {
        let named = match owner.item(db) {
            Item::Fn(i) => hir
                .effect_locals
                .iter()
                .find(|&&(_, k)| items.fns[i as usize].effects.get(k).map(|e| &e.ty) == Some(t)),
            _ => None,
        };
        let name = named.map(|&(l, _)| hir.locals[l].name.clone());
        let local = b.cfg.new_local(Ty::reference(false, t.clone()), name);
        if let Some(&(l, _)) = named {
            b.locals[idx(l)] = Some(local);
        }
        b.evidence.insert(n, Place::local(local));
    }
    for &(local, pat) in &hir.param_pats {
        let place = Place::local(b.local(local));
        let fail = b.cfg.new_block();
        b.test_pat(pat, &place, fail);
    }
    // Contracts (`contract.1`): the preconditions, then the values `old(e)` keeps.
    for &e in &hir.requires {
        b.contract(e, TrapClass::Precondition);
    }
    for &(local, e) in &hir.olds {
        let l = b.local(local);
        b.eval_into(l, e);
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
    b.ensures();
    b.leave_scopes(0);
    b.cfg.terminate(Terminator::Return);
    b.cleanups();
    Some(b.cfg.finish())
}

/// `types` with the opaque result types (`type.impl.ret.1`) replaced by their functions'
/// hidden types, if the program has any.
fn revealed<'a>(
    db: &dyn Db,
    program: moddil_sema::Program,
    types: &'a Inference,
) -> std::borrow::Cow<'a, Inference> {
    let items = moddil_sema::item_tree(db, program);
    if items.fns.iter().any(|f| matches!(f.ret, Ty::Opaque(_))) {
        std::borrow::Cow::Owned(types.map_types(&mut |t| moddil_sema::reveal(db, program, t)))
    } else {
        std::borrow::Cow::Borrowed(types)
    }
}

/// The MIR of the closure at expression `expr` of `owner`'s function's or test's body: its
/// captures and its arguments are its two parameters, both tuples.
fn closure_mir<'db>(db: &'db dyn Db, owner: Owner<'db>, expr: u32) -> Body {
    let program = owner.program(db);
    let base = Owner::new(db, program, owner.item(db).owner());
    let items = moddil_sema::item_tree(db, program);
    let hir = &moddil_sema::body(db, base).body;
    let types = revealed(db, program, moddil_sema::infer(db, base));
    let types = &*types;
    let fx = moddil_sema::effects(db, program);
    let e = ExprId::from_raw(la_arena::RawIdx::from_u32(expr));
    let Ty::Closure(c) = types.expr(e) else {
        unreachable!("a closure's type")
    };
    let hir::ExprKind::Closure { params, body, .. } = &hir.exprs[e].kind else {
        unreachable!("a closure expression")
    };
    let (src, preds, name) = match base.item(db) {
        Item::Fn(i) => {
            let sig = &items.fns[i as usize];
            (sig.src, sig.generics.preds.as_slice(), fn_name(items, i))
        }
        Item::Test(i) => (
            Src::File(items.tests[i as usize].file),
            &[][..],
            format!("{:?}", items.tests[i as usize].name),
        ),
        Item::Closure(..) => unreachable!("an owner is a function or test"),
    };
    let mut b = Builder {
        hir,
        types,
        items,
        solver: Solver::new(items, preds),
        glue_only: None,
        cfg: Cfg::new(Body {
            name: format!("{name}::{{closure}}"),
            src,
            track_caller: false,
            locals: vec![LocalDecl {
                ty: c.ret.clone(),
                name: None,
            }],
            arg_count: 2,
            blocks: Vec::new(),
            strs: Vec::new(),
            cleanups: Vec::new(),
            cleanup_locals: 0,
        }),
        scopes: Scopes::new(),
        locals: vec![None; hir.locals.len()],
        recording: HashMap::new(),
        moved_parts: HashSet::new(),
        compare_expr: ExprId::from_raw(la_arena::RawIdx::from_u32(0)),
        targets: vec![Target::Out],
        on_error: HashMap::new(),
        selectables: selectables(db, owner.program(db), hir, types),
        binds: None,
        fx,
        evidence: HashMap::new(),
        installed: HashMap::new(),
        uncoerced: None,
        captured: HashMap::new(),
        in_closure: true,
    };
    let env = b.cfg.new_local(
        Ty::tuple(c.captures.to_vec()),
        Some("{captures}".to_string()),
    );
    let args = b.cfg.new_local(
        Ty::tuple(c.params.to_vec()),
        Some("{arguments}".to_string()),
    );
    for (k, cap) in types.captures(e).iter().enumerate() {
        let field = Place::local(env).field(k as u32);
        match *cap {
            moddil_sema::Capture::Local(l, moddil_sema::CaptureMode::Move) => {
                b.captured.insert(l, field);
            }
            moddil_sema::Capture::Local(l, _) => {
                b.captured.insert(l, field.deref());
            }
            moddil_sema::Capture::Evidence(moddil_sema::Evidence::Param(i)) => {
                b.evidence.insert(i, field);
            }
            moddil_sema::Capture::Evidence(moddil_sema::Evidence::Install(w, k)) => {
                b.installed.insert((w, k), field);
            }
        }
    }
    for (i, (pat, _)) in params.iter().enumerate() {
        let place = Place::local(args).field(i as u32);
        let fail = b.cfg.new_block();
        b.match_pat(*pat, &place, fail);
    }
    b.eval_into(Local::RETURN, *body);
    b.leave_scopes(0);
    b.cfg.terminate(Terminator::Return);
    b.cleanups();
    b.cfg.finish()
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
    /// The parts of the `assert` being built whose values were moved away (to a call that takes
    /// them, say), so a failure cannot show them: their locals are empty then.
    moved_parts: HashSet<ExprId>,
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
    /// Where the function's own handlers are, by their index in its needs: its hidden
    /// parameters, or, in a closure's body, its captures.
    evidence: HashMap<usize, Place>,
    /// Where the handlers each `with` installs are: by the `with` and the handler's index.
    installed: HashMap<(ExprId, usize), Place>,
    /// In a closure's body: where each local of the enclosing body it captures is.
    captured: HashMap<hir::LocalId, Place>,
    /// Whether the body built is a closure's, whose returns check none of its function's
    /// postconditions.
    in_closure: bool,
    /// The expression being evaluated before its conversion to a `dyn` borrow, which is not
    /// converted again.
    uncoerced: Option<ExprId>,
    /// For each `#[selectable]` function a `select` here waits on: the types of its two parts,
    /// the closures that give its receiver and do the rest (`task.select.4`).
    selectables: HashMap<u32, (Ty, Ty)>,
}

/// The parts of each `#[selectable]` function the `select`s of `hir` wait on.
fn selectables(
    db: &dyn Db,
    program: moddil_sema::Program,
    hir: &hir::Body,
    types: &Inference,
) -> HashMap<u32, (Ty, Ty)> {
    let items = moddil_sema::item_tree(db, program);
    let mut out = HashMap::new();
    for (_, x) in hir.exprs.iter() {
        let hir::ExprKind::Select(arms) = &x.kind else {
            continue;
        };
        for arm in arms {
            let call = match hir.exprs[arm.op].kind {
                hir::ExprKind::Question(c) => c,
                _ => arm.op,
            };
            let Some(f) = types.callee(call).map(moddil_sema::Callee::func) else {
                continue;
            };
            if !items.fns[f as usize].selectable || out.contains_key(&f) {
                continue;
            }
            let o = Owner::new(db, program, Item::Fn(f));
            if let Some(p) = moddil_sema::body(db, o).body.select {
                let inf = moddil_sema::infer(db, o);
                out.insert(f, (inf.expr(p.chan), inf.expr(p.done)));
            }
        }
    }
    out
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
            self.cfg.node = self.scopes.node();
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
        let temps = self.scopes.take_temps(mark);
        self.cfg.node = self.scopes.node();
        for t in temps {
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
        self.cfg.node = self.scopes.node();
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

impl Builder<'_> {
    /// Traps with `trap` unless condition `e` holds.
    fn contract(&mut self, e: ExprId, trap: TrapClass) {
        let ok = self.temp(Ty::Bool);
        self.eval_into(ok, e);
        let range = self.range(e);
        self.cfg.push(Statement::Assert {
            cond: Operand::Copy(Place::local(ok)),
            expected: true,
            trap,
            range,
            parts: Vec::new(),
        });
    }

    /// Before returning: the postconditions, with `result` holding the value returned.
    pub(super) fn ensures(&mut self) {
        if self.hir.ensures.is_empty() || self.in_closure {
            return;
        }
        let result = self.hir.result.map(|r| self.local(r));
        if let Some(r) = result {
            self.cfg
                .assign(r, Rvalue::Use(Operand::Move(Place::local(Local::RETURN))));
        }
        for e in self.hir.ensures.clone() {
            self.contract(e, TrapClass::Postcondition);
        }
        if let Some(r) = result {
            self.cfg
                .assign(Local::RETURN, Rvalue::Use(Operand::Move(Place::local(r))));
        }
    }
}

impl Builder<'_> {
    /// The cleanup blocks (`task.cancel.1`): for each scope node, a block that undoes its item
    /// (drops a local or temporary, runs a `defer` or `errdefer`) and goes on to its parent's;
    /// node 0's resumes unwinding in the caller.
    fn cleanups(&mut self) {
        let nodes = self.scopes.nodes();
        let locals = self.cfg.local_count();
        self.cfg.cleanup = true;
        let resume = self.cfg.new_block();
        self.cfg.continue_in(resume);
        self.cfg.terminate(Terminator::Resume);
        let mut blocks = vec![resume; nodes.len()];
        for (n, (item, parent)) in nodes.iter().enumerate().skip(1) {
            let start = self.cfg.new_block();
            self.cfg.continue_in(start);
            self.cfg.node = 0;
            match item.expect("an item") {
                ScopeItem::Local(l) | ScopeItem::Temp(l) => {
                    let ty = self.cfg.local_ty(l);
                    let glue = self.glue_only == Some(l);
                    self.drop_place(Place::local(l), &ty, glue);
                }
                ScopeItem::Defer(body) => {
                    let t = self.temp(Ty::Unit);
                    self.eval_into(t, body);
                }
                ScopeItem::ErrDefer(body) => self.run_errdefer(body),
            }
            // Parents come before their children, so the parent's block is known.
            self.cfg
                .terminate(Terminator::Goto(blocks[*parent as usize]));
            blocks[n] = start;
        }
        self.cfg
            .set_cleanups(blocks, self.cfg.local_count() - locals);
    }
}

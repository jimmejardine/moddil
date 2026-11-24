//! Hover, go to definition and inlay hints: what is at an offset of a file, found in its body's
//! HIR and types, or (in signatures, `use` lines and the like) by the name under the cursor.

use moddil_sema::hir::{ExprId, ExprKind, PatKind};
use moddil_sema::{
    Callee, Container, Db, Def, Inference, Item, ItemTree, Lowered, Owner, Program, SPAWN, SUSPEND,
    SelfKind, Solver, Src, TraitRef, Ty,
};
use moddil_syntax::{SyntaxKind, TextRange};

use crate::Hint;

/// Queries about one file of a program.
/// Where a definition is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Place {
    /// In a file of the program, by its index.
    File(u32, TextRange),
    /// In the prelude or `core`, at this range of [`moddil_sources::PRELUDE_AND_CORE`].
    Library(TextRange),
}

impl Place {
    /// Where `range` of source `src` is; derived code has no source to go to.
    fn of(src: Src, range: TextRange) -> Option<Place> {
        match src {
            Src::File(f) => Some(Place::File(f, range)),
            Src::Prelude => Some(Place::Library(range)),
            Src::Derived => None,
        }
    }
}

pub(crate) struct Query<'db> {
    db: &'db dyn Db,
    program: Program,
    file: u32,
    items: &'db ItemTree,
}

/// One body: its HIR and its types.
struct Body<'db> {
    lowered: &'db Lowered,
    types: &'db Inference,
}

/// Whether `r` contains `offset`, its end included (the cursor just after a name is on it).
fn contains(r: TextRange, offset: u32) -> bool {
    r.start <= offset && offset <= r.end && r.end > r.start
}

impl<'db> Query<'db> {
    pub(crate) fn new(db: &'db dyn Db, program: Program, file: u32) -> Self {
        Query {
            db,
            program,
            file,
            items: moddil_sema::item_tree(db, program),
        }
    }

    /// The bodies of this file: each function's and test's.
    fn owners(&self) -> Vec<(Item, TextRange)> {
        let f = self.file;
        let fns = self
            .items
            .fns
            .iter()
            .enumerate()
            .filter(|(_, s)| s.src == Src::File(f) && s.has_body)
            .map(|(i, s)| (Item::Fn(i as u32), s.node));
        let tests = self
            .items
            .tests
            .iter()
            .enumerate()
            .filter(|(_, t)| t.file == f)
            .map(|(i, t)| (Item::Test(i as u32), t.node));
        fns.chain(tests).collect()
    }

    fn body(&self, item: Item) -> Body<'db> {
        let owner = Owner::new(self.db, self.program, item);
        Body {
            lowered: moddil_sema::body(self.db, owner),
            types: moddil_sema::infer(self.db, owner),
        }
    }

    /// The body `offset` is in, if any.
    fn body_at(&self, offset: u32) -> Option<Body<'db>> {
        let (item, _) = self
            .owners()
            .into_iter()
            .filter(|(_, r)| contains(*r, offset))
            .min_by_key(|(_, r)| r.len())?;
        Some(self.body(item))
    }

    /// The innermost expression of `b` at `offset`.
    fn expr_at(b: &Body<'_>, offset: u32) -> Option<ExprId> {
        b.lowered
            .body
            .exprs
            .iter()
            .filter(|(_, e)| contains(e.range, offset))
            .min_by_key(|(_, e)| e.range.len())
            .map(|(id, _)| id)
    }

    /// The function a path expression `e` names, as the call it is the callee of resolves it.
    fn path_callee(b: &Body<'_>, e: ExprId) -> Option<Callee> {
        let (call, _) = b
            .lowered
            .body
            .exprs
            .iter()
            .find(|(_, x)| matches!(&x.kind, ExprKind::Call { callee, .. } if *callee == e))?;
        b.types.callee(call).cloned()
    }

    /// The function `c` runs: for a trait's method, the impl's where it can be found.
    fn resolve(&self, c: &Callee) -> u32 {
        match c {
            Callee::Fn { func, .. } => *func,
            Callee::Trait { func, args } => {
                let items = self.items;
                let found = items.trait_of(*func).and_then(|t| {
                    let n = items.traits[t as usize].generics.params.len();
                    let tr = TraitRef {
                        trait_: t,
                        name: items.traits[t as usize].name.as_str().into(),
                        args: args.get(1..n)?.to_vec().into(),
                    };
                    Solver::new(items, &[]).impl_method(*func, args.first()?, &tr)
                });
                found.map_or(*func, |(f, _)| f)
            }
        }
    }

    /// The module this file is.
    fn module(&self) -> u32 {
        self.items
            .modules
            .iter()
            .position(|m| m.file == Some(self.file))
            .unwrap_or(0) as u32
    }

    /// The identifier at `offset`, and its range.
    fn ident_at(&self, offset: u32) -> Option<(String, TextRange)> {
        let source = self.program.files(self.db).get(self.file as usize)?.source;
        let tree = moddil_sema::parse(self.db, source).syntax();
        tree.tokens()
            .into_iter()
            .find(|t| t.kind() == SyntaxKind::IDENT && contains(t.text_range(), offset))
            .map(|t| (t.text().to_string(), t.text_range()))
    }

    /// What the name at `offset` stands for in this file's module, if it names an item.
    fn def_at(&self, offset: u32) -> Option<Def> {
        let (name, _) = self.ident_at(offset)?;
        self.items.lookup(self.module(), &name, false)
    }

    /// Where `def`'s name is: in a file of the program, or in the prelude or `core`.
    fn def_location(&self, def: Def) -> Option<Place> {
        let items = self.items;
        let (src, range) = match def {
            Def::Fn(f) => (items.fns[f as usize].src, items.fns[f as usize].name_range),
            Def::Adt(a) => (
                items.adts[a as usize].src,
                items.adts[a as usize].name_range,
            ),
            Def::Type(t) => (
                items.types[t as usize].src,
                items.types[t as usize].name_range,
            ),
            Def::Trait(t) => (
                items.traits[t as usize].src,
                items.traits[t as usize].name_range,
            ),
            Def::Variant(a, v) => (
                items.adts[a as usize].src,
                items.adts[a as usize].variants[v as usize].name_range,
            ),
            Def::Module(m) => {
                return items.modules[m as usize]
                    .file
                    .map(|f| Place::File(f, TextRange::new(0, 0)));
            }
        };
        Place::of(src, range)
    }

    /// Where what is at `offset` is defined.
    pub(crate) fn definition(&self, offset: u32) -> Option<Place> {
        if let Some(b) = self.body_at(offset) {
            let here = |r: TextRange| Some(Place::File(self.file, r));
            // A binding's own name, or a local used.
            for (_, p) in b.lowered.body.pats.iter() {
                if let PatKind::Bind { local, .. } = &p.kind
                    && contains(p.range, offset)
                {
                    return here(b.lowered.body.locals[*local].range);
                }
            }
            if let Some(e) = Self::expr_at(&b, offset) {
                match &b.lowered.body.exprs[e].kind {
                    ExprKind::Local(l) => return here(b.lowered.body.locals[*l].range),
                    ExprKind::Fn { func, .. } => return self.def_location(Def::Fn(*func)),
                    ExprKind::AssocFn { .. } | ExprKind::TraitFn { .. } => {
                        if let Some(c) = Self::path_callee(&b, e) {
                            return self.def_location(Def::Fn(self.resolve(&c)));
                        }
                    }
                    ExprKind::MethodCall { name_range, .. } if contains(*name_range, offset) => {
                        if let Some(c) = b.types.callee(e) {
                            return self.def_location(Def::Fn(self.resolve(c)));
                        }
                    }
                    ExprKind::Field {
                        base, name_range, ..
                    } if contains(*name_range, offset) => {
                        let k = b.types.field_index(e)?;
                        if let Ty::Adt(a, ..) = strip(&b.types.expr(*base)) {
                            let adt = &self.items.adts[a as usize];
                            let field = adt.variants.first()?.fields.get(k as usize)?;
                            return Place::of(adt.src, field.range);
                        }
                    }
                    _ => {}
                }
            }
        }
        self.def_location(self.def_at(offset)?)
    }

    /// What to show on hovering over `offset`, as Markdown.
    pub(crate) fn hover(&self, offset: u32) -> Option<String> {
        let code = |s: String| Some(format!("```moddil\n{s}\n```"));
        // An item's own name.
        for (i, f) in self.items.fns.iter().enumerate() {
            if f.src == Src::File(self.file) && contains(f.name_range, offset) {
                return code(self.signature(i as u32));
            }
        }
        if let Some(b) = self.body_at(offset) {
            let body = &b.lowered.body;
            for (_, p) in body.pats.iter() {
                if let PatKind::Bind { local, .. } = &p.kind
                    && contains(p.range, offset)
                {
                    let l = &body.locals[*local];
                    return code(format!("{}: {}", l.name, b.types.local(*local)));
                }
            }
            if let Some(e) = Self::expr_at(&b, offset) {
                let ty = b.types.expr(e);
                match &body.exprs[e].kind {
                    ExprKind::Local(l) => {
                        let l = &body.locals[*l];
                        let m = if l.mutable { "mut " } else { "" };
                        return code(format!("{m}{}: {ty}", l.name));
                    }
                    ExprKind::Fn { func, .. } => return code(self.signature(*func)),
                    ExprKind::AssocFn { .. } | ExprKind::TraitFn { .. } => {
                        if let Some(c) = Self::path_callee(&b, e) {
                            return code(self.signature(self.resolve(&c)));
                        }
                    }
                    ExprKind::MethodCall { name_range, .. } if contains(*name_range, offset) => {
                        if let Some(c) = b.types.callee(e) {
                            return code(self.signature(self.resolve(c)));
                        }
                    }
                    ExprKind::Field {
                        name, name_range, ..
                    } if contains(*name_range, offset) => {
                        return code(format!("{name}: {ty}"));
                    }
                    _ => {}
                }
                if ty != Ty::Error {
                    return code(ty.to_string());
                }
            }
        }
        // A name in a signature, a type or a `use`: the item it stands for.
        match self.def_at(offset)? {
            Def::Fn(f) => code(self.signature(f)),
            Def::Adt(a) => {
                let adt = &self.items.adts[a as usize];
                let kw = if adt.is_enum { "enum" } else { "struct" };
                code(format!("{kw} {}", adt.name))
            }
            Def::Type(t) => {
                let t = &self.items.types[t as usize];
                code(format!("type {} = {}", t.name, t.ty))
            }
            Def::Trait(t) => code(format!("trait {}", self.items.traits[t as usize].name)),
            Def::Variant(a, v) => {
                let adt = &self.items.adts[a as usize];
                code(format!("{}::{}", adt.name, adt.variants[v as usize].name))
            }
            Def::Module(m) => code(format!("mod {}", self.items.module_path(m))),
        }
    }

    /// Function `f`'s signature, with what it needs (as written, or inferred for a private
    /// function): `fn get(&self, key: &K) -> Option<&V> with Net, suspend, fail(E)`.
    pub(crate) fn signature(&self, f: u32) -> String {
        signature(self.db, self.program, f)
    }

    /// After each call that may suspend or spawn, a hint saying so.
    pub(crate) fn hints(&self) -> Vec<Hint> {
        let fx = moddil_sema::effects(self.db, self.program);
        let mut out = Vec::new();
        for (item, _) in self.owners() {
            let b = self.body(item);
            for (e, x) in b.lowered.body.exprs.iter() {
                if !matches!(x.kind, ExprKind::Call { .. } | ExprKind::MethodCall { .. }) {
                    continue;
                }
                let Some(c) = b.types.callee(e) else { continue };
                let f = self.resolve(c);
                let control = fx.fns.get(f as usize).map_or(0, |e| e.control);
                let label = match (control & SUSPEND != 0, control & SPAWN != 0) {
                    (true, true) => "suspends, spawns",
                    (true, false) => "suspends",
                    (false, true) => "spawns",
                    (false, false) => continue,
                };
                out.push(Hint {
                    offset: x.range.end,
                    label: label.to_string(),
                });
            }
        }
        out.sort_by_key(|h| h.offset);
        out.dedup();
        out
    }
}

/// `t` without the borrows around it.
pub(crate) fn strip(t: &Ty) -> Ty {
    match t {
        Ty::Ref(_, inner) => strip(inner),
        t => t.clone(),
    }
}

/// Function `f`'s signature, as [`Query::signature`].
pub(crate) fn signature(db: &dyn Db, program: Program, f: u32) -> String {
    let items = moddil_sema::item_tree(db, program);
    let s = &items.fns[f as usize];
    let owner = match s.container {
        Container::Impl(k) => format!("{}::", items.impls[k as usize].self_ty),
        Container::Trait(t) => format!("{}::", items.traits[t as usize].name),
        Container::Free => String::new(),
    };
    let mut params: Vec<String> = Vec::new();
    let named_self = s.params.first().and_then(|p| p.name.as_deref()) == Some("self");
    if !named_self {
        match s.self_kind {
            Some(SelfKind::Value) => params.push("self".to_string()),
            Some(SelfKind::Ref) => params.push("&self".to_string()),
            Some(SelfKind::RefMut) => params.push("&mut self".to_string()),
            None => {}
        }
    }
    for (i, p) in s.params.iter().enumerate() {
        let name = p.name.as_deref().unwrap_or("_");
        if i == 0 && named_self {
            params.push(match s.self_kind {
                Some(SelfKind::Ref) => "&self".to_string(),
                Some(SelfKind::RefMut) => "&mut self".to_string(),
                _ => "self".to_string(),
            });
        } else {
            params.push(format!("{name}: {}", p.ty));
        }
    }
    let mut out = format!("fn {owner}{}({})", s.name, params.join(", "));
    if s.ret != Ty::Unit {
        out.push_str(&format!(" -> {}", s.ret));
    }
    let fx = moddil_sema::effects(db, program);
    let mut with: Vec<String> = Vec::new();
    if let Some(e) = fx.fns.get(f as usize) {
        with.extend(e.needs.iter().map(ToString::to_string));
        if e.control & SUSPEND != 0 {
            with.push("suspend".to_string());
        }
        if e.control & SPAWN != 0 {
            with.push("spawn".to_string());
        }
        if e.fails != Ty::Never {
            with.push(format!("fail({})", e.fails));
        }
    }
    if !with.is_empty() {
        out.push_str(" with ");
        out.push_str(&with.join(", "));
    }
    out
}

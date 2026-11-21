//! Building the item tree: the `parse` and `item_tree` queries, and resolving the types written
//! in signatures.

use std::collections::HashMap;
use std::sync::Arc;

use moddil_syntax::ast::{self, AstNode};
use moddil_syntax::{Parse, SyntaxKind as K, SyntaxNode, TextRange};

use crate::derive::{self, DERIVABLE, Expansion};
use crate::items::{
    AdtDef, COPY_CAP, Container, FieldDef, FnSig, GenericParam, Generics, ImplDef, ItemTree,
    ParamSig, Pred, SelfKind, Shape, Src, TestSig, TraitDef, VariantDef,
};
use crate::traits::{Answer, Solver, elaborate};
use crate::ty::{AssocTy, TraitRef};
use crate::{Code, Db, Diagnostic, SourceFile, Ty};

/// The prelude's source.
pub(crate) const PRELUDE: &str = include_str!("../../../reference/2027/prelude.moddil");

/// The file parsed.
#[salsa::tracked(returns(ref))]
pub fn parse(db: &dyn Db, file: SourceFile) -> Parse {
    moddil_syntax::parse(file.text(db))
}

/// The prelude parsed (the same for every file).
#[salsa::tracked(returns(ref))]
pub(crate) fn prelude_parse(db: &dyn Db, file: SourceFile) -> Parse {
    let _ = file;
    let _ = db;
    moddil_syntax::parse(PRELUDE)
}

/// The impls the derives of the prelude and the file expand to, and their parse.
#[salsa::tracked(returns(ref))]
pub(crate) fn derived(db: &dyn Db, file: SourceFile) -> (Expansion, Parse) {
    let prelude = prelude_parse(db, file).syntax();
    let root = parse(db, file).syntax();
    let items = prelude
        .children()
        .map(|n| (Src::Prelude, n))
        .chain(root.children().map(|n| (Src::File, n)));
    let expansion = derive::expand(items);
    let parse = moddil_syntax::parse(&expansion.text);
    (expansion, parse)
}

/// The root and text of one of a file's sources.
pub(crate) fn source(db: &dyn Db, file: SourceFile, src: Src) -> (SyntaxNode, &str) {
    match src {
        Src::Prelude => (prelude_parse(db, file).syntax(), PRELUDE),
        Src::File => (parse(db, file).syntax(), file.text(db).as_str()),
        Src::Derived => {
            let (e, p) = derived(db, file);
            (p.syntax(), e.text.as_str())
        }
    }
}

/// The node of `kind` at `range` in the tree under `root`.
pub(crate) fn node_at(root: &SyntaxNode, kind: K, range: TextRange) -> Option<SyntaxNode> {
    root.descendants()
        .into_iter()
        .find(|n| n.kind() == kind && n.text_range() == range)
}

/// One segment of a path: its name, where it is, and its generic arguments (`Vec::<int>`).
#[derive(Clone, Debug)]
pub(crate) struct Segment {
    pub name: String,
    pub range: TextRange,
    pub args: Option<SyntaxNode>,
}

/// The segments of the `PATH` node `path`, with the generic arguments written on each (inside the
/// segment in types, after `::` in expressions). `None` for a qualified path (`<T as Tr>::X`).
pub(crate) fn path_segments(path: &SyntaxNode) -> Option<Vec<Segment>> {
    let mut out: Vec<Segment> = Vec::new();
    for c in path.children() {
        match c.kind() {
            K::QUALIFIED_PATH => return None,
            K::PATH_SEGMENT => {
                let tok = c.child_tokens().find(|t| {
                    matches!(
                        t.kind(),
                        K::IDENT | K::SELF_TYPE_KW | K::SELF_KW | K::SUPER_KW | K::CRATE_KW
                    )
                });
                out.push(Segment {
                    name: tok
                        .as_ref()
                        .map(|t| t.text().to_string())
                        .unwrap_or_default(),
                    range: tok.map_or_else(|| c.text_range(), |t| t.text_range()),
                    args: c.child(K::GENERIC_ARGS),
                });
            }
            K::GENERIC_ARGS => {
                if let Some(last) = out.last_mut() {
                    last.args = Some(c);
                }
            }
            _ => {}
        }
    }
    Some(out)
}

/// What the names in a type refer to.
pub(crate) struct Scope<'a> {
    pub tree: &'a ItemTree,
    /// The type parameters in scope.
    pub params: &'a [GenericParam],
    /// The bounds in scope, for `T::Name`.
    pub preds: &'a [Pred],
    /// What `Self` is.
    pub self_ty: Option<Ty>,
    /// Inside a trait: the trait, so `Self::Name` is its associated type.
    pub trait_ctx: Option<u32>,
    /// Inside an impl of a trait: the trait and the impl's associated types.
    pub impl_assoc: Option<(TraitRef, Vec<Option<Ty>>)>,
    /// Whether `_` and a generic struct or enum without arguments leave types to infer.
    pub holes: bool,
}

impl<'a> Scope<'a> {
    pub(crate) fn new(tree: &'a ItemTree) -> Self {
        Scope {
            tree,
            params: &[],
            preds: &[],
            self_ty: None,
            trait_ctx: None,
            impl_assoc: None,
            holes: false,
        }
    }

    fn param(&self, name: &str) -> Option<Ty> {
        self.params
            .iter()
            .rposition(|p| p.name == name)
            .map(|i| Ty::Param(i as u32, name.into()))
    }
}

fn hole() -> Ty {
    Ty::Infer(Ty::HOLE)
}

/// The type written as `t`.
pub(crate) fn resolve_type(t: &ast::Type, scope: &Scope, diags: &mut Vec<Diagnostic>) -> Ty {
    let node = t.syntax();
    let range = node.text_range();
    match t {
        ast::Type::Path(_) => {
            let Some(path) = node.child(K::PATH) else {
                return Ty::Error;
            };
            match path_segments(&path) {
                Some(segs) => path_type(&segs, range, scope, diags),
                None => qualified_type(&path, scope, diags),
            }
        }
        ast::Type::Tuple(_) => {
            let elements: Vec<Ty> = node
                .children()
                .filter_map(ast::Type::cast)
                .map(|t| resolve_type(&t, scope, diags))
                .collect();
            if elements.iter().any(Ty::has_ref) {
                diags.push(Diagnostic::error(
                    Code::E0309,
                    range,
                    "a tuple cannot hold a borrow: borrows live only in parameters and locals",
                ));
                return Ty::Error;
            }
            Ty::tuple(elements)
        }
        ast::Type::Ref(_) => {
            let mutable = node.token(K::MUT_KW).is_some();
            let inner = match node.children().find_map(ast::Type::cast) {
                Some(t) => resolve_type(&t, scope, diags),
                None => Ty::Error,
            };
            if inner.has_ref() {
                diags.push(Diagnostic::error(
                    Code::E0309,
                    range,
                    "a borrow cannot point to a borrow",
                ));
                return Ty::Error;
            }
            Ty::reference(mutable, inner)
        }
        ast::Type::Paren(_) => match node.children().find_map(ast::Type::cast) {
            Some(inner) => resolve_type(&inner, scope, diags),
            None => Ty::Error,
        },
        ast::Type::Never(_) => Ty::Never,
        ast::Type::Infer(_) if scope.holes => hole(),
        ast::Type::Infer(_) => {
            diags.push(Diagnostic::error(
                Code::E0102,
                range,
                "`_` stands for a type to infer, which a signature must write out",
            ));
            Ty::Error
        }
        _ => {
            diags.push(Diagnostic::unsupported(range, "this type"));
            Ty::Error
        }
    }
}

/// The type arguments and associated-type bindings (`Item = int`) of a `GENERIC_ARGS` node.
pub(crate) fn generic_args(
    node: &SyntaxNode,
    scope: &Scope,
    diags: &mut Vec<Diagnostic>,
) -> (Vec<Ty>, Vec<(String, TextRange, Ty)>) {
    let mut types = Vec::new();
    let mut assoc = Vec::new();
    for c in node.children() {
        match c.kind() {
            K::TYPE_ARG => match c.children().find_map(ast::Type::cast) {
                Some(t) => types.push(resolve_type(&t, scope, diags)),
                None => {
                    diags.push(Diagnostic::unsupported(c.text_range(), "a const argument"));
                    types.push(Ty::Error);
                }
            },
            K::ASSOC_ARG => {
                let name = c.token(K::IDENT);
                let t = match c.children().find_map(ast::Type::cast) {
                    Some(t) => resolve_type(&t, scope, diags),
                    None => Ty::Error,
                };
                if let Some(n) = name {
                    assoc.push((n.text().to_string(), n.text_range(), t));
                }
            }
            K::CONST_ARG => {
                diags.push(Diagnostic::unsupported(c.text_range(), "a const argument"));
                types.push(Ty::Error);
            }
            _ => {}
        }
    }
    (types, assoc)
}

/// Checks that `n` arguments were given for `want` parameters (E0404).
pub(crate) fn arity(
    what: &str,
    want: usize,
    n: usize,
    range: TextRange,
    diags: &mut Vec<Diagnostic>,
) -> bool {
    if want == n {
        return true;
    }
    diags.push(Diagnostic::error(
        Code::E0404,
        range,
        format!(
            "{what} takes {want} type argument{}, but {n} {} given",
            if want == 1 { "" } else { "s" },
            if n == 1 { "was" } else { "were" },
        ),
    ));
    false
}

/// The arguments of a struct or enum written with `args` (or without any).
pub(crate) fn adt_args(
    adt: u32,
    args: Option<&SyntaxNode>,
    range: TextRange,
    scope: &Scope,
    diags: &mut Vec<Diagnostic>,
) -> Vec<Ty> {
    let def = &scope.tree.adts[adt as usize];
    let want = def.generics.params.len();
    match args {
        Some(a) => {
            let (types, assoc) = generic_args(a, scope, diags);
            for (_, r, _) in assoc {
                diags.push(Diagnostic::error(
                    Code::E0413,
                    r,
                    format!("`{}` has no associated types", def.name),
                ));
            }
            if arity(&format!("`{}`", def.name), want, types.len(), range, diags) {
                types
            } else {
                vec![Ty::Error; want]
            }
        }
        None if want == 0 => Vec::new(),
        None if scope.holes => vec![hole(); want],
        None => {
            arity(&format!("`{}`", def.name), want, 0, range, diags);
            vec![Ty::Error; want]
        }
    }
}

fn path_type(segs: &[Segment], range: TextRange, scope: &Scope, diags: &mut Vec<Diagnostic>) -> Ty {
    let tree = scope.tree;
    let no_args = |s: &Segment, diags: &mut Vec<Diagnostic>| {
        if let Some(a) = &s.args {
            diags.push(Diagnostic::error(
                Code::E0404,
                a.text_range(),
                format!("`{}` takes no type arguments", s.name),
            ));
        }
    };
    match segs {
        [one] => {
            let name = one.name.as_str();
            match name {
                "int" => {
                    no_args(one, diags);
                    return Ty::Int;
                }
                "bool" => {
                    no_args(one, diags);
                    return Ty::Bool;
                }
                "Self" => {
                    no_args(one, diags);
                    return match &scope.self_ty {
                        Some(t) => t.clone(),
                        None => {
                            diags.push(Diagnostic::error(
                                Code::E0102,
                                range,
                                "`Self` is the type of an impl or trait, and there is none here",
                            ));
                            Ty::Error
                        }
                    };
                }
                _ => {}
            }
            if let Some(p) = scope.param(name) {
                no_args(one, diags);
                return p;
            }
            if let Some(a) = tree.adt_named(name) {
                let args = adt_args(a, one.args.as_ref(), range, scope, diags);
                return tree.adts[a as usize].ty_with(a, args);
            }
            let what = if tree.trait_named(name).is_some() {
                format!("`{name}` is a trait, not a type")
            } else {
                format!("there is no type called `{name}`")
            };
            diags.push(Diagnostic::error(Code::E0102, range, what));
            Ty::Error
        }
        [base, assoc] => {
            let b = path_type(std::slice::from_ref(base), base.range, scope, diags);
            if b == Ty::Error {
                return Ty::Error;
            }
            no_args(assoc, diags);
            assoc_type(&b, base.name == "Self", &assoc.name, range, scope, diags)
        }
        _ => {
            diags.push(Diagnostic::unsupported(range, "this path"));
            Ty::Error
        }
    }
}

/// `base::name`: an associated type of a trait `base` implements.
fn assoc_type(
    base: &Ty,
    is_self: bool,
    name: &str,
    range: TextRange,
    scope: &Scope,
    diags: &mut Vec<Diagnostic>,
) -> Ty {
    let tree = scope.tree;
    if is_self && let Some((tr, assoc)) = &scope.impl_assoc {
        let def = &tree.traits[tr.trait_ as usize];
        if let Some(i) = def.assoc.iter().position(|(n, _)| n == name) {
            return assoc.get(i).cloned().flatten().unwrap_or(Ty::Error);
        }
    }
    if is_self && let Some(t) = scope.trait_ctx {
        let def = &tree.traits[t as usize];
        if let Some(i) = def.assoc.iter().position(|(n, _)| n == name) {
            let args: Vec<Ty> = def.generics.identity().into_iter().skip(1).collect();
            return Ty::Assoc(Arc::new(AssocTy {
                self_ty: base.clone(),
                trait_ref: TraitRef {
                    trait_: t,
                    name: def.name.as_str().into(),
                    args: args.into(),
                },
                index: i as u32,
                name: name.into(),
            }));
        }
    }
    for p in elaborate(tree, scope.preds) {
        if p.ty != *base {
            continue;
        }
        let def = &tree.traits[p.trait_ref.trait_ as usize];
        if let Some(i) = def.assoc.iter().position(|(n, _)| n == name) {
            if let Some((_, t)) = p.assoc.iter().find(|(k, _)| *k == i as u32) {
                return t.clone();
            }
            return Ty::Assoc(Arc::new(AssocTy {
                self_ty: base.clone(),
                trait_ref: p.trait_ref.clone(),
                index: i as u32,
                name: name.into(),
            }));
        }
    }
    diags.push(Diagnostic::error(
        Code::E0413,
        range,
        format!("`{base}` has no associated type `{name}` through the bounds in scope"),
    ));
    Ty::Error
}

/// `<T as Trait>::Name`.
fn qualified_type(path: &SyntaxNode, scope: &Scope, diags: &mut Vec<Diagnostic>) -> Ty {
    let range = path.text_range();
    let Some(q) = path.child(K::QUALIFIED_PATH) else {
        return Ty::Error;
    };
    let base = match q.children().find_map(ast::Type::cast) {
        Some(t) => resolve_type(&t, scope, diags),
        None => return Ty::Error,
    };
    let Some(tpath) = q.child(K::PATH) else {
        diags.push(Diagnostic::unsupported(range, "this path"));
        return Ty::Error;
    };
    let Some((trait_ref, _)) = trait_path(&tpath, scope, diags) else {
        return Ty::Error;
    };
    let name = path
        .children()
        .filter(|c| c.kind() == K::PATH_SEGMENT)
        .last()
        .map(|s| s.text().trim().to_string());
    let Some(name) = name else {
        return Ty::Error;
    };
    let def = &scope.tree.traits[trait_ref.trait_ as usize];
    match def.assoc.iter().position(|(n, _)| *n == name) {
        Some(i) => Ty::Assoc(Arc::new(AssocTy {
            self_ty: base,
            trait_ref,
            index: i as u32,
            name: name.as_str().into(),
        })),
        None => {
            diags.push(Diagnostic::error(
                Code::E0413,
                range,
                format!("`{}` has no associated type `{name}`", def.name),
            ));
            Ty::Error
        }
    }
}

/// The trait named by the `PATH` `path` (`Clone`, `Add<Output = int>`), with the associated
/// types it fixes.
pub(crate) fn trait_path(
    path: &SyntaxNode,
    scope: &Scope,
    diags: &mut Vec<Diagnostic>,
) -> Option<(TraitRef, Vec<(u32, Ty)>)> {
    let range = path.text_range();
    let segs = path_segments(path)?;
    let [seg] = segs.as_slice() else {
        diags.push(Diagnostic::unsupported(range, "this path"));
        return None;
    };
    let Some(t) = scope.tree.trait_named(&seg.name) else {
        let what = if scope.tree.adt_named(&seg.name).is_some()
            || scope.param(&seg.name).is_some()
            || matches!(seg.name.as_str(), "int" | "bool")
        {
            format!("`{}` is a type, not a trait", seg.name)
        } else {
            format!("there is no trait called `{}`", seg.name)
        };
        diags.push(Diagnostic::error(Code::E0401, range, what));
        return None;
    };
    let def = &scope.tree.traits[t as usize];
    let want = def.generics.params.len() - 1;
    let (args, assoc) = match &seg.args {
        Some(a) => generic_args(a, scope, diags),
        None => (Vec::new(), Vec::new()),
    };
    let args = if arity(&format!("`{}`", def.name), want, args.len(), range, diags) {
        args
    } else {
        vec![Ty::Error; want]
    };
    let mut fixed = Vec::new();
    for (name, r, ty) in assoc {
        match def.assoc.iter().position(|(n, _)| *n == name) {
            Some(i) => fixed.push((i as u32, ty)),
            None => diags.push(Diagnostic::error(
                Code::E0413,
                r,
                format!("`{}` has no associated type `{name}`", def.name),
            )),
        }
    }
    Some((
        TraitRef {
            trait_: t,
            name: def.name.as_str().into(),
            args: args.into(),
        },
        fixed,
    ))
}

/// The bounds of `bounded` parameters and of the `where` clause of `item`.
fn preds_of(
    item: &SyntaxNode,
    bounded: &[(usize, SyntaxNode)],
    scope: &Scope,
    ds: &mut Vec<Diagnostic>,
) -> Vec<Pred> {
    let mut out = Vec::new();
    for (i, node) in bounded {
        let ty = Ty::Param(*i as u32, scope.params[*i].name.as_str().into());
        out.extend(bounds(node, &ty, scope, ds));
    }
    if let Some(w) = item.child(K::WHERE_CLAUSE) {
        for p in w.children().filter(|c| c.kind() == K::WHERE_PRED) {
            let Some(t) = p.children().find_map(ast::Type::cast) else {
                continue;
            };
            let ty = resolve_type(&t, scope, ds);
            out.extend(bounds(&p, &ty, scope, ds));
        }
    }
    out
}

/// The bounds `node` (a `TYPE_PARAM`, `WHERE_PRED` or trait item) puts on `ty`: its `BOUND`
/// children.
fn bounds(node: &SyntaxNode, ty: &Ty, scope: &Scope, diags: &mut Vec<Diagnostic>) -> Vec<Pred> {
    let mut out = Vec::new();
    for b in node.children().filter(|c| c.kind() == K::BOUND) {
        if b.child(K::FN_SUGAR).is_some() || b.token(K::L_PAREN).is_some() {
            diags.push(Diagnostic::unsupported(
                b.text_range(),
                "a function-type bound",
            ));
            continue;
        }
        let Some(path) = b.child(K::PATH) else {
            continue;
        };
        if let Some((trait_ref, assoc)) = trait_path(&path, scope, diags) {
            out.push(Pred {
                ty: ty.clone(),
                trait_ref,
                assoc,
                range: b.text_range(),
            });
        }
    }
    out
}

fn name_of(node: &SyntaxNode) -> (String, TextRange) {
    match ast::name(node) {
        Some(n) => (n.text(), n.syntax().text_range()),
        None => (String::new(), node.text_range()),
    }
}

/// The range of the item's first keyword, or of the whole item.
fn keyword_range(node: &SyntaxNode) -> TextRange {
    node.child_tokens()
        .find(|t| t.kind().is_keyword())
        .map_or_else(|| node.text_range(), |t| t.text_range())
}

/// The name a pattern binds and whether it is `mut`, if it is just a name (`x`, `mut x`) or `_`
/// (`None`). A name that is a unit struct or a variant named on its own is a pattern, not a
/// binding.
pub(crate) fn simple_binding(pat: &ast::Pat, tree: &ItemTree) -> Option<(Option<String>, bool)> {
    let node = pat.syntax();
    match pat {
        ast::Pat::Wildcard(_) => Some((None, false)),
        ast::Pat::Ident(n) if node.token(K::AT).is_none() && n.child(K::AT_PAT).is_none() => {
            let name = n.child(K::NAME).and_then(ast::Name::cast).map(|n| n.text());
            Some((name, n.token(K::MUT_KW).is_some()))
        }
        ast::Pat::Path(n) => {
            let segments = n
                .child(K::PATH)
                .and_then(ast::Path::cast)
                .map(|p| p.segments())
                .unwrap_or_default();
            match segments.as_slice() {
                [one] if tree.adt_named(one).is_none() && tree.bare_variant(one).is_none() => {
                    Some((Some(one.clone()), false))
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// The file's items, the prelude's, and the derived impls, with their signatures resolved.
#[salsa::tracked(returns(ref))]
pub fn item_tree(db: &dyn Db, file: SourceFile) -> ItemTree {
    let (expansion, dparse) = derived(db, file);
    let sources = [
        (Src::Prelude, prelude_parse(db, file).syntax()),
        (Src::File, parse(db, file).syntax()),
        (Src::Derived, dparse.syntax()),
    ];
    let mut c = Collector {
        tree: ItemTree::default(),
        expansion,
        diags: Vec::new(),
    };
    c.run(&sources);
    let mut tree = c.tree;
    tree.diagnostics = c.diags;
    tree
}

/// The parts of an item to collect, by source.
#[derive(Default)]
struct Nodes {
    adts: Vec<(Src, SyntaxNode)>,
    traits: Vec<(Src, SyntaxNode)>,
    impls: Vec<(Src, SyntaxNode)>,
    fns: Vec<(Src, SyntaxNode)>,
    tests: Vec<SyntaxNode>,
}

struct Collector<'a> {
    tree: ItemTree,
    expansion: &'a Expansion,
    diags: Vec<Diagnostic>,
}

impl Collector<'_> {
    /// Reports `ds`, found in source `src` (at `origin`, for a derived item): only problems in
    /// the file are reported, and those in derived code at the `derive` attribute.
    fn report(&mut self, src: Src, origin: Option<TextRange>, ds: Vec<Diagnostic>) {
        for mut d in ds {
            match src {
                Src::File => self.diags.push(d),
                Src::Derived => match origin {
                    Some(o) => {
                        d.range = o;
                        d.labels.clear();
                        self.diags.push(d);
                    }
                    None => self.tree.hidden.push(d),
                },
                Src::Prelude => self.tree.hidden.push(d),
            }
        }
    }

    /// For an item in the derived source: the `derive` attribute in the file it comes from.
    fn origin(&self, src: Src, range: TextRange) -> Option<TextRange> {
        if src != Src::Derived {
            return None;
        }
        self.expansion
            .origins
            .iter()
            .find(|(r, _, _)| r.start <= range.start && range.end <= r.end)
            .and_then(|&(_, s, attr)| (s == Src::File).then_some(attr))
    }

    fn run(&mut self, sources: &[(Src, SyntaxNode)]) {
        let mut nodes = Nodes::default();
        for (src, root) in sources {
            for node in root.children() {
                match node.kind() {
                    K::STRUCT_ITEM | K::ENUM_ITEM => nodes.adts.push((*src, node)),
                    K::TRAIT_ITEM => nodes.traits.push((*src, node)),
                    K::IMPL_ITEM => nodes.impls.push((*src, node)),
                    K::FN_ITEM => nodes.fns.push((*src, node)),
                    K::TEST_ITEM if *src == Src::File => nodes.tests.push(node),
                    K::INNER_ATTR => self.report(
                        *src,
                        None,
                        vec![Diagnostic::unsupported(node.text_range(), "an attribute")],
                    ),
                    K::ERROR => {}
                    _ => self.report(
                        *src,
                        None,
                        vec![Diagnostic::unsupported(
                            keyword_range(&node),
                            "this kind of item",
                        )],
                    ),
                }
            }
        }
        self.names(&nodes);
        for (src, node) in &nodes.adts {
            self.adt_skeleton(*src, node);
        }
        for (src, node) in &nodes.traits {
            self.trait_skeleton(*src, node);
        }
        for (i, (src, node)) in nodes.adts.iter().enumerate() {
            self.adt_fields(i, *src, node);
        }
        for (i, (src, node)) in nodes.traits.iter().enumerate() {
            self.trait_header(i, *src, node);
        }
        for (i, (src, node)) in nodes.traits.iter().enumerate() {
            self.trait_methods(i, *src, node);
        }
        for (src, node) in &nodes.impls {
            self.impl_item(*src, node);
        }
        for (src, node) in &nodes.fns {
            if let Some(f) = ast::FnItem::cast(node.clone()) {
                let sig = self.fn_sig(
                    &f,
                    *src,
                    Container::Free,
                    &Generics::default(),
                    None,
                    None,
                    None,
                );
                self.tree.fns.push(sig);
            }
        }
        for node in &nodes.tests {
            if let Some(t) = ast::TestItem::cast(node.clone()) {
                let sig = self.test_sig(&t);
                self.tree.tests.push(sig);
            }
        }
        self.recursive_types();
        self.copy_types();
        self.derived_fields();
        self.check_impls();
        self.overlap();
    }

    /// Item names share one namespace: two items of the file cannot have the same name, nor can
    /// an item of the file have the name of one of the prelude's.
    fn names(&mut self, nodes: &Nodes) {
        let mut prelude: HashMap<String, ()> = HashMap::new();
        let mut seen: HashMap<String, TextRange> = HashMap::new();
        let all = nodes.adts.iter().chain(&nodes.traits).chain(&nodes.fns);
        let mut found = Vec::new();
        for (src, node) in all {
            let (name, range) = name_of(node);
            if name.is_empty() {
                continue;
            }
            match src {
                Src::Prelude => {
                    prelude.insert(name, ());
                    if node.kind() == K::ENUM_ITEM && name_of(node).0 == "Option" {
                        for v in node
                            .child(K::VARIANT_LIST)
                            .into_iter()
                            .flat_map(|l| l.children().collect::<Vec<_>>())
                            .filter(|v| v.kind() == K::VARIANT)
                        {
                            prelude.insert(name_of(&v).0, ());
                        }
                    }
                }
                Src::File => found.push((name, range)),
                Src::Derived => {}
            }
        }
        for (name, range) in found {
            if prelude.contains_key(&name) {
                self.diags.push(Diagnostic::error(
                    Code::E0103,
                    range,
                    format!("`{name}` is defined by the prelude, so it cannot be defined again"),
                ));
            } else if let Some(first) = seen.get(&name) {
                self.diags.push(
                    Diagnostic::error(
                        Code::E0103,
                        range,
                        format!("`{name}` is defined more than once"),
                    )
                    .with_label(*first, "first defined here"),
                );
            } else {
                seen.insert(name, range);
            }
        }
    }

    /// The type parameters a `GENERIC_PARAMS` node declares, after `parent`'s, and the nodes of
    /// their bounds.
    fn params(
        &mut self,
        src: Src,
        origin: Option<TextRange>,
        node: Option<SyntaxNode>,
        parent: &[GenericParam],
    ) -> (Vec<GenericParam>, Vec<(usize, SyntaxNode)>) {
        let mut params = parent.to_vec();
        let mut bounded = Vec::new();
        let mut ds = Vec::new();
        if let Some(g) = node {
            for p in g.children() {
                match p.kind() {
                    K::TYPE_PARAM => {
                        let (name, range) = name_of(&p);
                        if let Some(first) = params.iter().find(|q| q.name == name) {
                            ds.push(
                                Diagnostic::error(
                                    Code::E0104,
                                    range,
                                    format!("the type parameter `{name}` is declared twice"),
                                )
                                .with_label(first.range, "first declared here"),
                            );
                        }
                        if p.token(K::EQ).is_some() {
                            ds.push(Diagnostic::unsupported(
                                p.text_range(),
                                "a default type parameter",
                            ));
                        }
                        bounded.push((params.len(), p.clone()));
                        params.push(GenericParam { name, range });
                    }
                    K::CONST_PARAM => {
                        ds.push(Diagnostic::unsupported(
                            p.text_range(),
                            "this kind of parameter",
                        ));
                    }
                    _ => {}
                }
            }
        }
        self.report(src, origin, ds);
        (params, bounded)
    }

    /// Reports the parts of an item this slice does not support. `attrs_handled` says the caller
    /// reads the item's attributes itself.
    fn unsupported_parts(&mut self, src: Src, node: &SyntaxNode, attrs_handled: bool) {
        let mut ds = Vec::new();
        for c in node.children() {
            let what = match c.kind() {
                K::OUTER_ATTR if attrs_handled => continue,
                K::OUTER_ATTR => "an attribute",
                K::EFFECT_CLAUSE => "an effect clause",
                K::CONTRACT => "a contract",
                K::RESUME_CLAUSE => "a `resume` clause",
                _ => continue,
            };
            ds.push(Diagnostic::unsupported(c.text_range(), what));
        }
        self.report(src, None, ds);
    }

    fn adt_skeleton(&mut self, src: Src, node: &SyntaxNode) {
        let (name, name_range) = name_of(node);
        let (params, bounded) = self.params(src, None, node.child(K::GENERIC_PARAMS), &[]);
        let mut ds = Vec::new();
        for (_, p) in &bounded {
            if p.child(K::BOUND).is_some() {
                ds.push(Diagnostic::unsupported(
                    p.text_range(),
                    "a bound on a struct's or enum's type parameter",
                ));
            }
        }
        if let Some(w) = node.child(K::WHERE_CLAUSE) {
            ds.push(Diagnostic::unsupported(
                w.text_range(),
                "a `where` clause on a struct or enum",
            ));
        }
        self.report(src, None, ds);
        let bare = src == Src::Prelude && name == "Option";
        self.tree.adts.push(AdtDef {
            name,
            name_range,
            src,
            is_enum: node.kind() == K::ENUM_ITEM,
            copy: false,
            generics: Generics {
                params,
                parent: 0,
                preds: Vec::new(),
            },
            variants: Vec::new(),
            bare_variants: bare,
        });
    }

    fn trait_skeleton(&mut self, src: Src, node: &SyntaxNode) {
        let (name, name_range) = name_of(node);
        let self_param = GenericParam {
            name: "Self".to_string(),
            range: name_range,
        };
        let (params, _) = self.params(src, None, node.child(K::GENERIC_PARAMS), &[self_param]);
        let mut assoc = Vec::new();
        let mut ds = Vec::new();
        if let Some(t) = node.token(K::UNSAFE_KW) {
            ds.push(Diagnostic::unsupported(t.text_range(), "an `unsafe` trait"));
        }
        for m in node
            .child(K::TRAIT_BODY)
            .into_iter()
            .flat_map(|b| b.children().collect::<Vec<_>>())
        {
            if m.kind() == K::ASSOC_TYPE {
                let (n, r) = name_of(&m);
                if m.child(K::BOUND).is_some() || m.token(K::COLON).is_some() {
                    ds.push(Diagnostic::unsupported(
                        m.text_range(),
                        "a bound on an associated type",
                    ));
                }
                if m.token(K::EQ).is_some() {
                    ds.push(Diagnostic::unsupported(
                        m.text_range(),
                        "a default for an associated type",
                    ));
                }
                if let Some((_, first)) = assoc.iter().find(|(x, _): &&(String, TextRange)| *x == n)
                {
                    ds.push(
                        Diagnostic::error(
                            Code::E0103,
                            r,
                            format!("the associated type `{n}` is declared twice"),
                        )
                        .with_label(*first, "first declared here"),
                    );
                }
                assoc.push((n, r));
            } else if m.kind() == K::CONST_ITEM {
                ds.push(Diagnostic::unsupported(
                    m.text_range(),
                    "an associated const",
                ));
            }
        }
        self.report(src, None, ds);
        self.tree.traits.push(TraitDef {
            name,
            name_range,
            src,
            generics: Generics {
                params,
                parent: 0,
                preds: Vec::new(),
            },
            supertraits: Vec::new(),
            assoc,
            methods: Vec::new(),
        });
    }

    fn adt_fields(&mut self, i: usize, src: Src, node: &SyntaxNode) {
        self.unsupported_parts(src, node, true);
        let mut ds = Vec::new();
        if let Some(t) = node.token(K::LINEAR_KW) {
            ds.push(Diagnostic::unsupported(t.text_range(), "a `linear` struct"));
        }
        let params = self.tree.adts[i].generics.params.clone();
        let scope = Scope {
            params: &params,
            ..Scope::new(&self.tree)
        };
        let variants = if node.kind() == K::STRUCT_ITEM {
            let (name, name_range) = name_of(node);
            vec![variant_def(node, name, name_range, &scope, &mut ds)]
        } else {
            let mut variants: Vec<VariantDef> = Vec::new();
            for v in node
                .child(K::VARIANT_LIST)
                .into_iter()
                .flat_map(|l| l.children().collect::<Vec<_>>())
                .filter(|c| c.kind() == K::VARIANT)
            {
                for c in v.children() {
                    if c.kind() == K::OUTER_ATTR {
                        ds.push(Diagnostic::unsupported(c.text_range(), "an attribute"));
                    }
                }
                if v.token(K::EQ).is_some() {
                    ds.push(Diagnostic::unsupported(
                        v.text_range(),
                        "an explicit discriminant",
                    ));
                }
                let (name, name_range) = name_of(&v);
                if let Some(first) = variants.iter().find(|x| x.name == name && !name.is_empty()) {
                    ds.push(
                        Diagnostic::error(
                            Code::E0103,
                            name_range,
                            format!("the variant `{name}` is defined more than once"),
                        )
                        .with_label(first.name_range, "first defined here"),
                    );
                }
                variants.push(variant_def(&v, name, name_range, &scope, &mut ds));
            }
            variants
        };
        // Attributes: only derives.
        let mut copy = false;
        for attr in node.children().filter(|c| c.kind() == K::OUTER_ATTR) {
            let is_derive = attr
                .child(K::ATTR)
                .and_then(|a| a.child(K::PATH))
                .and_then(ast::Path::cast)
                .is_some_and(|p| p.segments() == ["derive"]);
            if !is_derive {
                ds.push(Diagnostic::unsupported(attr.text_range(), "this attribute"));
            }
        }
        for (range, names) in derive::derive_attrs(node) {
            for n in &names {
                if !DERIVABLE.contains(&n.as_str()) {
                    ds.push(Diagnostic::unsupported(range, &format!("deriving `{n}`")));
                }
            }
            if names.iter().any(|n| n == "Copy") {
                copy = true;
                if !names.iter().any(|n| n == "Clone") {
                    ds.push(Diagnostic::error(
                        Code::E0311,
                        range,
                        "a `Copy` type is also `Clone`: write `#[derive(Copy, Clone)]`",
                    ));
                }
            }
        }
        self.report(src, None, ds);
        let def = &mut self.tree.adts[i];
        def.variants = variants;
        def.copy = copy;
    }

    fn trait_header(&mut self, i: usize, src: Src, node: &SyntaxNode) {
        self.unsupported_parts(src, node, false);
        let def = &self.tree.traits[i];
        let params = def.generics.params.clone();
        let bounded: Vec<(usize, SyntaxNode)> = node
            .child(K::GENERIC_PARAMS)
            .into_iter()
            .flat_map(|g| g.children().collect::<Vec<_>>())
            .filter(|c| c.kind() == K::TYPE_PARAM)
            .enumerate()
            .map(|(k, p)| (k + 1, p))
            .collect();
        let self_ty = Ty::Param(0, "Self".into());
        let scope = Scope {
            params: &params,
            self_ty: Some(self_ty.clone()),
            trait_ctx: Some(i as u32),
            ..Scope::new(&self.tree)
        };
        let mut ds = Vec::new();
        let supers: Vec<TraitRef> = bounds(node, &self_ty, &scope, &mut ds)
            .into_iter()
            .map(|p| p.trait_ref)
            .collect();
        let preds = preds_of(
            node,
            &bounded,
            &Scope {
                params: &params,
                self_ty: Some(self_ty),
                trait_ctx: Some(i as u32),
                ..Scope::new(&self.tree)
            },
            &mut ds,
        );
        self.report(src, None, ds);
        let def = &mut self.tree.traits[i];
        def.supertraits = supers;
        def.generics.preds = preds;
    }

    /// The bound a trait's items have on `Self`: `Self: Trait<…>`.
    fn self_pred(&self, t: u32) -> Pred {
        let def = &self.tree.traits[t as usize];
        let args: Vec<Ty> = def.generics.identity().into_iter().skip(1).collect();
        Pred {
            ty: Ty::Param(0, "Self".into()),
            trait_ref: TraitRef {
                trait_: t,
                name: def.name.as_str().into(),
                args: args.into(),
            },
            assoc: Vec::new(),
            range: def.name_range,
        }
    }

    fn trait_methods(&mut self, i: usize, src: Src, node: &SyntaxNode) {
        let mut generics = self.tree.traits[i].generics.clone();
        generics.preds.insert(0, self.self_pred(i as u32));
        generics.parent = generics.params.len() as u32;
        let mut names: HashMap<String, TextRange> = HashMap::new();
        for m in node
            .child(K::TRAIT_BODY)
            .into_iter()
            .flat_map(|b| b.children().collect::<Vec<_>>())
        {
            let Some(f) = ast::FnItem::cast(m) else {
                continue;
            };
            let sig = self.fn_sig(
                &f,
                src,
                Container::Trait(i as u32),
                &generics,
                Some(&Ty::Param(0, "Self".into())),
                Some(i as u32),
                None,
            );
            if let Some(first) = names.get(&sig.name) {
                let d = Diagnostic::error(
                    Code::E0103,
                    sig.name_range,
                    format!("the method `{}` is declared twice", sig.name),
                )
                .with_label(*first, "first declared here");
                self.report(src, None, vec![d]);
            } else {
                names.insert(sig.name.clone(), sig.name_range);
            }
            self.tree.traits[i].methods.push(self.tree.fns.len() as u32);
            self.tree.fns.push(sig);
        }
    }

    fn impl_item(&mut self, src: Src, node: &SyntaxNode) {
        let origin = self.origin(src, node.text_range());
        self.unsupported_parts(src, node, false);
        let index = self.tree.impls.len() as u32;
        let (params, bounded) = self.params(src, origin, node.child(K::GENERIC_PARAMS), &[]);
        let types: Vec<ast::Type> = node.children().filter_map(ast::Type::cast).collect();
        let is_trait = node.token(K::FOR_KW).is_some();
        let mut ds = Vec::new();
        if let Some(t) = node.token(K::UNSAFE_KW) {
            ds.push(Diagnostic::unsupported(t.text_range(), "an `unsafe` impl"));
        }
        let scope = Scope {
            params: &params,
            ..Scope::new(&self.tree)
        };
        let self_node = if is_trait {
            types.get(1)
        } else {
            types.first()
        };
        let self_ty = match self_node {
            Some(t) => resolve_type(t, &scope, &mut ds),
            None => Ty::Error,
        };
        let header_end = self_node.map_or(node.text_range().end, |t| t.syntax().text_range().end);
        let start = node
            .child_tokens()
            .find(|t| matches!(t.kind(), K::IMPL_KW | K::UNSAFE_KW))
            .map_or(node.text_range().start, |t| t.text_range().start);
        let range = TextRange::new(start, header_end);
        let trait_ref = if is_trait {
            types
                .first()
                .and_then(|t| t.syntax().child(K::PATH))
                .and_then(|p| {
                    let r = trait_path(&p, &scope, &mut ds);
                    if let Some((_, fixed)) = &r
                        && !fixed.is_empty()
                    {
                        ds.push(Diagnostic::error(
                            Code::E0413,
                            p.text_range(),
                            "an impl defines associated types in its body, not in the trait's \
                             arguments",
                        ));
                    }
                    r.map(|(t, _)| t)
                })
        } else {
            None
        };
        self.report(src, origin, ds);
        // An impl of something that is not a trait is already reported; it is not inherent.
        let self_ty = if is_trait && trait_ref.is_none() {
            Ty::Error
        } else {
            self_ty
        };
        let scope = Scope {
            params: &params,
            self_ty: Some(self_ty.clone()),
            ..Scope::new(&self.tree)
        };
        let mut ds = Vec::new();
        let preds = preds_of(node, &bounded, &scope, &mut ds);
        // Every parameter must be fixed by the self type or the trait's arguments.
        for (k, p) in params.iter().enumerate() {
            let me = Ty::Param(k as u32, p.name.as_str().into());
            let used = self_ty.any(&|t| *t == me)
                || trait_ref
                    .as_ref()
                    .is_some_and(|t| t.args.iter().any(|a| a.any(&|x| *x == me)));
            if !used {
                ds.push(Diagnostic::error(
                    Code::E0415,
                    p.range,
                    format!(
                        "`{}` is not used by the type or trait the impl is for, so nothing can \
                         say what it is",
                        p.name
                    ),
                ));
            }
        }
        // Associated types.
        let n_assoc = trait_ref
            .as_ref()
            .map_or(0, |t| self.tree.traits[t.trait_ as usize].assoc.len());
        let mut assoc: Vec<Option<Ty>> = vec![None; n_assoc];
        let members: Vec<SyntaxNode> = node
            .child(K::IMPL_BODY)
            .into_iter()
            .flat_map(|b| b.children().collect::<Vec<_>>())
            .collect();
        for m in &members {
            if m.token(K::DEFAULT_KW).is_some() {
                ds.push(Diagnostic::unsupported(
                    m.text_range(),
                    "specialisation (`default`)",
                ));
            }
            match m.kind() {
                K::ASSOC_TYPE => {
                    let (n, r) = name_of(m);
                    let Some(tr) = &trait_ref else {
                        ds.push(Diagnostic::unsupported(
                            m.text_range(),
                            "an associated type in an inherent impl",
                        ));
                        continue;
                    };
                    let tdef = &self.tree.traits[tr.trait_ as usize];
                    let Some(k) = tdef.assoc.iter().position(|(x, _)| *x == n) else {
                        ds.push(Diagnostic::error(
                            Code::E0408,
                            r,
                            format!("`{}` has no associated type `{n}`", tdef.name),
                        ));
                        continue;
                    };
                    let ty = match m.children().find_map(ast::Type::cast) {
                        Some(t) => resolve_type(&t, &scope, &mut ds),
                        None => Ty::Error,
                    };
                    assoc[k] = Some(ty);
                }
                K::CONST_ITEM => {
                    ds.push(Diagnostic::unsupported(
                        m.text_range(),
                        "an associated const",
                    ));
                }
                _ => {}
            }
        }
        self.report(src, origin, ds);
        self.tree.impls.push(ImplDef {
            src,
            range,
            generics: Generics {
                params: params.clone(),
                parent: 0,
                preds: preds.clone(),
            },
            trait_ref: trait_ref.clone(),
            self_ty: self_ty.clone(),
            assoc: assoc.clone(),
            methods: Vec::new(),
            origin,
        });
        // Methods.
        let generics = Generics {
            parent: params.len() as u32,
            params,
            preds,
        };
        let mut names: HashMap<String, TextRange> = HashMap::new();
        for m in members {
            let Some(f) = ast::FnItem::cast(m) else {
                continue;
            };
            let sig = self.fn_sig(
                &f,
                src,
                Container::Impl(index),
                &generics,
                Some(&self_ty),
                None,
                trait_ref.clone().map(|t| (t, assoc.clone())),
            );
            if let Some(first) = names.get(&sig.name) {
                let d = Diagnostic::error(
                    Code::E0103,
                    sig.name_range,
                    format!("the method `{}` is defined twice", sig.name),
                )
                .with_label(*first, "first defined here");
                self.report(src, origin, vec![d]);
            } else {
                names.insert(sig.name.clone(), sig.name_range);
            }
            self.tree.impls[index as usize]
                .methods
                .push(self.tree.fns.len() as u32);
            self.tree.fns.push(sig);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn fn_sig(
        &mut self,
        f: &ast::FnItem,
        src: Src,
        container: Container,
        parent: &Generics,
        self_ty: Option<&Ty>,
        trait_ctx: Option<u32>,
        impl_assoc: Option<(TraitRef, Vec<Option<Ty>>)>,
    ) -> FnSig {
        let self_ty = self_ty.cloned();
        let node = f.syntax();
        let origin = self.origin(src, node.text_range());
        self.unsupported_parts(src, node, false);
        let mut ds = Vec::new();
        for t in node.child_tokens() {
            if t.kind() == K::FN_KW {
                break;
            }
            if t.kind().is_keyword() && !matches!(t.kind(), K::PUB_KW | K::MOD_KW | K::DEFAULT_KW) {
                ds.push(Diagnostic::unsupported(
                    t.text_range(),
                    &format!("`{}` on a function", t.text()),
                ));
            }
        }
        let (name, name_range) = f.name().map_or_else(
            || (String::new(), node.text_range()),
            |n| (n.text(), n.syntax().text_range()),
        );
        let has_body = f.body().is_some();
        if !has_body && !matches!(container, Container::Trait(_)) {
            ds.push(Diagnostic::unsupported(
                name_range,
                "a function without a body",
            ));
        }
        self.report(src, origin, ds);
        let (params, bounded) =
            self.params(src, origin, node.child(K::GENERIC_PARAMS), &parent.params);
        let mut generics = Generics {
            params,
            parent: parent.params.len() as u32,
            preds: parent.preds.clone(),
        };
        let mut ds = Vec::new();
        let own = {
            let scope = Scope {
                params: &generics.params,
                preds: &generics.preds,
                self_ty: self_ty.clone(),
                trait_ctx,
                impl_assoc: impl_assoc.clone(),
                ..Scope::new(&self.tree)
            };
            preds_of(node, &bounded, &scope, &mut ds)
        };
        self.report(src, origin, ds);
        generics.preds.extend(own);
        let scope = Scope {
            params: &generics.params,
            preds: &generics.preds,
            self_ty: self_ty.clone(),
            trait_ctx,
            impl_assoc,
            ..Scope::new(&self.tree)
        };
        let mut ds = Vec::new();
        let mut params = Vec::new();
        let mut self_kind = None;
        let mut names: HashMap<String, TextRange> = HashMap::new();
        if let Some(list) = f.params() {
            for c in list.syntax().children() {
                match c.kind() {
                    K::PARAM => {
                        let p = ast::Param::cast(c).expect("a PARAM");
                        let sig = param_sig(&p, &scope, &mut ds);
                        if let Some(n) = &sig.name {
                            if let Some(first) = names.get(n) {
                                ds.push(
                                    Diagnostic::error(
                                        Code::E0104,
                                        sig.range,
                                        format!("the parameter `{n}` is declared twice"),
                                    )
                                    .with_label(*first, "first declared here"),
                                );
                            } else {
                                names.insert(n.clone(), sig.range);
                            }
                        }
                        params.push(sig);
                    }
                    K::SELF_PARAM => {
                        let Some(st) = &self_ty else {
                            ds.push(Diagnostic::error(
                                Code::E0102,
                                c.text_range(),
                                "only a method of an impl or trait takes `self`",
                            ));
                            continue;
                        };
                        if !params.is_empty() {
                            ds.push(Diagnostic::error(
                                Code::E0001,
                                c.text_range(),
                                "`self` must be the first parameter",
                            ));
                            continue;
                        }
                        if c.children().any(|x| ast::Type::can_cast(x.kind())) {
                            ds.push(Diagnostic::unsupported(
                                c.text_range(),
                                "a `self` parameter with a type",
                            ));
                        }
                        let borrowed = c.token(K::AMP).is_some();
                        let mutable = c.token(K::MUT_KW).is_some();
                        let (kind, ty) = match (borrowed, mutable) {
                            (true, true) => (SelfKind::RefMut, Ty::reference(true, st.clone())),
                            (true, false) => (SelfKind::Ref, Ty::reference(false, st.clone())),
                            (false, _) => (SelfKind::Value, st.clone()),
                        };
                        self_kind = Some(kind);
                        names.insert("self".to_string(), c.text_range());
                        params.push(ParamSig {
                            name: Some("self".to_string()),
                            mutable: mutable && !borrowed,
                            range: c.text_range(),
                            ty,
                            destructures: false,
                        });
                    }
                    K::NAMED_ONLY_MARKER => ds.push(Diagnostic::unsupported(
                        c.text_range(),
                        "a name-only parameter marker",
                    )),
                    _ => {}
                }
            }
        }
        let (ret, ret_range) = match f.ret_type().and_then(|r| r.ty()) {
            Some(t) => (
                resolve_type(&t, &scope, &mut ds),
                Some(t.syntax().text_range()),
            ),
            None => (Ty::Unit, None),
        };
        let ret_from = ret_sources(node, &params, &ret, ret_range, &mut ds);
        self.report(src, origin, ds);
        FnSig {
            name,
            name_range,
            src,
            node: node.text_range(),
            container,
            generics,
            self_kind,
            params,
            ret,
            ret_range,
            ret_from,
            has_body,
            origin,
        }
    }

    fn test_sig(&mut self, t: &ast::TestItem) -> TestSig {
        let node = t.syntax();
        self.unsupported_parts(Src::File, node, false);
        if node.child(K::GENERIC_PARAMS).is_some() || node.child(K::WHERE_CLAUSE).is_some() {
            self.diags
                .push(Diagnostic::unsupported(node.text_range(), "a generic test"));
        }
        if node
            .child_tokens()
            .any(|t| matches!(t.kind(), K::FOR_KW | K::WITH_KW))
        {
            self.diags.push(Diagnostic::unsupported(
                node.text_range(),
                "a test with `for` or `with`",
            ));
        }
        let sig = match t.name_token() {
            Some(tok) => {
                let text = tok.text();
                let name = text
                    .strip_prefix('"')
                    .and_then(|s| s.strip_suffix('"'))
                    .unwrap_or(text)
                    .to_string();
                TestSig {
                    name,
                    name_range: tok.text_range(),
                }
            }
            None => TestSig {
                name: String::new(),
                name_range: node.text_range(),
            },
        };
        if let Some(first) = self.tree.tests.iter().find(|s| s.name == sig.name) {
            self.diags.push(
                Diagnostic::error(
                    Code::E0103,
                    sig.name_range,
                    format!("a test called \"{}\" already exists", sig.name),
                )
                .with_label(first.name_range, "first defined here"),
            );
        }
        sig
    }

    /// Reports every struct or enum that contains itself without indirection (E0211), and makes
    /// the fields of each an error so that later passes need not care.
    fn recursive_types(&mut self) {
        fn contains(t: &Ty, out: &mut Vec<u32>) {
            match t {
                Ty::Adt(i, _, args) => {
                    out.push(*i);
                    args.iter().for_each(|t| contains(t, out));
                }
                Ty::Tuple(ts) => ts.iter().for_each(|t| contains(t, out)),
                _ => {}
            }
        }
        let tree = &mut self.tree;
        let edges: Vec<Vec<u32>> = tree
            .adts
            .iter()
            .map(|a| {
                let mut out = Vec::new();
                for f in a.variants.iter().flat_map(|v| &v.fields) {
                    contains(&f.ty, &mut out);
                }
                out
            })
            .collect();
        let mut cyclic = Vec::new();
        for start in 0..tree.adts.len() {
            let mut seen = vec![false; tree.adts.len()];
            let mut work: Vec<u32> = edges[start].clone();
            while let Some(n) = work.pop() {
                if n as usize == start {
                    cyclic.push(start);
                    break;
                }
                if !std::mem::replace(&mut seen[n as usize], true) {
                    work.extend(&edges[n as usize]);
                }
            }
        }
        let mut found = Vec::new();
        for start in cyclic {
            let a = &mut tree.adts[start];
            let d = Diagnostic::error(
                Code::E0211,
                a.name_range,
                format!(
                    "`{}` contains itself, so it would have infinite size",
                    a.name
                ),
            );
            let src = a.src;
            for f in a.variants.iter_mut().flat_map(|v| &mut v.fields) {
                f.ty = Ty::Error;
            }
            found.push((src, d));
        }
        for (src, d) in found {
            self.report(src, None, vec![d]);
        }
    }

    /// Checks the structs and enums that are `Copy` (derived, or by a hand-written impl): every
    /// field must be `Copy`, the type at most 64 bytes, and it may not have a destructor. A
    /// type parameter counts as `Copy`, since the impl requires it to be.
    fn copy_types(&mut self) {
        let Some(copy) = self.tree.lang("Copy") else {
            return;
        };
        let mut found: Vec<(Src, Option<TextRange>, Diagnostic)> = Vec::new();
        for imp in &self.tree.impls {
            if imp.trait_ref.as_ref().map(|t| t.trait_) != Some(copy) {
                continue;
            }
            let Some(a) = imp.self_ty.adt() else {
                continue;
            };
            let def = &self.tree.adts[a as usize];
            // A derived impl is reported at the type's name, a hand-written one at the impl.
            let (src, at) = if imp.origin.is_some() {
                (def.src, def.name_range)
            } else {
                (imp.src, imp.range)
            };
            let mut solver = Solver::new(&self.tree, &imp.generics.preds);
            let bad: Vec<String> = (0..def.variants.len() as u32)
                .flat_map(|v| {
                    let types = self.tree.field_types(&imp.self_ty, v);
                    self.tree
                        .variant(a, v)
                        .fields
                        .iter()
                        .zip(types)
                        .collect::<Vec<_>>()
                })
                .filter(|(_, t)| !matches!(t, Ty::Param(..)) && !solver.is_copy(t))
                .map(|(f, t)| format!("`{}: {t}`", f.name))
                .collect();
            if !bad.is_empty() {
                found.push((
                    src,
                    None,
                    Diagnostic::error(
                        Code::E0311,
                        at,
                        format!(
                            "`{}` cannot be `Copy`: the field {} is not `Copy`",
                            def.name,
                            bad.join(", ")
                        ),
                    ),
                ));
                continue;
            }
            if self.tree.has_drop_impl(a) {
                found.push((
                    src,
                    None,
                    Diagnostic::error(
                        Code::E0411,
                        at,
                        format!(
                            "`{}` has a destructor, so it cannot be `Copy`: a copy would run it \
                             twice",
                            def.name
                        ),
                    ),
                ));
                continue;
            }
            let size = self.tree.size_of(&imp.self_ty).0;
            if size > COPY_CAP {
                found.push((
                    src,
                    None,
                    Diagnostic::error(
                        Code::E0312,
                        at,
                        format!(
                            "`{}` is {size} bytes, over the {COPY_CAP}-byte cap for `Copy` types; \
                             copy it with `.clone()` instead",
                            def.name
                        ),
                    ),
                ));
            }
        }
        for (src, origin, d) in found {
            self.report(src, origin, vec![d]);
        }
    }

    /// A derive needs every field to implement the trait derived (E0402, at the attribute). A
    /// type parameter's field counts, since the impl requires it to. `Clone` with `Copy` is
    /// left to the `Copy` check.
    fn derived_fields(&mut self) {
        let mut found = Vec::new();
        let tree = &self.tree;
        for imp in &tree.impls {
            let (Some(origin), Some(tr), Some(a)) = (imp.origin, &imp.trait_ref, imp.self_ty.adt())
            else {
                continue;
            };
            let def = &tree.adts[a as usize];
            let name = tr.name.as_ref();
            if matches!(name, "Copy" | "Eq") || (name == "Clone" && def.copy) {
                continue;
            }
            let mut solver = Solver::new(tree, &imp.generics.preds);
            let bad = (0..def.variants.len() as u32)
                .flat_map(|v| tree.field_types(&imp.self_ty, v))
                .find(|t| !matches!(t, Ty::Param(..)) && solver.implements(t, tr) == Answer::No);
            if let Some(t) = bad {
                found.push(Diagnostic::error(
                    Code::E0402,
                    origin,
                    format!(
                        "`{}` cannot derive `{tr}`: its field of type `{t}` does not implement it",
                        def.name
                    ),
                ));
            }
        }
        self.diags.extend(found);
    }

    /// Checks each impl of a trait against the trait: it defines every required method and
    /// associated type and nothing else, its methods have the trait's signatures, and the type
    /// implements the trait's supertraits. Destructors may only be written for the impl's own
    /// struct or enum, for all its type arguments.
    fn check_impls(&mut self) {
        let mut found: Vec<(Src, Option<TextRange>, Diagnostic)> = Vec::new();
        let tree = &self.tree;
        let drop = tree.lang("Drop");
        for imp in &tree.impls {
            let push = |found: &mut Vec<_>, d: Diagnostic| found.push((imp.src, imp.origin, d));
            let Some(tr) = &imp.trait_ref else {
                // Inherent impls: only for the file's own structs and enums.
                match imp.self_ty.adt() {
                    Some(a) if tree.adts[a as usize].src == imp.src => {}
                    Some(_) | None if imp.self_ty == Ty::Error => {}
                    _ => push(
                        &mut found,
                        Diagnostic::error(
                            Code::E0414,
                            imp.range,
                            format!(
                                "methods can be added only to a struct or enum defined here, not \
                                 to `{}`; define a trait for them instead",
                                imp.self_ty
                            ),
                        ),
                    ),
                }
                continue;
            };
            let tdef = &tree.traits[tr.trait_ as usize];
            // Associated types.
            for (k, (n, _)) in tdef.assoc.iter().enumerate() {
                if imp.assoc.get(k).is_none_or(Option::is_none) {
                    push(
                        &mut found,
                        Diagnostic::error(
                            Code::E0407,
                            imp.range,
                            format!("the impl of `{tr}` must define the associated type `{n}`"),
                        ),
                    );
                }
            }
            // Methods.
            let mut args = vec![imp.self_ty.clone()];
            args.extend(tr.args.iter().cloned());
            for &tm in &tdef.methods {
                let tsig = &tree.fns[tm as usize];
                let Some(&im) = imp
                    .methods
                    .iter()
                    .find(|&&m| tree.fns[m as usize].name == tsig.name)
                else {
                    if !tsig.has_body {
                        push(
                            &mut found,
                            Diagnostic::error(
                                Code::E0407,
                                imp.range,
                                format!(
                                    "the impl of `{tr}` must define the method `{}`",
                                    tsig.name
                                ),
                            ),
                        );
                    }
                    continue;
                };
                let isig = &tree.fns[im as usize];
                if let Some(d) = signature_mismatch(tree, &args, tsig, isig) {
                    found.push((isig.src, isig.origin, d));
                }
            }
            for &im in &imp.methods {
                let isig = &tree.fns[im as usize];
                if !tdef
                    .methods
                    .iter()
                    .any(|&m| tree.fns[m as usize].name == isig.name)
                {
                    found.push((
                        isig.src,
                        isig.origin,
                        Diagnostic::error(
                            Code::E0408,
                            isig.name_range,
                            format!("`{tr}` has no method `{}`", isig.name),
                        ),
                    ));
                }
            }
            // Supertraits.
            let mut solver = Solver::new(tree, &imp.generics.preds);
            for s in &tdef.supertraits {
                let s = s.subst(&args);
                if solver.implements(&imp.self_ty, &s) == Answer::No {
                    push(
                        &mut found,
                        Diagnostic::error(
                            Code::E0402,
                            imp.range,
                            format!(
                                "`{}` must implement `{s}` to implement `{tr}`, and it does not",
                                imp.self_ty
                            ),
                        ),
                    );
                }
            }
            // Destructors.
            if Some(tr.trait_) == drop {
                let own = imp.self_ty.adt().is_some_and(|a| {
                    let def = &tree.adts[a as usize];
                    def.src == imp.src && imp.self_ty.args() == imp.generics.identity().as_slice()
                });
                if !own && imp.self_ty != Ty::Error {
                    push(
                        &mut found,
                        Diagnostic::error(
                            Code::E0411,
                            imp.range,
                            "a destructor is written for a struct or enum defined here, for all \
                             its type arguments: `impl<T> Drop for Name<T>`",
                        ),
                    );
                } else if !imp.generics.preds.is_empty() {
                    push(
                        &mut found,
                        Diagnostic::error(
                            Code::E0411,
                            imp.range,
                            "a destructor cannot have bounds: it runs for every value of the type",
                        ),
                    );
                }
            }
        }
        for (src, origin, d) in found {
            self.report(src, origin, vec![d]);
        }
    }

    /// Two impls of the same trait that could apply to the same type (E0406), and inherent
    /// methods defined twice for the same type (E0103).
    fn overlap(&mut self) {
        let tree = &self.tree;
        let mut found = Vec::new();
        for (j, b) in tree.impls.iter().enumerate() {
            for a in &tree.impls[..j] {
                if a.src == Src::Prelude && b.src == Src::Prelude {
                    continue;
                }
                let same_trait = match (&a.trait_ref, &b.trait_ref) {
                    (Some(x), Some(y)) => x.trait_ == y.trait_,
                    (None, None) => true,
                    _ => false,
                };
                if !same_trait || a.self_ty == Ty::Error || b.self_ty == Ty::Error {
                    continue;
                }
                let mut s = Solver::new(tree, &[]);
                let xa = s.fresh_n(a.generics.params.len());
                let xb = s.fresh_n(b.generics.params.len());
                let mut overlaps = s.unify(&a.self_ty.subst(&xa), &b.self_ty.subst(&xb));
                if let (Some(x), Some(y)) = (&a.trait_ref, &b.trait_ref) {
                    overlaps &= x
                        .subst(&xa)
                        .args
                        .iter()
                        .zip(y.subst(&xb).args.iter())
                        .all(|(p, q)| s.unify(p, q));
                }
                if !overlaps {
                    continue;
                }
                match &b.trait_ref {
                    Some(t) => found.push((
                        b.src,
                        b.origin,
                        Diagnostic::error(
                            Code::E0406,
                            b.range,
                            format!(
                                "`{t}` is already implemented for `{}`: a type has at most one \
                                 impl of a trait",
                                b.self_ty
                            ),
                        )
                        .with_label(a.range, "the other impl"),
                    )),
                    None => {
                        for &m in &b.methods {
                            let name = &tree.fns[m as usize].name;
                            if let Some(&first) = a
                                .methods
                                .iter()
                                .find(|&&x| tree.fns[x as usize].name == *name)
                            {
                                found.push((
                                    b.src,
                                    b.origin,
                                    Diagnostic::error(
                                        Code::E0103,
                                        tree.fns[m as usize].name_range,
                                        format!(
                                            "`{}` already has a method called `{name}`",
                                            b.self_ty
                                        ),
                                    )
                                    .with_label(
                                        tree.fns[first as usize].name_range,
                                        "first defined here",
                                    ),
                                ));
                            }
                        }
                    }
                }
            }
        }
        for (src, origin, d) in found {
            self.report(src, origin, vec![d]);
        }
    }
}

/// Why impl method `isig` does not have the signature of trait method `tsig`, if it does not.
/// `args` are `Self` and the trait's arguments.
fn signature_mismatch(
    tree: &ItemTree,
    args: &[Ty],
    tsig: &FnSig,
    isig: &FnSig,
) -> Option<Diagnostic> {
    let at = isig.name_range;
    let mismatch = |what: String| {
        Some(
            Diagnostic::error(
                Code::E0409,
                at,
                format!("`{}` does not match the trait: {what}", isig.name),
            )
            .with_label(tsig.name_range, "declared here"),
        )
    };
    if tsig.self_kind != isig.self_kind {
        return mismatch("it takes `self` differently".to_string());
    }
    if tsig.params.len() != isig.params.len() {
        return mismatch(format!(
            "it has {} parameters, and the trait's has {}",
            isig.params.len(),
            tsig.params.len()
        ));
    }
    let t_own = tsig.generics.params.len() - tsig.generics.parent as usize;
    let i_own = isig.generics.params.len() - isig.generics.parent as usize;
    if t_own != i_own {
        return mismatch(format!(
            "it has {i_own} type parameters, and the trait's has {t_own}"
        ));
    }
    // The trait's method with `Self` and the trait's parameters replaced, and its own type
    // parameters renamed to the impl method's.
    let mut subst = args.to_vec();
    for k in 0..t_own {
        let p = &isig.generics.params[isig.generics.parent as usize + k];
        subst.push(Ty::Param(
            isig.generics.parent + k as u32,
            p.name.as_str().into(),
        ));
    }
    let mut solver = Solver::new(tree, &isig.generics.preds);
    let mut same = |a: &Ty, b: &Ty| {
        let a = solver.deep(&a.subst(&subst));
        let b = solver.deep(b);
        a == b || a.has_error() || b.has_error()
    };
    for (k, (tp, ip)) in tsig.params.iter().zip(&isig.params).enumerate() {
        if !same(&tp.ty, &ip.ty) {
            let t = tp.ty.subst(&subst);
            return mismatch(format!(
                "parameter {} has type `{}`, and the trait's has `{t}`",
                k + 1,
                ip.ty
            ));
        }
    }
    if !same(&tsig.ret, &isig.ret) {
        return mismatch(format!(
            "it returns `{}`, and the trait's returns `{}`",
            isig.ret,
            tsig.ret.subst(&subst)
        ));
    }
    None
}

/// The parameters a returned borrow may come from (`memory.md`: Returning borrows): those a `from`
/// clause names, which must be borrowed parameters, or else every borrowed parameter. A function
/// that returns a borrow needs at least one (E0310).
fn ret_sources(
    node: &SyntaxNode,
    params: &[ParamSig],
    ret: &Ty,
    ret_range: Option<TextRange>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<u32> {
    let borrowed: Vec<u32> = params
        .iter()
        .enumerate()
        .filter(|(_, p)| matches!(p.ty, Ty::Ref(..)))
        .map(|(i, _)| i as u32)
        .collect();
    let Some(clause) = node.child(K::FROM_CLAUSE) else {
        if ret.has_ref() && borrowed.is_empty() {
            diagnostics.push(Diagnostic::error(
                Code::E0310,
                ret_range.unwrap_or_else(|| node.text_range()),
                "a function returning a borrow needs a borrowed parameter for it to come from",
            ));
        }
        return borrowed;
    };
    if !ret.has_ref() {
        diagnostics.push(Diagnostic::error(
            Code::E0310,
            clause.text_range(),
            "`from` names where a returned borrow comes from, but this function does not return \
             one",
        ));
    }
    let mut from = Vec::new();
    for t in clause
        .child_tokens()
        .filter(|t| matches!(t.kind(), K::IDENT | K::SELF_KW))
    {
        match params
            .iter()
            .position(|p| p.name.as_deref() == Some(t.text()))
        {
            Some(i) if borrowed.contains(&(i as u32)) => from.push(i as u32),
            Some(_) => diagnostics.push(Diagnostic::error(
                Code::E0310,
                t.text_range(),
                format!(
                    "`{}` is not a borrowed parameter, so a returned borrow cannot come from it",
                    t.text()
                ),
            )),
            None => diagnostics.push(Diagnostic::error(
                Code::E0101,
                t.text_range(),
                format!("there is no parameter called `{}`", t.text()),
            )),
        }
    }
    from
}

/// A struct's or variant's fields, from its `STRUCT_BODY` or `TUPLE_FIELDS` child.
fn variant_def(
    node: &SyntaxNode,
    name: String,
    name_range: TextRange,
    scope: &Scope,
    diagnostics: &mut Vec<Diagnostic>,
) -> VariantDef {
    let mut fields: Vec<FieldDef> = Vec::new();
    let shape = if let Some(body) = node.child(K::STRUCT_BODY) {
        for m in body.children() {
            if m.kind() != K::FIELD {
                if m.kind() != K::OUTER_ATTR {
                    diagnostics.push(Diagnostic::unsupported(m.text_range(), "an invariant"));
                }
                continue;
            }
            for c in m.children() {
                if c.kind() == K::OUTER_ATTR {
                    diagnostics.push(Diagnostic::unsupported(c.text_range(), "an attribute"));
                }
            }
            if m.token(K::EQ).is_some() {
                diagnostics.push(Diagnostic::unsupported(
                    m.text_range(),
                    "a field's default value",
                ));
            }
            let (fname, range) = name_of(&m);
            let ty = match m.children().find_map(ast::Type::cast) {
                Some(t) => field_type(&t, scope, diagnostics),
                None => Ty::Error,
            };
            if let Some(first) = fields.iter().find(|f| f.name == fname) {
                diagnostics.push(
                    Diagnostic::error(
                        Code::E0103,
                        range,
                        format!("the field `{fname}` is declared more than once"),
                    )
                    .with_label(first.range, "first declared here"),
                );
            }
            fields.push(FieldDef {
                name: fname,
                range,
                ty,
            });
        }
        Shape::Named
    } else if let Some(list) = node.child(K::TUPLE_FIELDS) {
        for (i, f) in list
            .children()
            .filter(|c| c.kind() == K::TUPLE_FIELD)
            .enumerate()
        {
            for c in f.children() {
                if c.kind() == K::OUTER_ATTR {
                    diagnostics.push(Diagnostic::unsupported(c.text_range(), "an attribute"));
                }
            }
            let ty = match f.children().find_map(ast::Type::cast) {
                Some(t) => field_type(&t, scope, diagnostics),
                None => Ty::Error,
            };
            fields.push(FieldDef {
                name: i.to_string(),
                range: f.text_range(),
                ty,
            });
        }
        Shape::Tuple
    } else {
        Shape::Unit
    };
    VariantDef {
        name,
        name_range,
        shape,
        fields,
    }
}

/// A field's type: borrows are second-class, so a field cannot hold one (E0309).
fn field_type(t: &ast::Type, scope: &Scope, diagnostics: &mut Vec<Diagnostic>) -> Ty {
    let ty = resolve_type(t, scope, diagnostics);
    if ty.has_ref() {
        diagnostics.push(Diagnostic::error(
            Code::E0309,
            t.syntax().text_range(),
            "a field cannot hold a borrow: borrows live only in parameters and locals",
        ));
        return Ty::Error;
    }
    ty
}

fn param_sig(p: &ast::Param, scope: &Scope, diagnostics: &mut Vec<Diagnostic>) -> ParamSig {
    let node = p.syntax();
    for c in node.children() {
        if c.kind() == K::OUTER_ATTR {
            diagnostics.push(Diagnostic::unsupported(c.text_range(), "an attribute"));
        }
    }
    if node.children().any(|c| ast::Expr::can_cast(c.kind())) {
        diagnostics.push(Diagnostic::unsupported(
            node.text_range(),
            "a default argument",
        ));
    }
    let (name, mutable, range, destructures) = match p.pat() {
        Some(pat) => match simple_binding(&pat, scope.tree) {
            Some((name, mutable)) => (name, mutable, pat.syntax().text_range(), false),
            None => (None, false, pat.syntax().text_range(), true),
        },
        None => (None, false, node.text_range(), false),
    };
    let ty = match p.ty() {
        Some(t) => resolve_type(&t, scope, diagnostics),
        None => {
            diagnostics.push(Diagnostic::unsupported(range, "a parameter without a type"));
            Ty::Error
        }
    };
    ParamSig {
        name,
        mutable,
        range,
        ty,
        destructures,
    }
}

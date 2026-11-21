//! Building the item tree: the `parse` and `item_tree` queries.
//!
//! [`Collector`] builds the tree in passes, by topic: [`adts`], [`traits`] (and impls) and
//! [`sigs`] (function and test signatures). Types are resolved by [`crate::resolve`]. Once the
//! tree is built, the checks in [`crate::validate`] run on it, and the [`report::Reporter`]
//! decides which problems the user sees.

use moddil_syntax::ast::{self, AstNode};
use moddil_syntax::{Parse, SyntaxKind as K, SyntaxNode, TextRange};

use crate::db::Program;
use crate::derive::{self, Expansion};
use crate::items::{
    Container, Def, Entry, GenericParam, Generics, Import, ItemTree, ModDef, ModuleDef,
    PRELUDE_MOD, PRELUDE_MODULE, Src, Vis,
};
use crate::resolve::Scope;
use crate::validate;
use crate::{Code, Db, Diagnostic, SourceFile};

mod adts;
mod report;
mod sigs;

pub(crate) use sigs::error_type;
mod traits;

use report::Reporter;

/// The prelude's source, followed by the built-in `core` mod's `collections` module
/// (`reference/2027/core/collections.moddil`), whose items start at [`CORE_START`].
pub(crate) const PRELUDE: &str = concat!(
    include_str!("../../../../reference/2027/prelude.moddil"),
    include_str!("../../../../reference/2027/core/collections.moddil")
);

/// Where the `core` mod's items start in [`PRELUDE`].
pub(crate) const CORE_START: usize =
    include_str!("../../../../reference/2027/prelude.moddil").len();

/// The file parsed.
#[salsa::tracked(returns(ref))]
pub fn parse(db: &dyn Db, file: SourceFile) -> Parse {
    moddil_syntax::parse(file.text(db))
}

/// The prelude parsed (the same for every program).
#[salsa::tracked(returns(ref))]
pub(crate) fn prelude_parse(db: &dyn Db, program: Program) -> Parse {
    let _ = program;
    let _ = db;
    moddil_syntax::parse(PRELUDE)
}

/// The impls the derives of the prelude and the program's files expand to, and their parse.
#[salsa::tracked(returns(ref))]
pub(crate) fn derived(db: &dyn Db, program: Program) -> (Expansion, Parse) {
    let prelude = prelude_parse(db, program).syntax();
    let mut items: Vec<(Src, SyntaxNode)> = prelude.children().map(|n| (Src::Prelude, n)).collect();
    for (f, file) in program.files(db).iter().enumerate() {
        let root = parse(db, file.source).syntax();
        items.extend(root.children().map(|n| (Src::File(f as u32), n)));
    }
    let expansion = derive::expand(items.into_iter());
    let parse = moddil_syntax::parse(&expansion.text);
    (expansion, parse)
}

/// File `f` of `program`.
pub(crate) fn file_source(db: &dyn Db, program: Program, f: u32) -> SourceFile {
    program.files(db)[f as usize].source
}

/// The root and text of one of a program's sources.
pub(crate) fn source(db: &dyn Db, program: Program, src: Src) -> (SyntaxNode, &str) {
    match src {
        Src::Prelude => (prelude_parse(db, program).syntax(), PRELUDE),
        Src::File(f) => {
            let file = file_source(db, program, f);
            (parse(db, file).syntax(), file.text(db).as_str())
        }
        Src::Derived => {
            let (e, p) = derived(db, program);
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

/// Who can name the item `node`, by its `pub` or `pub(mod)`.
pub(crate) fn vis_of(node: &SyntaxNode) -> Vis {
    match node.child(K::VIS) {
        Some(v) if v.token(K::MOD_KW).is_some() => Vis::Mod,
        Some(_) => Vis::Pub,
        None => Vis::Private,
    }
}

/// The name a pattern binds and whether it is `mut`, if it is just a name (`x`, `mut x`) or `_`
/// (`None`). A name that is a unit struct or a variant named on its own (in module `module`) is
/// a pattern, not a binding.
pub(crate) fn simple_binding(
    pat: &ast::Pat,
    tree: &ItemTree,
    module: u32,
) -> Option<(Option<String>, bool)> {
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
                [one]
                    if !matches!(
                        tree.lookup(module, one, false),
                        Some(Def::Adt(_) | Def::Variant(..))
                    ) =>
                {
                    Some((Some(one.clone()), false))
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// The program's items, the prelude's, and the derived impls, with their signatures resolved.
#[salsa::tracked(returns(ref))]
pub fn item_tree(db: &dyn Db, program: Program) -> ItemTree {
    let (expansion, dparse) = derived(db, program);
    let mut sources = vec![(Src::Prelude, prelude_parse(db, program).syntax())];
    for (f, file) in program.files(db).iter().enumerate() {
        sources.push((Src::File(f as u32), parse(db, file.source).syntax()));
    }
    sources.push((Src::Derived, dparse.syntax()));
    let mut c = Collector {
        tree: ItemTree::default(),
        reporter: Reporter::new(expansion),
        module: PRELUDE_MODULE,
        prelude_first: false,
        file_modules: Vec::new(),
        core_module: PRELUDE_MODULE,
    };
    c.modules(program.files(db), program.mods(db));
    c.run(&sources);
    let (shown, hidden) = c.reporter.finish();
    let mut tree = c.tree;
    tree.diagnostics = shown;
    tree.hidden.extend(hidden);
    tree
}

/// An item to collect: its source, its module and its node.
type Site = (Src, u32, SyntaxNode);

/// The parts of an item to collect, by source.
#[derive(Default)]
struct Nodes {
    adts: Vec<Site>,
    traits: Vec<Site>,
    impls: Vec<Site>,
    fns: Vec<Site>,
    tests: Vec<(u32, u32, SyntaxNode)>,
}

struct Collector<'a> {
    tree: ItemTree,
    reporter: Reporter<'a>,
    /// The module of the item being collected.
    module: u32,
    /// Whether the item being collected is derived code, whose names are the prelude's first.
    prelude_first: bool,
    /// The module of each file of the program.
    file_modules: Vec<u32>,
    /// The built-in `core` mod's `collections` module.
    core_module: u32,
}

impl Collector<'_> {
    /// What names in types mean in the item being collected.
    fn scope(&self) -> Scope<'_> {
        Scope {
            module: self.module,
            prelude_first: self.prelude_first,
            ..Scope::new(&self.tree)
        }
    }

    /// Enters the item at `site`: its module, and whether it is derived.
    fn enter(&mut self, src: Src, module: u32) {
        self.module = module;
        self.prelude_first = src == Src::Derived;
    }

    /// The module tree (`modules.md`: Mods and modules): the prelude's module, each mod's root
    /// module, and a module for each file and each directory of files, nested by their paths.
    fn modules(&mut self, files: &[crate::db::ProgramFile], mods: &[crate::db::ModInfo]) {
        let module = |mod_: u32, path: Vec<String>| ModuleDef {
            mod_,
            path,
            file: None,
            exe: false,
            items: std::collections::BTreeMap::new(),
            imports: Vec::new(),
            children: std::collections::BTreeMap::new(),
        };
        self.tree.modules.push(module(PRELUDE_MOD, Vec::new()));
        for (i, m) in mods.iter().enumerate() {
            let root = self.tree.modules.len() as u32;
            self.tree.modules.push(module(i as u32, Vec::new()));
            self.tree.mods.push(ModDef {
                name: m.name.clone(),
                root,
                deps: m.deps.iter().cloned().collect(),
            });
        }
        for (f, file) in files.iter().enumerate() {
            let mut at = self.tree.mods[file.mod_ as usize].root;
            for (k, name) in file.module.iter().enumerate() {
                at = match self.tree.modules[at as usize].children.get(name) {
                    Some(&c) => c,
                    None => {
                        let c = self.tree.modules.len() as u32;
                        self.tree
                            .modules
                            .push(module(file.mod_, file.module[..=k].to_vec()));
                        self.tree.modules[at as usize]
                            .children
                            .insert(name.clone(), c);
                        c
                    }
                };
            }
            let m = &mut self.tree.modules[at as usize];
            m.file = Some(f as u32);
            m.exe = file.exe;
            self.file_modules.push(at);
        }
        // The built-in `core` mod, which every mod depends on (as `core`, unless it names a
        // dependency so itself), and its `collections` module.
        let core = self.tree.mods.len() as u32;
        let root = self.tree.modules.len() as u32;
        self.tree.modules.push(module(core, Vec::new()));
        let collections = root + 1;
        self.tree
            .modules
            .push(module(core, vec!["collections".to_string()]));
        self.tree.modules[root as usize]
            .children
            .insert("collections".to_string(), collections);
        for m in &mut self.tree.mods {
            m.deps.entry("core".to_string()).or_insert(core);
        }
        self.tree.mods.push(ModDef {
            name: "core".to_string(),
            root,
            deps: std::collections::BTreeMap::new(),
        });
        self.core_module = collections;
    }

    /// The module the derived item `node` belongs to: its type's.
    fn derived_module(&self, node: &SyntaxNode) -> u32 {
        match self.reporter.origin_src(node.text_range()) {
            Some(Src::File(f)) => self.file_modules[f as usize],
            _ => PRELUDE_MODULE,
        }
    }

    /// Declares `name` in module `module` (E0103 if the module already has the name).
    fn declare(&mut self, src: Src, name: &str, def: Def, vis: Vis, range: TextRange) {
        if name.is_empty() || src == Src::Derived {
            return;
        }
        let m = &mut self.tree.modules[self.module as usize];
        if let Some(first) = m.items.get(name) {
            let first = first.range;
            self.reporter.report(
                src,
                None,
                vec![
                    Diagnostic::error(
                        Code::E0103,
                        range,
                        format!("`{name}` is defined more than once"),
                    )
                    .with_label(first, "first defined here"),
                ],
            );
            return;
        }
        if let Some(i) = m.imports.iter().find(|i| i.name == name) {
            let first = i.range;
            self.reporter.report(
                src,
                None,
                vec![
                    Diagnostic::error(
                        Code::E0103,
                        range,
                        format!("`{name}` is imported here, so it cannot be defined too"),
                    )
                    .with_label(first, "imported here"),
                ],
            );
            return;
        }
        m.items.insert(name.to_string(), Entry { def, vis, range });
    }

    /// The imports of a `use` item (`modules.md`: Imports).
    fn use_item(&mut self, src: Src, node: &SyntaxNode) {
        let vis = vis_of(node);
        let mut out = Vec::new();
        let mut ds = Vec::new();
        if let Some(tree) = node.child(K::USE_TREE) {
            use_tree(&tree, &[], vis, &mut out, &mut ds);
        }
        self.reporter.report(src, None, ds);
        for i in out {
            let m = &self.tree.modules[self.module as usize];
            let first = m.imports.iter().find(|x| x.name == i.name).map(|x| x.range);
            if let Some(first) = first {
                self.reporter.report(
                    src,
                    None,
                    vec![
                        Diagnostic::error(
                            Code::E0103,
                            i.range,
                            format!("`{}` is imported more than once", i.name),
                        )
                        .with_label(first, "first imported here"),
                    ],
                );
                continue;
            }
            self.tree.modules[self.module as usize].imports.push(i);
        }
    }

    fn run(&mut self, sources: &[(Src, SyntaxNode)]) {
        let mut nodes = Nodes::default();
        for (src, root) in sources {
            for node in root.children() {
                let module = match src {
                    Src::Prelude if node.text_range().start as usize >= CORE_START => {
                        self.core_module
                    }
                    Src::Prelude => PRELUDE_MODULE,
                    Src::File(f) => self.file_modules[*f as usize],
                    Src::Derived => self.derived_module(&node),
                };
                match node.kind() {
                    K::STRUCT_ITEM | K::ENUM_ITEM | K::VIEW_ITEM => {
                        nodes.adts.push((*src, module, node));
                    }
                    K::TRAIT_ITEM => nodes.traits.push((*src, module, node)),
                    K::IMPL_ITEM => nodes.impls.push((*src, module, node)),
                    K::FN_ITEM | K::CONST_ITEM => nodes.fns.push((*src, module, node)),
                    K::TEST_ITEM if src.is_file() => {
                        nodes.tests.push((src.file().unwrap_or(0), module, node));
                    }
                    K::USE_ITEM if src.is_file() => {
                        self.enter(*src, module);
                        self.use_item(*src, &node);
                    }
                    K::MOD_ITEM => self.reporter.report(
                        *src,
                        None,
                        vec![Diagnostic::error(
                            Code::E0702,
                            keyword_range(&node),
                            "a file is a module, and modules are not declared: \
                             `src/a/b.moddil` is the module `a::b`",
                        )],
                    ),
                    K::INNER_ATTR => self.reporter.report(
                        *src,
                        None,
                        vec![Diagnostic::unsupported(node.text_range(), "an attribute")],
                    ),
                    K::ERROR => {}
                    _ => self.reporter.report(
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
        for (src, module, node) in &nodes.adts {
            self.enter(*src, *module);
            self.adt_skeleton(*src, node);
        }
        for (src, module, node) in &nodes.traits {
            self.enter(*src, *module);
            self.trait_skeleton(*src, node);
        }
        // The functions are declared before any signature is resolved, so `use` can name them.
        let first_free = self.free_fns(&nodes.fns);
        for (i, (src, module, node)) in nodes.adts.iter().enumerate() {
            self.enter(*src, *module);
            self.adt_fields(i, *src, node);
        }
        for (i, (src, module, node)) in nodes.traits.iter().enumerate() {
            self.enter(*src, *module);
            self.trait_header(i, *src, node);
        }
        for (i, (src, module, node)) in nodes.traits.iter().enumerate() {
            self.enter(*src, *module);
            self.trait_methods(i, *src, node);
        }
        for (src, module, node) in &nodes.impls {
            self.enter(*src, *module);
            self.impl_item(*src, node);
        }
        for (k, (src, module, node)) in nodes.fns.iter().enumerate() {
            self.enter(*src, *module);
            if node.kind() == K::CONST_ITEM {
                let sig = self.const_sig(node, *src);
                self.tree.fns[first_free + k] = sig;
            } else if let Some(f) = ast::FnItem::cast(node.clone()) {
                let sig = self.fn_sig(
                    &f,
                    *src,
                    Container::Free,
                    &Generics::default(),
                    None,
                    None,
                    None,
                );
                self.tree.fns[first_free + k] = sig;
            }
        }
        for (file, module, node) in &nodes.tests {
            self.enter(Src::File(*file), *module);
            if let Some(t) = ast::TestItem::cast(node.clone()) {
                let sig = self.test_sig(&t, *file);
                self.tree.tests.push(sig);
            }
        }
        // The checks on the finished tree. Recursive types come first: they make the fields of
        // a type that contains itself errors, so the later checks need not care.
        let found = validate::recursive_types(&mut self.tree);
        self.report_found(found);
        let found = validate::copy_types(&self.tree);
        self.report_found(found);
        for d in validate::derived_fields(&self.tree) {
            self.reporter.push_in(d.file, d);
        }
        let found = validate::check_impls(&self.tree);
        self.report_found(found);
        let found = validate::overlap(&self.tree);
        self.report_found(found);
        let found = validate::entry_points(&self.tree);
        self.report_found(found);
        let found = validate::signature_effects(&mut self.tree);
        self.report_found(found);
        let found = validate::imports(&self.tree);
        self.report_found(found);
        let found = validate::exposed(&self.tree);
        self.report_found(found);
    }

    /// Declares the free functions `fns` in their modules, with placeholder signatures that are
    /// filled in later, and returns the index of the first.
    fn free_fns(&mut self, fns: &[Site]) -> usize {
        let first = self.tree.fns.len();
        for (k, (src, module, node)) in fns.iter().enumerate() {
            self.enter(*src, *module);
            let (name, range) = name_of(node);
            let vis = match src {
                Src::File(_) => vis_of(node),
                _ => Vis::Pub,
            };
            self.declare(*src, &name, Def::Fn((first + k) as u32), vis, range);
            self.tree
                .fns
                .push(crate::items::FnSig::placeholder(name, *src, *module, vis));
        }
        first
    }

    fn report_found(&mut self, found: Vec<validate::Found>) {
        for (src, origin, d) in found {
            self.reporter.report(src, origin, vec![d]);
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
                        params.push(GenericParam {
                            name,
                            range,
                            konst: false,
                        });
                    }
                    // `const N: int` (`generic.const.1`): its arguments are lengths.
                    K::CONST_PARAM => {
                        let (name, range) = name_of(&p);
                        if let Some(first) = params.iter().find(|q| q.name == name) {
                            ds.push(
                                Diagnostic::error(
                                    Code::E0104,
                                    range,
                                    format!("the parameter `{name}` is declared twice"),
                                )
                                .with_label(first.range, "first declared here"),
                            );
                        }
                        let ty = p
                            .children()
                            .find_map(ast::Type::cast)
                            .map(|t| t.syntax().text().to_string());
                        if ty.as_deref() != Some("int") {
                            ds.push(Diagnostic::unsupported(
                                p.text_range(),
                                "a const parameter of a type other than `int`",
                            ));
                        } else if p.token(K::EQ).is_some() {
                            ds.push(Diagnostic::unsupported(
                                p.text_range(),
                                "a default const parameter",
                            ));
                        }
                        params.push(GenericParam {
                            name,
                            range,
                            konst: true,
                        });
                    }
                    _ => {}
                }
            }
        }
        self.reporter.report(src, origin, ds);
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
                // A function's effects are read with its signature.
                K::EFFECT_CLAUSE if node.kind() == K::FN_ITEM => continue,
                K::EFFECT_CLAUSE => "an effect clause",
                K::CONTRACT => "a contract",
                K::RESUME_CLAUSE => "a `resume` clause",
                _ => continue,
            };
            ds.push(Diagnostic::unsupported(c.text_range(), what));
        }
        self.reporter.report(src, None, ds);
    }
}

/// The imports a `USE_TREE` makes, after the path `prefix`: `a::b`, `a::b as c`, `a::{b, c}`.
fn use_tree(
    tree: &SyntaxNode,
    prefix: &[(String, TextRange)],
    vis: Vis,
    out: &mut Vec<Import>,
    ds: &mut Vec<Diagnostic>,
) {
    let mut path = prefix.to_vec();
    if let Some(p) = tree.child(K::PATH) {
        for seg in p.children().filter(|c| c.kind() == K::PATH_SEGMENT) {
            if let Some(t) = seg.child_tokens().find(|t| !t.kind().is_trivia()) {
                path.push((t.text().to_string(), t.text_range()));
            }
        }
    }
    if let Some(list) = tree.child(K::USE_LIST) {
        for sub in list.children().filter(|c| c.kind() == K::USE_TREE) {
            use_tree(&sub, &path, vis, out, ds);
        }
        return;
    }
    if let Some(star) = tree.token(K::STAR) {
        ds.push(Diagnostic::error(
            Code::E0702,
            star.text_range(),
            "imports name what they bring in: `use x::*` is only for a mod's declared prelude",
        ));
        return;
    }
    let (name, range) = match tree.child(K::NAME).and_then(ast::Name::cast) {
        Some(n) => (n.text(), n.syntax().text_range()),
        None => match path.last() {
            Some((n, r)) => (n.clone(), *r),
            None => return,
        },
    };
    if matches!(name.as_str(), "mod" | "self" | "super") {
        ds.push(Diagnostic::error(
            Code::E0702,
            range,
            format!("`use` imports an item or a module, and `{name}` alone is neither"),
        ));
        return;
    }
    out.push(Import {
        name,
        path,
        vis,
        range,
    });
}

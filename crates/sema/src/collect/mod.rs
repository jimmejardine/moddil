//! Building the item tree: the `parse` and `item_tree` queries.
//!
//! [`Collector`] builds the tree in passes, by topic: [`adts`], [`traits`] (and impls) and
//! [`sigs`] (function and test signatures). Types are resolved by [`crate::resolve`]. Once the
//! tree is built, the checks in [`crate::validate`] run on it, and the [`report::Reporter`]
//! decides which problems the user sees.

use std::collections::HashMap;

use moddil_syntax::ast::{self, AstNode};
use moddil_syntax::{Parse, SyntaxKind as K, SyntaxNode, TextRange};

use crate::db::Program;
use crate::derive::{self, Expansion};
use crate::items::{
    Container, Def, Entry, FnSig, GenericParam, Generics, Import, ItemTree, ModDef, ModuleDef,
    PRELUDE_MOD, PRELUDE_MODULE, Src, Vis,
};
use crate::resolve::{Scope, resolve_type};
use crate::validate;
use crate::{Code, Db, Diagnostic, SourceFile, Ty};

mod adts;
mod report;
mod sigs;
mod templates;

pub(crate) use sigs::error_type;
mod traits;

use report::Reporter;

/// The prelude's source, followed by the built-in `core` mod's modules
/// (`reference/2027/core/`), in the order of [`CORE_MODULES`] (`moddil-sources`).
pub(crate) const PRELUDE: &str = moddil_sources::PRELUDE_AND_CORE;

/// The `core` mod's modules, in [`PRELUDE`]'s order: each one's name and source.
pub(crate) const CORE_MODULES: [(&str, &str); 7] = moddil_sources::CORE_MODULES;

/// Where the `core` mod's items start in [`PRELUDE`].
pub(crate) const CORE_START: usize = moddil_sources::PRELUDE.len();

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
    let mut items: Vec<(Src, &str, SyntaxNode)> = prelude
        .children()
        .map(|n| (Src::Prelude, PRELUDE, n))
        .collect();
    let mut paths = Vec::new();
    for (f, file) in program.files(db).iter().enumerate() {
        let root = parse(db, file.source).syntax();
        let text = file.source.text(db).as_str();
        items.extend(root.children().map(|n| (Src::File(f as u32), text, n)));
        paths.push(file.module.clone());
    }
    let expansion = derive::expand(items.into_iter(), &paths);
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

/// The node of `kind` at `range` in the tree under `root`. It goes down only through the nodes
/// that contain `range`, so it costs the depth of the tree, not its size: it runs for every
/// function of the program, the prelude's included.
pub(crate) fn node_at(root: &SyntaxNode, kind: K, range: TextRange) -> Option<SyntaxNode> {
    let mut node = root.clone();
    loop {
        if node.kind() == kind && node.text_range() == range {
            return Some(node);
        }
        let next = node.children().into_iter().find(|c| {
            let r = c.text_range();
            r.start <= range.start && range.end <= r.end
        })?;
        node = next;
    }
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
        core_modules: Vec::new(),
        derivers: HashMap::new(),
        derive_for: HashMap::new(),
        template: None,
        derive_by_names: HashMap::new(),
        extern_fns: std::collections::HashSet::new(),
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
    types: Vec<Site>,
    traits: Vec<Site>,
    impls: Vec<Site>,
    fns: Vec<Site>,
    /// Each test: its file, its module, its node, and for an example in a doc comment
    /// (`test.doc.1`), the test's name.
    tests: Vec<(u32, u32, SyntaxNode, Option<String>)>,
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
    /// The built-in `core` mod's modules: where each one's source starts in the prelude's, and
    /// the module.
    core_modules: Vec<(usize, u32)>,
    /// Each derive function (`comptime.derive.1`), by index among the functions: its source,
    /// its module and its node.
    derivers: HashMap<u32, (Src, u32, SyntaxNode)>,
    /// Each prelude derive function's `#[derive_for(…)]` (`comptime.derive.for.1`), by the
    /// attribute's range: the function, whose template each impl it makes is.
    derive_for: HashMap<TextRange, u32>,
    /// While a template's methods are collected: what its parameter is.
    template: Option<crate::items::Template>,
    /// Each trait's `#[derive_by(f)]`, by the trait's index: the name it gives, before it is
    /// resolved.
    derive_by_names: HashMap<u32, (String, TextRange)>,
    /// The functions declared in `extern "C"` blocks (`ffi.extern.1`), by source and node.
    extern_fns: std::collections::HashSet<(Src, TextRange)>,
}

impl Collector<'_> {
    /// What names in types mean in the item being collected.
    fn scope(&self) -> Scope<'_> {
        Scope {
            module: self.module,
            prelude_first: self.prelude_first,
            template: self.template.as_ref().map(|t| (t.var.as_str(), &t.ty)),
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
                stub: m.stub,
                unsafe_ok: m.unsafe_ok,
                os_ok: m.os_ok,
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
        // dependency so itself), and its modules, each starting where its source does.
        let core = self.tree.mods.len() as u32;
        let root = self.tree.modules.len() as u32;
        self.tree.modules.push(module(core, Vec::new()));
        let mut start = CORE_START;
        for (name, src) in CORE_MODULES {
            let m = self.tree.modules.len() as u32;
            self.tree.modules.push(module(core, vec![name.to_string()]));
            self.tree.modules[root as usize]
                .children
                .insert(name.to_string(), m);
            self.core_modules.push((start, m));
            start += src.len();
        }
        for m in &mut self.tree.mods {
            m.deps.entry("core".to_string()).or_insert(core);
        }
        self.tree.mods.push(ModDef {
            name: "core".to_string(),
            root,
            deps: std::collections::BTreeMap::new(),
            stub: false,
            unsafe_ok: true,
            os_ok: true,
        });
    }

    /// The module of the prelude's source at offset `at`: the prelude's, or a `core` module's.
    fn prelude_module_at(&self, at: usize) -> u32 {
        if at < CORE_START {
            return PRELUDE_MODULE;
        }
        self.core_modules
            .iter()
            .rev()
            .find(|(s, _)| at >= *s)
            .map_or(PRELUDE_MODULE, |(_, m)| *m)
    }

    /// The module the derived item `node` belongs to: its type's.
    /// For a derived impl at `range`: the derive function whose `#[derive_for(…)]` made it
    /// (`comptime.derive.for.1`), if one did.
    fn derive_for_of(&self, range: TextRange) -> Option<u32> {
        match self.reporter.origin_src(range) {
            Some((Src::Prelude, attr)) => self.derive_for.get(&attr).copied(),
            _ => None,
        }
    }

    fn derived_module(&self, node: &SyntaxNode) -> u32 {
        match self.reporter.origin_src(node.text_range()) {
            Some((Src::File(f), _)) => self.file_modules[f as usize],
            // A `core` module's type, whose derive is in that module.
            Some((Src::Prelude, attr)) => self.prelude_module_at(attr.start as usize),
            _ => PRELUDE_MODULE,
        }
    }

    /// Declares `name` in module `module` (E0103 if the module already has the name).
    fn declare(&mut self, src: Src, name: &str, def: Def, vis: Vis, range: TextRange) {
        // Of derived code, only the items a wrapped function's expansion names have names.
        if name.is_empty() || (src == Src::Derived && !name.starts_with("__")) {
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
            // The examples in its doc comments, each a test of its module (`test.doc.1`), its
            // code where the example is.
            if let Src::File(f) = src {
                let text = root.text().to_string();
                let mut seen: Vec<String> = Vec::new();
                for ex in moddil_syntax::doc_examples(root, &text) {
                    let parse = moddil_syntax::parse(&ex.source);
                    let ds: Vec<Diagnostic> = parse
                        .errors()
                        .iter()
                        .map(|e| Diagnostic::error(Code::E0001, e.range, e.message.clone()))
                        .collect();
                    if !ds.is_empty() {
                        self.reporter.report(*src, None, ds);
                        continue;
                    }
                    let Some(t) = node_at(&parse.syntax(), K::TEST_ITEM, ex.range) else {
                        continue;
                    };
                    let mut name = if ex.item.is_empty() {
                        "example".to_string()
                    } else {
                        format!("example of {}", ex.item)
                    };
                    let n = seen.iter().filter(|x| **x == name).count();
                    seen.push(name.clone());
                    if n > 0 {
                        name = format!("{name} {}", n + 1);
                    }
                    nodes
                        .tests
                        .push((*f, self.file_modules[*f as usize], t, Some(name)));
                }
            }
            for node in root.children() {
                let module = match src {
                    Src::Prelude => self.prelude_module_at(node.text_range().start as usize),
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
                    // C functions (`ffi.extern.1`): free functions without bodies, by symbol.
                    K::EXTERN_BLOCK => {
                        let abi = node
                            .token(K::STRING)
                            .map(|t| t.text().trim_matches('"').to_string());
                        if abi.as_deref() != Some("C") {
                            self.reporter.report(
                                *src,
                                None,
                                vec![Diagnostic::error(
                                    Code::E0222,
                                    keyword_range(&node),
                                    "an `extern` block declares C functions: `extern \"C\" { … }`",
                                )],
                            );
                        }
                        for f in node.children().filter(|c| c.kind() == K::FN_ITEM) {
                            self.extern_fns.insert((*src, f.text_range()));
                            nodes.fns.push((*src, module, f));
                        }
                    }
                    K::TYPE_ITEM => nodes.types.push((*src, module, node)),
                    K::TEST_ITEM if src.is_file() => {
                        nodes
                            .tests
                            .push((src.file().unwrap_or(0), module, node, None));
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
                    K::INNER_ATTR if crate::is_doc(&node) => {}
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
        // The `type` items' names come before any type is resolved.
        for (src, module, node) in &nodes.types {
            self.enter(*src, *module);
            self.type_skeleton(*src, node);
        }
        let first_free = self.free_fns(&nodes.fns);
        self.const_values(first_free, &nodes.fns);
        for (k, (src, module, node)) in nodes.fns.iter().enumerate() {
            if templates::is_deriver(node) {
                self.derivers
                    .insert((first_free + k) as u32, (*src, *module, node.clone()));
                if *src == Src::Prelude
                    && let Some((range, _)) = derive::derive_for_attr(node)
                {
                    self.derive_for.insert(range, (first_free + k) as u32);
                }
            }
        }
        // Each after the ones its type names, so an alias may name one written after it.
        for k in type_order(&nodes.types) {
            let (src, module, node) = &nodes.types[k];
            self.enter(*src, *module);
            self.type_item(k, *src, node);
        }
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
            // A derive of a trait that is neither derivable nor a stub mod's was reported at
            // its attribute (`mod.stub.1`).
            if *src == Src::Derived
                && let Some(tr) = derived_trait(node)
                && !derive::DERIVABLE.contains(&tr.as_str())
                && !self.stub_trait(&tr)
                && self.derive_fn(&tr).is_none()
                && self.derive_for_of(node.text_range()).is_none()
            {
                continue;
            }
            self.impl_item(*src, node);
        }
        for (k, (src, module, node)) in nodes.fns.iter().enumerate() {
            self.enter(*src, *module);
            if node.kind() == K::CONST_ITEM {
                let sig = self.const_sig(node, *src);
                self.tree.fns[first_free + k] = sig;
            } else if self.derivers.contains_key(&((first_free + k) as u32)) {
                let sig = self.deriver_sig(node, *src);
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
        self.defaults(sources);
        self.number_opaques();
        for (file, module, node, doc) in &nodes.tests {
            self.enter(Src::File(*file), *module);
            if let Some(t) = ast::TestItem::cast(node.clone()) {
                let sig = self.test_sig(&t, *file, doc.clone());
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
        // A refinement's base is `Debug`, for `RefinementError` (`refine.new.1`).
        if let Some(debug) = self.tree.lang_ref("Debug") {
            let mut found = Vec::new();
            for t in &self.tree.types {
                if t.refinement.is_none() || t.ty == Ty::Error {
                    continue;
                }
                let mut solver = crate::traits::Solver::new(&self.tree, &[]);
                if solver.implements(&t.ty, &debug) == crate::traits::Answer::No {
                    found.push((
                        t.src,
                        None,
                        Diagnostic::error(
                            Code::E0402,
                            t.name_range,
                            format!(
                                "`{}` refines `{}`, which must implement `Debug` for `RefinementError`",
                                t.name, t.ty
                            ),
                        ),
                    ));
                }
            }
            self.report_found(found);
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
            // A prelude derive function with `#[derive_for]` is never named: its impls are
            // made where it is (`comptime.derive.for.1`), and its name is not the prelude's.
            let unnamed = *src == Src::Prelude && derive::derive_for_attr(node).is_some();
            if !unnamed {
                self.declare(*src, &name, Def::Fn((first + k) as u32), vis, range);
            }
            self.tree.fns.push(FnSig::free(name, *src, *module, vis));
        }
        first
    }

    /// The value of each free `int` const item whose initialiser folds from its syntax
    /// (`generic.const.3`), the first of `fns` being function `first`.
    fn const_values(&mut self, first: usize, fns: &[Site]) {
        let consts: HashMap<u32, (u32, SyntaxNode)> = fns
            .iter()
            .enumerate()
            .filter(|(_, (src, _, node))| {
                node.kind() == K::CONST_ITEM
                    && src.is_file()
                    && node
                        .children()
                        .find_map(ast::Type::cast)
                        .is_some_and(|t| t.syntax().text() == "int")
            })
            .map(|(k, (_, module, node))| ((first + k) as u32, (*module, node.clone())))
            .collect();
        let mut done = std::collections::BTreeMap::new();
        for &f in consts.keys() {
            let _ = fold_const(&self.tree, &consts, &mut done, f, &mut Vec::new());
        }
        self.tree.const_values = done;
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
                K::OUTER_ATTR if attrs_handled || crate::is_doc(&c) => continue,
                K::OUTER_ATTR => "an attribute",
                // A function's effects are read with its signature, its contracts with its body.
                K::EFFECT_CLAUSE | K::CONTRACT if node.kind() == K::FN_ITEM => continue,
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
        let mut selfs = 0;
        for sub in list.children().filter(|c| c.kind() == K::USE_TREE) {
            // `self` in a group imports the module the path names (`mod.use.self.1`):
            // `use a::b::{self, C}`, `{self as h}`.
            let segs: Vec<moddil_syntax::SyntaxToken> = sub
                .child(K::PATH)
                .into_iter()
                .flat_map(|p| {
                    p.children()
                        .filter(|c| c.kind() == K::PATH_SEGMENT)
                        .collect::<Vec<_>>()
                })
                .filter_map(|seg| seg.child_tokens().find(|t| !t.kind().is_trivia()))
                .collect();
            if let [only] = segs.as_slice()
                && only.text() == "self"
                && sub.child(K::USE_LIST).is_none()
                && let Some((last, last_range)) = path.last().cloned()
            {
                selfs += 1;
                if selfs > 1 {
                    ds.push(Diagnostic::error(
                        Code::E0702,
                        only.text_range(),
                        "`self` is named once in an import group",
                    ));
                    continue;
                }
                let (name, range) = match sub.child(K::NAME).and_then(ast::Name::cast) {
                    Some(n) => (n.text(), n.syntax().text_range()),
                    None => (last, last_range),
                };
                out.push(Import {
                    name,
                    path: path.clone(),
                    vis,
                    range,
                });
                continue;
            }
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

impl Collector<'_> {
    /// Declares a `type` item's name (`refine.1`); its type is resolved with
    /// [`Collector::type_item`].
    fn type_skeleton(&mut self, src: Src, node: &SyntaxNode) {
        let (name, name_range) = name_of(node);
        let vis = match src {
            Src::File(_) => vis_of(node),
            _ => Vis::Pub,
        };
        let index = self.tree.types.len() as u32;
        self.declare(src, &name, Def::Type(index), vis, name_range);
        // A refinement written in a type is shown as written: `(u32 where self >= 1)`.
        let name = if name.starts_with("__Refined_") {
            let base = node.children().find_map(ast::Type::cast);
            let pred = node.children().find_map(ast::Expr::cast);
            match (base, pred) {
                (Some(b), Some(p)) => {
                    format!("({} where {})", b.syntax().text(), p.syntax().text())
                }
                _ => name,
            }
        } else {
            name
        };
        self.tree.types.push(crate::items::TypeDef {
            name,
            name_range,
            src,
            module: self.module,
            vis,
            node: node.text_range(),
            ty: Ty::Error,
            refinement: None,
        });
    }

    /// A `type` item's type: an alias's, or a refinement's base, with the refinement's functions
    /// `X::holds` and `X::new`.
    fn type_item(&mut self, k: usize, src: Src, node: &SyntaxNode) {
        let mut ds = Vec::new();
        if let Some(g) = node.child(K::GENERIC_PARAMS) {
            ds.push(Diagnostic::unsupported(
                g.text_range(),
                "a generic `type` item",
            ));
        }
        let ty = match node.children().find_map(ast::Type::cast) {
            Some(t) => resolve_type(&t, &self.scope(), &mut ds),
            None => Ty::Error,
        };
        let pred = node.children().find_map(ast::Expr::cast);
        let name_range = self.tree.types[k].name_range;
        if pred.is_some() && self.tree.is_view(&ty) {
            ds.push(Diagnostic::error(
                Code::E0309,
                name_range,
                "a refinement's base holds no borrow: its values are kept as the base's are",
            ));
        }
        let origin = self.reporter.origin(src, node.text_range());
        self.reporter.report(src, origin, ds);
        self.tree.types[k].ty = ty.clone();
        let Some(pred) = pred else {
            return;
        };
        let def = &self.tree.types[k];
        let refined = Ty::Refined(k as u32, def.name.as_str().into());
        let param = crate::items::ParamSig {
            name: Some("self".to_string()),
            mutable: false,
            range: name_range,
            ty: ty.clone(),
            destructures: false,
        };
        let error = self.tree.lang("RefinementError").map_or(Ty::Error, |a| {
            self.tree.adts[a as usize].ty_with(a, Vec::new())
        });
        let sig = |name: &str, ret: Ty, fails: Ty, is_new: bool| FnSig {
            name_range,
            node: node.text_range(),
            params: vec![param.clone()],
            ret,
            has_body: true,
            origin,
            fails,
            track_caller: true,
            is_const: !is_new,
            refine: Some((k as u32, is_new)),
            ..FnSig::free(name.to_string(), src, def.module, def.vis)
        };
        let holds = self.tree.fns.len() as u32;
        // `holds` gives the value back, so a base that is not `Copy` is not used up.
        let holds_sig = sig(
            "holds",
            Ty::tuple(vec![Ty::Bool, ty.clone()]),
            Ty::Never,
            false,
        );
        let new_sig = sig("new", refined, error, true);
        self.tree.fns.push(holds_sig);
        self.tree.fns.push(new_sig);
        self.tree.types[k].refinement = Some(crate::items::Refinement {
            clause: pred.syntax().text().to_string().trim().to_string(),
            holds,
            new: holds + 1,
        });
    }
}

impl Collector<'_> {
    /// The defaults of the functions' parameters (`expr.call.default.1`): each a hidden const
    /// of the parameter's type, in the function's module, whose value is the default.
    /// Whether `name`, named in the current module, is a trait of a stub mod, whose derive
    /// declares an impl of it (`mod.stub.1`).
    pub(super) fn stub_trait(&self, name: &str) -> bool {
        match self.tree.path(self.module, &[name], false) {
            Ok((Def::Trait(t), 1)) => {
                let m = self.tree.traits[t as usize].module;
                self.tree
                    .mods
                    .get(self.tree.mod_of(m) as usize)
                    .is_some_and(|d| d.stub)
            }
            _ => false,
        }
    }

    /// Each opaque result type (`type.impl.ret.1`) names its function.
    fn number_opaques(&mut self) {
        let number = |t: &Ty, f: u32| match t {
            Ty::Opaque(o) => Ty::Opaque(std::sync::Arc::new(crate::ty::OpaqueTy {
                func: f,
                ..(**o).clone()
            })),
            t => t.clone(),
        };
        for (f, sig) in self.tree.fns.iter_mut().enumerate() {
            if !matches!(sig.ret, Ty::Opaque(_)) {
                continue;
            }
            sig.ret = number(&sig.ret, f as u32);
            for p in &mut sig.opaque {
                p.ty = number(&p.ty, f as u32);
            }
        }
    }

    fn defaults(&mut self, sources: &[(Src, SyntaxNode)]) {
        for f in 0..self.tree.fns.len() {
            let sig = &self.tree.fns[f];
            if sig.const_item || sig.default_arg {
                continue;
            }
            let Some(node) = sources
                .iter()
                .find(|(s, _)| *s == sig.src)
                .and_then(|(_, root)| node_at(root, K::FN_ITEM, sig.node))
                .and_then(ast::FnItem::cast)
            else {
                continue;
            };
            let skip = usize::from(sig.self_kind.is_some());
            let (src, module, container) = (sig.src, sig.module, sig.container);
            let params: Vec<ast::Param> = node
                .params()
                .map(|l| l.params().collect())
                .unwrap_or_default();
            let mut defaults = vec![None; sig.params.len()];
            for (k, p) in params.iter().enumerate() {
                let Some(e) = p.syntax().children().find_map(ast::Expr::cast) else {
                    continue;
                };
                let Some(param) = self.tree.fns[f].params.get(k + skip).cloned() else {
                    continue;
                };
                let range = e.syntax().text_range();
                let what = if matches!(container, Container::Impl(i)
                    if self.tree.impls[i as usize].trait_ref.is_some())
                {
                    Some(Diagnostic::error(
                        Code::E0216,
                        range,
                        "a default is written on the trait's method, which this one inherits",
                    ))
                } else if param.ty.has_param() {
                    Some(Diagnostic::unsupported(
                        range,
                        "a default of a type that names a type parameter",
                    ))
                } else {
                    None
                };
                if let Some(d) = what {
                    let origin = self.reporter.origin(src, node.syntax().text_range());
                    self.reporter.report(src, origin, vec![d]);
                    continue;
                }
                let name = self.tree.fns[f].name.clone();
                defaults[k + skip] = Some(self.tree.fns.len() as u32);
                let hidden = format!("{name}::{}", param.name.clone().unwrap_or_default());
                self.tree.fns.push(FnSig {
                    name_range: range,
                    node: range,
                    ret: param.ty.clone(),
                    has_body: true,
                    default_arg: true,
                    is_const: true,
                    const_item: true,
                    ..FnSig::free(hidden, src, module, Vis::Pub)
                });
            }
            if defaults.iter().any(Option::is_some) {
                self.tree.fns[f].defaults = defaults;
            }
        }
    }
}

/// The trait a derived impl, `impl Trait for T { … }`, is of.
fn derived_trait(node: &SyntaxNode) -> Option<String> {
    let t = node.children().find_map(ast::Type::cast)?;
    Some(t.syntax().text().to_string().trim().to_string())
}

/// The order to resolve `type` items in: each after the `type` items of its module that its type
/// names by a one-segment path (depth first, a cycle stopping where it closes), so an item may
/// name one written after it.
fn type_order(types: &[(Src, u32, SyntaxNode)]) -> Vec<usize> {
    let name = |n: &SyntaxNode| {
        n.child(K::NAME)
            .and_then(ast::Name::cast)
            .map(|x| x.text())
            .unwrap_or_default()
    };
    let names: Vec<String> = types.iter().map(|(_, _, n)| name(n)).collect();
    // The other `type` items each one's type names.
    let deps: Vec<Vec<usize>> = types
        .iter()
        .map(|(src, module, node)| {
            let Some(t) = node.children().find_map(ast::Type::cast) else {
                return Vec::new();
            };
            t.syntax()
                .descendants()
                .into_iter()
                .filter(|d| d.kind() == K::PATH)
                .filter_map(|p| {
                    let segs: Vec<SyntaxNode> = p
                        .children()
                        .filter(|c| c.kind() == K::PATH_SEGMENT)
                        .collect();
                    let [one] = segs.as_slice() else { return None };
                    let id = one.child_tokens().find(|t| t.kind() == K::IDENT)?;
                    (0..types.len()).find(|&j| {
                        names[j] == id.text() && types[j].0 == *src && types[j].1 == *module
                    })
                })
                .collect()
        })
        .collect();
    let mut order = Vec::new();
    let mut seen = vec![false; types.len()];
    fn visit(k: usize, deps: &[Vec<usize>], seen: &mut [bool], order: &mut Vec<usize>) {
        if seen[k] {
            return;
        }
        seen[k] = true;
        for &d in &deps[k] {
            visit(d, deps, seen, order);
        }
        order.push(k);
    }
    for k in 0..types.len() {
        visit(k, &deps, &mut seen, &mut order);
    }
    order
}

/// Const item `f`'s value, folded from its initialiser (`generic.const.3`), with the others it
/// names first; `seen` are those being folded, so one naming itself is not followed forever.
fn fold_const(
    tree: &ItemTree,
    consts: &HashMap<u32, (u32, SyntaxNode)>,
    done: &mut std::collections::BTreeMap<u32, Result<i64, String>>,
    f: u32,
    seen: &mut Vec<u32>,
) -> Result<i64, String> {
    if let Some(v) = done.get(&f) {
        return v.clone();
    }
    if seen.contains(&f) {
        return Err("it is defined by itself".to_string());
    }
    let Some((module, node)) = consts.get(&f) else {
        return Err("it is not an `int` const".to_string());
    };
    let Some(init) = node.children().find_map(ast::Expr::cast) else {
        return Err("it has no value".to_string());
    };
    seen.push(f);
    let v = crate::constfold::fold(&init, &mut |segs| {
        let names: Vec<&str> = segs.iter().map(String::as_str).collect();
        match tree.path(*module, &names, false) {
            Ok((Def::Fn(g), n)) if n == segs.len() && consts.contains_key(&g) => {
                Some(fold_const(tree, consts, done, g, seen))
            }
            _ => None,
        }
    });
    seen.pop();
    done.insert(f, v.clone());
    v
}

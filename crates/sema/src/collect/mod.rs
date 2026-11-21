//! Building the item tree: the `parse` and `item_tree` queries.
//!
//! [`Collector`] builds the tree in passes, by topic: [`adts`], [`traits`] (and impls) and
//! [`sigs`] (function and test signatures). Types are resolved by [`crate::resolve`]. Once the
//! tree is built, the checks in [`crate::validate`] run on it, and the [`report::Reporter`]
//! decides which problems the user sees.

use std::collections::HashMap;

use moddil_syntax::ast::{self, AstNode};
use moddil_syntax::{Parse, SyntaxKind as K, SyntaxNode, TextRange};

use crate::derive::{self, Expansion};
use crate::items::{Container, GenericParam, Generics, ItemTree, Src};
use crate::validate;
use crate::{Code, Db, Diagnostic, SourceFile};

mod adts;
mod report;
mod sigs;

pub(crate) use sigs::error_type;
mod traits;

use report::Reporter;

/// The prelude's source.
pub(crate) const PRELUDE: &str = include_str!("../../../../reference/2027/prelude.moddil");

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
        reporter: Reporter::new(expansion),
    };
    c.run(&sources);
    let (shown, hidden) = c.reporter.finish();
    let mut tree = c.tree;
    tree.diagnostics = shown;
    tree.hidden.extend(hidden);
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
    reporter: Reporter<'a>,
}

impl Collector<'_> {
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
        // The checks on the finished tree. Recursive types come first: they make the fields of
        // a type that contains itself errors, so the later checks need not care.
        let found = validate::recursive_types(&mut self.tree);
        self.report_found(found);
        let found = validate::copy_types(&self.tree);
        self.report_found(found);
        for d in validate::derived_fields(&self.tree) {
            self.reporter.push(d);
        }
        let found = validate::check_impls(&self.tree);
        self.report_found(found);
        let found = validate::overlap(&self.tree);
        self.report_found(found);
        let found = validate::entry_points(&self.tree);
        self.report_found(found);
    }

    fn report_found(&mut self, found: Vec<validate::Found>) {
        for (src, origin, d) in found {
            self.reporter.report(src, origin, vec![d]);
        }
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
                    if node.kind() == K::ENUM_ITEM
                        && matches!(name_of(node).0.as_str(), "Option" | "Result")
                    {
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
                self.reporter.push(Diagnostic::error(
                    Code::E0103,
                    range,
                    format!("`{name}` is defined by the prelude, so it cannot be defined again"),
                ));
            } else if let Some(first) = seen.get(&name) {
                self.reporter.push(
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

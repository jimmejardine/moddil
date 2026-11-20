//! The items of a file and their signatures: `parse` and `item_tree`.

use std::collections::HashMap;

use moddil_syntax::ast::{self, AstNode};
use moddil_syntax::{Parse, SyntaxKind as K, SyntaxNode, TextRange};

use crate::{Code, Db, Diagnostic, SourceFile, Ty};

/// The file parsed.
#[salsa::tracked(returns(ref))]
pub fn parse(db: &dyn Db, file: SourceFile) -> Parse {
    moddil_syntax::parse(file.text(db))
}

/// A file's functions, tests, structs and enums, with their signatures resolved.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct ItemTree {
    /// The functions, in source order.
    pub fns: Vec<FnSig>,
    /// The structs and enums, in source order.
    pub adts: Vec<AdtDef>,
    /// The tests, in source order.
    pub tests: Vec<TestSig>,
    /// Problems found in the items' headers.
    pub diagnostics: Vec<Diagnostic>,
}

/// A function's signature.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FnSig {
    /// The name.
    pub name: String,
    /// Where the name is.
    pub name_range: TextRange,
    /// The parameters.
    pub params: Vec<ParamSig>,
    /// The return type (`()` when none is written).
    pub ret: Ty,
    /// Where the return type is written, if it is.
    pub ret_range: Option<TextRange>,
}

/// A parameter.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ParamSig {
    /// The name bound (`None` for `_` or a destructuring pattern).
    pub name: Option<String>,
    /// Whether it is declared `mut`.
    pub mutable: bool,
    /// Where the pattern is.
    pub range: TextRange,
    /// The declared type.
    pub ty: Ty,
    /// Whether the pattern is more than a name or `_`, so the body destructures it.
    pub destructures: bool,
}

/// A struct or an enum. A struct has exactly one variant, named like the struct.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AdtDef {
    /// The name.
    pub name: String,
    /// Where the name is.
    pub name_range: TextRange,
    /// Whether it is an enum.
    pub is_enum: bool,
    /// The variants, in order (one for a struct).
    pub variants: Vec<VariantDef>,
}

/// One variant of an enum, or a struct's shape.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct VariantDef {
    /// The name.
    pub name: String,
    /// Where the name is.
    pub name_range: TextRange,
    /// Named fields `{ x: int }`, positional fields `(int, int)`, or none.
    pub shape: Shape,
    /// The fields, in order; positional fields are named `0`, `1`, ….
    pub fields: Vec<FieldDef>,
}

/// How a struct or variant writes its fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Shape {
    /// `{ x: int, y: int }`.
    Named,
    /// `(int, int)`.
    Tuple,
    /// No fields: `struct Unit;`, `Colour::Red`.
    Unit,
}

/// A field.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FieldDef {
    /// The name (`0`, `1`, … for positional fields).
    pub name: String,
    /// Where it is declared.
    pub range: TextRange,
    /// Its type.
    pub ty: Ty,
}

impl VariantDef {
    /// The index of the field called `name`.
    pub fn field(&self, name: &str) -> Option<u32> {
        self.fields
            .iter()
            .position(|f| f.name == name)
            .map(|i| i as u32)
    }
}

impl AdtDef {
    /// The index of the variant called `name`.
    pub fn variant(&self, name: &str) -> Option<u32> {
        self.variants
            .iter()
            .position(|v| v.name == name)
            .map(|i| i as u32)
    }

    /// The type, given the struct's or enum's index.
    pub fn ty(&self, index: u32) -> Ty {
        Ty::Adt(index, self.name.as_str().into())
    }

    /// How variant `v` is written: `Point` for a struct, `Shape::Circle` for an enum's variant.
    pub fn path(&self, v: u32) -> String {
        if self.is_enum {
            format!("{}::{}", self.name, self.variants[v as usize].name)
        } else {
            self.name.clone()
        }
    }
}

/// A test.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TestSig {
    /// The test's name, without quotes.
    pub name: String,
    /// Where the name is.
    pub name_range: TextRange,
}

impl ItemTree {
    /// The function called `name`.
    pub fn fn_named(&self, name: &str) -> Option<u32> {
        self.fns
            .iter()
            .position(|f| f.name == name)
            .map(|i| i as u32)
    }

    /// The struct or enum called `name`.
    pub fn adt_named(&self, name: &str) -> Option<u32> {
        adt_named(&self.adts, name)
    }

    /// The variant of struct or enum `adt`.
    pub fn variant(&self, adt: u32, variant: u32) -> &VariantDef {
        &self.adts[adt as usize].variants[variant as usize]
    }
}

fn adt_named(adts: &[AdtDef], name: &str) -> Option<u32> {
    adts.iter().position(|a| a.name == name).map(|i| i as u32)
}

/// The `n`th node of `kind` at the top level of the file.
pub(crate) fn item_node(root: &SyntaxNode, kind: K, n: u32) -> Option<SyntaxNode> {
    root.children().filter(|c| c.kind() == kind).nth(n as usize)
}

/// The file's functions, tests, structs and enums.
#[salsa::tracked(returns(ref))]
pub fn item_tree(db: &dyn Db, file: SourceFile) -> ItemTree {
    let root = parse(db, file).syntax();
    let mut tree = ItemTree::default();
    // Item names share one namespace, so a struct and a function cannot have the same name.
    let mut seen: HashMap<String, TextRange> = HashMap::new();
    let mut seen_tests: HashMap<String, TextRange> = HashMap::new();
    let mut duplicate = |name: &str, range: TextRange, diagnostics: &mut Vec<Diagnostic>| {
        if name.is_empty() {
            return;
        }
        if let Some(first) = seen.get(name) {
            diagnostics.push(
                Diagnostic::error(
                    Code::E0103,
                    range,
                    format!("`{name}` is defined more than once"),
                )
                .with_label(*first, "first defined here"),
            );
        } else {
            seen.insert(name.to_string(), range);
        }
    };
    // Structs and enums first, by name only, so that any signature can name any of them.
    let adt_nodes: Vec<SyntaxNode> = root
        .children()
        .filter(|n| matches!(n.kind(), K::STRUCT_ITEM | K::ENUM_ITEM))
        .collect();
    for node in &adt_nodes {
        let (name, name_range) = item_name(node);
        tree.adts.push(AdtDef {
            name,
            name_range,
            is_enum: node.kind() == K::ENUM_ITEM,
            variants: Vec::new(),
        });
    }
    let skeleton = tree.adts.clone();
    for (i, node) in adt_nodes.iter().enumerate() {
        tree.adts[i].variants = adt_variants(node, &skeleton, &mut tree.diagnostics);
    }
    for node in root.children() {
        match node.kind() {
            K::FN_ITEM => {
                let f = ast::FnItem::cast(node).expect("a FN_ITEM");
                let sig = fn_sig(&f, &skeleton, &mut tree.diagnostics);
                duplicate(&sig.name, sig.name_range, &mut tree.diagnostics);
                tree.fns.push(sig);
            }
            K::STRUCT_ITEM | K::ENUM_ITEM => {
                let (name, range) = item_name(&node);
                duplicate(&name, range, &mut tree.diagnostics);
            }
            K::TEST_ITEM => {
                let t = ast::TestItem::cast(node).expect("a TEST_ITEM");
                let sig = test_sig(&t, &mut tree.diagnostics);
                if let Some(first) = seen_tests.get(&sig.name) {
                    tree.diagnostics.push(
                        Diagnostic::error(
                            Code::E0103,
                            sig.name_range,
                            format!("a test called \"{}\" already exists", sig.name),
                        )
                        .with_label(*first, "first defined here"),
                    );
                } else {
                    seen_tests.insert(sig.name.clone(), sig.name_range);
                }
                tree.tests.push(sig);
            }
            K::INNER_ATTR => tree
                .diagnostics
                .push(Diagnostic::unsupported(node.text_range(), "an attribute")),
            K::ERROR => {}
            _ => tree.diagnostics.push(Diagnostic::unsupported(
                keyword_range(&node),
                "this kind of item",
            )),
        }
    }
    recursive_types(&mut tree);
    tree
}

/// An item's name and its range (the whole item's if the name is missing).
fn item_name(node: &SyntaxNode) -> (String, TextRange) {
    match node.child(K::NAME).and_then(ast::Name::cast) {
        Some(n) => (n.text(), n.syntax().text_range()),
        None => (String::new(), node.text_range()),
    }
}

/// The variants of a struct (exactly one) or an enum.
fn adt_variants(
    node: &SyntaxNode,
    adts: &[AdtDef],
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<VariantDef> {
    unsupported_parts(node, diagnostics);
    if let Some(t) = node.token(K::LINEAR_KW) {
        diagnostics.push(Diagnostic::unsupported(t.text_range(), "a `linear` struct"));
    }
    if node.kind() == K::STRUCT_ITEM {
        let (name, name_range) = item_name(node);
        return vec![variant_def(node, name, name_range, adts, diagnostics)];
    }
    let mut variants: Vec<VariantDef> = Vec::new();
    let Some(list) = node.child(K::VARIANT_LIST) else {
        return variants;
    };
    for v in list.children().filter(|c| c.kind() == K::VARIANT) {
        unsupported_parts(&v, diagnostics);
        if v.token(K::EQ).is_some() {
            diagnostics.push(Diagnostic::unsupported(
                v.text_range(),
                "an explicit discriminant",
            ));
        }
        let (name, name_range) = item_name(&v);
        if let Some(first) = variants.iter().find(|x| x.name == name && !name.is_empty()) {
            diagnostics.push(
                Diagnostic::error(
                    Code::E0103,
                    name_range,
                    format!("the variant `{name}` is defined more than once"),
                )
                .with_label(first.name_range, "first defined here"),
            );
        }
        variants.push(variant_def(&v, name, name_range, adts, diagnostics));
    }
    variants
}

/// A struct's or variant's fields, from its `STRUCT_BODY` or `TUPLE_FIELDS` child.
fn variant_def(
    node: &SyntaxNode,
    name: String,
    name_range: TextRange,
    adts: &[AdtDef],
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
            unsupported_parts(&m, diagnostics);
            if m.token(K::EQ).is_some() {
                diagnostics.push(Diagnostic::unsupported(
                    m.text_range(),
                    "a field's default value",
                ));
            }
            let (fname, range) = item_name(&m);
            let ty = match m.children().find_map(ast::Type::cast) {
                Some(t) => resolve_type(&t, adts, diagnostics),
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
            unsupported_parts(&f, diagnostics);
            let ty = match f.children().find_map(ast::Type::cast) {
                Some(t) => resolve_type(&t, adts, diagnostics),
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

/// Reports every struct or enum that contains itself without indirection (E0211), and makes the
/// fields of each an error so that later passes need not care.
fn recursive_types(tree: &mut ItemTree) {
    fn contains(t: &Ty, out: &mut Vec<u32>) {
        match t {
            Ty::Adt(i, _) => out.push(*i),
            Ty::Tuple(ts) => ts.iter().for_each(|t| contains(t, out)),
            _ => {}
        }
    }
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
    for start in cyclic {
        let a = &mut tree.adts[start];
        tree.diagnostics.push(Diagnostic::error(
            Code::E0211,
            a.name_range,
            format!(
                "`{}` contains itself, so it would have infinite size",
                a.name
            ),
        ));
        for f in a.variants.iter_mut().flat_map(|v| &mut v.fields) {
            f.ty = Ty::Error;
        }
    }
}

/// The range of the item's first keyword, or of the whole item.
fn keyword_range(node: &SyntaxNode) -> TextRange {
    node.child_tokens()
        .find(|t| t.kind().is_keyword())
        .map_or_else(|| node.text_range(), |t| t.text_range())
}

fn unsupported_parts(node: &SyntaxNode, diagnostics: &mut Vec<Diagnostic>) {
    for c in node.children() {
        let what = match c.kind() {
            K::OUTER_ATTR => "an attribute",
            K::GENERIC_PARAMS => "a generic item",
            K::EFFECT_CLAUSE => "an effect clause",
            K::WHERE_CLAUSE => "a `where` clause",
            K::CONTRACT => "a contract",
            K::RESUME_CLAUSE => "a `resume` clause",
            K::FROM_CLAUSE => "a `from` clause",
            _ => continue,
        };
        diagnostics.push(Diagnostic::unsupported(c.text_range(), what));
    }
}

fn fn_sig(f: &ast::FnItem, adts: &[AdtDef], diagnostics: &mut Vec<Diagnostic>) -> FnSig {
    let node = f.syntax();
    unsupported_parts(node, diagnostics);
    for t in node.child_tokens() {
        if t.kind() == K::FN_KW {
            break;
        }
        if t.kind().is_keyword() && t.kind() != K::PUB_KW && t.kind() != K::MOD_KW {
            diagnostics.push(Diagnostic::unsupported(
                t.text_range(),
                &format!("`{}` on a function", t.text()),
            ));
        }
    }
    let (name, name_range) = f.name().map_or_else(
        || (String::new(), node.text_range()),
        |n| (n.text(), n.syntax().text_range()),
    );
    if f.body().is_none() {
        diagnostics.push(Diagnostic::unsupported(
            name_range,
            "a function without a body",
        ));
    }
    let mut params = Vec::new();
    let mut names: HashMap<String, TextRange> = HashMap::new();
    if let Some(list) = f.params() {
        for c in list.syntax().children() {
            match c.kind() {
                K::PARAM => {
                    let p = ast::Param::cast(c).expect("a PARAM");
                    let sig = param_sig(&p, adts, diagnostics);
                    if let Some(n) = &sig.name {
                        if let Some(first) = names.get(n) {
                            diagnostics.push(
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
                    diagnostics.push(Diagnostic::unsupported(c.text_range(), "`self`"));
                }
                K::NAMED_ONLY_MARKER => diagnostics.push(Diagnostic::unsupported(
                    c.text_range(),
                    "a name-only parameter marker",
                )),
                _ => {}
            }
        }
    }
    let (ret, ret_range) = match f.ret_type().and_then(|r| r.ty()) {
        Some(t) => (
            resolve_type(&t, adts, diagnostics),
            Some(t.syntax().text_range()),
        ),
        None => (Ty::Unit, None),
    };
    FnSig {
        name,
        name_range,
        params,
        ret,
        ret_range,
    }
}

fn param_sig(p: &ast::Param, adts: &[AdtDef], diagnostics: &mut Vec<Diagnostic>) -> ParamSig {
    let node = p.syntax();
    unsupported_parts(node, diagnostics);
    if node.children().any(|c| ast::Expr::can_cast(c.kind())) {
        diagnostics.push(Diagnostic::unsupported(
            node.text_range(),
            "a default argument",
        ));
    }
    let (name, mutable, range, destructures) = match p.pat() {
        Some(pat) => match simple_binding(&pat, adts) {
            Some((name, mutable)) => (name, mutable, pat.syntax().text_range(), false),
            None => (None, false, pat.syntax().text_range(), true),
        },
        None => (None, false, node.text_range(), false),
    };
    let ty = match p.ty() {
        Some(t) => resolve_type(&t, adts, diagnostics),
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

/// The name a pattern binds and whether it is `mut`, if it is just a name (`x`, `mut x`) or `_`
/// (`None`). A name that is a unit struct is a pattern, not a binding.
pub(crate) fn simple_binding(pat: &ast::Pat, adts: &[AdtDef]) -> Option<(Option<String>, bool)> {
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
                [one] if adt_named(adts, one).is_none() => Some((Some(one.clone()), false)),
                _ => None,
            }
        }
        _ => None,
    }
}

/// The type written as `t`.
pub(crate) fn resolve_type(
    t: &ast::Type,
    adts: &[AdtDef],
    diagnostics: &mut Vec<Diagnostic>,
) -> Ty {
    let node = t.syntax();
    let range = node.text_range();
    match t {
        ast::Type::Path(_) => {
            let segments = node
                .child(K::PATH)
                .and_then(ast::Path::cast)
                .map(|p| p.segments())
                .unwrap_or_default();
            let generic = node
                .descendants()
                .iter()
                .any(|d| d.kind() == K::GENERIC_ARGS);
            match segments.as_slice() {
                [one] if !generic => match one.as_str() {
                    "int" => Ty::Int,
                    "bool" => Ty::Bool,
                    other => match adt_named(adts, other) {
                        Some(i) => adts[i as usize].ty(i),
                        None => {
                            diagnostics.push(Diagnostic::error(
                                Code::E0102,
                                range,
                                format!("there is no type called `{other}`"),
                            ));
                            Ty::Error
                        }
                    },
                },
                _ => {
                    diagnostics.push(Diagnostic::unsupported(range, "this type"));
                    Ty::Error
                }
            }
        }
        ast::Type::Tuple(_) => Ty::tuple(
            node.children()
                .filter_map(ast::Type::cast)
                .map(|t| resolve_type(&t, adts, diagnostics))
                .collect(),
        ),
        ast::Type::Paren(_) => match node.children().find_map(ast::Type::cast) {
            Some(inner) => resolve_type(&inner, adts, diagnostics),
            None => Ty::Error,
        },
        ast::Type::Never(_) => Ty::Never,
        _ => {
            diagnostics.push(Diagnostic::unsupported(range, "this type"));
            Ty::Error
        }
    }
}

fn test_sig(t: &ast::TestItem, diagnostics: &mut Vec<Diagnostic>) -> TestSig {
    let node = t.syntax();
    unsupported_parts(node, diagnostics);
    if node
        .child_tokens()
        .any(|t| matches!(t.kind(), K::FOR_KW | K::WITH_KW))
    {
        diagnostics.push(Diagnostic::unsupported(
            node.text_range(),
            "a test with `for` or `with`",
        ));
    }
    match t.name_token() {
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
    }
}

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

/// A file's functions and tests, with their signatures resolved.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct ItemTree {
    /// The functions, in source order.
    pub fns: Vec<FnSig>,
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
    /// The name bound (`None` for `_`).
    pub name: Option<String>,
    /// Whether it is declared `mut`.
    pub mutable: bool,
    /// Where the pattern is.
    pub range: TextRange,
    /// The declared type.
    pub ty: Ty,
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
}

/// The `n`th node of `kind` at the top level of the file.
pub(crate) fn item_node(root: &SyntaxNode, kind: K, n: u32) -> Option<SyntaxNode> {
    root.children().filter(|c| c.kind() == kind).nth(n as usize)
}

/// The file's functions and tests.
#[salsa::tracked(returns(ref))]
pub fn item_tree(db: &dyn Db, file: SourceFile) -> ItemTree {
    let root = parse(db, file).syntax();
    let mut tree = ItemTree::default();
    let mut seen: HashMap<String, TextRange> = HashMap::new();
    let mut seen_tests: HashMap<String, TextRange> = HashMap::new();
    for node in root.children() {
        match node.kind() {
            K::FN_ITEM => {
                let f = ast::FnItem::cast(node).expect("a FN_ITEM");
                let sig = fn_sig(&f, &mut tree.diagnostics);
                if let Some(first) = seen.get(&sig.name) {
                    tree.diagnostics.push(
                        Diagnostic::error(
                            Code::E0103,
                            sig.name_range,
                            format!("`{}` is defined more than once", sig.name),
                        )
                        .with_label(*first, "first defined here"),
                    );
                } else if !sig.name.is_empty() {
                    seen.insert(sig.name.clone(), sig.name_range);
                }
                tree.fns.push(sig);
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
    tree
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
            K::GENERIC_PARAMS => "a generic function",
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

fn fn_sig(f: &ast::FnItem, diagnostics: &mut Vec<Diagnostic>) -> FnSig {
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
                    let sig = param_sig(&p, diagnostics);
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
        Some(t) => (resolve_type(&t, diagnostics), Some(t.syntax().text_range())),
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

fn param_sig(p: &ast::Param, diagnostics: &mut Vec<Diagnostic>) -> ParamSig {
    let node = p.syntax();
    unsupported_parts(node, diagnostics);
    if node.children().any(|c| ast::Expr::can_cast(c.kind())) {
        diagnostics.push(Diagnostic::unsupported(
            node.text_range(),
            "a default argument",
        ));
    }
    let (name, mutable, range) = match p.pat() {
        Some(pat) => binding(&pat, diagnostics),
        None => (None, false, node.text_range()),
    };
    let ty = match p.ty() {
        Some(t) => resolve_type(&t, diagnostics),
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
    }
}

/// The name a simple binding pattern binds (`x`, `mut x`, `_`), whether it is `mut`, and its range.
pub(crate) fn binding(
    pat: &ast::Pat,
    diagnostics: &mut Vec<Diagnostic>,
) -> (Option<String>, bool, TextRange) {
    let node = pat.syntax();
    let range = node.text_range();
    match pat {
        ast::Pat::Wildcard(_) => (None, false, range),
        ast::Pat::Ident(n) => {
            let mutable = n.token(K::MUT_KW).is_some();
            if n.token(K::REF_KW).is_some() {
                diagnostics.push(Diagnostic::unsupported(range, "a `ref` binding"));
            }
            let name = n.child(K::NAME).and_then(ast::Name::cast).map(|n| n.text());
            (name, mutable, range)
        }
        ast::Pat::Path(n) => {
            let segments = n
                .child(K::PATH)
                .and_then(ast::Path::cast)
                .map(|p| p.segments())
                .unwrap_or_default();
            if let [one] = segments.as_slice() {
                (Some(one.clone()), false, range)
            } else {
                diagnostics.push(Diagnostic::unsupported(range, "this pattern"));
                (None, false, range)
            }
        }
        _ => {
            diagnostics.push(Diagnostic::unsupported(range, "this pattern"));
            (None, false, range)
        }
    }
}

/// The type written as `t`.
pub(crate) fn resolve_type(t: &ast::Type, diagnostics: &mut Vec<Diagnostic>) -> Ty {
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
                    other => {
                        diagnostics.push(Diagnostic::error(
                            Code::E0102,
                            range,
                            format!("there is no type called `{other}`"),
                        ));
                        Ty::Error
                    }
                },
                _ => {
                    diagnostics.push(Diagnostic::unsupported(range, "this type"));
                    Ty::Error
                }
            }
        }
        ast::Type::Tuple(_) if node.children().next().is_none() => Ty::Unit,
        ast::Type::Paren(_) => match node.children().find_map(ast::Type::cast) {
            Some(inner) => resolve_type(&inner, diagnostics),
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

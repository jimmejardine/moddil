//! What the compiler writes for a program's own items (`moddil explain`, `toolchain.explain.1`):
//! the impls its derives expand to, the hidden items of its wrapped functions and tagged
//! literals with fields, and each wrapped function's body as the wrapper call it is
//! (`attr.around.1`).

use moddil_syntax::ast::{self, AstNode};
use moddil_syntax::{SyntaxKind as K, SyntaxNode, TextRange};

use crate::{Db, Program, Src};

/// One piece of the expansion: the file and range of what it comes from (the attribute, or the
/// tagged literal), the item that is in (`Point`, `Counter::bump`), and its Moddil source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Explained {
    /// The file it comes from.
    pub file: u32,
    /// What in that file it comes from: an attribute, or a tagged literal.
    pub origin: TextRange,
    /// The item that is in: `Point`, `Counter::bump`.
    pub item: String,
    /// Its Moddil source.
    pub text: String,
    /// For an impl whose members a derive function's template gives (`attr.derive-by.1`),
    /// which function's: they are made when its methods are lowered, not written as source.
    pub template: Option<String>,
}

/// The expansion of `program`'s files, in the order of the files and, within one, of what each
/// piece comes from.
pub fn explain(db: &dyn Db, program: Program) -> Vec<Explained> {
    let (expansion, _) = crate::collect::derived(db, program);
    let tree = crate::collect::typed_item_tree(db, program);
    let mut out = Vec::new();
    for (range, src, origin) in &expansion.origins {
        let Src::File(f) = *src else {
            continue;
        };
        let root = crate::collect::parse(db, crate::collect::file_source(db, program, f)).syntax();
        let text = expansion.text[range.start as usize..range.end as usize].to_string();
        let template = template_of(tree, &text);
        out.push(Explained {
            file: f,
            origin: whole_attr(&root, *origin),
            item: item_at(&root, *origin),
            text,
            template,
        });
    }
    for (f, file) in program.files(db).iter().enumerate() {
        let root = crate::collect::parse(db, file.source).syntax();
        for node in root.descendants() {
            if node.kind() != K::FN_ITEM {
                continue;
            }
            let attrs = crate::wrap::around_attrs(&node);
            let (Some(first), Some(params)) = (attrs.first(), crate::wrap::params(&node)) else {
                continue;
            };
            let owner = node
                .parent()
                .and_then(|b| b.parent())
                .and_then(|i| crate::wrap::impl_self(&i));
            let text =
                wrapped_body(&node, &attrs, &params, owner.as_ref().map(|(n, _)| n)).map(|t| {
                    match &owner {
                        // A method, in an impl of its type.
                        Some((_, ty)) => format!("impl {ty} {{ {t} }}"),
                        None => t,
                    }
                });
            if let Some(text) = text {
                out.push(Explained {
                    file: f as u32,
                    origin: whole_attr(&root, first.text_range()),
                    item: item_at(&root, first.text_range()),
                    text,
                    template: None,
                });
            }
        }
    }
    out.sort_by_key(|e| (e.file, e.origin.start));
    out
}

/// `range`, or where it is one attribute of a `#[…]`, the whole `#[…]`.
fn whole_attr(root: &SyntaxNode, range: TextRange) -> TextRange {
    root.descendants()
        .into_iter()
        .find(|n| n.kind() == K::ATTR && n.text_range() == range)
        .and_then(|n| n.parent())
        .filter(|p| p.kind() == K::OUTER_ATTR)
        .map_or(range, |p| p.text_range())
}

/// For a derived impl with no members written, `impl Tr for T {}`, the derive function of `Tr`
/// whose template gives them.
fn template_of(tree: &crate::ItemTree, text: &str) -> Option<String> {
    let body = text
        .trim_end()
        .strip_suffix('}')?
        .trim_end()
        .strip_suffix('{')?;
    let rest = body.trim().strip_prefix("impl")?;
    // Past the impl's own generic parameters.
    let rest = match rest.strip_prefix('<') {
        Some(r) => {
            let mut depth = 1;
            let end = r.char_indices().find(|&(_, c)| {
                depth += match c {
                    '<' => 1,
                    '>' => -1,
                    _ => 0,
                };
                depth == 0
            })?;
            &r[end.0 + 1..]
        }
        None => rest,
    };
    let tr = rest.split(" for ").next()?.trim();
    let tr = tr.split('<').next()?.rsplit("::").next()?;
    let f = tree
        .traits
        .iter()
        .find(|t| t.name == tr && t.derive_by.is_some())?
        .derive_by?;
    Some(tree.fns[f as usize].name.clone())
}

/// The name of the item at `range` of a file: a struct's, an enum's or a function's, a method's
/// with its type's (`Counter::bump`).
fn item_at(root: &SyntaxNode, range: TextRange) -> String {
    let mut best: Option<SyntaxNode> = None;
    for n in root.descendants() {
        let r = n.text_range();
        let named = matches!(
            n.kind(),
            K::STRUCT_ITEM | K::ENUM_ITEM | K::VIEW_ITEM | K::FN_ITEM | K::CONST_ITEM
        );
        if named && r.start <= range.start && range.end <= r.end {
            best = Some(n);
        }
    }
    let Some(n) = best else {
        return String::new();
    };
    let name = ast::name(&n).map(|x| x.text()).unwrap_or_default();
    match n
        .parent()
        .and_then(|b| b.parent())
        .and_then(|i| crate::wrap::impl_self(&i))
    {
        Some((ty, _)) if n.kind() == K::FN_ITEM => format!("{ty}::{name}"),
        _ => name,
    }
}

/// Function `node`, wrapped by `attrs`, with its body the wrapper calls it is: `w₁(c…, &INFO,
/// f::Args { p… }, |f::Args { p… }| w₂(…, |…| body))` (`attr.around.1`).
fn wrapped_body(
    node: &SyntaxNode,
    attrs: &[SyntaxNode],
    params: &[(String, bool, String)],
    owner: Option<&String>,
) -> Option<String> {
    let f = ast::FnItem::cast(node.clone())?;
    let name = f.name()?.text();
    let body = f.body()?;
    let (key, shown) = match owner {
        Some(ty) => (format!("{ty}_{name}"), format!("{ty}::{name}::Args")),
        None => (name.clone(), format!("{name}::Args")),
    };
    let info = crate::wrap::info_name(&key);
    // The fields, each the parameter of its name: `this` holds `self`.
    let fields: Vec<String> = params
        .iter()
        .map(|(n, _, t)| {
            if n == "this" && matches!(t.as_str(), "Self" | "&Self" | "&mut Self") {
                "this: self".to_string()
            } else {
                n.clone()
            }
        })
        .collect();
    let args = if fields.is_empty() {
        shown.clone()
    } else {
        format!("{shown} {{ {} }}", fields.join(", "))
    };
    // The closure binds the parameters again: `this` is `self` in the body.
    let names: Vec<&str> = params.iter().map(|(n, _, _)| n.as_str()).collect();
    let pattern = if names.is_empty() {
        shown.clone()
    } else {
        format!("{shown} {{ {} }}", names.join(", "))
    };
    let mut call = body.syntax().text().to_string();
    for attr in attrs.iter().rev() {
        let input = attr.child(K::ATTR_INPUT)?.text().to_string();
        let w = input
            .trim()
            .strip_prefix('(')?
            .strip_suffix(')')?
            .trim()
            .to_string();
        let given = format!("&{info}, {args}, |{pattern}| {call}");
        call = match w.strip_suffix(')') {
            Some(open) if open.trim_end().ends_with('(') => format!("{open}{given})"),
            Some(open) => format!("{open}, {given})"),
            None => format!("{w}({given})"),
        };
    }
    // The signature as written, without its attributes.
    let start = node
        .children_with_tokens()
        .find(|c| c.kind() != K::OUTER_ATTR && !c.kind().is_trivia())
        .map(|c| match c {
            moddil_syntax::SyntaxElement::Node(n) => n.text_range().start,
            moddil_syntax::SyntaxElement::Token(t) => t.text_range().start,
        })?;
    let whole = node.text().to_string();
    let from = (start - node.text_range().start) as usize;
    let to = (body.syntax().text_range().start - node.text_range().start) as usize;
    let sig = whole.get(from..to)?.trim_end();
    Some(format!("{sig} {{ {call} }}"))
}

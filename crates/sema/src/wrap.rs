//! Function wrappers (`attr.around.1`): `#[around(w)]` on a function makes its body a call of the
//! ordinary generic function `w`, given the function's description (`&INFO`), its arguments as
//! one value (`f::Args`) and the original body as a closure taking them back.
//!
//! The two items the expansion names are generated as Moddil source, with the derived impls
//! (`derive`): `struct __Args_f<P₁, …> { p₁: P₁, … }`, generic so that it holds borrows and has
//! `Debug`, `Clone` and `PartialEq` exactly when the parameters do, and the const item
//! `__INFO_f: FnInfo`. The call itself is built in HIR when the body is lowered.

use moddil_syntax::ast::{self, AstNode};
use moddil_syntax::{SyntaxKind as K, SyntaxNode};

/// The `#[around(…)]` attributes on `node`, in the order written: each `ATTR` node.
pub(crate) fn around_attrs(node: &SyntaxNode) -> Vec<SyntaxNode> {
    node.children()
        .filter(|c| c.kind() == K::OUTER_ATTR)
        .flat_map(|a| {
            a.children()
                .filter(|c| c.kind() == K::ATTR)
                .collect::<Vec<_>>()
        })
        .filter(|a| {
            a.child(K::PATH)
                .and_then(ast::Path::cast)
                .is_some_and(|p| p.segments() == ["around"])
        })
        .collect()
}

/// The name of the hidden struct holding `f`'s arguments.
pub(crate) fn args_name(f: &str) -> String {
    format!("__Args_{f}")
}

/// The name of the hidden const describing `f`.
pub(crate) fn info_name(f: &str) -> String {
    format!("__INFO_{f}")
}

/// A wrapped function's parameters, each its name, whether it is `mut`, and its type as
/// written; `None` if one is not a plain name (or is `self`).
pub(crate) fn params(node: &SyntaxNode) -> Option<Vec<(String, bool, String)>> {
    let f = ast::FnItem::cast(node.clone())?;
    if node
        .descendants()
        .into_iter()
        .any(|n| n.kind() == K::SELF_PARAM)
    {
        return None;
    }
    let mut out = Vec::new();
    for p in f
        .params()
        .map(|l| l.params().collect::<Vec<_>>())
        .unwrap_or_default()
    {
        let pat = p.pat()?;
        let pat = pat.syntax();
        // `mut x`, or `x`, which is parsed as a path.
        let (name, mutable) = match pat.kind() {
            K::IDENT_PAT if pat.children().all(|c| c.kind() == K::NAME) => {
                (ast::name(pat)?.text(), true)
            }
            K::PATH_PAT => {
                let segments = pat.child(K::PATH).and_then(ast::Path::cast)?.segments();
                let [one] = segments.as_slice() else {
                    return None;
                };
                if pat
                    .descendants()
                    .into_iter()
                    .any(|n| n.kind() == K::GENERIC_ARGS)
                {
                    return None;
                }
                (one.clone(), false)
            }
            _ => return None,
        };
        let ty = p.ty()?.syntax().text().to_string();
        let ty = ty.split_whitespace().collect::<Vec<_>>().join(" ");
        out.push((name, mutable, ty));
    }
    Some(out)
}

/// The wrapper an `#[around(…)]` attribute names, from the file's `text`: its input parsed as
/// an expression, with the ranges it has in the file. `None` if it has no input.
pub(crate) fn input(text: &str, attr: &SyntaxNode) -> Option<ast::Expr> {
    let input = attr.child(K::ATTR_INPUT)?;
    let r = input.text_range();
    // Inside the parentheses, which `#[around` comes before, so the prefix fits.
    let (start, end) = (r.start as usize + 1, (r.end as usize).saturating_sub(1));
    if end <= start || !text.is_char_boundary(start) || !text.is_char_boundary(end) {
        return None;
    }
    const PREFIX: &str = "fn f(){";
    if start < PREFIX.len() {
        return None;
    }
    let mut padded = String::with_capacity(end + 1);
    padded.push_str(PREFIX);
    padded.push_str(&" ".repeat(start - PREFIX.len()));
    padded.push_str(&text[start..end]);
    padded.push('}');
    let parse = moddil_syntax::parse(&padded);
    if !parse.errors().is_empty() {
        return None;
    }
    let root = parse.syntax();
    let body = ast::FnItem::cast(root.children().find(|c| c.kind() == K::FN_ITEM)?)?.body()?;
    let exprs: Vec<ast::Expr> = body
        .syntax()
        .children()
        .filter_map(ast::Expr::cast)
        .collect();
    let [e] = exprs.as_slice() else {
        return None;
    };
    Some(e.clone())
}

/// The Moddil source of the items a wrapped function's expansion names, one per line: the
/// arguments' struct, its derived impls, and the description; `path` is the function's path.
pub(crate) fn items(f: &str, path: &str, params: &[(String, bool, String)]) -> Vec<String> {
    let args = args_name(f);
    let generics: Vec<String> = (0..params.len()).map(|i| format!("P{i}")).collect();
    let mut out = Vec::new();
    if params.is_empty() {
        out.push(format!("struct {args};"));
    } else {
        let fields: Vec<String> = params
            .iter()
            .zip(&generics)
            .map(|((n, _, _), g)| format!("{n}: {g}"))
            .collect();
        out.push(format!(
            "struct {args}<{}> {{ {} }}",
            generics.join(", "),
            fields.join(", ")
        ));
    }
    let shown = format!("{f}::Args");
    let names: Vec<String> = params.iter().map(|(n, _, _)| n.clone()).collect();
    for tr in ["Debug", "Clone", "PartialEq"] {
        out.push(crate::derive::struct_impl(
            tr, &args, &shown, &generics, &names,
        ));
    }
    let infos: Vec<String> = params
        .iter()
        .map(|(n, _, t)| format!("ParamInfo {{ name: fmt{n:?}, ty: fmt{:?} }}", escape(t)))
        .collect();
    out.push(format!(
        "const {}: FnInfo = FnInfo {{ path: fmt{:?}, params: [{}] }};",
        info_name(f),
        escape(path),
        infos.join(", ")
    ));
    out
}

/// `s` with its braces doubled, for a `fmt"…"` literal.
fn escape(s: &str) -> String {
    s.replace('{', "{{").replace('}', "}}")
}

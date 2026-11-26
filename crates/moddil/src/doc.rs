//! `moddil doc`: the API pages of a mod, one Markdown file per module (`docs.md`), for a site to
//! include. Each public item is its declaration as written up to its body (its signature, with
//! the effects, errors and contracts it states, or a type's fields and variants), then its doc
//! comment, examples and all. A module's own `//!` comment comes first.

use std::fmt::Write as _;
use std::path::Path;

use moddil_project::Project;
use moddil_syntax::{SyntaxKind as K, SyntaxNode};

/// Writes the pages of `project`'s root mod into `out`: `out/<mod>/<module path>.md`, the root
/// module's `index.md`. The paths written, in order.
pub(crate) fn write(project: &Project, out: &Path) -> std::io::Result<Vec<std::path::PathBuf>> {
    let name = project
        .mods
        .first()
        .map(|m| m.name.clone())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "main".to_string());
    let dir = out.join(name.replace('/', "_"));
    let mut written = Vec::new();
    for f in project.files.iter().filter(|f| f.mod_ == 0) {
        let root = moddil_syntax::parse(&f.text).syntax();
        let title = if f.module.is_empty() {
            name.clone()
        } else {
            format!("{name}::{}", f.module.join("::"))
        };
        let page = page(&title, &root);
        let file = if f.module.is_empty() {
            "index.md".to_string()
        } else {
            format!("{}.md", f.module.join("/"))
        };
        let path = dir.join(file);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, page)?;
        written.push(path);
    }
    Ok(written)
}

/// The page of the module whose file parses to `root`, titled `title`.
fn page(title: &str, root: &SyntaxNode) -> String {
    let mut out = format!("# `{title}`\n\n");
    let inner = doc_text(root, K::INNER_ATTR, K::DOC_INNER);
    if !inner.is_empty() {
        out.push_str(&inner);
        out.push_str("\n\n");
    }
    for item in root.children() {
        item_section(&mut out, &item, 2);
    }
    out
}

/// The section for `item`, if it is public, at heading `level`; an impl's public members under
/// its own heading.
fn item_section(out: &mut String, item: &SyntaxNode, level: usize) {
    let hashes = "#".repeat(level);
    match item.kind() {
        K::FN_ITEM
        | K::STRUCT_ITEM
        | K::VIEW_ITEM
        | K::ENUM_ITEM
        | K::TRAIT_ITEM
        | K::TYPE_ITEM
        | K::CONST_ITEM
            if is_pub(item) =>
        {
            let decl = declaration(item);
            let name = item
                .child(K::NAME)
                .map(|n| n.text().to_string())
                .unwrap_or_default();
            let _ = write!(
                out,
                "{hashes} `{}`\n\n```moddil\n{decl}\n```\n\n",
                name.trim()
            );
            let doc = doc_text(item, K::OUTER_ATTR, K::DOC_OUTER);
            if !doc.is_empty() {
                out.push_str(&doc);
                out.push_str("\n\n");
            }
        }
        K::IMPL_ITEM => {
            let members: Vec<SyntaxNode> = item
                .child(K::IMPL_BODY)
                .into_iter()
                .flat_map(|b| b.children().collect::<Vec<_>>())
                .filter(|m| m.kind() == K::FN_ITEM && (is_pub(m) || is_trait_impl(item)))
                .collect();
            if members.is_empty() {
                return;
            }
            let _ = write!(out, "{hashes} `{}`\n\n", declaration(item));
            for m in &members {
                item_section(out, m, level + 1);
            }
        }
        _ => {}
    }
}

/// Whether `item` is declared `pub`.
fn is_pub(item: &SyntaxNode) -> bool {
    item.child(K::VIS)
        .is_some_and(|v| v.token(K::MOD_KW).is_none())
}

/// Whether `item`, an impl, is of a trait: its methods are as public as the trait.
fn is_trait_impl(item: &SyntaxNode) -> bool {
    item.token(K::FOR_KW).is_some()
}

/// `item` as written up to its body, without its doc comment and attributes: a function's
/// signature with its effects and contracts, an impl's header; a type's whole declaration.
fn declaration(item: &SyntaxNode) -> String {
    let start = item
        .children_with_tokens()
        .find(|c| {
            !matches!(
                c.kind(),
                K::OUTER_ATTR | K::WHITESPACE | K::LINE_COMMENT | K::DOC_OUTER
            )
        })
        .map_or(item.text_range().start, |c| match c {
            moddil_syntax::SyntaxElement::Node(n) => n.text_range().start,
            moddil_syntax::SyntaxElement::Token(t) => t.text_range().start,
        });
    let body = item
        .children()
        .find(|c| matches!(c.kind(), K::BLOCK | K::IMPL_BODY | K::TRAIT_BODY));
    let end = body.map_or(item.text_range().end, |b| b.text_range().start);
    let text = item.text().to_string();
    let from = (start - item.text_range().start) as usize;
    let to = (end - item.text_range().start) as usize;
    text[from..to].trim().to_string()
}

/// The text of the doc comments in `node`'s `attr` children (their `doc` tokens), without their
/// `///` or `//!` and one space after.
fn doc_text(node: &SyntaxNode, attr: K, doc: K) -> String {
    let lines: Vec<String> = node
        .children()
        .filter(|c| c.kind() == attr)
        .flat_map(|a| a.child_tokens().collect::<Vec<_>>())
        .filter(|t| t.kind() == doc)
        .map(|t| {
            let s = t.text();
            let s = s
                .strip_prefix("///")
                .or_else(|| s.strip_prefix("//!"))
                .unwrap_or(s);
            s.strip_prefix(' ').unwrap_or(s).trim_end().to_string()
        })
        .collect();
    lines.join("\n").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_shows_each_public_item_s_declaration_and_doc() {
        let text = "//! Accounts.\n\n/// Opens one.\n///\n/// # Example\n/// open(1)  // => 1\npub fn open(n: int) -> int requires n > 0 {\n    n\n}\n\nfn hidden() {}\n\n/// A balance.\npub struct Balance {\n    cents: int,\n}\n";
        let root = moddil_syntax::parse(text).syntax();
        let p = page("bank", &root);
        assert!(p.starts_with("# `bank`\n\nAccounts.\n\n"), "{p}");
        assert!(
            p.contains("## `open`\n\n```moddil\npub fn open(n: int) -> int requires n > 0\n```\n\nOpens one.\n\n# Example\nopen(1)  // => 1"),
            "{p}"
        );
        assert!(p.contains("## `Balance`"), "{p}");
        assert!(!p.contains("hidden"), "{p}");
    }
}

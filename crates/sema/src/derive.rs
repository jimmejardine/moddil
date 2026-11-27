//! Derives: `#[derive(Clone, PartialEq, …)]` on a struct or enum expands to impls, which are
//! then checked and compiled like any other code (`values.md`: Explicit derives). Most are
//! empty here: their methods come from their trait's template, the prelude's
//! `derives.moddil` for the built-in derives (`attr.derive.1`), when the impl is collected.
//! `Copy`, `Eq` and a `Copy` type's `*self` clone are written here, as are a wrapped function's
//! `Args` impls (`struct_impl`).

use moddil_syntax::ast::{self, AstNode};
use moddil_syntax::{SyntaxKind as K, SyntaxNode, TextRange};

use crate::items::Src;

/// The traits that can be derived.
pub(crate) const DERIVABLE: &[&str] = &[
    "Copy",
    "Clone",
    "PartialEq",
    "Eq",
    "PartialOrd",
    "Ord",
    "Hash",
    "Debug",
    "Default",
];

/// The impls derived for the structs and enums of a file, as Moddil source, with where each one
/// comes from.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub(crate) struct Expansion {
    /// The source text: one impl per line.
    pub text: String,
    /// For each impl, in order: its range in `text`, and the source and range of the `derive`
    /// attribute it comes from.
    pub origins: Vec<(TextRange, Src, TextRange)>,
}

/// The name of the hidden `type` item a refinement written in a type at `range` of its file is
/// (`refine.inline.1`).
pub(crate) fn inline_refinement_name(range: TextRange) -> String {
    format!("__Refined_{}_{}", range.start, range.end)
}

/// A struct's or variant's fields, as a derive sees them.
struct Variant {
    /// How it is written in a pattern or expression: `Point`, `Shape::Circle`.
    path: String,
    /// How `Debug` shows it: its path, but for a wrapped function's `f::Args`.
    shown: String,
    /// The fields' names (empty for positional fields).
    fields: Vec<String>,
    /// Whether the fields are named.
    named: bool,
    /// Whether it has no fields at all (`Colour::Red`, `struct Marker;`).
    unit: bool,
}

/// The derive names of each `#[derive(…)]` attribute on `node`, with the attribute's range.
pub(crate) fn derive_attrs(node: &SyntaxNode) -> Vec<(TextRange, Vec<String>)> {
    let mut out = Vec::new();
    for attr in node.children().filter(|c| c.kind() == K::OUTER_ATTR) {
        let Some(a) = attr.child(K::ATTR) else {
            continue;
        };
        let path = a
            .child(K::PATH)
            .and_then(ast::Path::cast)
            .map(|p| p.segments())
            .unwrap_or_default();
        if path != ["derive"] {
            continue;
        }
        let names = a
            .child(K::ATTR_INPUT)
            .map(|i| {
                i.child_tokens()
                    .filter(|t| t.kind() == K::IDENT)
                    .map(|t| t.text().to_string())
                    .collect()
            })
            .unwrap_or_default();
        out.push((attr.text_range(), names));
    }
    out
}

/// The types `#[derive_for(…)]` on derive function `node` lists (`comptime.derive.for.1`), with
/// the attribute's range.
pub(crate) fn derive_for_attr(node: &SyntaxNode) -> Option<(TextRange, Vec<String>)> {
    for attr in node.children().filter(|c| c.kind() == K::OUTER_ATTR) {
        let Some(a) = attr.child(K::ATTR) else {
            continue;
        };
        let path = a
            .child(K::PATH)
            .and_then(ast::Path::cast)
            .map(|p| p.segments())
            .unwrap_or_default();
        if path != ["derive_for"] {
            continue;
        }
        let input = a
            .child(K::ATTR_INPUT)
            .map(|i| i.text().to_string())
            .unwrap_or_default();
        let input = input.trim();
        let inner = input
            .strip_prefix('(')
            .and_then(|i| i.strip_suffix(')'))
            .unwrap_or(input);
        let types = inner
            .split(',')
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_string)
            .collect();
        return Some((attr.text_range(), types));
    }
    None
}

/// The types one entry of `#[derive_for(…)]` stands for, each with the impl's generic
/// parameters (`<A: Tr, B: Tr>`, or nothing): a built-in type's name, or `tuples(m..=n)`, the
/// tuples of `m` to `n` elements `A`, `B`, …, each bounded by trait `tr`.
fn for_types(entry: &str, tr: &str) -> Vec<(String, String)> {
    let sizes = entry
        .strip_prefix("tuples(")
        .and_then(|r| r.strip_suffix(')'))
        .and_then(|r| r.split_once("..="))
        .and_then(|(m, n)| {
            Some((
                m.trim().parse::<usize>().ok()?,
                n.trim().parse::<usize>().ok()?,
            ))
        });
    let Some((m, n)) = sizes else {
        return vec![(String::new(), entry.to_string())];
    };
    const NAMES: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    (m.max(2)..=n.min(NAMES.len()))
        .map(|k| {
            let names: Vec<String> = NAMES[..k]
                .iter()
                .map(|c| (*c as char).to_string())
                .collect();
            let bounds: Vec<String> = names.iter().map(|p| format!("{p}: {tr}")).collect();
            (
                format!("<{}>", bounds.join(", ")),
                format!("({})", names.join(", ")),
            )
        })
        .collect()
}

/// The associated types (`type Output = t;`) of derive function `node`'s template, for type
/// `ty`: each written as it is, with the function's parameter replaced by `ty`, then a space.
/// The template's methods are collected from the template itself.
fn template_assoc_types(node: &SyntaxNode, ty: &str) -> String {
    let var = ast::FnItem::cast(node.clone())
        .and_then(|f| f.params())
        .and_then(|l| l.params().next())
        .and_then(|p| p.pat())
        .map(|p| p.syntax().text().trim().to_string())
        .unwrap_or_default();
    let Some(template) = node.child(K::BLOCK).and_then(|b| {
        b.descendants()
            .into_iter()
            .find(|n| n.kind() == K::IMPL_ITEM)
    }) else {
        return String::new();
    };
    let mut out = String::new();
    for m in template
        .child(K::IMPL_BODY)
        .into_iter()
        .flat_map(|b| b.children().collect::<Vec<_>>())
        .filter(|m| m.kind() == K::ASSOC_TYPE)
    {
        for t in m.tokens() {
            if t.kind() == K::IDENT && t.text() == var {
                out.push_str(ty);
            } else {
                out.push_str(t.text());
            }
        }
        out.push(' ');
    }
    out
}

/// The trait a derive function's `-> Impl<Tr>` names.
fn impl_trait(node: &SyntaxNode) -> Option<String> {
    let ret = ast::FnItem::cast(node.clone())?.ret_type()?;
    let text = ret.syntax().text().to_string();
    let inner = text
        .trim_start_matches("->")
        .trim()
        .strip_prefix("Impl<")?
        .strip_suffix('>')?;
    Some(inner.trim().to_string())
}

fn name_of(node: &SyntaxNode) -> String {
    ast::name(node).map(|n| n.text()).unwrap_or_default()
}

/// Whether a variant is marked `#[default]`.
pub(crate) fn is_default(node: &SyntaxNode) -> bool {
    node.children()
        .filter(|c| c.kind() == K::OUTER_ATTR)
        .filter_map(|a| a.child(K::ATTR))
        .any(|a| {
            a.child(K::PATH)
                .and_then(ast::Path::cast)
                .is_some_and(|p| p.segments() == ["default"])
        })
}

impl Variant {
    /// A pattern binding each field to `prefix0`, `prefix1`, …: `Shape::Circle(a0)`.
    fn bind(&self, prefix: &str) -> String {
        if self.unit {
            return self.path.clone();
        }
        let names: Vec<String> = (0..self.fields.len())
            .map(|i| format!("{prefix}{i}"))
            .collect();
        self.build(&names)
    }

    /// The struct or variant built (or matched) from one expression per field.
    fn build(&self, values: &[String]) -> String {
        if self.unit {
            return self.path.clone();
        }
        if self.named {
            let fields: Vec<String> = self
                .fields
                .iter()
                .zip(values)
                .map(|(f, v)| format!("{f}: {v}"))
                .collect();
            format!("{} {{ {} }}", self.path, fields.join(", "))
        } else {
            format!("{}({})", self.path, values.join(", "))
        }
    }
}

/// The derived impls of every struct and enum among `items` (top-level nodes, with their source
/// and its text), and the items each wrapped function's expansion names (`crate::wrap`);
/// `paths` is each file's module path.
pub(crate) fn expand<'t>(
    items: impl Iterator<Item = (Src, &'t str, SyntaxNode)>,
    paths: &[Vec<String>],
) -> Expansion {
    let mut out = Expansion::default();
    for (src, _text, node) in items {
        // A refinement written in a type, `(u32 where self >= 1)` (`refine.inline.1`): a hidden
        // `type` item of its own, which the type names by where it is.
        if let Src::File(_) = src {
            for r in node
                .descendants()
                .into_iter()
                .filter(|d| d.kind() == K::REFINED_TYPE)
            {
                let base = r.children().find_map(ast::Type::cast);
                let pred = r.children().find_map(ast::Expr::cast);
                let (Some(base), Some(pred)) = (base, pred) else {
                    continue;
                };
                let line = format!(
                    "type {} = {} where {};",
                    inline_refinement_name(r.text_range()),
                    base.syntax().text(),
                    pred.syntax().text()
                );
                let start = out.text.len() as u32;
                out.text.push_str(&line);
                let end = out.text.len() as u32;
                out.text.push('\n');
                out.origins
                    .push((TextRange::new(start, end), src, r.text_range()));
            }
        }
        // A prelude derive function's `#[derive_for(…)]`: its template's impl for each type
        // listed (`comptime.derive.for.1`). Elsewhere it is reported where it is collected.
        if node.kind() == K::FN_ITEM
            && src == Src::Prelude
            && let Some((range, types)) = derive_for_attr(&node)
            && let Some(tr) = impl_trait(&node)
        {
            for (params, ty) in types.iter().flat_map(|t| for_types(t, &tr)) {
                let assoc = template_assoc_types(&node, &ty);
                let start = out.text.len() as u32;
                out.text
                    .push_str(&format!("impl{params} {tr} for {ty} {{ {assoc}}}"));
                let end = out.text.len() as u32;
                out.text.push('\n');
                out.origins.push((TextRange::new(start, end), src, range));
            }
            continue;
        }
        // A wrapped function's items (`attr.around.1`), and those of each wrapped method of an
        // inherent impl (`attr.around.method.1`).
        if node.kind() == K::FN_ITEM || node.kind() == K::IMPL_ITEM {
            let Src::File(f) = src else {
                continue;
            };
            let module = paths.get(f as usize).cloned().unwrap_or_default();
            let (fns, owner) = if node.kind() == K::FN_ITEM {
                (vec![node.clone()], None)
            } else {
                let Some(owner) = crate::wrap::impl_self(&node) else {
                    continue;
                };
                let fns = node
                    .child(K::IMPL_BODY)
                    .map(|b| b.children().filter(|c| c.kind() == K::FN_ITEM).collect())
                    .unwrap_or_default();
                (fns, Some(owner))
            };
            for func in fns {
                let attrs = crate::wrap::around_attrs(&func);
                let (Some(first), Some(params)) = (attrs.first(), crate::wrap::params(&func))
                else {
                    continue;
                };
                let name = name_of(&func);
                let mut path = module.clone();
                let mut generics = Vec::new();
                let (key, shown) = match &owner {
                    Some((ty, _)) => {
                        path.push(ty.clone());
                        generics.extend(crate::wrap::generic_decls(&node));
                        (format!("{ty}_{name}"), format!("{ty}::{name}::Args"))
                    }
                    None => (name.clone(), format!("{name}::Args")),
                };
                path.push(name.clone());
                generics.extend(crate::wrap::generic_decls(&func));
                let named = crate::wrap::Named {
                    key: &key,
                    shown,
                    path: path.join("::"),
                    generics,
                    self_ty: owner.as_ref().map(|(_, t)| t.clone()),
                };
                let range = first.text_range();
                let described = crate::wrap::attrs_and_effects(&func);
                for line in crate::wrap::items(&named, &params, &described) {
                    let start = out.text.len() as u32;
                    out.text.push_str(&line);
                    let end = out.text.len() as u32;
                    out.text.push('\n');
                    out.origins.push((TextRange::new(start, end), src, range));
                }
            }
            if node.kind() == K::FN_ITEM {
                continue;
            }
        }
        if !matches!(node.kind(), K::STRUCT_ITEM | K::ENUM_ITEM | K::VIEW_ITEM) {
            continue;
        }
        let name = name_of(&node);
        // Its generic parameters, in order: each a type parameter, or a const parameter with
        // its declaration (`const S: int`), which the impl declares as it is, with no bound.
        let generics: Vec<(String, Option<String>)> = node
            .child(K::GENERIC_PARAMS)
            .map(|g| {
                g.children()
                    .filter(|c| matches!(c.kind(), K::TYPE_PARAM | K::CONST_PARAM))
                    .map(|p| {
                        let decl = (p.kind() == K::CONST_PARAM)
                            .then(|| p.text().to_string().trim().to_string());
                        (name_of(&p), decl)
                    })
                    .collect()
            })
            .unwrap_or_default();
        let type_params = generics.iter().any(|(_, decl)| decl.is_none());
        // A struct's invariants (`contract.invariant.1`): the prelude's `__Invariant`, whose
        // `__holds` is them all, the fields named as they are.
        if let Some(line) = invariant_impl(&node, &name, &generics) {
            let at = node
                .child(K::STRUCT_BODY)
                .and_then(|b| b.children().find(|c| c.kind() == K::INVARIANT))
                .map_or(node.text_range(), |i| i.text_range());
            let start = out.text.len() as u32;
            out.text.push_str(&line);
            let end = out.text.len() as u32;
            out.text.push('\n');
            out.origins.push((TextRange::new(start, end), src, at));
        }
        let attrs = derive_attrs(&node);
        let all: Vec<&String> = attrs.iter().flat_map(|(_, n)| n).collect();
        let copy = all.iter().any(|n| *n == "Copy");
        let has_clone = all.iter().any(|n| *n == "Clone");
        for (range, names) in &attrs {
            for d in names {
                // `Copy` without `Clone` is reported where the item is collected.
                if d == "Copy" && !has_clone {
                    continue;
                }
                // A `Copy` type without type parameters clones by copying.
                let copies = d == "Clone" && copy && !type_params;
                // A trait whose derive is a template (the prelude's `derives.moddil` for
                // `Clone`, `PartialEq`, `PartialOrd`, `Ord`, `Hash`, `Debug` and `Default`), or a
                // stub mod's, whose derive declares the impl (`mod.stub.1`): an impl whose
                // methods and bounds are added where it is collected. An impl of any other
                // trait is dropped there.
                if !matches!(d.as_str(), "Copy" | "Eq") && !copies {
                    let start = out.text.len() as u32;
                    out.text.push_str(&format!(
                        "impl{} {d} for {name}{} {{ }}",
                        unbounded(&generics),
                        args_of(&generics)
                    ));
                    let end = out.text.len() as u32;
                    out.text.push('\n');
                    out.origins.push((TextRange::new(start, end), src, *range));
                    continue;
                }
                // `Copy` and `Eq` have no methods; a copying `Clone` is `*self`. A const
                // parameter does not stop `*self`: only a type parameter, which might not be
                // `Copy`, does.
                let header = header(d, &name, &generics);
                let body = if copies {
                    "fn clone(&self) -> Self { *self }"
                } else {
                    ""
                };
                let start = out.text.len() as u32;
                out.text.push_str(&format!("{header} {{ {body} }}"));
                let end = out.text.len() as u32;
                out.text.push('\n');
                out.origins.push((TextRange::new(start, end), src, *range));
            }
        }
    }
    out
}

/// The impl of the prelude's `__Invariant` for struct `node`, named `name` with `generics`, if it
/// has `invariant`s (`contract.invariant.1`): `__holds` binds each field by its name, borrowed,
/// and is every invariant joined by `&&`.
fn invariant_impl(
    node: &SyntaxNode,
    name: &str,
    generics: &[(String, Option<String>)],
) -> Option<String> {
    let body = node.child(K::STRUCT_BODY)?;
    let conds: Vec<String> = body
        .children()
        .filter(|c| c.kind() == K::INVARIANT)
        .filter_map(|c| c.children().find_map(ast::Expr::cast))
        .map(|e| format!("({})", e.syntax().text()))
        .collect();
    if conds.is_empty() {
        return None;
    }
    let fields: Vec<String> = body
        .children()
        .filter(|c| c.kind() == K::FIELD)
        .map(|f| name_of(&f))
        .collect();
    Some(format!(
        "impl{} __Invariant for {name}{} {{ fn __holds(&self) -> bool {{ let {name} {{ {} }} = \
         self; {} }} }}",
        unbounded(generics),
        args_of(generics),
        fields.join(", "),
        conds.join(" && ")
    ))
}

/// The derived impl of trait `tr` for struct `name` with type parameters `params` and named
/// fields `fields` (a unit struct if none), shown by `Debug` as `shown`.
pub(crate) fn struct_impl(
    tr: &str,
    name: &str,
    shown: &str,
    params: &[String],
    fields: &[String],
) -> String {
    let v = [Variant {
        path: name.to_string(),
        shown: shown.to_string(),
        fields: fields.to_vec(),
        named: !fields.is_empty(),
        unit: fields.is_empty(),
    }];
    let body = match tr {
        "Debug" => debug(&v),
        "Clone" => clone(&v),
        _ => partial_eq(&v),
    };
    let generics: Vec<(String, Option<String>)> =
        params.iter().map(|p| (p.clone(), None)).collect();
    format!("{} {{ {body} }}", header(tr, name, &generics))
}

/// `<A, S>` for generic parameters `generics` (type parameters, and const parameters with their
/// declarations), or nothing.
fn args_of(generics: &[(String, Option<String>)]) -> String {
    if generics.is_empty() {
        String::new()
    } else {
        let names: Vec<&str> = generics.iter().map(|(n, _)| n.as_str()).collect();
        format!("<{}>", names.join(", "))
    }
}

/// `<T, const S: int>` for `generics`, each declared with no bound, or nothing: an impl of a
/// trait whose derive gives no bounds here. A template's derive is given its bounds where it is
/// collected (`comptime.template.1`).
fn unbounded(generics: &[(String, Option<String>)]) -> String {
    let decls: Vec<&str> = generics
        .iter()
        .map(|(n, d)| d.as_deref().unwrap_or(n))
        .collect();
    if decls.is_empty() {
        String::new()
    } else {
        format!("<{}>", decls.join(", "))
    }
}

/// `impl<T: Trait, const S: int> Trait for Name<T, S>`.
fn header(tr: &str, name: &str, generics: &[(String, Option<String>)]) -> String {
    if generics.is_empty() {
        return format!("impl {tr} for {name}");
    }
    let decls: Vec<String> = generics
        .iter()
        .map(|(n, decl)| decl.clone().unwrap_or_else(|| format!("{n}: {tr}")))
        .collect();
    format!(
        "impl<{}> {tr} for {name}{}",
        decls.join(", "),
        args_of(generics)
    )
}

fn clone(variants: &[Variant]) -> String {
    let arms: Vec<String> = variants
        .iter()
        .map(|v| {
            let values: Vec<String> = (0..v.fields.len())
                .map(|i| format!("a{i}.clone()"))
                .collect();
            format!("{} => {},", v.bind("a"), v.build(&values))
        })
        .collect();
    format!(
        "fn clone(&self) -> Self {{ match self {{ {} }} }}",
        arms.join(" ")
    )
}

/// `Debug` (`attr.derive.debug.1`): the value as Moddil source writes it, `Point { x: 1, y: 2 }`
/// or `Shape::Circle(3)`, each field by its own `Debug`.
fn debug(variants: &[Variant]) -> String {
    let arms: Vec<String> = variants
        .iter()
        .map(|v| {
            let mut parts = Vec::new();
            let text = |s: &str| format!("out.write_str({s:?});");
            if v.unit {
                parts.push(text(&v.shown));
            } else if v.named {
                parts.push(text(&format!("{} {{ ", v.shown)));
                for (i, f) in v.fields.iter().enumerate() {
                    let sep = if i == 0 { "" } else { ", " };
                    parts.push(text(&format!("{sep}{f}: ")));
                    parts.push(format!("Debug::fmt(a{i}, out);"));
                }
                parts.push(text(" }"));
            } else {
                parts.push(text(&format!("{}(", v.shown)));
                for i in 0..v.fields.len() {
                    if i > 0 {
                        parts.push(text(", "));
                    }
                    parts.push(format!("Debug::fmt(a{i}, out);"));
                }
                parts.push(text(")"));
            }
            format!("{} => {{ {} }}", v.bind("a"), parts.join(" "))
        })
        .collect();
    format!(
        "fn fmt(&self, out: &mut dyn TextWrite) {{ match self {{ {} }} }}",
        arms.join(" ")
    )
}

fn partial_eq(variants: &[Variant]) -> String {
    let arms: Vec<String> = variants
        .iter()
        .map(|v| {
            let mut same: Vec<String> = (0..v.fields.len())
                .map(|i| format!("*a{i} == *b{i}"))
                .collect();
            if same.is_empty() {
                same.push("true".to_string());
            }
            let other = if variants.len() > 1 {
                " _ => false,"
            } else {
                ""
            };
            format!(
                "{} => match other {{ {} => {},{other} }},",
                v.bind("a"),
                v.bind("b"),
                same.join(" && ")
            )
        })
        .collect();
    format!(
        "fn eq(&self, other: &Self) -> bool {{ match self {{ {} }} }}",
        arms.join(" ")
    )
}

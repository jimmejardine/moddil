//! Derives: `#[derive(Clone, PartialEq, …)]` on a struct or enum expands to impls written in
//! Moddil, which are then checked and compiled like any other code (`values.md`: Explicit
//! derives). The expansion is structural: field by field, and variant order for enums.

use moddil_syntax::ast;
use moddil_syntax::{SyntaxKind as K, SyntaxNode, TextRange};

use crate::items::Src;

/// The traits that can be derived.
pub(crate) const DERIVABLE: &[&str] = &["Copy", "Clone", "PartialEq", "Eq", "PartialOrd", "Ord"];

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

/// A struct's or variant's fields, as a derive sees them.
struct Variant {
    /// How it is written in a pattern or expression: `Point`, `Shape::Circle`.
    path: String,
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

fn name_of(node: &SyntaxNode) -> String {
    node.child(K::NAME)
        .and_then(ast::Name::cast)
        .map(|n| n.text())
        .unwrap_or_default()
}

fn variant(node: &SyntaxNode, path: String) -> Variant {
    if let Some(body) = node.child(K::STRUCT_BODY) {
        let fields = body
            .children()
            .filter(|f| f.kind() == K::FIELD)
            .map(|f| name_of(&f))
            .collect();
        Variant {
            path,
            fields,
            named: true,
            unit: false,
        }
    } else if let Some(list) = node.child(K::TUPLE_FIELDS) {
        let n = list
            .children()
            .filter(|f| f.kind() == K::TUPLE_FIELD)
            .count();
        Variant {
            path,
            fields: vec![String::new(); n],
            named: false,
            unit: false,
        }
    } else {
        Variant {
            path,
            fields: Vec::new(),
            named: false,
            unit: true,
        }
    }
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

    /// A pattern matching the variant whatever its fields hold.
    fn any(&self) -> String {
        match (self.unit, self.named) {
            (true, _) => self.path.clone(),
            (false, true) => format!("{} {{ .. }}", self.path),
            (false, false) => format!("{}(..)", self.path),
        }
    }
}

/// The derived impls of every struct and enum among `items` (top-level nodes, with their source).
pub(crate) fn expand(items: impl Iterator<Item = (Src, SyntaxNode)>) -> Expansion {
    let mut out = Expansion::default();
    for (src, node) in items {
        if !matches!(node.kind(), K::STRUCT_ITEM | K::ENUM_ITEM) {
            continue;
        }
        let name = name_of(&node);
        let params: Vec<String> = node
            .child(K::GENERIC_PARAMS)
            .map(|g| {
                g.children()
                    .filter(|c| c.kind() == K::TYPE_PARAM)
                    .map(|p| name_of(&p))
                    .collect()
            })
            .unwrap_or_default();
        let variants: Vec<Variant> = if node.kind() == K::STRUCT_ITEM {
            vec![variant(&node, name.clone())]
        } else {
            node.child(K::VARIANT_LIST)
                .map(|l| {
                    l.children()
                        .filter(|v| v.kind() == K::VARIANT)
                        .map(|v| {
                            let path = format!("{name}::{}", name_of(&v));
                            variant(&v, path)
                        })
                        .collect()
                })
                .unwrap_or_default()
        };
        let attrs = derive_attrs(&node);
        let all: Vec<&String> = attrs.iter().flat_map(|(_, n)| n).collect();
        let copy = all.iter().any(|n| *n == "Copy");
        let has_clone = all.iter().any(|n| *n == "Clone");
        for (range, names) in &attrs {
            for d in names {
                // `Copy` without `Clone` is reported where the item is collected.
                if !DERIVABLE.contains(&d.as_str()) || (d == "Copy" && !has_clone) {
                    continue;
                }
                let header = header(d, &name, &params);
                let body = match d.as_str() {
                    "Copy" | "Eq" => String::new(),
                    "Clone" if copy && params.is_empty() => {
                        "fn clone(&self) -> Self { *self }".to_string()
                    }
                    "Clone" => clone(&variants),
                    "PartialEq" => partial_eq(&variants),
                    "PartialOrd" => ordering(&variants, true),
                    _ => ordering(&variants, false),
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

/// `impl<T: Trait> Trait for Name<T>`.
fn header(tr: &str, name: &str, params: &[String]) -> String {
    if params.is_empty() {
        return format!("impl {tr} for {name}");
    }
    let bounded: Vec<String> = params.iter().map(|p| format!("{p}: {tr}")).collect();
    format!(
        "impl<{}> {tr} for {name}<{}>",
        bounded.join(", "),
        params.join(", ")
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

/// `partial_cmp` (`partial` is true) or `cmp`: by variant, then field by field.
fn ordering(variants: &[Variant], partial: bool) -> String {
    let (method, ret) = if partial {
        ("partial_cmp", "Option<Ordering>")
    } else {
        ("cmp", "Ordering")
    };
    let wrap = |s: &str| {
        if partial {
            format!("Some({s})")
        } else {
            s.to_string()
        }
    };
    let arms: Vec<String> = variants
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let mut fields = wrap("Ordering::Equal");
            for f in (0..v.fields.len()).rev() {
                fields = format!(
                    "match a{f}.{method}(b{f}) {{ {} => {fields}, c => c, }}",
                    wrap("Ordering::Equal")
                );
            }
            let mut inner = vec![format!("{} => {fields},", v.bind("b"))];
            for (j, w) in variants.iter().enumerate() {
                if j != i {
                    let o = if i < j {
                        "Ordering::Less"
                    } else {
                        "Ordering::Greater"
                    };
                    inner.push(format!("{} => {},", w.any(), wrap(o)));
                }
            }
            format!("{} => match other {{ {} }},", v.bind("a"), inner.join(" "))
        })
        .collect();
    format!(
        "fn {method}(&self, other: &Self) -> {ret} {{ match self {{ {} }} }}",
        arms.join(" ")
    )
}

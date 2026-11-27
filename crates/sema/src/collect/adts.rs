//! Structs and enums: their names and type parameters first, then their fields and variants.

use moddil_syntax::ast::{self, AstNode};
use moddil_syntax::{SyntaxKind as K, SyntaxNode, TextRange};

use crate::derive::{self, DERIVABLE};
use crate::items::{AdtDef, Def, FieldDef, Generics, Shape, Src, VariantDef, Vis};
use crate::resolve::{Scope, resolve_type};
use crate::{Code, Diagnostic, Ty};

use super::{Collector, name_of, vis_of};

impl Collector<'_> {
    pub(super) fn adt_skeleton(&mut self, src: Src, node: &SyntaxNode) {
        let (name, name_range) = name_of(node);
        let (params, bounded) = self.params(src, None, node.child(K::GENERIC_PARAMS), &[]);
        let mut ds = Vec::new();
        for (_, p) in &bounded {
            if p.child(K::BOUND).is_some() {
                ds.push(Diagnostic::unsupported(
                    p.text_range(),
                    "a bound on a struct's or enum's type parameter",
                ));
            }
        }
        if let Some(w) = node.child(K::WHERE_CLAUSE) {
            ds.push(Diagnostic::unsupported(
                w.text_range(),
                "a `where` clause on a struct or enum",
            ));
        }
        self.reporter.report(src, None, ds);
        let bare = src == Src::Prelude && matches!(name.as_str(), "Option" | "Result");
        let vis = match src {
            Src::File(_) => vis_of(node),
            _ => Vis::Pub,
        };
        let index = self.tree.adts.len() as u32;
        self.declare(src, &name, Def::Adt(index), vis, name_range);
        if bare {
            // `Some`, `None`, `Ok` and `Err` are the prelude's names too.
            for (k, v) in node
                .child(K::VARIANT_LIST)
                .into_iter()
                .flat_map(|l| l.children().collect::<Vec<_>>())
                .filter(|v| v.kind() == K::VARIANT)
                .enumerate()
            {
                let (n, r) = name_of(&v);
                self.declare(src, &n, Def::Variant(index, k as u32), Vis::Pub, r);
            }
        }
        self.tree.adts.push(AdtDef {
            name,
            name_range,
            src,
            is_enum: node.kind() == K::ENUM_ITEM,
            is_view: node.kind() == K::VIEW_ITEM,
            copy: false,
            generics: Generics {
                params,
                parent: 0,
                preds: Vec::new(),
            },
            variants: Vec::new(),
            bare_variants: bare,
            vis,
            module: self.module,
            root: false,
            repr_c: false,
            py_export: false,
            shown: None,
        });
    }

    pub(super) fn adt_fields(&mut self, i: usize, src: Src, node: &SyntaxNode) {
        self.unsupported_parts(src, node, true);
        let mut ds = Vec::new();
        if let Some(t) = node.token(K::LINEAR_KW) {
            ds.push(Diagnostic::unsupported(t.text_range(), "a `linear` struct"));
        }
        let params = self.tree.adts[i].generics.params.clone();
        // Its type parameters' defaults (`generic.default.1`).
        let mut with_defaults = params.clone();
        {
            let scope = Scope {
                params: &params,
                ..self.scope()
            };
            Self::param_defaults(node, 0, &mut with_defaults, &scope, &mut ds);
        }
        self.tree.adts[i].generics.params = with_defaults;
        let scope = Scope {
            params: &params,
            ..self.scope()
        };
        let variants = if matches!(node.kind(), K::STRUCT_ITEM | K::VIEW_ITEM) {
            let (name, name_range) = name_of(node);
            vec![variant_def(node, name, name_range, &scope, &mut ds)]
        } else {
            let mut variants: Vec<VariantDef> = Vec::new();
            for v in node
                .child(K::VARIANT_LIST)
                .into_iter()
                .flat_map(|l| l.children().collect::<Vec<_>>())
                .filter(|c| c.kind() == K::VARIANT)
            {
                for c in v.children() {
                    // `#[default]` marks the variant `#[derive(Default)]` gives.
                    let is_default = c
                        .child(K::ATTR)
                        .and_then(|a| a.child(K::PATH))
                        .and_then(ast::Path::cast)
                        .is_some_and(|p| p.segments() == ["default"]);
                    if c.kind() == K::OUTER_ATTR && !is_default && !crate::is_doc(&c) {
                        ds.push(Diagnostic::unsupported(c.text_range(), "an attribute"));
                    }
                }
                if v.token(K::EQ).is_some() {
                    ds.push(Diagnostic::unsupported(
                        v.text_range(),
                        "an explicit discriminant",
                    ));
                }
                let (name, name_range) = name_of(&v);
                if let Some(first) = variants.iter().find(|x| x.name == name && !name.is_empty()) {
                    ds.push(
                        Diagnostic::error(
                            Code::E0103,
                            name_range,
                            format!("the variant `{name}` is defined more than once"),
                        )
                        .with_label(first.name_range, "first defined here"),
                    );
                }
                variants.push(variant_def(&v, name, name_range, &scope, &mut ds));
            }
            variants
        };
        // Attributes: derives, and `#[root_capability]` on the struct of a stub mod or of one
        // with the `os` capability (`main.root.1`).
        let mut copy = false;
        let stub = src.is_file()
            && self
                .tree
                .mods
                .get(self.tree.mod_of(self.module) as usize)
                .is_some_and(|m| m.stub || m.os_ok);
        for attr in node
            .children()
            .filter(|c| c.kind() == K::OUTER_ATTR && !crate::is_doc(c))
        {
            let path = attr
                .child(K::ATTR)
                .and_then(|a| a.child(K::PATH))
                .and_then(ast::Path::cast)
                .map(|p| p.segments())
                .unwrap_or_default();
            let input = attr
                .child(K::ATTR)
                .and_then(|a| a.child(K::ATTR_INPUT))
                .map(|t| t.text().replace(char::is_whitespace, ""));
            if path == ["root_capability"] && stub && node.kind() == K::STRUCT_ITEM {
                self.tree.adts[i].root = true;
            } else if path == ["repr"]
                && input.as_deref() == Some("(C)")
                && node.kind() == K::STRUCT_ITEM
            {
                // C's layout (`ffi.repr.1`).
                self.tree.adts[i].repr_c = true;
            } else if path == ["export"]
                && input.as_deref() == Some("(python)")
                && node.kind() == K::STRUCT_ITEM
            {
                // A Python class (`ffi.python.2`).
                self.tree.adts[i].py_export = true;
            } else if path == ["shown"]
                && self.tree.adts[i].name.starts_with("__Args_")
                && let Some(s) = input
                    .as_deref()
                    .and_then(|s| s.strip_prefix("(\""))
                    .and_then(|s| s.strip_suffix("\")"))
            {
                // How the expansion's hidden struct is shown (`attr.around.args.1`).
                self.tree.adts[i].shown = Some(s.to_string());
            } else if path != ["derive"] {
                ds.push(Diagnostic::unsupported(attr.text_range(), "this attribute"));
            }
        }
        for (range, names) in derive::derive_attrs(node) {
            for n in &names {
                if !DERIVABLE.contains(&n.as_str())
                    && !self.stub_trait(n)
                    && self.derive_fn(n).is_none()
                {
                    ds.push(Diagnostic::unsupported(range, &format!("deriving `{n}`")));
                }
            }
            // An enum's default is one of its unit variants, marked `#[default]`.
            if names.iter().any(|n| n == "Default") && node.kind() == K::ENUM_ITEM {
                let marked: Vec<SyntaxNode> = node
                    .child(K::VARIANT_LIST)
                    .into_iter()
                    .flat_map(|l| l.children().collect::<Vec<_>>())
                    .filter(|v| {
                        v.children()
                            .filter_map(|c| c.child(K::ATTR))
                            .filter_map(|a| a.child(K::PATH))
                            .filter_map(ast::Path::cast)
                            .any(|p| p.segments() == ["default"])
                    })
                    .collect();
                let unit = |v: &SyntaxNode| {
                    v.child(K::STRUCT_BODY).is_none() && v.child(K::TUPLE_FIELDS).is_none()
                };
                if marked.len() != 1 || !unit(&marked[0]) {
                    ds.push(Diagnostic::error(
                        Code::E0416,
                        range,
                        "an enum derives `Default` from one unit variant marked `#[default]`",
                    ));
                }
            }
            if names.iter().any(|n| n == "Copy") {
                copy = true;
                if !names.iter().any(|n| n == "Clone") {
                    ds.push(Diagnostic::error(
                        Code::E0311,
                        range,
                        "a `Copy` type is also `Clone`: write `#[derive(Copy, Clone)]`",
                    ));
                }
            }
        }
        self.reporter.report(src, None, ds);
        // The prelude's, `core`'s and derived types' fields are everyone's; so are an enum's
        // variants' fields, which are reached by matching (`mod.vis.2`).
        let mut variants = variants;
        if !src.is_file() || node.kind() == K::ENUM_ITEM {
            for f in variants.iter_mut().flat_map(|v| v.fields.iter_mut()) {
                f.vis = Vis::Pub;
            }
        }
        let def = &mut self.tree.adts[i];
        def.variants = variants;
        def.copy = copy;
    }
}

/// A struct's or variant's fields, from its `STRUCT_BODY` or `TUPLE_FIELDS` child.
pub(super) fn variant_def(
    node: &SyntaxNode,
    name: String,
    name_range: TextRange,
    scope: &Scope,
    diagnostics: &mut Vec<Diagnostic>,
) -> VariantDef {
    let mut fields: Vec<FieldDef> = Vec::new();
    let shape = if let Some(body) = node.child(K::STRUCT_BODY) {
        for m in body.children() {
            // An invariant is the derived `__Invariant`'s (`contract.invariant.1`).
            if m.kind() != K::FIELD {
                continue;
            }
            for c in m.children() {
                if c.kind() == K::OUTER_ATTR && !crate::is_doc(&c) {
                    diagnostics.push(Diagnostic::unsupported(c.text_range(), "an attribute"));
                }
            }
            if m.token(K::EQ).is_some() {
                diagnostics.push(Diagnostic::unsupported(
                    m.text_range(),
                    "a field's default value",
                ));
            }
            let (fname, range) = name_of(&m);
            let ty = match m.children().find_map(ast::Type::cast) {
                Some(t) => field_type(&t, scope, diagnostics),
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
                vis: vis_of(&m),
            });
        }
        Shape::Named
    } else if let Some(list) = node.child(K::TUPLE_FIELDS) {
        for (i, f) in list
            .children()
            .filter(|c| c.kind() == K::TUPLE_FIELD)
            .enumerate()
        {
            for c in f.children() {
                if c.kind() == K::OUTER_ATTR && !crate::is_doc(&c) {
                    diagnostics.push(Diagnostic::unsupported(c.text_range(), "an attribute"));
                }
            }
            let ty = match f.children().find_map(ast::Type::cast) {
                Some(t) => field_type(&t, scope, diagnostics),
                None => Ty::Error,
            };
            fields.push(FieldDef {
                name: i.to_string(),
                range: f.text_range(),
                ty,
                vis: vis_of(&f),
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
        default: node.kind() == K::VARIANT && derive::is_default(node),
    }
}

/// A field's type: borrows and views are second-class, so only a view's field can hold one
/// (E0309).
pub(super) fn field_type(t: &ast::Type, scope: &Scope, diagnostics: &mut Vec<Diagnostic>) -> Ty {
    let ty = resolve_type(t, scope, diagnostics);
    let in_view = t.syntax().ancestors().any(|a| a.kind() == K::VIEW_ITEM);
    if !in_view && scope.tree.is_view(&ty) {
        diagnostics.push(Diagnostic::error(
            Code::E0309,
            t.syntax().text_range(),
            "a field cannot hold a borrow or a view: borrows live only in parameters, locals \
             and views (`view Name { … }`)",
        ));
        return Ty::Error;
    }
    ty
}

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
        });
    }

    pub(super) fn adt_fields(&mut self, i: usize, src: Src, node: &SyntaxNode) {
        self.unsupported_parts(src, node, true);
        let mut ds = Vec::new();
        if let Some(t) = node.token(K::LINEAR_KW) {
            ds.push(Diagnostic::unsupported(t.text_range(), "a `linear` struct"));
        }
        let params = self.tree.adts[i].generics.params.clone();
        let scope = Scope {
            params: &params,
            ..self.scope()
        };
        let variants = if node.kind() == K::STRUCT_ITEM {
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
                    if c.kind() == K::OUTER_ATTR {
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
        // Attributes: only derives.
        let mut copy = false;
        for attr in node.children().filter(|c| c.kind() == K::OUTER_ATTR) {
            let is_derive = attr
                .child(K::ATTR)
                .and_then(|a| a.child(K::PATH))
                .and_then(ast::Path::cast)
                .is_some_and(|p| p.segments() == ["derive"]);
            if !is_derive {
                ds.push(Diagnostic::unsupported(attr.text_range(), "this attribute"));
            }
        }
        for (range, names) in derive::derive_attrs(node) {
            for n in &names {
                if !DERIVABLE.contains(&n.as_str()) {
                    ds.push(Diagnostic::unsupported(range, &format!("deriving `{n}`")));
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
            if m.kind() != K::FIELD {
                if m.kind() != K::OUTER_ATTR {
                    diagnostics.push(Diagnostic::unsupported(m.text_range(), "an invariant"));
                }
                continue;
            }
            for c in m.children() {
                if c.kind() == K::OUTER_ATTR {
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
                if c.kind() == K::OUTER_ATTR {
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

/// A field's type: borrows are second-class, so a field cannot hold one (E0309).
pub(super) fn field_type(t: &ast::Type, scope: &Scope, diagnostics: &mut Vec<Diagnostic>) -> Ty {
    let ty = resolve_type(t, scope, diagnostics);
    if ty.has_ref() {
        diagnostics.push(Diagnostic::error(
            Code::E0309,
            t.syntax().text_range(),
            "a field cannot hold a borrow: borrows live only in parameters and locals",
        ));
        return Ty::Error;
    }
    ty
}

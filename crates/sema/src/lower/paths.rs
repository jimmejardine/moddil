//! Value paths: what a name or `a::b` refers to (a local, a function, a constructor, an
//! associated or trait function), with its type arguments.

use moddil_syntax::{SyntaxKind as K, SyntaxNode, TextRange};

use super::{Lowerer, Resolved};
use crate::hir::{ExprId, ExprKind};
use crate::items::Shape;
use crate::resolve::{adt_args, generic_args, path_segments};
use crate::{Code, Diagnostic, Ty};

impl Lowerer<'_> {
    /// The segments of the `PATH` in `node` (without generic arguments), or `None` after
    /// reporting a path this slice does not support.
    pub(super) fn segments(&mut self, node: &SyntaxNode) -> Option<Vec<String>> {
        let segs = node.child(K::PATH).and_then(|p| path_segments(&p));
        match segs {
            Some(segs) if !segs.is_empty() => Some(segs.into_iter().map(|s| s.name).collect()),
            _ => {
                self.error(Diagnostic::unsupported(node.text_range(), "this path"));
                None
            }
        }
    }

    /// The type arguments of struct or enum `adt` written on a segment, or holes.
    pub(super) fn adt_args(
        &mut self,
        adt: u32,
        args: Option<&SyntaxNode>,
        range: TextRange,
    ) -> Vec<Ty> {
        let mut ds = Vec::new();
        let args = adt_args(adt, args, range, &self.types, &mut ds);
        self.out.diagnostics.extend(ds);
        args
    }

    /// What the value path in `node` names, or `None` after reporting why it names nothing.
    pub(super) fn resolve(&mut self, node: &SyntaxNode) -> Option<Resolved> {
        let range = node.text_range();
        let Some(segs) = node.child(K::PATH).and_then(|p| path_segments(&p)) else {
            self.error(Diagnostic::unsupported(range, "this path"));
            return None;
        };
        match segs.as_slice() {
            [seg] => {
                let name = seg.name.as_str();
                if let Some(local) = self.lookup(name) {
                    if seg.args.is_some() {
                        self.report(
                            Code::E0404,
                            range,
                            format!("`{name}` takes no type arguments"),
                        );
                    }
                    return Some(Resolved::Local(local));
                }
                if let Some(f) = self.items.fn_named(name) {
                    let sig = &self.items.fns[f as usize];
                    let want = sig.generics.params.len() - sig.generics.parent as usize;
                    let args = match &seg.args {
                        Some(a) => self.type_args(a, want, &format!("`{name}`")),
                        None => vec![Ty::Infer(Ty::HOLE); want],
                    };
                    return Some(Resolved::Fn(f, args));
                }
                if let Some((a, v)) = self.items.bare_variant(name) {
                    let args = self.adt_args(a, seg.args.as_ref(), range);
                    return Some(Resolved::Ctor(a, v, args));
                }
                if let Some(a) = self.items.adt_named(name) {
                    if !self.items.adts[a as usize].is_enum {
                        let args = self.adt_args(a, seg.args.as_ref(), range);
                        return Some(Resolved::Ctor(a, 0, args));
                    }
                    self.report(
                        Code::E0111,
                        range,
                        format!("`{name}` is an enum; name one of its variants: `{name}::…`"),
                    );
                    return None;
                }
                if name == "Self"
                    && let Some(Ty::Adt(a, _, args)) = self.types.self_ty.clone()
                    && !self.items.adts[a as usize].is_enum
                {
                    return Some(Resolved::Ctor(a, 0, args.to_vec()));
                }
                if name == "assert" {
                    self.report(
                        Code::E0112,
                        range,
                        "`assert` is a built-in, not a value: call it as `assert(condition)`",
                    );
                    return None;
                }
                self.report(
                    Code::E0101,
                    range,
                    format!("there is nothing called `{name}` here"),
                );
                None
            }
            [ty, item] => {
                let own_args = |cx: &mut Self| -> Option<Vec<Ty>> {
                    item.args.as_ref().map(|a| {
                        let mut ds = Vec::new();
                        let (types, _) = generic_args(a, &cx.types, &mut ds);
                        cx.out.diagnostics.extend(ds);
                        types
                    })
                };
                // `Trait::method`.
                if let Some(t) = self.items.trait_named(&ty.name) {
                    let def = &self.items.traits[t as usize];
                    let Some(&func) = def
                        .methods
                        .iter()
                        .find(|&&m| self.items.fns[m as usize].name == item.name)
                    else {
                        self.report(
                            Code::E0413,
                            item.range,
                            format!("`{}` has no method `{}`", ty.name, item.name),
                        );
                        return None;
                    };
                    let want = def.generics.params.len() - 1;
                    let trait_args = match &ty.args {
                        Some(a) => self.type_args(a, want, &format!("`{}`", ty.name)),
                        None => vec![Ty::Infer(Ty::HOLE); want],
                    };
                    let args = own_args(self);
                    return Some(Resolved::TraitFn(t, func, trait_args, args));
                }
                // The type: an enum's variant, or a function of the type.
                let base = match ty.name.as_str() {
                    "int" => Ty::Int,
                    "bool" => Ty::Bool,
                    "Self" => match &self.types.self_ty {
                        Some(t) => t.clone(),
                        None => {
                            self.report(
                                Code::E0102,
                                ty.range,
                                "`Self` is the type of an impl or trait, and there is none here",
                            );
                            return None;
                        }
                    },
                    name => {
                        if let Some(i) = self.types.params.iter().rposition(|p| p.name == name) {
                            Ty::Param(i as u32, name.into())
                        } else if let Some(a) = self.items.adt_named(name) {
                            let args = self.adt_args(a, ty.args.as_ref(), ty.range);
                            self.items.adts[a as usize].ty_with(a, args)
                        } else {
                            self.report(
                                Code::E0101,
                                ty.range,
                                format!("there is no type called `{name}`"),
                            );
                            return None;
                        }
                    }
                };
                if let Ty::Adt(a, _, args) = &base {
                    let def = &self.items.adts[*a as usize];
                    if def.is_enum
                        && let Some(v) = def.variant(&item.name)
                    {
                        if item.args.is_some() {
                            self.report(
                                Code::E0404,
                                item.range,
                                "a variant takes no type arguments: write them on the enum",
                            );
                        }
                        return Some(Resolved::Ctor(*a, v, args.to_vec()));
                    }
                }
                let args = own_args(self);
                Some(Resolved::AssocFn(base, item.name.clone(), item.range, args))
            }
            _ => {
                self.error(Diagnostic::unsupported(range, "this path"));
                None
            }
        }
    }

    /// The expression for a resolved path.
    pub(super) fn resolved(&mut self, r: Resolved, range: TextRange) -> ExprId {
        match r {
            Resolved::Local(l) => self.alloc(ExprKind::Local(l), range),
            Resolved::Fn(func, args) => self.alloc(ExprKind::Fn { func, args }, range),
            Resolved::AssocFn(ty, name, name_range, args) => self.alloc(
                ExprKind::AssocFn {
                    ty,
                    name,
                    name_range,
                    args,
                },
                range,
            ),
            Resolved::TraitFn(trait_, func, trait_args, args) => self.alloc(
                ExprKind::TraitFn {
                    trait_,
                    func,
                    trait_args,
                    args,
                },
                range,
            ),
            Resolved::Ctor(adt, variant, args) => {
                let def = self.items.variant(adt, variant);
                match def.shape {
                    Shape::Unit => self.alloc(
                        ExprKind::Adt {
                            adt,
                            variant,
                            fields: Vec::new(),
                            args,
                        },
                        range,
                    ),
                    Shape::Tuple => self.unsupported(range, "a constructor used as a value"),
                    Shape::Named => {
                        let path = self.items.adts[adt as usize].path(variant);
                        self.report(
                            Code::E0111,
                            range,
                            format!("`{path}` has named fields: write `{path} {{ … }}`"),
                        );
                        self.missing(range)
                    }
                }
            }
        }
    }
}

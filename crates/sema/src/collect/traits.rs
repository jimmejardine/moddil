//! Traits and impls: trait headers, their methods and associated types, and each impl.

use std::collections::HashMap;

use moddil_syntax::ast::{self, AstNode};
use moddil_syntax::{SyntaxKind as K, SyntaxNode, TextRange};

use crate::items::{Container, Def, GenericParam, Generics, ImplDef, Pred, Src, TraitDef, Vis};
use crate::resolve::{Scope, bounds, preds_of, resolve_type, trait_path};
use crate::ty::TraitRef;
use crate::{Code, Diagnostic, Ty};

use super::{Collector, name_of, vis_of};

impl Collector<'_> {
    pub(super) fn trait_skeleton(&mut self, src: Src, node: &SyntaxNode) {
        let (name, name_range) = name_of(node);
        let self_param = GenericParam {
            name: "Self".to_string(),
            range: name_range,
            konst: false,
        };
        let (params, _) = self.params(src, None, node.child(K::GENERIC_PARAMS), &[self_param]);
        let mut assoc = Vec::new();
        let mut ds = Vec::new();
        if let Some(t) = node.token(K::UNSAFE_KW) {
            ds.push(Diagnostic::unsupported(t.text_range(), "an `unsafe` trait"));
        }
        for m in node
            .child(K::TRAIT_BODY)
            .into_iter()
            .flat_map(|b| b.children().collect::<Vec<_>>())
        {
            if m.kind() == K::ASSOC_TYPE {
                let (n, r) = name_of(&m);
                if m.child(K::BOUND).is_some() || m.token(K::COLON).is_some() {
                    ds.push(Diagnostic::unsupported(
                        m.text_range(),
                        "a bound on an associated type",
                    ));
                }
                if let Some((_, first)) = assoc.iter().find(|(x, _): &&(String, TextRange)| *x == n)
                {
                    ds.push(
                        Diagnostic::error(
                            Code::E0103,
                            r,
                            format!("the associated type `{n}` is declared twice"),
                        )
                        .with_label(*first, "first declared here"),
                    );
                }
                assoc.push((n, r));
            } else if m.kind() == K::CONST_ITEM {
                ds.push(Diagnostic::unsupported(
                    m.text_range(),
                    "an associated const",
                ));
            }
        }
        self.reporter.report(src, None, ds);
        let vis = match src {
            Src::File(_) => vis_of(node),
            _ => Vis::Pub,
        };
        let index = self.tree.traits.len() as u32;
        self.declare(src, &name, Def::Trait(index), vis, name_range);
        if let Some(d) = super::templates::derive_by_attr(node) {
            self.derive_by_names.insert(index, d);
        }
        self.tree.traits.push(TraitDef {
            name,
            name_range,
            src,
            generics: Generics {
                params,
                parent: 0,
                preds: Vec::new(),
            },
            supertraits: Vec::new(),
            assoc_defaults: vec![None; assoc.len()],
            assoc,
            methods: Vec::new(),
            vis,
            module: self.module,
            derive_by: None,
        });
    }

    pub(super) fn trait_header(&mut self, i: usize, src: Src, node: &SyntaxNode) {
        self.unsupported_parts(src, node, true);
        self.derive_by(i, src, node);
        let def = &self.tree.traits[i];
        let params = def.generics.params.clone();
        let bounded: Vec<(usize, SyntaxNode)> = node
            .child(K::GENERIC_PARAMS)
            .into_iter()
            .flat_map(|g| g.children().collect::<Vec<_>>())
            .filter(|c| c.kind() == K::TYPE_PARAM)
            .enumerate()
            .map(|(k, p)| (k + 1, p))
            .collect();
        let self_ty = Ty::Param(0, "Self".into());
        let scope = Scope {
            params: &params,
            self_ty: Some(self_ty.clone()),
            trait_ctx: Some(i as u32),
            ..self.scope()
        };
        let mut ds = Vec::new();
        let supers: Vec<TraitRef> = bounds(node, &self_ty, &scope, &mut ds)
            .into_iter()
            .map(|p| p.trait_ref)
            .collect();
        let preds = preds_of(
            node,
            &bounded,
            &Scope {
                params: &params,
                self_ty: Some(self_ty),
                trait_ctx: Some(i as u32),
                ..self.scope()
            },
            &mut ds,
        );
        // The associated types' defaults (`trait.assoc.default.1`).
        let mut defaults = Vec::new();
        for m in node
            .child(K::TRAIT_BODY)
            .into_iter()
            .flat_map(|b| b.children().collect::<Vec<_>>())
            .filter(|m| m.kind() == K::ASSOC_TYPE)
        {
            let ty = match m.token(K::EQ) {
                Some(_) => m.children().find_map(ast::Type::cast).map(|t| {
                    resolve_type(
                        &t,
                        &Scope {
                            params: &params,
                            trait_ctx: Some(i as u32),
                            ..self.scope()
                        },
                        &mut ds,
                    )
                }),
                None => None,
            };
            defaults.push(ty);
        }
        self.reporter.report(src, None, ds);
        let def = &mut self.tree.traits[i];
        def.supertraits = supers;
        def.generics.preds = preds;
        if defaults.len() == def.assoc.len() {
            def.assoc_defaults = defaults;
        }
    }

    /// The bound a trait's items have on `Self`: `Self: Trait<…>`.
    pub(super) fn self_pred(&self, t: u32) -> Pred {
        let def = &self.tree.traits[t as usize];
        let args: Vec<Ty> = def.generics.identity().into_iter().skip(1).collect();
        Pred {
            ty: Ty::Param(0, "Self".into()),
            trait_ref: TraitRef {
                trait_: t,
                name: def.name.as_str().into(),
                args: args.into(),
            },
            assoc: Vec::new(),
            control: 0,
            range: def.name_range,
        }
    }

    pub(super) fn trait_methods(&mut self, i: usize, src: Src, node: &SyntaxNode) {
        let mut generics = self.tree.traits[i].generics.clone();
        generics.preds.insert(0, self.self_pred(i as u32));
        generics.parent = generics.params.len() as u32;
        let mut names: HashMap<String, TextRange> = HashMap::new();
        for m in node
            .child(K::TRAIT_BODY)
            .into_iter()
            .flat_map(|b| b.children().collect::<Vec<_>>())
        {
            let Some(f) = ast::FnItem::cast(m) else {
                continue;
            };
            let sig = self.fn_sig(
                &f,
                src,
                Container::Trait(i as u32),
                &generics,
                Some(&Ty::Param(0, "Self".into())),
                Some(i as u32),
                None,
            );
            if let Some(first) = names.get(&sig.name) {
                let d = Diagnostic::error(
                    Code::E0103,
                    sig.name_range,
                    format!("the method `{}` is declared twice", sig.name),
                )
                .with_label(*first, "first declared here");
                self.reporter.report(src, None, vec![d]);
            } else {
                names.insert(sig.name.clone(), sig.name_range);
            }
            self.tree.traits[i].methods.push(self.tree.fns.len() as u32);
            self.tree.fns.push(sig);
        }
    }

    pub(super) fn impl_item(&mut self, src: Src, node: &SyntaxNode) {
        let origin = self.reporter.origin(src, node.text_range());
        self.unsupported_parts(src, node, false);
        let index = self.tree.impls.len() as u32;
        let (params, bounded) = self.params(src, origin, node.child(K::GENERIC_PARAMS), &[]);
        let types: Vec<ast::Type> = node.children().filter_map(ast::Type::cast).collect();
        let is_trait = node.token(K::FOR_KW).is_some();
        let mut ds = Vec::new();
        if let Some(t) = node.token(K::UNSAFE_KW) {
            ds.push(Diagnostic::unsupported(t.text_range(), "an `unsafe` impl"));
        }
        let scope = Scope {
            params: &params,
            ..self.scope()
        };
        let self_node = if is_trait {
            types.get(1)
        } else {
            types.first()
        };
        let self_ty = match self_node {
            Some(t) => resolve_type(t, &scope, &mut ds),
            None => Ty::Error,
        };
        let header_end = self_node.map_or(node.text_range().end, |t| t.syntax().text_range().end);
        let start = node
            .child_tokens()
            .find(|t| matches!(t.kind(), K::IMPL_KW | K::UNSAFE_KW))
            .map_or(node.text_range().start, |t| t.text_range().start);
        let range = TextRange::new(start, header_end);
        let trait_ref = if is_trait {
            types
                .first()
                .and_then(|t| t.syntax().child(K::PATH))
                .and_then(|p| {
                    let r = trait_path(&p, &scope, &mut ds);
                    if let Some((_, fixed)) = &r
                        && !fixed.is_empty()
                    {
                        ds.push(Diagnostic::error(
                            Code::E0413,
                            p.text_range(),
                            "an impl defines associated types in its body, not in the trait's \
                             arguments",
                        ));
                    }
                    r.map(|(t, _)| t)
                })
        } else {
            None
        };
        self.reporter.report(src, origin, ds);
        // An impl of something that is not a trait is already reported; it is not inherent.
        let self_ty = if is_trait && trait_ref.is_none() {
            Ty::Error
        } else {
            self_ty
        };
        let scope = Scope {
            params: &params,
            self_ty: Some(self_ty.clone()),
            ..self.scope()
        };
        let mut ds = Vec::new();
        let mut preds = preds_of(node, &bounded, &scope, &mut ds);
        // A template's derive for a generic struct bounds each of its type parameters by the
        // trait, as the built-in derives do (`comptime.template.1`).
        if src == Src::Derived
            && self.derive_for_of(node.text_range()).is_none()
            && let Some(t) = &trait_ref
            && self.tree.traits[t.trait_ as usize].derive_by.is_some()
        {
            for (k, p) in params.iter().enumerate().filter(|(_, p)| !p.konst) {
                preds.push(Pred {
                    ty: Ty::Param(k as u32, p.name.as_str().into()),
                    trait_ref: t.clone(),
                    assoc: Vec::new(),
                    control: 0,
                    range: p.range,
                });
            }
        }
        // Every parameter must be fixed by the self type or the trait's arguments, or by what a
        // bound on such a parameter fixes (`F: FnMut(A) -> B` fixes `B` by `F`).
        let mut fixed: Vec<bool> = params
            .iter()
            .enumerate()
            .map(|(k, p)| {
                let me = Ty::Param(k as u32, p.name.as_str().into());
                self_ty.any(&|t| *t == me)
                    || trait_ref
                        .as_ref()
                        .is_some_and(|t| t.args.iter().any(|a| a.any(&|x| *x == me)))
            })
            .collect();
        loop {
            let mut more = false;
            for p in &preds {
                let by_fixed = match &p.ty {
                    Ty::Param(k, _) => fixed.get(*k as usize).copied().unwrap_or(true),
                    _ => true,
                };
                if !by_fixed {
                    continue;
                }
                for (_, t) in &p.assoc {
                    for (k, f) in fixed.iter_mut().enumerate() {
                        let me = Ty::Param(k as u32, params[k].name.as_str().into());
                        if !*f && t.any(&|x| *x == me) {
                            *f = true;
                            more = true;
                        }
                    }
                }
            }
            if !more {
                break;
            }
        }
        for (k, p) in params.iter().enumerate() {
            if !fixed[k] {
                ds.push(Diagnostic::error(
                    Code::E0415,
                    p.range,
                    format!(
                        "`{}` is not used by the type or trait the impl is for, so nothing can \
                         say what it is",
                        p.name
                    ),
                ));
            }
        }
        // Associated types.
        let n_assoc = trait_ref
            .as_ref()
            .map_or(0, |t| self.tree.traits[t.trait_ as usize].assoc.len());
        let mut assoc: Vec<Option<Ty>> = vec![None; n_assoc];
        // The associated types see the impl's bounds: `type Fails = I::Fails | F::Fails;`.
        let scope = Scope {
            preds: &preds,
            ..scope
        };
        let defaults: Vec<Option<Ty>> = trait_ref.as_ref().map_or_else(Vec::new, |t| {
            self.tree.traits[t.trait_ as usize].assoc_defaults.clone()
        });
        let members: Vec<SyntaxNode> = node
            .child(K::IMPL_BODY)
            .into_iter()
            .flat_map(|b| b.children().collect::<Vec<_>>())
            .collect();
        for m in &members {
            if m.token(K::DEFAULT_KW).is_some() {
                ds.push(Diagnostic::unsupported(
                    m.text_range(),
                    "specialisation (`default`)",
                ));
            }
            match m.kind() {
                K::ASSOC_TYPE => {
                    let (n, r) = name_of(m);
                    let Some(tr) = &trait_ref else {
                        ds.push(Diagnostic::unsupported(
                            m.text_range(),
                            "an associated type in an inherent impl",
                        ));
                        continue;
                    };
                    let tdef = &self.tree.traits[tr.trait_ as usize];
                    let Some(k) = tdef.assoc.iter().position(|(x, _)| *x == n) else {
                        ds.push(Diagnostic::error(
                            Code::E0408,
                            r,
                            format!("`{}` has no associated type `{n}`", tdef.name),
                        ));
                        continue;
                    };
                    // One type, or an error set (`type Fails = I::Fails | F::Fails;`).
                    let written: Vec<ast::Type> =
                        m.children().filter_map(ast::Type::cast).collect();
                    let ty = match written.as_slice() {
                        [] => Ty::Error,
                        [t] => resolve_type(t, &scope, &mut ds),
                        ts => {
                            let mut members = Vec::new();
                            for t in ts {
                                let ty = resolve_type(t, &scope, &mut ds);
                                if super::error_type(
                                    &ty,
                                    t.syntax().text_range(),
                                    &members,
                                    &mut ds,
                                ) {
                                    members.push(ty);
                                }
                            }
                            Ty::errors(members)
                        }
                    };
                    assoc[k] = Some(ty);
                }
                K::CONST_ITEM => {
                    ds.push(Diagnostic::unsupported(
                        m.text_range(),
                        "an associated const",
                    ));
                }
                _ => {}
            }
        }
        for (k, d) in defaults.into_iter().enumerate() {
            if let Some(slot @ None) = assoc.get_mut(k) {
                *slot = d;
            }
        }
        self.reporter.report(src, origin, ds);
        // A derive of a trait whose derive function is a template (`comptime.derive.1`), or an
        // impl a prelude derive function's `#[derive_for]` makes (`comptime.derive.for.1`): its
        // methods are the template's.
        let derive_for = (src == Src::Derived)
            .then(|| self.derive_for_of(node.text_range()))
            .flatten();
        let template = (src == Src::Derived)
            .then(|| {
                derive_for.or_else(|| {
                    trait_ref
                        .as_ref()
                        .and_then(|t| self.tree.traits[t.trait_ as usize].derive_by)
                })
            })
            .flatten();
        self.tree.impls.push(ImplDef {
            src,
            range,
            generics: Generics {
                params: params.clone(),
                parent: 0,
                preds: preds.clone(),
            },
            trait_ref: trait_ref.clone(),
            self_ty: self_ty.clone(),
            assoc: assoc.clone(),
            methods: Vec::new(),
            module: self.module,
            origin,
        });
        // Methods.
        let generics = Generics {
            parent: params.len() as u32,
            params,
            preds,
        };
        if let Some(f) = template {
            self.template_methods(
                f,
                index,
                &self_ty,
                &generics,
                trait_ref.as_ref(),
                &assoc,
                origin,
                derive_for.is_some(),
            );
            return;
        }
        // A function and a method may share a name, told apart by `self`
        // (`item.impl.same-name.1`): two of one kind may not.
        let mut names: HashMap<(String, bool), TextRange> = HashMap::new();
        for m in members {
            let Some(f) = ast::FnItem::cast(m) else {
                continue;
            };
            let sig = self.fn_sig(
                &f,
                src,
                Container::Impl(index),
                &generics,
                Some(&self_ty),
                None,
                trait_ref.clone().map(|t| (t, assoc.clone())),
            );
            let key = (sig.name.clone(), sig.self_kind.is_some());
            if let Some(first) = names.get(&key) {
                let d = Diagnostic::error(
                    Code::E0103,
                    sig.name_range,
                    format!("the method `{}` is defined twice", sig.name),
                )
                .with_label(*first, "first defined here");
                self.reporter.report(src, origin, vec![d]);
            } else {
                names.insert(key, sig.name_range);
            }
            self.tree.impls[index as usize]
                .methods
                .push(self.tree.fns.len() as u32);
            self.tree.fns.push(sig);
        }
    }
}

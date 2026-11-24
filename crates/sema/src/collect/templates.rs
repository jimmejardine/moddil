//! Derives written in Moddil (`comptime.derive.1`): a trait's `#[derive_by(f)]`, the derive
//! function `f`, `const fn f(t: type) -> Impl<Tr> { impl Tr for t { … } }`, and the impl a
//! `#[derive(Tr)]` instantiates from it. The template's methods are collected in the template's
//! own scope, with `t` and `Self` naming the struct; their bodies are lowered from the
//! template's source ([`crate::lower`]), which expands `inline for` and `if const`.

use moddil_syntax::ast::{self, AstNode};
use moddil_syntax::{SyntaxKind as K, SyntaxNode, TextRange};

use crate::items::{Container, Def, FnSig, Generics, ParamSig, Src, Template};
use crate::ty::TraitRef;
use crate::{Code, Diagnostic, Ty};

use super::{Collector, name_of};

/// Whether `node` is written as a derive function: a `const fn` whose one parameter's type is
/// `type`. Its shape is checked when its signature is collected.
pub(super) fn is_deriver(node: &SyntaxNode) -> bool {
    let Some(f) = ast::FnItem::cast(node.clone()) else {
        return false;
    };
    let params: Vec<ast::Param> = f.params().map(|l| l.params().collect()).unwrap_or_default();
    matches!(params.as_slice(), [p] if p.ty().is_some_and(|t| t.syntax().kind() == K::TYPE_TYPE))
}

/// The name `#[derive_by(f)]` on trait `node` gives, and where it is.
pub(super) fn derive_by_attr(node: &SyntaxNode) -> Option<(String, TextRange)> {
    for attr in node.children().filter(|c| c.kind() == K::OUTER_ATTR) {
        let Some(a) = attr.child(K::ATTR) else {
            continue;
        };
        let path = a
            .child(K::PATH)
            .and_then(ast::Path::cast)
            .map(|p| p.segments())
            .unwrap_or_default();
        if path != ["derive_by"] {
            continue;
        }
        let name = a
            .child(K::ATTR_INPUT)
            .and_then(|i| i.child_tokens().find(|t| t.kind() == K::IDENT))
            .map(|t| t.text().to_string())
            .unwrap_or_default();
        return Some((name, attr.text_range()));
    }
    None
}

/// The name of a derive function's parameter (`t`).
fn param_name(node: &SyntaxNode) -> String {
    ast::FnItem::cast(node.clone())
        .and_then(|f| f.params())
        .and_then(|l| l.params().next())
        .and_then(|p| p.pat())
        .map(|p| p.syntax().text().trim().to_string())
        .unwrap_or_default()
}

/// The template a derive function's body holds: its one `impl` item.
fn template_impl(node: &SyntaxNode) -> Option<SyntaxNode> {
    let body = node.child(K::BLOCK)?;
    let items: Vec<SyntaxNode> = body
        .descendants()
        .into_iter()
        .filter(|n| n.kind() == K::IMPL_ITEM)
        .filter(|n| {
            n.parent().is_some_and(|p| {
                p.kind() == K::BLOCK || p.parent().is_some_and(|q| q.kind() == K::BLOCK)
            })
        })
        .collect();
    match items.as_slice() {
        [one] => Some(one.clone()),
        _ => None,
    }
}

impl Collector<'_> {
    /// The derive function of the trait named `name` here, if its `#[derive_by]` names one
    /// (resolved, or still to be).
    pub(super) fn derive_fn(&self, name: &str) -> Option<u32> {
        match self.tree.path(self.module, &[name], false) {
            Ok((Def::Trait(t), 1)) => self
                .tree
                .traits
                .get(t as usize)
                .and_then(|d| d.derive_by)
                .or_else(|| self.derive_by_names.get(&t).map(|_| u32::MAX)),
            _ => None,
        }
    }

    /// Trait `i`'s `#[derive_by(f)]` (`attr.derive-by.1`): `f` is a derive function of this
    /// module, and its template implements the trait. Any other attribute waits.
    pub(super) fn derive_by(&mut self, i: usize, src: Src, node: &SyntaxNode) {
        let mut ds = Vec::new();
        for attr in node
            .children()
            .filter(|c| c.kind() == K::OUTER_ATTR && !crate::is_doc(c))
        {
            let is_derive_by = attr
                .child(K::ATTR)
                .and_then(|a| a.child(K::PATH))
                .and_then(ast::Path::cast)
                .is_some_and(|p| p.segments() == ["derive_by"]);
            if !is_derive_by {
                ds.push(Diagnostic::unsupported(attr.text_range(), "this attribute"));
            }
        }
        if let Some((name, range)) = derive_by_attr(node) {
            let f = match self.tree.path(self.module, &[name.as_str()], false) {
                Ok((Def::Fn(f), 1)) if self.derivers.contains_key(&f) => Some(f),
                _ => None,
            };
            match f {
                Some(f) => self.tree.traits[i].derive_by = Some(f),
                None => ds.push(Diagnostic::error(
                    Code::E1006,
                    range,
                    format!(
                        "`#[derive_by({name})]` names the derive function of this trait: a \
                         `const fn {name}(t: type) -> Impl<{}>` in this module",
                        self.tree.traits[i].name
                    ),
                )),
            }
        }
        self.reporter.report(src, None, ds);
    }

    /// A derive function's signature (`comptime.derive.1`), its shape checked: one parameter of
    /// type `type`, the result `Impl<Tr>`, and a body that is one `impl Tr for t { … }`.
    pub(super) fn deriver_sig(&mut self, node: &SyntaxNode, src: Src) -> FnSig {
        let (name, name_range) = name_of(node);
        let mut ds = Vec::new();
        let f = ast::FnItem::cast(node.clone());
        let param = f
            .as_ref()
            .and_then(ast::FnItem::params)
            .and_then(|l| l.params().next());
        let var = param_name(node);
        let is_const = node
            .child_tokens()
            .take_while(|t| t.kind() != K::FN_KW)
            .any(|t| t.kind() == K::CONST_KW);
        let ret_ok = f.as_ref().and_then(ast::FnItem::ret_type).is_some_and(|r| {
            r.syntax()
                .text()
                .to_string()
                .trim_start_matches("->")
                .trim()
                .starts_with("Impl<")
        });
        let template = template_impl(node);
        let header_ok = template.as_ref().is_some_and(|i| {
            let types: Vec<ast::Type> = i.children().filter_map(ast::Type::cast).collect();
            i.token(K::FOR_KW).is_some()
                && types
                    .get(1)
                    .is_some_and(|t| t.syntax().text().to_string().trim() == var)
        });
        if !is_const || !ret_ok || template.is_none() || !header_ok || var.is_empty() {
            ds.push(Diagnostic::error(
                Code::E1006,
                name_range,
                "a derive function is `const fn f(t: type) -> Impl<Tr>`, its body one \
                 `impl Tr for t { … }`",
            ));
        }
        // `#[derive_for(…)]` makes impls for built-in types: the prelude's, for now
        // (`comptime.derive.for.1`).
        if src != Src::Prelude
            && let Some((range, _)) = crate::derive::derive_for_attr(node)
        {
            ds.push(Diagnostic::unsupported(
                range,
                "`#[derive_for]` outside the prelude",
            ));
        }
        let origin = self.reporter.origin(src, node.text_range());
        self.reporter.report(src, origin, ds);
        let mut sig = FnSig::placeholder(name, src, self.module, super::vis_of(node));
        sig.name_range = name_range;
        sig.node = node.text_range();
        sig.is_const = true;
        sig.params = vec![ParamSig {
            name: Some(var.clone()),
            mutable: false,
            range: param.map_or(name_range, |p| p.syntax().text_range()),
            ty: Ty::Error,
            destructures: false,
        }];
        sig.deriver = Some(var);
        sig
    }

    /// The methods of impl `index`, a derive of a trait whose derive function `f` is a
    /// template, or an impl `f`'s `#[derive_for]` makes (`for_types`): each of the template's
    /// methods, collected where the template is, with its parameter and `Self` naming `self_ty`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn template_methods(
        &mut self,
        f: u32,
        index: u32,
        self_ty: &Ty,
        generics: &Generics,
        trait_ref: Option<&TraitRef>,
        assoc: &[Option<Ty>],
        origin: Option<TextRange>,
        for_types: bool,
    ) {
        let Some((src, module, node)) = self.derivers.get(&f).cloned() else {
            return;
        };
        let Some(template) = template_impl(&node) else {
            return;
        };
        // The function's signature is collected after the impls: its parameter is read here.
        let var = param_name(&node);
        let adt = match self_ty {
            Ty::Adt(adt, ..) => Some(*adt),
            // A built-in type, from `#[derive_for]`: it has no fields to reflect on.
            _ if for_types => None,
            _ => {
                let d = Diagnostic::unsupported(
                    origin.unwrap_or_default(),
                    "deriving with a template for anything but a struct",
                );
                self.reporter.report(Src::Derived, origin, vec![d]);
                return;
            }
        };
        let (old_module, old_first) = (self.module, self.prelude_first);
        self.module = module;
        self.prelude_first = false;
        self.template = Some(Template {
            var,
            ty: self_ty.clone(),
            adt,
        });
        let members: Vec<SyntaxNode> = template
            .child(K::IMPL_BODY)
            .into_iter()
            .flat_map(|b| b.children().collect::<Vec<_>>())
            .collect();
        for m in members {
            let Some(fi) = ast::FnItem::cast(m) else {
                continue;
            };
            let mut sig = self.fn_sig(
                &fi,
                src,
                Container::Impl(index),
                generics,
                Some(self_ty),
                None,
                trait_ref.map(|t| (t.clone(), assoc.to_vec())),
            );
            sig.origin = origin;
            self.tree.impls[index as usize]
                .methods
                .push(self.tree.fns.len() as u32);
            self.tree.fns.push(sig);
        }
        self.template = None;
        self.module = old_module;
        self.prelude_first = old_first;
    }
}

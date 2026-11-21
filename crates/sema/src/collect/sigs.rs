//! Signatures: functions (free, trait and impl methods) and tests.

use std::collections::HashMap;

use moddil_syntax::ast::{self, AstNode};
use moddil_syntax::{SyntaxKind as K, SyntaxNode, TextRange};

use crate::items::{Container, FnSig, Generics, ParamSig, SelfKind, Src, TestSig};
use crate::resolve::{Scope, preds_of, resolve_type};
use crate::ty::TraitRef;
use crate::{Code, Diagnostic, Ty};

use super::{Collector, simple_binding};

impl Collector<'_> {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn fn_sig(
        &mut self,
        f: &ast::FnItem,
        src: Src,
        container: Container,
        parent: &Generics,
        self_ty: Option<&Ty>,
        trait_ctx: Option<u32>,
        impl_assoc: Option<(TraitRef, Vec<Option<Ty>>)>,
    ) -> FnSig {
        let self_ty = self_ty.cloned();
        let node = f.syntax();
        let origin = self.reporter.origin(src, node.text_range());
        self.unsupported_parts(src, node, false);
        let mut ds = Vec::new();
        for t in node.child_tokens() {
            if t.kind() == K::FN_KW {
                break;
            }
            if t.kind().is_keyword() && !matches!(t.kind(), K::PUB_KW | K::MOD_KW | K::DEFAULT_KW) {
                ds.push(Diagnostic::unsupported(
                    t.text_range(),
                    &format!("`{}` on a function", t.text()),
                ));
            }
        }
        let (name, name_range) = f.name().map_or_else(
            || (String::new(), node.text_range()),
            |n| (n.text(), n.syntax().text_range()),
        );
        let has_body = f.body().is_some();
        if !has_body && !matches!(container, Container::Trait(_)) {
            ds.push(Diagnostic::unsupported(
                name_range,
                "a function without a body",
            ));
        }
        self.reporter.report(src, origin, ds);
        let (params, bounded) =
            self.params(src, origin, node.child(K::GENERIC_PARAMS), &parent.params);
        let mut generics = Generics {
            params,
            parent: parent.params.len() as u32,
            preds: parent.preds.clone(),
        };
        let mut ds = Vec::new();
        let own = {
            let scope = Scope {
                params: &generics.params,
                preds: &generics.preds,
                self_ty: self_ty.clone(),
                trait_ctx,
                impl_assoc: impl_assoc.clone(),
                ..Scope::new(&self.tree)
            };
            preds_of(node, &bounded, &scope, &mut ds)
        };
        self.reporter.report(src, origin, ds);
        generics.preds.extend(own);
        let scope = Scope {
            params: &generics.params,
            preds: &generics.preds,
            self_ty: self_ty.clone(),
            trait_ctx,
            impl_assoc,
            ..Scope::new(&self.tree)
        };
        let mut ds = Vec::new();
        let mut params = Vec::new();
        let mut self_kind = None;
        let mut names: HashMap<String, TextRange> = HashMap::new();
        if let Some(list) = f.params() {
            for c in list.syntax().children() {
                match c.kind() {
                    K::PARAM => {
                        let p = ast::Param::cast(c).expect("a PARAM");
                        let sig = param_sig(&p, &scope, &mut ds);
                        if let Some(n) = &sig.name {
                            if let Some(first) = names.get(n) {
                                ds.push(
                                    Diagnostic::error(
                                        Code::E0104,
                                        sig.range,
                                        format!("the parameter `{n}` is declared twice"),
                                    )
                                    .with_label(*first, "first declared here"),
                                );
                            } else {
                                names.insert(n.clone(), sig.range);
                            }
                        }
                        params.push(sig);
                    }
                    K::SELF_PARAM => {
                        let Some(st) = &self_ty else {
                            ds.push(Diagnostic::error(
                                Code::E0102,
                                c.text_range(),
                                "only a method of an impl or trait takes `self`",
                            ));
                            continue;
                        };
                        if !params.is_empty() {
                            ds.push(Diagnostic::error(
                                Code::E0001,
                                c.text_range(),
                                "`self` must be the first parameter",
                            ));
                            continue;
                        }
                        if c.children().any(|x| ast::Type::can_cast(x.kind())) {
                            ds.push(Diagnostic::unsupported(
                                c.text_range(),
                                "a `self` parameter with a type",
                            ));
                        }
                        let borrowed = c.token(K::AMP).is_some();
                        let mutable = c.token(K::MUT_KW).is_some();
                        let (kind, ty) = match (borrowed, mutable) {
                            (true, true) => (SelfKind::RefMut, Ty::reference(true, st.clone())),
                            (true, false) => (SelfKind::Ref, Ty::reference(false, st.clone())),
                            (false, _) => (SelfKind::Value, st.clone()),
                        };
                        self_kind = Some(kind);
                        names.insert("self".to_string(), c.text_range());
                        params.push(ParamSig {
                            name: Some("self".to_string()),
                            mutable: mutable && !borrowed,
                            range: c.text_range(),
                            ty,
                            destructures: false,
                        });
                    }
                    K::NAMED_ONLY_MARKER => ds.push(Diagnostic::unsupported(
                        c.text_range(),
                        "a name-only parameter marker",
                    )),
                    _ => {}
                }
            }
        }
        let (ret, ret_range) = match f.ret_type().and_then(|r| r.ty()) {
            Some(t) => (
                resolve_type(&t, &scope, &mut ds),
                Some(t.syntax().text_range()),
            ),
            None => (Ty::Unit, None),
        };
        let ret_from = ret_sources(node, &params, &ret, ret_range, &mut ds);
        self.reporter.report(src, origin, ds);
        FnSig {
            name,
            name_range,
            src,
            node: node.text_range(),
            container,
            generics,
            self_kind,
            params,
            ret,
            ret_range,
            ret_from,
            has_body,
            origin,
        }
    }

    pub(super) fn test_sig(&mut self, t: &ast::TestItem) -> TestSig {
        let node = t.syntax();
        self.unsupported_parts(Src::File, node, false);
        if node.child(K::GENERIC_PARAMS).is_some() || node.child(K::WHERE_CLAUSE).is_some() {
            self.reporter
                .push(Diagnostic::unsupported(node.text_range(), "a generic test"));
        }
        if node
            .child_tokens()
            .any(|t| matches!(t.kind(), K::FOR_KW | K::WITH_KW))
        {
            self.reporter.push(Diagnostic::unsupported(
                node.text_range(),
                "a test with `for` or `with`",
            ));
        }
        let sig = match t.name_token() {
            Some(tok) => {
                let text = tok.text();
                let name = text
                    .strip_prefix('"')
                    .and_then(|s| s.strip_suffix('"'))
                    .unwrap_or(text)
                    .to_string();
                TestSig {
                    name,
                    name_range: tok.text_range(),
                }
            }
            None => TestSig {
                name: String::new(),
                name_range: node.text_range(),
            },
        };
        if let Some(first) = self.tree.tests.iter().find(|s| s.name == sig.name) {
            self.reporter.push(
                Diagnostic::error(
                    Code::E0103,
                    sig.name_range,
                    format!("a test called \"{}\" already exists", sig.name),
                )
                .with_label(first.name_range, "first defined here"),
            );
        }
        sig
    }
}

/// The parameters a returned borrow may come from (`memory.md`: Returning borrows): those a `from`
/// clause names, which must be borrowed parameters, or else every borrowed parameter. A function
/// that returns a borrow needs at least one (E0310).
pub(super) fn ret_sources(
    node: &SyntaxNode,
    params: &[ParamSig],
    ret: &Ty,
    ret_range: Option<TextRange>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<u32> {
    let borrowed: Vec<u32> = params
        .iter()
        .enumerate()
        .filter(|(_, p)| matches!(p.ty, Ty::Ref(..)))
        .map(|(i, _)| i as u32)
        .collect();
    let Some(clause) = node.child(K::FROM_CLAUSE) else {
        if ret.has_ref() && borrowed.is_empty() {
            diagnostics.push(Diagnostic::error(
                Code::E0310,
                ret_range.unwrap_or_else(|| node.text_range()),
                "a function returning a borrow needs a borrowed parameter for it to come from",
            ));
        }
        return borrowed;
    };
    if !ret.has_ref() {
        diagnostics.push(Diagnostic::error(
            Code::E0310,
            clause.text_range(),
            "`from` names where a returned borrow comes from, but this function does not return \
             one",
        ));
    }
    let mut from = Vec::new();
    for t in clause
        .child_tokens()
        .filter(|t| matches!(t.kind(), K::IDENT | K::SELF_KW))
    {
        match params
            .iter()
            .position(|p| p.name.as_deref() == Some(t.text()))
        {
            Some(i) if borrowed.contains(&(i as u32)) => from.push(i as u32),
            Some(_) => diagnostics.push(Diagnostic::error(
                Code::E0310,
                t.text_range(),
                format!(
                    "`{}` is not a borrowed parameter, so a returned borrow cannot come from it",
                    t.text()
                ),
            )),
            None => diagnostics.push(Diagnostic::error(
                Code::E0101,
                t.text_range(),
                format!("there is no parameter called `{}`", t.text()),
            )),
        }
    }
    from
}

pub(super) fn param_sig(
    p: &ast::Param,
    scope: &Scope,
    diagnostics: &mut Vec<Diagnostic>,
) -> ParamSig {
    let node = p.syntax();
    for c in node.children() {
        if c.kind() == K::OUTER_ATTR {
            diagnostics.push(Diagnostic::unsupported(c.text_range(), "an attribute"));
        }
    }
    if node.children().any(|c| ast::Expr::can_cast(c.kind())) {
        diagnostics.push(Diagnostic::unsupported(
            node.text_range(),
            "a default argument",
        ));
    }
    let (name, mutable, range, destructures) = match p.pat() {
        Some(pat) => match simple_binding(&pat, scope.tree) {
            Some((name, mutable)) => (name, mutable, pat.syntax().text_range(), false),
            None => (None, false, pat.syntax().text_range(), true),
        },
        None => (None, false, node.text_range(), false),
    };
    let ty = match p.ty() {
        Some(t) => resolve_type(&t, scope, diagnostics),
        None => {
            diagnostics.push(Diagnostic::unsupported(range, "a parameter without a type"));
            Ty::Error
        }
    };
    ParamSig {
        name,
        mutable,
        range,
        ty,
        destructures,
    }
}

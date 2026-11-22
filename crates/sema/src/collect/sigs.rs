//! Signatures: functions (free, trait and impl methods) and tests.

use std::collections::HashMap;

use moddil_syntax::ast::{self, AstNode};
use moddil_syntax::{SyntaxKind as K, SyntaxNode, TextRange};

use crate::items::{Container, EffectSig, FnSig, Generics, ParamSig, SelfKind, Src, TestSig};
use crate::resolve::{Scope, effect_type, preds_of, resolve_type};
use crate::ty::TraitRef;
use crate::{Code, Diagnostic, Ty};

use super::{Collector, name_of, simple_binding};

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
        self.unsupported_parts(src, node, true);
        // Contracts (`contract.1`): not yet on a trait's method, nor `requires always`.
        for c in node.children().filter(|c| c.kind() == K::CONTRACT) {
            let what = if matches!(container, Container::Trait(_)) {
                Some("a contract on a trait's method")
            } else if c.token(K::ALWAYS_KW).is_some() {
                Some("`requires always`")
            } else {
                None
            };
            if let Some(what) = what {
                let origin = self.reporter.origin(src, node.text_range());
                self.reporter.report(
                    src,
                    origin,
                    vec![Diagnostic::unsupported(c.text_range(), what)],
                );
            }
        }
        let mut ds = Vec::new();
        // `#[track_caller]` (`attr.track-caller.1`) and `#[around(w)]` (`attr.around.1`); other
        // attributes on a function wait.
        let mut track_caller = false;
        let mut around = Vec::new();
        let mut intrinsic = false;
        let mut selectable = false;
        let mut selectable_at = None;
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
            let input = attr.child(K::ATTR).and_then(|a| a.child(K::ATTR_INPUT));
            if path == ["track_caller"] && input.is_none() {
                track_caller = true;
            } else if path == ["selectable"] && input.is_none() {
                selectable = true;
                selectable_at = Some(attr.text_range());
            } else if path == ["around"] && src.is_file() {
                if input.is_none() {
                    ds.push(Diagnostic::error(
                        Code::E1005,
                        attr.text_range(),
                        "`#[around(w)]` names the function that wraps this one",
                    ));
                }
                around.push(attr.text_range());
            } else if path == ["intrinsic"] && input.is_none() && src == Src::Prelude {
                intrinsic = true;
            } else {
                ds.push(Diagnostic::unsupported(attr.text_range(), "this attribute"));
            }
        }
        // A wrapped function is free, not `const`, and has plain parameters, for now.
        if let Some(&at) = around.first() {
            let what = if !matches!(container, Container::Free) {
                Some("a wrapper on a method")
            } else if node
                .child_tokens()
                .take_while(|t| t.kind() != K::FN_KW)
                .any(|t| t.kind() == K::CONST_KW)
            {
                Some("a wrapper on a `const fn`")
            } else if crate::wrap::params(node).is_none() {
                Some("a wrapper on a function with a parameter written as a pattern")
            } else {
                None
            };
            if let Some(what) = what {
                ds.push(Diagnostic::unsupported(at, what));
            }
        }
        // `const fn` (`comptime.const-fn.1`).
        let mut is_const = false;
        for t in node.child_tokens() {
            if t.kind() == K::FN_KW {
                break;
            }
            if t.kind() == K::CONST_KW {
                is_const = true;
                continue;
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
        // A stub mod's functions may be signatures only (`mod.stub.1`).
        let stub = src.is_file()
            && self
                .tree
                .mods
                .get(self.tree.mod_of(self.module) as usize)
                .is_some_and(|m| m.stub);
        if !has_body && !intrinsic && !stub && !matches!(container, Container::Trait(_)) {
            ds.push(Diagnostic::unsupported(
                name_range,
                "a function without a body",
            ));
        }
        self.reporter.report(src, origin, ds);
        let (mut params, mut bounded) =
            self.params(src, origin, node.child(K::GENERIC_PARAMS), &parent.params);
        // A parameter of type `impl Fn(A) -> R` has a type parameter of its own, bounded by it
        // (`expr.closure.param.1`).
        let mut impls = Vec::new();
        if let Some(list) = f.params() {
            for p in list.params() {
                // `impl Fn() with E`: the bound's effects are the type's (`expr.closure.effects.2`).
                let written = p.ty();
                let ty = match &written {
                    Some(ast::Type::Effect(e)) => e.children().find_map(ast::Type::cast),
                    t => t.clone(),
                };
                if let Some(t @ ast::Type::Impl(_)) = ty {
                    let range = t.syntax().text_range();
                    let name = written.map_or_else(
                        || t.syntax().text().to_string(),
                        |w| w.syntax().text().to_string(),
                    );
                    impls.push((range, params.len()));
                    bounded.push((params.len(), t.syntax().clone()));
                    params.push(crate::items::GenericParam {
                        name,
                        range,
                        konst: false,
                    });
                }
            }
        }
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
                impls: &impls,
                self_ty: self_ty.clone(),
                trait_ctx,
                impl_assoc: impl_assoc.clone(),
                ..self.scope()
            };
            preds_of(node, &bounded, &scope, &mut ds)
        };
        self.reporter.report(src, origin, ds);
        generics.preds.extend(own);
        let scope = Scope {
            params: &generics.params,
            preds: &generics.preds,
            impls: &impls,
            self_ty: self_ty.clone(),
            trait_ctx,
            impl_assoc,
            ..self.scope()
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
        let mut opaque = Vec::new();
        let (ret, ret_range) = match f.ret_type().and_then(|r| r.ty()) {
            // `-> impl Trait` (`type.impl.ret.1`): an opaque type, numbered once the function
            // has its index.
            Some(t @ ast::Type::Impl(_)) if !matches!(container, Container::Trait(_)) => {
                let ty = Ty::Opaque(std::sync::Arc::new(crate::ty::OpaqueTy {
                    func: u32::MAX,
                    args: generics.identity().into(),
                    name: t.syntax().text().to_string().trim().into(),
                }));
                opaque = crate::resolve::bounds(t.syntax(), &ty, &scope, &mut ds);
                (ty, Some(t.syntax().text_range()))
            }
            Some(t) => (
                resolve_type(&t, &scope, &mut ds),
                Some(t.syntax().text_range()),
            ),
            None => (Ty::Unit, None),
        };
        let ret_from = ret_sources(&self.tree, node, &params, &ret, &mut ds);
        let (fails, fails_range, effects, control) = match node.child(K::EFFECT_CLAUSE) {
            Some(c) => effect_clause(&c, &scope, &mut ds),
            None => (Ty::Never, None, Vec::new(), 0),
        };
        // A `select` waits on what may suspend (`task.select.3`).
        if let Some(at) = selectable_at
            && control & crate::items::SUSPEND == 0
        {
            ds.push(Diagnostic::error(
                Code::E0217,
                at,
                "a `#[selectable]` function is one that waits: it needs `suspend`",
            ));
        }
        for e in &effects {
            if let Some(n) = &e.name {
                if let Some(first) = names.get(n) {
                    ds.push(
                        Diagnostic::error(
                            Code::E0104,
                            e.range,
                            format!("`{n}` is already a parameter"),
                        )
                        .with_label(*first, "first declared here"),
                    );
                } else {
                    names.insert(n.clone(), e.range);
                }
            }
        }
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
            fails,
            fails_range,
            effects,
            track_caller,
            control,
            defaults: Vec::new(),
            opaque,
            selectable,
            default_arg: false,
            intrinsic,
            is_const,
            const_item: false,
            refine: None,
            vis: match (src, container) {
                (Src::File(_), Container::Free | Container::Impl(_)) => super::vis_of(node),
                _ => crate::items::Vis::Pub,
            },
            module: self.module,
        }
    }

    /// A const item, `const NAME: T = e;` (`comptime.const.1`), as a function of no parameters
    /// returning `T`.
    pub(super) fn const_sig(&mut self, node: &SyntaxNode, src: Src) -> FnSig {
        let (name, name_range) = name_of(node);
        let mut ds = Vec::new();
        let ret = match node.children().find_map(ast::Type::cast) {
            Some(t) => resolve_type(&t, &self.scope(), &mut ds),
            None => Ty::Error,
        };
        // Its value is static data, which holds no borrow but of text (`borrow.static.1`).
        let text = matches!(&ret, Ty::Ref(false, t) if **t == Ty::Str);
        if !text && self.tree.is_view(&ret) {
            ds.push(Diagnostic::error(
                Code::E1003,
                name_range,
                "a const holds no borrow but of text: its value is static data, kept for the \
                 whole run",
            ));
        }
        let origin = self.reporter.origin(src, node.text_range());
        self.reporter.report(src, origin, ds);
        FnSig {
            name,
            name_range,
            src,
            node: node.text_range(),
            container: Container::Free,
            generics: Generics::default(),
            self_kind: None,
            params: Vec::new(),
            ret,
            ret_range: None,
            ret_from: Vec::new(),
            has_body: true,
            origin,
            fails: Ty::Never,
            fails_range: None,
            effects: Vec::new(),
            track_caller: false,
            control: 0,
            defaults: Vec::new(),
            opaque: Vec::new(),
            selectable: false,
            default_arg: false,
            intrinsic: false,
            is_const: true,
            const_item: true,
            refine: None,
            vis: match src {
                Src::File(_) => super::vis_of(node),
                _ => crate::items::Vis::Pub,
            },
            module: self.module,
        }
    }

    /// The function a test's subject `path` names: a free function, a function of a type's
    /// inherent impl, or a trait's method.
    fn test_subject(&self, path: &SyntaxNode) -> Option<u32> {
        let segs = crate::resolve::path_segments(path)?;
        let names: Vec<&str> = segs.iter().map(|s| s.name.as_str()).collect();
        let (def, n) = self.tree.path(self.module, &names, false).ok()?;
        let last = *names.last()?;
        let named = |f: &u32| self.tree.fns[*f as usize].name == last;
        match def {
            crate::items::Def::Fn(f) if n == names.len() => Some(f),
            crate::items::Def::Adt(a) if n + 1 == names.len() => self
                .tree
                .impls
                .iter()
                .filter(|i| i.trait_ref.is_none() && i.self_ty.adt() == Some(a))
                .find_map(|i| i.methods.iter().copied().find(named)),
            crate::items::Def::Trait(t) if n + 1 == names.len() => self.tree.traits[t as usize]
                .methods
                .iter()
                .copied()
                .find(named),
            _ => None,
        }
    }

    pub(super) fn test_sig(&mut self, t: &ast::TestItem, file: u32) -> TestSig {
        let node = t.syntax();
        let src = Src::File(file);
        self.unsupported_parts(src, node, false);
        if node.child(K::GENERIC_PARAMS).is_some() || node.child(K::WHERE_CLAUSE).is_some() {
            self.reporter.report(
                src,
                None,
                vec![Diagnostic::unsupported(node.text_range(), "a generic test")],
            );
        }
        for (kind, what) in [
            (K::TEST_INSTANCES, "a test run for listed types"),
            (K::TEST_CAPABILITIES, "a test's capabilities"),
        ] {
            if let Some(c) = node.child(kind) {
                self.reporter.report(
                    src,
                    None,
                    vec![Diagnostic::unsupported(c.text_range(), what)],
                );
            }
        }
        // `for subject` (`test.for.1`): a function, a type's function, or a trait's method.
        let subject = match node.child(K::PATH) {
            Some(p) => {
                let found = self.test_subject(&p);
                if found.is_none() {
                    let path = p.text().to_string();
                    self.reporter.report(
                        src,
                        None,
                        vec![Diagnostic::error(
                            Code::E0101,
                            p.text_range(),
                            format!("there is no function called `{path}` for the test to be for"),
                        )],
                    );
                }
                found
            }
            None => None,
        };
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
                    file,
                    node: node.text_range(),
                    module: self.module,
                    subject,
                }
            }
            None => TestSig {
                name: String::new(),
                name_range: node.text_range(),
                file,
                node: node.text_range(),
                module: self.module,
                subject,
            },
        };
        // Tests are named once per module.
        if let Some(first) = self
            .tree
            .tests
            .iter()
            .find(|s| s.name == sig.name && s.module == self.module)
        {
            self.reporter.push_in(
                file,
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

/// What an effect clause lists: the errors it lets a function fail with (`fail(A | B)`) and
/// where that is written, and the other effects (`time: Time`, `Db`, `config: &AppConfig`).
fn effect_clause(
    clause: &SyntaxNode,
    scope: &Scope,
    ds: &mut Vec<Diagnostic>,
) -> (Ty, Option<TextRange>, Vec<EffectSig>, crate::items::Control) {
    let mut control = 0;
    let mut members = Vec::new();
    let mut fails_range: Option<TextRange> = None;
    let mut effects = Vec::new();
    for e in clause.children().filter(|c| c.kind() == K::SIG_EFFECT) {
        let Some(effect) = e.child(K::EFFECT) else {
            ds.push(Diagnostic::unsupported(e.text_range(), "this effect"));
            continue;
        };
        let name = e.token(K::IDENT).map(|t| t.text().to_string());
        // The control effects (`eff.control.1`), which are not named.
        let bit = if effect.token(K::SUSPEND_KW).is_some() {
            crate::items::SUSPEND
        } else if effect.token(K::SPAWN_KW).is_some() {
            crate::items::SPAWN
        } else {
            0
        };
        if bit != 0 {
            if name.is_some() {
                ds.push(Diagnostic::error(
                    Code::E0601,
                    e.text_range(),
                    "`suspend` and `spawn` are not given names",
                ));
            }
            if control & bit != 0 {
                ds.push(Diagnostic::error(
                    Code::E0606,
                    e.text_range(),
                    "an effect is listed once",
                ));
            }
            control |= bit;
            continue;
        }
        if effect.token(K::FAIL_KW).is_some() {
            let Some(types) = effect.child(K::ERROR_TYPES) else {
                ds.push(Diagnostic::unsupported(e.text_range(), "this effect"));
                continue;
            };
            if name.is_some() {
                ds.push(Diagnostic::error(
                    Code::E0601,
                    e.text_range(),
                    "`fail` is not given a name: write `fail(E)`",
                ));
            }
            fails_range.get_or_insert(e.text_range());
            for t in types.children().filter_map(ast::Type::cast) {
                let ty = resolve_type(&t, scope, ds);
                if error_type(&ty, t.syntax().text_range(), &members, ds) {
                    members.push(ty);
                }
            }
            continue;
        }
        let Some(t) = effect.children().find_map(ast::Type::cast) else {
            ds.push(Diagnostic::unsupported(e.text_range(), "this effect"));
            continue;
        };
        let borrowed = effect.token(K::AMP).is_some();
        let ty = effect_type(&t, scope, ds);
        // An effect variable, `with E`: the errors of the closures whose bounds name it
        // (`expr.closure.effects.2`).
        if let Ty::Param(..) = ty
            && !borrowed
        {
            if name.is_some() {
                ds.push(Diagnostic::error(
                    Code::E0601,
                    e.text_range(),
                    "an effect variable is not given a name",
                ));
            }
            fails_range.get_or_insert(e.text_range());
            members.push(ty);
            continue;
        }
        match (&ty, borrowed, &name) {
            (Ty::Dyn(..), true, _) => ds.push(Diagnostic::error(
                Code::E0601,
                effect.text_range(),
                "an effect's trait is written without `&`: its name is a borrow already \
                 (`time: Time` binds a `&dyn Time`)",
            )),
            (Ty::Adt(..), false, Some(_)) => ds.push(Diagnostic::error(
                Code::E0601,
                effect.text_range(),
                format!("context data is read-only, so its name is a shared borrow: write `&{ty}`"),
            )),
            _ => {}
        }
        effects.push(EffectSig {
            ty,
            name,
            range: e.text_range(),
        });
    }
    (Ty::errors(members), fails_range, effects, control)
}

/// Whether `ty` can be in an error set with `others`: it is a struct or enum (E0508), and not
/// another instance of a generic one already there.
pub(crate) fn error_type(ty: &Ty, at: TextRange, others: &[Ty], ds: &mut Vec<Diagnostic>) -> bool {
    match ty {
        Ty::Error => false,
        // A closure's errors (`<F as FnOnce<Args>>::Fails`): whatever the closure fails with;
        // and an effect variable (`with E`), which stands for them.
        Ty::Assoc(a) if a.name.as_ref() == "Fails" => !others.contains(ty),
        Ty::Param(..) => !others.contains(ty),
        Ty::Adt(a, ..) => {
            if others.iter().any(|o| o.adt() == Some(*a) && o != ty) {
                ds.push(Diagnostic::error(
                    Code::E0508,
                    at,
                    format!("an error set holds one instance of each type, and `{ty}` is another"),
                ));
                return false;
            }
            true
        }
        t => {
            ds.push(Diagnostic::error(
                Code::E0508,
                at,
                format!("`{t}` is not an error type: errors are structs and enums"),
            ));
            false
        }
    }
}

/// The parameters a returned borrow may come from (`memory.md`: Returning borrows): those a `from`
/// clause names, which must be borrowed parameters, or else every borrowed parameter. With none,
/// it may return only static data (`memory.md`: Borrows of static data).
pub(super) fn ret_sources(
    tree: &crate::ItemTree,
    node: &SyntaxNode,
    params: &[ParamSig],
    ret: &Ty,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<u32> {
    // Borrowed parameters, and views, which hold borrows of their own.
    let borrowed: Vec<u32> = params
        .iter()
        .enumerate()
        .filter(|(_, p)| tree.is_view(&p.ty))
        .map(|(i, _)| i as u32)
        .collect();
    // A result of a type parameter may hold borrows too, for the arguments that do.
    let returns = tree.is_view(ret) || ret.has_param();
    // With no borrowed parameter, what it returns borrows static data (`memory.md`: Borrows of
    // static data), which the ownership check sees to.
    let Some(clause) = node.child(K::FROM_CLAUSE) else {
        return borrowed;
    };
    if !returns {
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
            // A type parameter's value may hold borrows too (a closure, `Wrap<&int>`).
            Some(i) if borrowed.contains(&(i as u32)) || matches!(params[i].ty, Ty::Param(..)) => {
                from.push(i as u32);
            }
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
        if c.kind() == K::OUTER_ATTR && !crate::is_doc(&c) {
            diagnostics.push(Diagnostic::unsupported(c.text_range(), "an attribute"));
        }
    }
    let (name, mutable, range, destructures) = match p.pat() {
        Some(pat) => match simple_binding(&pat, scope.tree, scope.module) {
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

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
        self.contracts_unsupported(src, node, container);
        let mut ds = Vec::new();
        let attrs = fn_attrs(node, src, &mut ds);
        wrapper_unsupported(node, container, &attrs.around, &mut ds);
        let is_const = const_and_modifiers(node, &mut ds);
        let foreign = self.extern_fns.contains(&(src, node.text_range()));
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
        if !has_body
            && !attrs.intrinsic
            && !stub
            && !foreign
            && !matches!(container, Container::Trait(_))
        {
            ds.push(Diagnostic::unsupported(
                name_range,
                "a function without a body",
            ));
        }
        self.reporter.report(src, origin, ds);
        let (mut params, mut bounded) =
            self.params(src, origin, node.child(K::GENERIC_PARAMS), &parent.params);
        let impls = impl_params(f, &mut params, &mut bounded);
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
        let (params, self_kind, mut names) = value_params(f, &scope, self_ty.as_ref(), &mut ds);
        let ret = self.ret_sig(f, container, &generics, &scope, &mut ds);
        let ret_from = ret_sources(&self.tree, node, &params, &ret.ty, &mut ds);
        let effects = match node.child(K::EFFECT_CLAUSE) {
            Some(c) => {
                let (fails, fails_range, effects, control) = effect_clause(&c, &scope, &mut ds);
                Effects {
                    fails,
                    fails_range,
                    effects,
                    control,
                }
            }
            None => Effects {
                fails: Ty::Never,
                fails_range: None,
                effects: Vec::new(),
                control: 0,
            },
        };
        if let Some(at) = attrs.kernel_at {
            check_kernel(at, container, &params, &ret.ty, &effects, &mut ds);
        }
        if foreign || attrs.export {
            let sig = CSig {
                foreign,
                has_body,
                name_range,
                container,
                params: &params,
                ret: &ret,
                effects: &effects,
            };
            self.check_c_boundary(&sig, &attrs, &mut ds);
        }
        // A `select` waits on what may suspend (`task.select.3`).
        if let Some(at) = attrs.selectable_at
            && effects.control & crate::items::SUSPEND == 0
        {
            ds.push(Diagnostic::error(
                Code::E0217,
                at,
                "a `#[selectable]` function is one that waits: it needs `suspend`",
            ));
        }
        effect_names(&effects.effects, &mut names, &mut ds);
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
            ret: ret.ty,
            ret_range: ret.range,
            ret_from,
            has_body,
            origin,
            fails: effects.fails,
            fails_range: effects.fails_range,
            effects: effects.effects,
            track_caller: attrs.track_caller,
            control: effects.control,
            defaults: Vec::new(),
            opaque: ret.opaque,
            opaque_fn: ret.opaque_fn,
            selectable: attrs.selectable_at.is_some(),
            kernel: attrs.kernel_at.is_some(),
            foreign,
            export: attrs.export,
            export_status: attrs.export_status,
            export_on_trap: attrs.on_trap,
            py_export: attrs.py_export,
            default_arg: false,
            intrinsic: attrs.intrinsic,
            is_const,
            const_item: false,
            refine: None,
            vis: match (src, container) {
                (Src::File(_), Container::Free | Container::Impl(_)) => super::vis_of(node),
                _ => crate::items::Vis::Pub,
            },
            module: self.module,
            deriver: None,
            template: self.template.clone(),
        }
    }

    /// Contracts (`contract.1`): not yet on a trait's method.
    fn contracts_unsupported(&mut self, src: Src, node: &SyntaxNode, container: Container) {
        for c in node.children().filter(|c| c.kind() == K::CONTRACT) {
            if matches!(container, Container::Trait(_)) {
                let what = "a contract on a trait's method";
                let origin = self.reporter.origin(src, node.text_range());
                self.reporter.report(
                    src,
                    origin,
                    vec![Diagnostic::unsupported(c.text_range(), what)],
                );
            }
        }
    }

    /// The return type, and for `-> impl Trait` its bounds.
    fn ret_sig(
        &self,
        f: &ast::FnItem,
        container: Container,
        generics: &Generics,
        scope: &Scope,
        ds: &mut Vec<Diagnostic>,
    ) -> RetSig {
        let mut opaque = Vec::new();
        let mut opaque_fn = None;
        // `-> (impl Fn() with E)`: the closure bound's effects are the parenthesised type's.
        let mut written = f.ret_type().and_then(|r| r.ty());
        while let Some(ast::Type::Paren(_) | ast::Type::Effect(_)) = &written {
            let inner = written
                .as_ref()
                .and_then(|w| w.syntax().children().find_map(ast::Type::cast));
            match inner {
                Some(i @ (ast::Type::Impl(_) | ast::Type::Paren(_) | ast::Type::Effect(_))) => {
                    written = Some(i);
                }
                _ => break,
            }
        }
        let (ty, range) = match written {
            // `-> impl Trait` (`type.impl.ret.1`): an opaque type, numbered once the function
            // has its index.
            Some(t @ ast::Type::Impl(_)) if !matches!(container, Container::Trait(_)) => {
                let ty = Ty::Opaque(std::sync::Arc::new(crate::ty::OpaqueTy {
                    func: u32::MAX,
                    args: generics.identity().into(),
                    name: t.syntax().text().to_string().trim().into(),
                }));
                opaque = crate::resolve::bounds(t.syntax(), &ty, scope, ds);
                // A closure it returns may be called after it has returned, so its type fixes
                // its errors, as a stored closure's does (`expr.closure.result.1`).
                let once = self.tree.lang("FnOnce");
                if opaque.iter().any(|p| Some(p.trait_ref.trait_) == once) {
                    for p in &mut opaque {
                        if Some(p.trait_ref.trait_) == once && !p.assoc.iter().any(|(k, _)| *k == 1)
                        {
                            p.assoc.push((1, Ty::Never));
                        }
                    }
                    opaque_fn = Some(crate::resolve::dyn_type(t.syntax(), scope, &mut Vec::new()));
                }
                (ty, Some(t.syntax().text_range()))
            }
            Some(t) => (resolve_type(&t, scope, ds), Some(t.syntax().text_range())),
            None => (Ty::Unit, None),
        };
        RetSig {
            ty,
            range,
            opaque,
            opaque_fn,
        }
    }

    /// C functions and exports: C's types only, and nothing C cannot do (`ffi.types.1`).
    fn check_c_boundary(&self, sig: &CSig<'_>, attrs: &FnAttrs, ds: &mut Vec<Diagnostic>) {
        let (foreign, name_range) = (sig.foreign, sig.name_range);
        let what = if foreign {
            "a C function"
        } else {
            "an `#[export]` function"
        };
        let at = attrs.export_at.unwrap_or(name_range);
        for p in sig.params {
            if !self.tree.is_c_type(&p.ty) {
                ds.push(Diagnostic::error(
                    Code::E0222,
                    p.range,
                    format!(
                        "{what}'s parameters have C's types: sized numbers, `int`, `bool`, raw \
                         pointers, `fn` types and `#[repr(C)]` structs of those, not `{}`",
                        p.ty
                    ),
                ));
            }
        }
        let ret = &sig.ret.ty;
        if *ret != Ty::Unit && !self.tree.is_c_type(ret) {
            ds.push(Diagnostic::error(
                Code::E0222,
                sig.ret.range.unwrap_or(name_range),
                format!("{what} gives a C type or nothing, not `{ret}`"),
            ));
        }
        // `on_trap`'s value is one of the result's integer type (`ffi.export.3`).
        if let Some(v) = attrs.on_trap {
            let range = match ret {
                Ty::Int => Some((i64::MIN.into(), i64::MAX.into())),
                Ty::Num(n) if !n.is_float() => Some(n.int_range()),
                _ => None,
            };
            match range {
                None => ds.push(Diagnostic::error(
                    Code::E0222,
                    at,
                    format!(
                        "`on_trap` gives an integer, and this function gives `{ret}`: without it, \
                         a trap gives C the zero value"
                    ),
                )),
                Some((lo, hi)) if v < lo || v > hi => ds.push(Diagnostic::error(
                    Code::E0222,
                    at,
                    format!("`on_trap = {v}` is not a `{ret}`"),
                )),
                Some(_) => {}
            }
        }
        if !sig.effects.is_none() {
            ds.push(Diagnostic::error(
                Code::E0222,
                at,
                format!("{what} needs no handlers, cannot fail, and does not suspend or spawn"),
            ));
        }
        if foreign {
            if sig.has_body {
                ds.push(Diagnostic::error(
                    Code::E0222,
                    name_range,
                    "a C function in an `extern \"C\"` block is a signature: `fn f(…) -> T;`",
                ));
            }
            let unsafe_ok = self
                .tree
                .mods
                .get(self.tree.mod_of(self.module) as usize)
                .is_none_or(|m| m.unsafe_ok);
            if !unsafe_ok {
                ds.push(Diagnostic::error(
                    Code::E0219,
                    name_range,
                    "C functions are declared only in a mod whose manifest lists the `unsafe` \
                     capability: `capabilities = [\"unsafe\"]` under `[mod]`",
                ));
            }
        }
        if attrs.export
            && (!matches!(sig.container, Container::Free)
                || !sig.params.iter().all(|p| p.name.is_some()))
        {
            ds.push(Diagnostic::error(
                Code::E0222,
                at,
                "an `#[export]` function is a free function with named parameters",
            ));
        }
    }

    /// A const item, `const NAME: T = e;` (`comptime.const.1`), as a function of no parameters
    /// returning `T`.
    pub(super) fn const_sig(&mut self, node: &SyntaxNode, src: Src) -> FnSig {
        self.const_sig_in(node, src, Container::Free, &Generics::default(), None)
    }

    /// A const item in `container`, a trait's or an impl's (`item.const.assoc.1`) with its
    /// generics and its `Self`, or a free one. A trait's may have no value: each impl gives one.
    pub(super) fn const_sig_in(
        &mut self,
        node: &SyntaxNode,
        src: Src,
        container: Container,
        generics: &Generics,
        self_ty: Option<&Ty>,
    ) -> FnSig {
        let (name, name_range) = name_of(node);
        let mut ds = Vec::new();
        let trait_ctx = match container {
            Container::Trait(t) => Some(t),
            _ => None,
        };
        let scope = Scope {
            params: &generics.params,
            preds: &generics.preds,
            self_ty: self_ty.cloned(),
            trait_ctx,
            ..self.scope()
        };
        let ret = match node.children().find_map(ast::Type::cast) {
            Some(t) => resolve_type(&t, &scope, &mut ds),
            None => Ty::Error,
        };
        // Its value is static data, which holds no borrow but shared ones of static data: text,
        // or a const's value (`borrow.static.1`, `comptime.static.3`).
        let text = static_borrows(&self.tree, &ret);
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
        let vis = match src {
            Src::File(_) => super::vis_of(node),
            _ => crate::items::Vis::Pub,
        };
        FnSig {
            name_range,
            node: node.text_range(),
            ret,
            has_body: node.children().any(|c| ast::Expr::can_cast(c.kind())),
            origin,
            is_const: true,
            const_item: true,
            container,
            generics: generics.clone(),
            ..FnSig::free(name, src, self.module, vis)
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

    /// The signature of test `t` in `file`; `doc`, for an example in a doc comment, its name.
    pub(super) fn test_sig(
        &mut self,
        t: &ast::TestItem,
        file: u32,
        doc: Option<String>,
    ) -> TestSig {
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
                    doc: false,
                }
            }
            None => TestSig {
                name: String::new(),
                name_range: node.text_range(),
                file,
                node: node.text_range(),
                module: self.module,
                subject,
                doc: false,
            },
        };
        let mut sig = sig;
        if let Some(name) = doc {
            sig.name = name;
            sig.doc = true;
        }
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

/// What a function's attributes say.
#[derive(Default)]
struct FnAttrs {
    /// `#[track_caller]` (`attr.track-caller.1`).
    track_caller: bool,
    /// Each `#[around(w)]` (`attr.around.1`).
    around: Vec<TextRange>,
    /// `#[intrinsic]`, in the prelude only.
    intrinsic: bool,
    /// Where `#[selectable]` is written (`task.select.3`).
    selectable_at: Option<TextRange>,
    /// Where `#[kernel]` is written (`gpu.kernel.1`).
    kernel_at: Option<TextRange>,
    /// `#[export]` or `#[export(status)]`, for C.
    export: bool,
    export_status: bool,
    /// `#[export(on_trap = v)]`'s `v` (`ffi.export.3`).
    on_trap: Option<i128>,
    /// `#[export(python)]`.
    py_export: bool,
    /// Where `#[export…]` is written.
    export_at: Option<TextRange>,
}

/// A function's attributes. Those not listed in [`FnAttrs`] wait.
fn fn_attrs(node: &SyntaxNode, src: Src, ds: &mut Vec<Diagnostic>) -> FnAttrs {
    let mut a = FnAttrs::default();
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
            a.track_caller = true;
        } else if path == ["selectable"] && input.is_none() {
            a.selectable_at = Some(attr.text_range());
        } else if path == ["kernel"] && input.is_none() {
            a.kernel_at = Some(attr.text_range());
        } else if path == ["export"] && src.is_file() {
            let how = input
                .as_ref()
                .map(|i| i.text().replace(char::is_whitespace, ""));
            match how.as_deref() {
                None => a.export = true,
                Some("(status)") => {
                    a.export = true;
                    a.export_status = true;
                }
                Some("(python)") => a.py_export = true,
                Some(s) if s.starts_with("(on_trap=") && s.ends_with(')') => {
                    a.export = true;
                    let v = &s["(on_trap=".len()..s.len() - 1];
                    match v.replace('_', "").parse::<i128>() {
                        Ok(v) => a.on_trap = Some(v),
                        Err(_) => ds.push(Diagnostic::error(
                            Code::E0222,
                            attr.text_range(),
                            "`on_trap` is an integer literal, the value C is given if the \
                             function traps",
                        )),
                    }
                }
                Some(_) => ds.push(Diagnostic::error(
                    Code::E0222,
                    attr.text_range(),
                    "`#[export]` is for C, `#[export(on_trap = v)]` gives `v` on a trap, \
                     `#[export(python)]` is for Python",
                )),
            }
            a.export_at = Some(attr.text_range());
        } else if path == ["around"] && src.is_file() {
            if input.is_none() {
                ds.push(Diagnostic::error(
                    Code::E1005,
                    attr.text_range(),
                    "`#[around(w)]` names the function that wraps this one",
                ));
            }
            a.around.push(attr.text_range());
        } else if path == ["intrinsic"] && input.is_none() && src == Src::Prelude {
            a.intrinsic = true;
        } else {
            ds.push(Diagnostic::unsupported(attr.text_range(), "this attribute"));
        }
    }
    a
}

/// A wrapped function is free, not `const`, and has plain parameters, for now.
fn wrapper_unsupported(
    node: &SyntaxNode,
    container: Container,
    around: &[TextRange],
    ds: &mut Vec<Diagnostic>,
) {
    let Some(&at) = around.first() else {
        return;
    };
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

/// Whether the function is a `const fn` (`comptime.const-fn.1`); other keywords before `fn`
/// wait.
fn const_and_modifiers(node: &SyntaxNode, ds: &mut Vec<Diagnostic>) -> bool {
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
    is_const
}

/// A parameter of type `impl Fn(A) -> R` has a type parameter of its own, bounded by it
/// (`expr.closure.param.1`): each is added to `params` and `bounded`, and its range and index
/// returned.
fn impl_params(
    f: &ast::FnItem,
    params: &mut Vec<crate::items::GenericParam>,
    bounded: &mut Vec<(usize, SyntaxNode)>,
) -> Vec<(TextRange, usize)> {
    let mut impls = Vec::new();
    let Some(list) = f.params() else {
        return impls;
    };
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
    impls
}

/// The value parameters, `self` among them, with `self`'s kind and where each name is declared.
fn value_params(
    f: &ast::FnItem,
    scope: &Scope,
    self_ty: Option<&Ty>,
    ds: &mut Vec<Diagnostic>,
) -> (Vec<ParamSig>, Option<SelfKind>, HashMap<String, TextRange>) {
    let mut params = Vec::new();
    let mut self_kind = None;
    let mut names: HashMap<String, TextRange> = HashMap::new();
    let Some(list) = f.params() else {
        return (params, self_kind, names);
    };
    for c in list.syntax().children() {
        match c.kind() {
            K::PARAM => {
                let p = ast::Param::cast(c).expect("a PARAM");
                let sig = param_sig(&p, scope, ds);
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
                let Some(st) = self_ty else {
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
    (params, self_kind, names)
}

/// A function's return type, where it is written, and for `-> impl Trait` its bounds and, for a
/// closure, its `dyn` type.
struct RetSig {
    ty: Ty,
    range: Option<TextRange>,
    opaque: Vec<crate::items::Pred>,
    opaque_fn: Option<Ty>,
}

/// What a function's effect clause lists ([`effect_clause`]).
struct Effects {
    fails: Ty,
    fails_range: Option<TextRange>,
    effects: Vec<EffectSig>,
    control: crate::items::Control,
}

impl Effects {
    /// Whether the function needs no handlers, cannot fail, and does not suspend or spawn.
    fn is_none(&self) -> bool {
        self.effects.is_empty() && self.control == 0 && self.fails == Ty::Never
    }
}

/// What [`Collector::check_c_boundary`] checks of a C function or an `#[export]`.
struct CSig<'a> {
    foreign: bool,
    has_body: bool,
    name_range: TextRange,
    container: Container,
    params: &'a [ParamSig],
    ret: &'a RetSig,
    effects: &'a Effects,
}

/// A kernel: a free function taking its parameters, `&P`, then its tile, `&mut Tile`, giving
/// nothing and listing no effects (`gpu.kernel.1`).
fn check_kernel(
    at: TextRange,
    container: Container,
    params: &[ParamSig],
    ret: &Ty,
    effects: &Effects,
    ds: &mut Vec<Diagnostic>,
) {
    let tile = |t: &Ty| matches!(t, Ty::Ref(true, inner) if matches!(&**inner, Ty::Adt(_, n, _) if &**n == "Tile"));
    let shape = matches!(container, Container::Free)
        && params.len() == 2
        && matches!(params[0].ty, Ty::Ref(false, _))
        && tile(&params[1].ty)
        && *ret == Ty::Unit;
    if !shape {
        ds.push(Diagnostic::error(
            Code::E0221,
            at,
            "a `#[kernel]` is a free function `fn k(params: &P, out: &mut Tile)`, giving \
             nothing",
        ));
    } else if !effects.is_none() {
        ds.push(Diagnostic::error(
            Code::E0221,
            at,
            "a `#[kernel]` needs no handlers, cannot fail, and does not suspend or spawn \
             (the kernel effect set)",
        ));
    }
}

/// An effect's name is not a parameter's too (E0104).
fn effect_names(
    effects: &[EffectSig],
    names: &mut HashMap<String, TextRange>,
    ds: &mut Vec<Diagnostic>,
) {
    for e in effects {
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

/// Whether every borrow type `t` holds is a shared one, of a value holding no other borrow but
/// such ones (`&str`, `&[int]`, `&[&str]`, `&Point`): what a const may hold, as static data
/// (`comptime.static.3`).
fn static_borrows(tree: &crate::ItemTree, t: &Ty) -> bool {
    match t {
        Ty::Ref(false, inner) => match &**inner {
            Ty::Str => true,
            Ty::Slice(e) | Ty::Array(e, _) => static_borrows(tree, e) || !tree.is_view(e),
            inner => !tree.is_view(inner) || static_borrows(tree, inner),
        },
        Ty::Array(e, _) | Ty::Slice(e) => static_borrows(tree, e),
        Ty::Tuple(xs) => xs
            .iter()
            .all(|x| static_borrows(tree, x) || !tree.is_view(x)),
        _ => false,
    }
}

//! Function wrappers (`attr.around.1`): `#[around(w)]` on a function makes its body a call of the
//! ordinary generic function `w`, given the function's description (`&INFO`), its arguments as
//! one value (`f::Args`) and the original body as a closure taking them back.
//!
//! The two items the expansion names are generated as Moddil source, with the derived impls
//! (`derive`): `struct __Args_f<P₁, …> { p₁: P₁, … }`, generic so that it holds borrows and has
//! `Debug`, `Clone` and `PartialEq` exactly when the parameters do, and the const item
//! `__INFO_f: FnInfo`. The call itself is built in HIR when the body is lowered.

use moddil_syntax::ast::{self, AstNode};
use moddil_syntax::{SyntaxKind as K, SyntaxNode};

/// The `#[around(…)]` attributes on `node`, in the order written: each `ATTR` node.
pub(crate) fn around_attrs(node: &SyntaxNode) -> Vec<SyntaxNode> {
    node.children()
        .filter(|c| c.kind() == K::OUTER_ATTR)
        .flat_map(|a| {
            a.children()
                .filter(|c| c.kind() == K::ATTR)
                .collect::<Vec<_>>()
        })
        .filter(|a| {
            a.child(K::PATH)
                .and_then(ast::Path::cast)
                .is_some_and(|p| p.segments() == ["around"])
        })
        .collect()
}

/// The name of the hidden struct holding `f`'s arguments.
pub(crate) fn args_name(f: &str) -> String {
    format!("__Args_{f}")
}

/// The name of the hidden const describing `f`.
pub(crate) fn info_name(f: &str) -> String {
    format!("__INFO_{f}")
}

/// The name of the hidden alias `f::Args` names: the arguments' struct at the parameters' types.
pub(crate) fn args_type_name(f: &str) -> String {
    format!("__ArgsType_{f}")
}

/// The name a wrapped function's hidden items are keyed by: a free function's name, or for a
/// method of an inherent impl, `Type_method` (`attr.around.method.1`). `None` for a method of a
/// trait, or of a trait's impl.
pub(crate) fn key(items: &crate::ItemTree, sig: &crate::items::FnSig) -> Option<String> {
    match sig.container {
        crate::Container::Free => Some(sig.name.clone()),
        crate::Container::Impl(k) => {
            let imp = &items.impls[k as usize];
            match (&imp.trait_ref, &imp.self_ty) {
                (None, crate::Ty::Adt(_, n, _)) => Some(format!("{n}_{}", sig.name)),
                _ => None,
            }
        }
        _ => None,
    }
}

/// The hidden items of wrapped function `f`'s `Args` (`attr.around.args.1`): its struct, and the
/// alias naming it at the parameters' types, if there is one. `None` if `f` is not wrapped.
pub(crate) fn args_items(items: &crate::ItemTree, f: u32) -> Option<(u32, Option<u32>)> {
    use crate::items::Def;
    let sig = &items.fns[f as usize];
    let key = key(items, sig)?;
    let Some(Def::Adt(adt)) = items.lookup(sig.module, &args_name(&key), false) else {
        return None;
    };
    let alias = match items.lookup(sig.module, &args_type_name(&key), false) {
        Some(Def::Type(t)) => Some(t),
        _ => None,
    };
    Some((adt, alias))
}

/// The method `name` of an inherent impl of struct or enum `adt` (`Type::method::Args`).
pub(crate) fn inherent_method(items: &crate::ItemTree, adt: u32, name: &str) -> Option<u32> {
    items.fns.iter().enumerate().find_map(|(i, sig)| {
        let crate::Container::Impl(k) = sig.container else {
            return None;
        };
        let imp = &items.impls[k as usize];
        (sig.name == name
            && imp.trait_ref.is_none()
            && matches!(imp.self_ty, crate::Ty::Adt(a, ..) if a == adt))
        .then_some(i as u32)
    })
}

/// For an inherent impl `node`, its self type's name (`Counter` of `impl<T> Counter<T>`) and its
/// text as written; `None` for a trait's impl.
pub(crate) fn impl_self(node: &SyntaxNode) -> Option<(String, String)> {
    if node.kind() != K::IMPL_ITEM || node.token(K::FOR_KW).is_some() {
        return None;
    }
    let ty = node.children().find_map(ast::Type::cast)?;
    let text = squeezed(&ty.syntax().text().to_string());
    let name = text
        .split('<')
        .next()?
        .rsplit("::")
        .next()?
        .trim()
        .to_string();
    Some((name, text))
}

/// The generic parameters of `node` (a function or impl), each as an alias declares it: a type
/// parameter's name, or a const parameter as written (`const N: int`).
pub(crate) fn generic_decls(node: &SyntaxNode) -> Vec<String> {
    node.child(K::GENERIC_PARAMS)
        .map(|g| {
            g.children()
                .filter_map(|p| match p.kind() {
                    K::TYPE_PARAM => ast::name(&p).map(|n| n.text()),
                    K::CONST_PARAM => Some(squeezed(&p.text().to_string())),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `ty` with each `Self` in it made `self_ty`.
fn with_self(ty: &str, self_ty: &str) -> String {
    let mut out = String::new();
    let mut word = String::new();
    for c in ty.chars().chain(std::iter::once(' ')) {
        if c.is_alphanumeric() || c == '_' {
            word.push(c);
            continue;
        }
        if word == "Self" {
            out.push_str(self_ty);
        } else {
            out.push_str(&word);
        }
        word.clear();
        out.push(c);
    }
    out.pop();
    out
}

/// Text with its whitespace runs made single spaces.
fn squeezed(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The attributes on function `node`, each as written inside `#[…]` (`track_caller`,
/// `around(trace::log)`), and the effects its `with` clause lists, each as written (`Net`,
/// `suspend`, `fail(E)`): what its `FnInfo` gives (`attr.around.info.1`).
pub(crate) fn attrs_and_effects(node: &SyntaxNode) -> (Vec<String>, Vec<String>) {
    let attrs = node
        .children()
        .filter(|c| c.kind() == K::OUTER_ATTR)
        .flat_map(|a| {
            a.children()
                .filter(|c| c.kind() == K::ATTR)
                .collect::<Vec<_>>()
        })
        .map(|a| squeezed(&a.text().to_string()))
        .collect();
    let effects = node
        .child(K::EFFECT_CLAUSE)
        .map(|c| {
            c.children()
                .filter(|e| e.kind() == K::SIG_EFFECT)
                .map(|e| squeezed(&e.text().to_string()))
                .collect()
        })
        .unwrap_or_default();
    (attrs, effects)
}

/// A wrapped function's parameters, each its name, whether it is `mut`, and its type as
/// written; `None` if one is not a plain name. A method's `self` is the field `this`, of type
/// `Self`, `&Self` or `&mut Self` (`attr.around.method.1`).
pub(crate) fn params(node: &SyntaxNode) -> Option<Vec<(String, bool, String)>> {
    let f = ast::FnItem::cast(node.clone())?;
    let mut out = Vec::new();
    if let Some(s) = f.params().and_then(|l| l.syntax().child(K::SELF_PARAM)) {
        let ty = match (s.token(K::AMP).is_some(), s.token(K::MUT_KW).is_some()) {
            (true, true) => "&mut Self",
            (true, false) => "&Self",
            (false, _) => "Self",
        };
        out.push(("this".to_string(), false, ty.to_string()));
    }
    for p in f
        .params()
        .map(|l| l.params().collect::<Vec<_>>())
        .unwrap_or_default()
    {
        let pat = p.pat()?;
        let pat = pat.syntax();
        // `mut x`, or `x`, which is parsed as a path.
        let (name, mutable) = match pat.kind() {
            K::IDENT_PAT if pat.children().all(|c| c.kind() == K::NAME) => {
                (ast::name(pat)?.text(), true)
            }
            K::PATH_PAT => {
                let segments = pat.child(K::PATH).and_then(ast::Path::cast)?.segments();
                let [one] = segments.as_slice() else {
                    return None;
                };
                if pat
                    .descendants()
                    .into_iter()
                    .any(|n| n.kind() == K::GENERIC_ARGS)
                {
                    return None;
                }
                (one.clone(), false)
            }
            _ => return None,
        };
        let ty = p.ty()?.syntax().text().to_string();
        let ty = ty.split_whitespace().collect::<Vec<_>>().join(" ");
        out.push((name, mutable, ty));
    }
    Some(out)
}

/// The wrapper an `#[around(…)]` attribute names, from the file's `text`: its input parsed as
/// an expression, with the ranges it has in the file. `None` if it has no input.
pub(crate) fn input(text: &str, attr: &SyntaxNode) -> Option<ast::Expr> {
    let input = attr.child(K::ATTR_INPUT)?;
    let r = input.text_range();
    // Inside the parentheses, which `#[around` comes before, so the prefix fits.
    let (start, end) = (r.start as usize + 1, (r.end as usize).saturating_sub(1));
    if end <= start || !text.is_char_boundary(start) || !text.is_char_boundary(end) {
        return None;
    }
    const PREFIX: &str = "fn f(){";
    if start < PREFIX.len() {
        return None;
    }
    let mut padded = String::with_capacity(end + 1);
    padded.push_str(PREFIX);
    padded.push_str(&" ".repeat(start - PREFIX.len()));
    padded.push_str(&text[start..end]);
    padded.push('}');
    let parse = moddil_syntax::parse(&padded);
    if !parse.errors().is_empty() {
        return None;
    }
    let root = parse.syntax();
    let body = ast::FnItem::cast(root.children().find(|c| c.kind() == K::FN_ITEM)?)?.body()?;
    let exprs: Vec<ast::Expr> = body
        .syntax()
        .children()
        .filter_map(ast::Expr::cast)
        .collect();
    let [e] = exprs.as_slice() else {
        return None;
    };
    Some(e.clone())
}

/// How a wrapped function is named in its expansion: the key of its hidden items (`key`), how
/// its `Args` is shown (`f::Args`, `Type::method::Args`), its path for `FnInfo`, its type
/// parameters (and its impl's) as the alias declares them, and an impl's self type as written.
pub(crate) struct Named<'a> {
    pub(crate) key: &'a str,
    pub(crate) shown: String,
    pub(crate) path: String,
    pub(crate) generics: Vec<String>,
    pub(crate) self_ty: Option<String>,
}

/// The Moddil source of the items a wrapped function's expansion names, one per line: the
/// arguments' struct, its derived impls, the alias `f::Args` names (generic over the function's
/// type parameters), and the description, with `attrs` and `effects` its attributes and effects
/// as written.
pub(crate) fn items(
    named: &Named<'_>,
    params: &[(String, bool, String)],
    (attrs, effects): &(Vec<String>, Vec<String>),
) -> Vec<String> {
    let f = named.key;
    let path = named.path.as_str();
    let args = args_name(f);
    let generics: Vec<String> = (0..params.len()).map(|i| format!("P{i}")).collect();
    let mut out = Vec::new();
    let shown_attr = format!("#[shown({:?})] ", named.shown);
    if params.is_empty() {
        out.push(format!("{shown_attr}struct {args};"));
    } else {
        let fields: Vec<String> = params
            .iter()
            .zip(&generics)
            .map(|((n, _, _), g)| format!("{n}: {g}"))
            .collect();
        out.push(format!(
            "{shown_attr}struct {args}<{}> {{ {} }}",
            generics.join(", "),
            fields.join(", ")
        ));
    }
    let shown = &named.shown;
    let names: Vec<String> = params.iter().map(|(n, _, _)| n.clone()).collect();
    for tr in ["Debug", "Clone", "PartialEq"] {
        out.push(crate::derive::struct_impl(
            tr, &args, shown, &generics, &names,
        ));
    }
    // `f::Args` as a type (`attr.around.args.1`): the struct at the parameters' types, generic
    // over the function's type parameters. A parameter of type `impl Trait` has no type to name.
    let types: Vec<String> = params
        .iter()
        .map(|(_, _, t)| match &named.self_ty {
            Some(s) => with_self(t, s),
            None => t.clone(),
        })
        .collect();
    if !types.iter().any(|t| {
        t.starts_with("impl ")
            || t.contains(" impl ")
            || t.contains("<impl ")
            || t.contains("&impl ")
    }) {
        let at = if types.is_empty() {
            String::new()
        } else {
            format!("<{}>", types.join(", "))
        };
        let decl = if named.generics.is_empty() {
            String::new()
        } else {
            format!("<{}>", named.generics.join(", "))
        };
        out.push(format!("type {}{decl} = {args}{at};", args_type_name(f)));
    }
    // A method's `self` is the field `this`, and the parameter `self`.
    let receiver = named.self_ty.is_some()
        && params.first().is_some_and(|(n, _, t)| {
            n == "this" && matches!(t.as_str(), "Self" | "&Self" | "&mut Self")
        });
    let infos: Vec<String> = params
        .iter()
        .enumerate()
        .map(|(i, (n, _, t))| {
            let n = if i == 0 && receiver {
                "self"
            } else {
                n.as_str()
            };
            format!("ParamInfo {{ name: fmt{n:?}, ty: fmt{:?} }}", escape(t))
        })
        .collect();
    let texts = |xs: &[String]| -> String {
        let xs: Vec<String> = xs.iter().map(|x| format!("fmt{:?}", escape(x))).collect();
        format!("[{}]", xs.join(", "))
    };
    out.push(format!(
        "const {}: FnInfo = FnInfo {{ path: fmt{:?}, params: [{}], attrs: {}, effects: {} }};",
        info_name(f),
        escape(path),
        infos.join(", "),
        texts(attrs),
        texts(effects)
    ));
    out
}

/// `s` with its braces doubled, for a `fmt"…"` literal.
fn escape(s: &str) -> String {
    s.replace('{', "{{").replace('}', "}}")
}

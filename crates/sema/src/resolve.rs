//! Resolving the types written in source: paths, generic arguments, associated types and
//! bounds, against a [`Scope`] that says what the names in a type refer to.
//!
//! Collecting signatures and lowering bodies both use it.

use std::sync::Arc;

use moddil_syntax::ast::{self, AstNode};
use moddil_syntax::{SyntaxKind as K, SyntaxNode, TextRange};

use crate::items::{Def, GenericParam, ItemTree, Pred};
use crate::modules::PathError;
use crate::traits::elaborate;
use crate::ty::{AssocTy, TraitRef};
use crate::{Code, Diagnostic, Ty};

/// One segment of a path: its name, where it is, and its generic arguments (`Vec::<int>`).
#[derive(Clone, Debug)]
pub(crate) struct Segment {
    pub name: String,
    pub range: TextRange,
    pub args: Option<SyntaxNode>,
}

/// The segments of the `PATH` node `path`, with the generic arguments written on each (inside the
/// segment in types, after `::` in expressions). `None` for a qualified path (`<T as Tr>::X`).
pub(crate) fn path_segments(path: &SyntaxNode) -> Option<Vec<Segment>> {
    let mut out: Vec<Segment> = Vec::new();
    for c in path.children() {
        match c.kind() {
            K::QUALIFIED_PATH => return None,
            K::PATH_SEGMENT => {
                // A keyword is a name here too (`Default::default`).
                let tok = c
                    .child_tokens()
                    .find(|t| t.kind() == K::IDENT || t.kind().is_keyword());
                out.push(Segment {
                    name: tok
                        .as_ref()
                        .map(|t| t.text().to_string())
                        .unwrap_or_default(),
                    range: tok.map_or_else(|| c.text_range(), |t| t.text_range()),
                    args: c.child(K::GENERIC_ARGS),
                });
            }
            K::GENERIC_ARGS => {
                if let Some(last) = out.last_mut() {
                    last.args = Some(c);
                }
            }
            _ => {}
        }
    }
    Some(out)
}

/// What the names in a type refer to.
#[derive(Clone)]
pub(crate) struct Scope<'a> {
    pub tree: &'a ItemTree,
    /// The type parameters in scope.
    pub params: &'a [GenericParam],
    /// The bounds in scope, for `T::Name`.
    pub preds: &'a [Pred],
    /// The `impl Trait` types written in a function's parameters, and the type parameter each
    /// stands for.
    pub impls: &'a [(TextRange, usize)],
    /// What `Self` is.
    pub self_ty: Option<Ty>,
    /// What `Self` is in a trait's default type argument (`generic.default.1`), if not
    /// `self_ty`: what a bound bounds (`T: Add` is `T: Add<T>`), or what an impl is for.
    pub default_self: Option<Ty>,
    /// Inside a trait: the trait, so `Self::Name` is its associated type.
    pub trait_ctx: Option<u32>,
    /// Inside an impl of a trait: the trait and the impl's associated types.
    pub impl_assoc: Option<(TraitRef, Vec<Option<Ty>>)>,
    /// Whether `_` and a generic struct or enum without arguments leave types to infer.
    pub holes: bool,
    /// The module whose names are in scope.
    pub module: u32,
    /// Whether the prelude's names come before the module's (in derived code).
    pub prelude_first: bool,
    /// In a template's method (`comptime.template.1`): the template's parameter's name, and the
    /// type it names.
    pub template: Option<(&'a str, &'a Ty)>,
}

impl<'a> Scope<'a> {
    pub(crate) fn new(tree: &'a ItemTree) -> Self {
        Scope {
            tree,
            params: &[],
            preds: &[],
            impls: &[],
            self_ty: None,
            default_self: None,
            trait_ctx: None,
            impl_assoc: None,
            holes: false,
            module: crate::items::PRELUDE_MODULE,
            prelude_first: false,
            template: None,
        }
    }

    fn param(&self, name: &str) -> Option<Ty> {
        self.params
            .iter()
            .rposition(|p| p.name == name)
            .map(|i| Ty::Param(i as u32, name.into()))
    }
}

pub(crate) fn hole() -> Ty {
    Ty::Infer(Ty::HOLE)
}

/// The type written as `t`.
pub(crate) fn resolve_type(t: &ast::Type, scope: &Scope, diags: &mut Vec<Diagnostic>) -> Ty {
    let node = t.syntax();
    let range = node.text_range();
    match t {
        ast::Type::Path(_) => {
            let Some(path) = node.child(K::PATH) else {
                return Ty::Error;
            };
            let t = match path_segments(&path) {
                Some(segs) => path_type(&segs, range, scope, diags),
                None => qualified_type(&path, scope, diags),
            };
            if t == Ty::Str && !behind_borrow(node) {
                diags.push(Diagnostic::error(
                    Code::E0901,
                    range,
                    "`str` has no size of its own, so it is used behind a borrow: `&str`",
                ));
                return Ty::Error;
            }
            // A const parameter is a length, not a type, but for the argument of another one.
            if let Ty::Param(i, name) = &t
                && scope.params.get(*i as usize).is_some_and(|p| p.konst)
                && node.parent().is_none_or(|p| p.kind() != K::TYPE_ARG)
            {
                diags.push(Diagnostic::error(
                    Code::E0102,
                    range,
                    format!("`{name}` is a const parameter, a length, not a type"),
                ));
                return Ty::Error;
            }
            t
        }
        ast::Type::Array(_) => {
            let elem = match node.children().find_map(ast::Type::cast) {
                Some(t) => resolve_type(&t, scope, diags),
                None => Ty::Error,
            };
            let len = match node.children().find_map(ast::Expr::cast) {
                Some(e) => array_len(&e, scope, diags),
                None => Ty::Error,
            };
            Ty::array(elem, len)
        }
        ast::Type::Slice(_) => {
            let elem = match node.children().find_map(ast::Type::cast) {
                Some(t) => resolve_type(&t, scope, diags),
                None => Ty::Error,
            };
            if !behind_borrow(node) {
                diags.push(Diagnostic::error(
                    Code::E0901,
                    range,
                    "a slice has no size of its own, so it is used behind a borrow: `&[T]`",
                ));
                return Ty::Error;
            }
            Ty::slice(elem)
        }
        ast::Type::Tuple(_) => {
            let elements: Vec<Ty> = node
                .children()
                .filter_map(ast::Type::cast)
                .map(|t| resolve_type(&t, scope, diags))
                .collect();
            Ty::tuple(elements)
        }
        ast::Type::Ref(_) => {
            let mutable = node.token(K::MUT_KW).is_some();
            let inner = match node.children().find_map(ast::Type::cast) {
                Some(t @ ast::Type::Dyn(_)) => dyn_type(t.syntax(), scope, diags),
                Some(t) => resolve_type(&t, scope, diags),
                None => Ty::Error,
            };
            // A shared borrow may point to a borrow; a `&mut` may not, so that nothing writes a
            // borrow that lives less in its place (`borrow.type.1`).
            if mutable && matches!(inner, Ty::Ref(..)) {
                diags.push(Diagnostic::error(
                    Code::E0309,
                    range,
                    "a `&mut` cannot point to a borrow, only a shared `&` can",
                ));
                return Ty::Error;
            }
            Ty::reference(mutable, inner)
        }
        // `*const T` and `*mut T` (`type.ptr.1`).
        ast::Type::Ptr(_) => {
            let mutable = node.token(K::MUT_KW).is_some();
            let inner = match node.children().find_map(ast::Type::cast) {
                Some(t) => resolve_type(&t, scope, diags),
                None => Ty::Error,
            };
            Ty::Ptr(mutable, Arc::new(inner))
        }
        // `fn(A, B) -> R`: a C function pointer (`type.fnptr.1`), of C's scalar types only.
        ast::Type::Fn(_) => {
            let params: Vec<Ty> = node
                .child(K::TYPE_LIST)
                .map(|l| {
                    l.children()
                        .filter_map(ast::Type::cast)
                        .map(|t| resolve_type(&t, scope, diags))
                        .collect()
                })
                .unwrap_or_default();
            let ret = node
                .children()
                .find_map(ast::Type::cast)
                .map_or(Ty::Unit, |t| resolve_type(&t, scope, diags));
            let scalar = |t: &Ty| match t {
                Ty::Int | Ty::Bool | Ty::Ptr(..) | Ty::FnPtr(..) | Ty::Error => true,
                Ty::Num(n) => !matches!(n, crate::Num::I128 | crate::Num::U128),
                _ => false,
            };
            if !params.iter().all(scalar) || !(ret == Ty::Unit || scalar(&ret)) {
                diags.push(Diagnostic::error(
                    Code::E0222,
                    range,
                    "a `fn` type is a C function pointer: its parameters and result are C's \
                     scalar types (sized numbers, `int`, `bool`, raw pointers and `fn` types)",
                ));
                return Ty::Error;
            }
            Ty::FnPtr(params.into(), Arc::new(ret))
        }
        ast::Type::Paren(_) => match node.children().find_map(ast::Type::cast) {
            Some(inner) => resolve_type(&inner, scope, diags),
            None => Ty::Error,
        },
        ast::Type::Never(_) => Ty::Never,
        ast::Type::TypeType(_) => {
            diags.push(Diagnostic::error(
                Code::E1006,
                range,
                "`type` is the type of a derive function's parameter only                  (`const fn f(t: type) -> Impl<Tr>`)",
            ));
            Ty::Error
        }
        // `impl Fn() with E` and `dyn Fn() with E`: the effects are its closure bound's
        // (`expr.closure.effects.2`), which `bounds` reads.
        ast::Type::Effect(_)
            if let Some(inner @ (ast::Type::Impl(_) | ast::Type::Dyn(_))) =
                node.children().find_map(ast::Type::cast) =>
        {
            resolve_type(&inner, scope, diags)
        }
        ast::Type::Impl(_) => match scope.impls.iter().find(|(r, _)| *r == range) {
            Some(&(_, i)) => Ty::Param(i as u32, scope.params[i].name.as_str().into()),
            None => {
                diags.push(Diagnostic::unsupported(
                    range,
                    "`impl Trait` other than as a parameter's type",
                ));
                Ty::Error
            }
        },
        ast::Type::Dyn(_) => {
            diags.push(Diagnostic::error(
                Code::E0605,
                range,
                "`dyn Trait` has no size of its own, so it is used behind a borrow: `&dyn Trait`",
            ));
            Ty::Error
        }
        // `(u32 where self >= 1)`: the hidden `type` item the expansion made of it
        // (`refine.inline.1`).
        ast::Type::Refined(_) => {
            let name = crate::derive::inline_refinement_name(range);
            match scope.tree.lookup(scope.module, &name, false) {
                Some(Def::Type(t)) => type_of_item(scope.tree, t),
                _ => {
                    diags.push(Diagnostic::unsupported(range, "this refinement, here,"));
                    Ty::Error
                }
            }
        }
        ast::Type::Infer(_) if scope.holes => hole(),
        ast::Type::Infer(_) => {
            diags.push(Diagnostic::error(
                Code::E0102,
                range,
                "`_` stands for a type to infer, which a signature must write out",
            ));
            Ty::Error
        }
        _ => {
            diags.push(Diagnostic::unsupported(range, "this type"));
            Ty::Error
        }
    }
}

/// The effect a type after `with` names: `dyn Time` for a trait `Time`, or the type itself
/// (context data).
pub(crate) fn effect_type(t: &ast::Type, scope: &Scope, diags: &mut Vec<Diagnostic>) -> Ty {
    if let ast::Type::Path(_) = t
        && let Some(path) = t.syntax().child(K::PATH)
        && let Some(segs) = path_segments(&path)
        && !(segs.len() == 1 && scope.param(&segs[0].name).is_some())
        && let names = segs.iter().map(|s| s.name.as_str()).collect::<Vec<_>>()
        && let Ok((Def::Trait(_), n)) = scope.tree.path(scope.module, &names, scope.prelude_first)
        && n == names.len()
    {
        return match trait_path(&path, scope, diags) {
            Some((trait_ref, fixed)) if fixed.is_empty() => Ty::dyn_of(trait_ref),
            Some(_) => {
                diags.push(Diagnostic::unsupported(
                    t.syntax().text_range(),
                    "associated types fixed on an effect",
                ));
                Ty::Error
            }
            None => Ty::Error,
        };
    }
    resolve_type(t, scope, diags)
}

/// `dyn Trait`, the `DYN_TYPE` node `node`: one trait, with no associated types fixed.
pub(crate) fn dyn_type(node: &SyntaxNode, scope: &Scope, diags: &mut Vec<Diagnostic>) -> Ty {
    let bounds: Vec<SyntaxNode> = node.children().filter(|c| c.kind() == K::BOUND).collect();
    let [bound] = bounds.as_slice() else {
        diags.push(Diagnostic::error(
            Code::E0605,
            node.text_range(),
            "`dyn` takes exactly one trait",
        ));
        return Ty::Error;
    };
    let Some(path) = bound.child(K::PATH) else {
        diags.push(Diagnostic::unsupported(bound.text_range(), "this bound"));
        return Ty::Error;
    };
    // `dyn Fn(A) -> R with …`: the closure trait, with `FnOnce`'s `Output` and `Fails` fixed, and
    // the needs and control effects a call has (`type.box.dyn.1`).
    if let Some(sugar) = bound.child(K::FN_SUGAR) {
        let effects = node
            .parent()
            .filter(|p| p.kind() == K::EFFECT_TYPE)
            .and_then(|p| p.child(K::TYPE_EFFECTS));
        let preds = closure_bound(
            &path,
            &sugar,
            effects.as_ref(),
            &Ty::Error,
            true,
            bound.text_range(),
            scope,
            diags,
        );
        let [p, once] = preds.as_slice() else {
            return Ty::Error;
        };
        let mut fixed = once.assoc.clone();
        if fixed.len() == 1 {
            fixed.push((1, Ty::Never));
        }
        if let Some(e) = &effects {
            let fx = bound_effects(e, scope, &mut Vec::new());
            if fx.control != 0 {
                fixed.push((crate::ty::DYN_CONTROL, Ty::Len(i64::from(fx.control))));
            }
        }
        return Ty::Dyn(p.trait_ref.clone(), fixed.into());
    }
    match trait_path(&path, scope, diags) {
        Some((trait_ref, fixed)) if fixed.is_empty() => Ty::dyn_of(trait_ref),
        Some(_) => {
            diags.push(Diagnostic::unsupported(
                bound.text_range(),
                "associated types fixed on a `dyn` type",
            ));
            Ty::Error
        }
        None => Ty::Error,
    }
}

/// The type a `type` item names: a refined type, or the type an alias stands for.
fn type_of_item(tree: &ItemTree, t: u32) -> Ty {
    let def = &tree.types[t as usize];
    match def.refinement {
        Some(_) => Ty::Refined(t, def.name.as_str().into()),
        None => def.ty.clone(),
    }
}

/// The length of an array type, `[T; n]`: an integer literal or a const parameter
/// (`type.array.1`).
pub(crate) fn array_len(e: &ast::Expr, scope: &Scope, diags: &mut Vec<Diagnostic>) -> Ty {
    let node = e.syntax();
    let range = node.text_range();
    if let ast::Expr::Literal(_) = e
        && let Some(tok) = node.child_tokens().find(|t| t.kind() == K::INTEGER)
    {
        return match ast::int_value(tok.text()).and_then(|v| i64::try_from(v).ok()) {
            Some(v) => Ty::Len(v),
            None => {
                diags.push(Diagnostic::error(
                    Code::E0204,
                    range,
                    "this length does not fit in `int`",
                ));
                Ty::Error
            }
        };
    }
    if let ast::Expr::Path(_) = e
        && let Some(segs) = node.child(K::PATH).and_then(|p| path_segments(&p))
        && let [one] = segs.as_slice()
        && let Some(i) = scope.params.iter().rposition(|p| p.name == one.name)
    {
        if scope.params[i].konst {
            return Ty::Param(i as u32, one.name.as_str().into());
        }
        diags.push(Diagnostic::error(
            Code::E0201,
            range,
            format!("`{}` is a type, not a length", one.name),
        ));
        return Ty::Error;
    }
    // A const item, or arithmetic on literals and const items (`generic.const.3`).
    match crate::constfold::fold(e, &mut |segs| const_named(scope, segs)) {
        Ok(v) => Ty::Len(v),
        Err(why) => {
            diags.push(Diagnostic::error(
                Code::E0201,
                range,
                format!("an array's length is known when compiling: {why}"),
            ));
            Ty::Error
        }
    }
}

/// The value of the `int` const item path `segs` names in `scope`, if it names one
/// (`generic.const.3`).
pub(crate) fn const_named(scope: &Scope, segs: &[String]) -> Option<Result<i64, String>> {
    let names: Vec<&str> = segs.iter().map(String::as_str).collect();
    match scope.tree.path(scope.module, &names, scope.prelude_first) {
        Ok((Def::Fn(f), n)) if n == segs.len() => scope.tree.const_values.get(&f).cloned(),
        _ => None,
    }
}

/// The const item a type argument names (`f::<LIMIT>()`, `Grid<SIZE>`), as a length, if it is one
/// (`generic.const.3`).
fn const_type_arg(t: &ast::Type, scope: &Scope) -> Option<Result<i64, String>> {
    let ast::Type::Path(_) = t else {
        return None;
    };
    let segs: Vec<String> = t
        .syntax()
        .descendants()
        .into_iter()
        .filter(|n| n.kind() == K::PATH_SEGMENT)
        .map(|s| {
            s.token(K::IDENT)
                .map(|t| t.text().to_string())
                .filter(|_| s.child(K::GENERIC_ARGS).is_none())
        })
        .collect::<Option<Vec<String>>>()?;
    if let [one] = segs.as_slice()
        && scope.params.iter().any(|p| p.name == *one)
    {
        return None;
    }
    const_named(scope, &segs)
}

/// Whether the type node `node` is what a borrow (`&T`) points to, or the type an impl is for:
/// the places a type without a size of its own may be named.
fn behind_borrow(node: &SyntaxNode) -> bool {
    node.parent()
        .is_some_and(|p| matches!(p.kind(), K::REF_TYPE | K::IMPL_ITEM))
}

/// Whether `GENERIC_ARGS` node `args` is a `Box`'s, whose argument may be a `dyn` type.
fn boxed_arg(args: &SyntaxNode) -> bool {
    args.parent()
        .and_then(|seg| seg.token(K::IDENT))
        .is_some_and(|t| t.text() == "Box")
}

/// The type arguments and associated-type bindings (`Item = int`) of a `GENERIC_ARGS` node.
pub(crate) fn generic_args(
    node: &SyntaxNode,
    scope: &Scope,
    diags: &mut Vec<Diagnostic>,
) -> (Vec<Ty>, Vec<(String, TextRange, Ty)>) {
    let mut types = Vec::new();
    let mut assoc = Vec::new();
    for c in node.children() {
        match c.kind() {
            K::TYPE_ARG => match c.children().find_map(ast::Type::cast) {
                // A `Box` owns a value of any type that implements the trait
                // (`type.box.dyn.1`): `Box<dyn Trait>`, `Box<dyn Fn() with E>`.
                Some(t)
                    if boxed_arg(node)
                        && let Some(d) = match &t {
                            ast::Type::Dyn(_) => Some(t.clone()),
                            ast::Type::Effect(_) => t
                                .syntax()
                                .children()
                                .find_map(ast::Type::cast)
                                .filter(|d| matches!(d, ast::Type::Dyn(_))),
                            _ => None,
                        } =>
                {
                    types.push(dyn_type(d.syntax(), scope, diags));
                }
                // A const item's name as a const argument (`generic.const.3`).
                Some(t) if let Some(v) = const_type_arg(&t, scope) => match v {
                    Ok(v) => types.push(Ty::Len(v)),
                    Err(why) => {
                        diags.push(Diagnostic::error(
                            Code::E0201,
                            c.text_range(),
                            format!("a const argument is known when compiling: {why}"),
                        ));
                        types.push(Ty::Error);
                    }
                },
                Some(t) => types.push(resolve_type(&t, scope, diags)),
                None => {
                    diags.push(Diagnostic::unsupported(c.text_range(), "a const argument"));
                    types.push(Ty::Error);
                }
            },
            K::ASSOC_ARG => {
                let name = c.token(K::IDENT);
                let t = match c.children().find_map(ast::Type::cast) {
                    Some(t) => resolve_type(&t, scope, diags),
                    None => Ty::Error,
                };
                if let Some(n) = name {
                    assoc.push((n.text().to_string(), n.text_range(), t));
                }
            }
            K::CONST_ARG => {
                let lit = c.child_tokens().find(|t| t.kind() == K::INTEGER);
                let neg = c.token(K::MINUS).is_some();
                match lit.and_then(|t| ast::int_value(t.text())) {
                    Some(v) => match i64::try_from(if neg { -v } else { v }) {
                        Ok(v) => types.push(Ty::Len(v)),
                        Err(_) => {
                            diags.push(Diagnostic::error(
                                Code::E0204,
                                c.text_range(),
                                "this length does not fit in `int`",
                            ));
                            types.push(Ty::Error);
                        }
                    },
                    // `{N * 2}` (`generic.const.3`).
                    None if let Some(b) = c.child(K::BLOCK) => {
                        match crate::constfold::fold_block(&b, &mut |segs| const_named(scope, segs))
                        {
                            Ok(v) => types.push(Ty::Len(v)),
                            Err(why) => {
                                diags.push(Diagnostic::unsupported(
                                    c.text_range(),
                                    &format!("this const argument ({why})"),
                                ));
                                types.push(Ty::Error);
                            }
                        }
                    }
                    None => {
                        diags.push(Diagnostic::unsupported(
                            c.text_range(),
                            "a const argument other than an integer",
                        ));
                        types.push(Ty::Error);
                    }
                }
            }
            _ => {}
        }
    }
    (types, assoc)
}

/// Checks that `n` arguments were given for `want` parameters (E0404).
pub(crate) fn arity(
    what: &str,
    want: usize,
    n: usize,
    range: TextRange,
    diags: &mut Vec<Diagnostic>,
) -> bool {
    if want == n {
        return true;
    }
    diags.push(Diagnostic::error(
        Code::E0404,
        range,
        format!(
            "{what} takes {want} type argument{}, but {n} {} given",
            if want == 1 { "" } else { "s" },
            if n == 1 { "was" } else { "were" },
        ),
    ));
    false
}

/// `args` with the arguments left out after them filled from `params`' defaults
/// (`generic.default.1`), each with the parameters before it as `before` and the arguments so
/// far: `None` if one left out has none.
pub(crate) fn with_defaults(
    params: &[GenericParam],
    before: &[Ty],
    args: Vec<Ty>,
) -> Option<Vec<Ty>> {
    let mut all: Vec<Ty> = before.iter().cloned().chain(args).collect();
    for p in &params[all.len()..] {
        all.push(p.default.as_ref()?.subst(&all));
    }
    Some(all.split_off(before.len()))
}

/// The arguments of a struct or enum written with `args` (or without any).
pub(crate) fn adt_args(
    adt: u32,
    args: Option<&SyntaxNode>,
    range: TextRange,
    scope: &Scope,
    diags: &mut Vec<Diagnostic>,
) -> Vec<Ty> {
    let def = &scope.tree.adts[adt as usize];
    let want = def.generics.params.len();
    match args {
        Some(a) => {
            let (types, assoc) = generic_args(a, scope, diags);
            for (_, r, _) in assoc {
                diags.push(Diagnostic::error(
                    Code::E0413,
                    r,
                    format!("`{}` has no associated types", def.name),
                ));
            }
            let types = if types.len() < want {
                with_defaults(&def.generics.params, &[], types.clone()).unwrap_or(types)
            } else {
                types
            };
            if arity(&format!("`{}`", def.name), want, types.len(), range, diags) {
                types
            } else {
                vec![Ty::Error; want]
            }
        }
        None if want == 0 => Vec::new(),
        None if scope.holes => vec![hole(); want],
        None => {
            arity(&format!("`{}`", def.name), want, 0, range, diags);
            vec![Ty::Error; want]
        }
    }
}

pub(crate) fn path_type(
    segs: &[Segment],
    range: TextRange,
    scope: &Scope,
    diags: &mut Vec<Diagnostic>,
) -> Ty {
    let tree = scope.tree;
    let no_args = |s: &Segment, diags: &mut Vec<Diagnostic>| {
        if let Some(a) = &s.args {
            diags.push(Diagnostic::error(
                Code::E0404,
                a.text_range(),
                format!("`{}` takes no type arguments", s.name),
            ));
        }
    };
    match segs {
        [one] if scope.template.is_some_and(|(v, _)| v == one.name) => {
            no_args(one, diags);
            scope.template.map(|(_, t)| t.clone()).unwrap_or(Ty::Error)
        }
        [one] => {
            let name = one.name.as_str();
            match name {
                "int" | "i64" => {
                    no_args(one, diags);
                    return Ty::Int;
                }
                n if crate::ty::Num::from_name(n).is_some() => {
                    no_args(one, diags);
                    return Ty::Num(crate::ty::Num::from_name(n).expect("a number type"));
                }
                "bool" => {
                    no_args(one, diags);
                    return Ty::Bool;
                }
                "char" => {
                    no_args(one, diags);
                    return Ty::Char;
                }
                "str" => {
                    no_args(one, diags);
                    return Ty::Str;
                }
                // The prelude's buffer, for the code that builds collections on it.
                "Buf" if scope.param("Buf").is_none() => {
                    let args = match &one.args {
                        Some(a) => generic_args(a, scope, diags).0,
                        None if scope.holes => vec![hole()],
                        None => Vec::new(),
                    };
                    return match args.as_slice() {
                        [t] => Ty::Buf(t.clone().into()),
                        _ => {
                            diags.push(Diagnostic::error(
                                Code::E0404,
                                range,
                                "`Buf` takes one type argument: `Buf<T>`",
                            ));
                            Ty::Error
                        }
                    };
                }
                "Self" => {
                    no_args(one, diags);
                    return match &scope.self_ty {
                        Some(t) => t.clone(),
                        None => {
                            diags.push(Diagnostic::error(
                                Code::E0102,
                                range,
                                "`Self` is the type of an impl or trait, and there is none here",
                            ));
                            Ty::Error
                        }
                    };
                }
                _ => {}
            }
            if let Some(p) = scope.param(name) {
                no_args(one, diags);
                return p;
            }
            let what = match tree.lookup(scope.module, name, scope.prelude_first) {
                Some(Def::Adt(a)) => {
                    let args = adt_args(a, one.args.as_ref(), range, scope, diags);
                    return tree.adts[a as usize].ty_with(a, args);
                }
                Some(Def::Type(t)) => {
                    no_args(one, diags);
                    return type_of_item(tree, t);
                }
                Some(Def::Trait(_)) => format!("`{name}` is a trait, not a type"),
                Some(Def::Module(_)) => format!("`{name}` is a module, not a type"),
                Some(_) => format!("`{name}` is not a type"),
                None => format!("there is no type called `{name}`"),
            };
            diags.push(Diagnostic::error(Code::E0102, range, what));
            Ty::Error
        }
        // `T::Name` and `Self::Name`: an associated type.
        [base, assoc] if base.name == "Self" || scope.param(&base.name).is_some() => {
            let b = path_type(std::slice::from_ref(base), base.range, scope, diags);
            if b == Ty::Error {
                return Ty::Error;
            }
            no_args(assoc, diags);
            assoc_type(&b, base.name == "Self", &assoc.name, range, scope, diags)
        }
        _ => {
            // A path through modules: `mod::shapes::Point`.
            let names: Vec<&str> = segs.iter().map(|s| s.name.as_str()).collect();
            let last = segs.last().expect("two or more segments");
            match tree.path(scope.module, &names, scope.prelude_first) {
                // `f::Args`: a wrapped function's arguments (`attr.around.args.1`), the alias
                // its expansion has beside it.
                Ok((Def::Fn(f), n)) if n + 1 == segs.len() && last.name == "Args" => {
                    no_args(last, diags);
                    let sig = &tree.fns[f as usize];
                    let alias = crate::wrap::args_type_name(&sig.name);
                    match tree.lookup(sig.module, &alias, false) {
                        Some(Def::Type(t)) => type_of_item(tree, t),
                        _ => {
                            diags.push(Diagnostic::error(
                                Code::E0102,
                                range,
                                format!(
                                    "`{}` has no `Args`: only a wrapped function without type \
                                     parameters has (`#[around(…)]`)",
                                    sig.name
                                ),
                            ));
                            Ty::Error
                        }
                    }
                }
                Ok((Def::Adt(a), n)) if n == segs.len() => {
                    let args = adt_args(a, last.args.as_ref(), range, scope, diags);
                    tree.adts[a as usize].ty_with(a, args)
                }
                Ok((Def::Type(t), n)) if n == segs.len() => {
                    no_args(last, diags);
                    type_of_item(tree, t)
                }
                Ok((Def::Adt(a), n)) if n + 1 == segs.len() => {
                    let base = &segs[n - 1];
                    let args = adt_args(a, base.args.as_ref(), base.range, scope, diags);
                    let b = tree.adts[a as usize].ty_with(a, args);
                    no_args(last, diags);
                    assoc_type(&b, false, &last.name, range, scope, diags)
                }
                Ok((def, n)) => {
                    let what = if n == segs.len() {
                        format!("`{}` is not a type", names.join("::"))
                    } else {
                        format!("`{}` has nothing called `{}`", tree.def_name(def), names[n])
                    };
                    diags.push(Diagnostic::error(Code::E0102, range, what));
                    Ty::Error
                }
                Err(PathError::Private(k, def)) => {
                    diags.push(tree.hidden(def, segs[k].range));
                    Ty::Error
                }
                Err(PathError::Unresolved(k)) => {
                    let what = if k == 0 {
                        format!("there is no type or module called `{}`", names[0])
                    } else {
                        format!(
                            "there is nothing called `{}` in `{}`",
                            names[k],
                            names[..k].join("::")
                        )
                    };
                    diags.push(Diagnostic::error(Code::E0102, segs[k].range, what));
                    Ty::Error
                }
            }
        }
    }
}

/// `base::name`: an associated type of a trait `base` implements.
pub(crate) fn assoc_type(
    base: &Ty,
    is_self: bool,
    name: &str,
    range: TextRange,
    scope: &Scope,
    diags: &mut Vec<Diagnostic>,
) -> Ty {
    let tree = scope.tree;
    if is_self && let Some((tr, assoc)) = &scope.impl_assoc {
        let def = &tree.traits[tr.trait_ as usize];
        if let Some(i) = def.assoc.iter().position(|(n, _)| n == name) {
            return assoc.get(i).cloned().flatten().unwrap_or(Ty::Error);
        }
    }
    if is_self && let Some(t) = scope.trait_ctx {
        let def = &tree.traits[t as usize];
        if let Some(i) = def.assoc.iter().position(|(n, _)| n == name) {
            let args: Vec<Ty> = def.generics.identity().into_iter().skip(1).collect();
            return Ty::Assoc(Arc::new(AssocTy {
                self_ty: base.clone(),
                trait_ref: TraitRef {
                    trait_: t,
                    name: def.name.as_str().into(),
                    args: args.into(),
                },
                index: i as u32,
                name: name.into(),
            }));
        }
    }
    for p in elaborate(tree, scope.preds) {
        if p.ty != *base {
            continue;
        }
        let def = &tree.traits[p.trait_ref.trait_ as usize];
        if let Some(i) = def.assoc.iter().position(|(n, _)| n == name) {
            if let Some((_, t)) = p.assoc.iter().find(|(k, _)| *k == i as u32) {
                return t.clone();
            }
            return Ty::Assoc(Arc::new(AssocTy {
                self_ty: base.clone(),
                trait_ref: p.trait_ref.clone(),
                index: i as u32,
                name: name.into(),
            }));
        }
    }
    diags.push(Diagnostic::error(
        Code::E0413,
        range,
        format!("`{base}` has no associated type `{name}` through the bounds in scope"),
    ));
    Ty::Error
}

/// `<T as Trait>::Name`.
pub(crate) fn qualified_type(path: &SyntaxNode, scope: &Scope, diags: &mut Vec<Diagnostic>) -> Ty {
    let range = path.text_range();
    let Some(q) = path.child(K::QUALIFIED_PATH) else {
        return Ty::Error;
    };
    let base = match q.children().find_map(ast::Type::cast) {
        Some(t) => resolve_type(&t, scope, diags),
        None => return Ty::Error,
    };
    let Some(tpath) = q.child(K::PATH) else {
        diags.push(Diagnostic::unsupported(range, "this path"));
        return Ty::Error;
    };
    let Some((trait_ref, _)) = trait_path(&tpath, scope, diags) else {
        return Ty::Error;
    };
    let name = path
        .children()
        .filter(|c| c.kind() == K::PATH_SEGMENT)
        .last()
        .map(|s| s.text().trim().to_string());
    let Some(name) = name else {
        return Ty::Error;
    };
    let def = &scope.tree.traits[trait_ref.trait_ as usize];
    match def.assoc.iter().position(|(n, _)| *n == name) {
        Some(i) => Ty::Assoc(Arc::new(AssocTy {
            self_ty: base,
            trait_ref,
            index: i as u32,
            name: name.as_str().into(),
        })),
        None => {
            diags.push(Diagnostic::error(
                Code::E0413,
                range,
                format!("`{}` has no associated type `{name}`", def.name),
            ));
            Ty::Error
        }
    }
}

/// The trait named by the `PATH` `path` (`Clone`, `Add<Output = int>`), with the associated
/// types it fixes.
pub(crate) fn trait_path(
    path: &SyntaxNode,
    scope: &Scope,
    diags: &mut Vec<Diagnostic>,
) -> Option<(TraitRef, Vec<(u32, Ty)>)> {
    let range = path.text_range();
    let segs = path_segments(path)?;
    let seg = segs.last()?;
    let names: Vec<&str> = segs.iter().map(|s| s.name.as_str()).collect();
    let found = if segs.len() == 1 && scope.param(&seg.name).is_some() {
        None
    } else {
        match scope.tree.path(scope.module, &names, scope.prelude_first) {
            Ok((def, n)) if n == names.len() => Some(def),
            Err(PathError::Private(k, def)) => {
                diags.push(scope.tree.hidden(def, segs[k].range));
                return None;
            }
            Ok(_) | Err(PathError::Unresolved(_)) => None,
        }
    };
    let Some(Def::Trait(t)) = found else {
        let what = if matches!(found, Some(Def::Adt(_)))
            || (segs.len() == 1 && scope.param(&seg.name).is_some())
            || matches!(seg.name.as_str(), "int" | "bool")
        {
            format!("`{}` is a type, not a trait", seg.name)
        } else {
            format!("there is no trait called `{}`", names.join("::"))
        };
        diags.push(Diagnostic::error(Code::E0401, range, what));
        return None;
    };
    let def = &scope.tree.traits[t as usize];
    let want = def.generics.params.len() - 1;
    let (args, assoc) = match &seg.args {
        Some(a) => generic_args(a, scope, diags),
        None => (Vec::new(), Vec::new()),
    };
    // Arguments left out take their defaults, `Self` being what the bound or impl is for
    // (`generic.default.1`): `Add` is `Add<Self>`.
    let args = if args.len() < want {
        let this = scope
            .default_self
            .clone()
            .or_else(|| scope.self_ty.clone())
            .unwrap_or(Ty::Error);
        with_defaults(&def.generics.params, &[this], args.clone()).unwrap_or(args)
    } else {
        args
    };
    let args = if arity(&format!("`{}`", def.name), want, args.len(), range, diags) {
        args
    } else {
        vec![Ty::Error; want]
    };
    let mut fixed = Vec::new();
    for (name, r, ty) in assoc {
        match def.assoc.iter().position(|(n, _)| *n == name) {
            Some(i) => fixed.push((i as u32, ty)),
            None => diags.push(Diagnostic::error(
                Code::E0413,
                r,
                format!("`{}` has no associated type `{name}`", def.name),
            )),
        }
    }
    Some((
        TraitRef {
            trait_: t,
            name: def.name.as_str().into(),
            args: args.into(),
        },
        fixed,
    ))
}

/// The bounds of `bounded` parameters and of the `where` clause of `item`.
pub(crate) fn preds_of(
    item: &SyntaxNode,
    bounded: &[(usize, SyntaxNode)],
    scope: &Scope,
    ds: &mut Vec<Diagnostic>,
) -> Vec<Pred> {
    let mut out: Vec<Pred> = Vec::new();
    for (i, node) in bounded {
        let ty = Ty::Param(*i as u32, scope.params[*i].name.as_str().into());
        // Each parameter's bounds see those of the parameters before it:
        // `impl<I: Iterator, F: FnMut(I::Item) -> B>`.
        let mut seen = scope.preds.to_vec();
        seen.extend(out.iter().cloned());
        let inner = Scope {
            preds: &seen,
            ..scope.clone()
        };
        let found = bounds(node, &ty, &inner, ds);
        out.extend(found);
    }
    if let Some(w) = item.child(K::WHERE_CLAUSE) {
        // The `where` clause sees the parameters' own bounds: `F::Fails` through `F: Fn(…)`.
        let mut seen = scope.preds.to_vec();
        seen.extend(out.iter().cloned());
        let inner = Scope {
            preds: &seen,
            ..scope.clone()
        };
        for p in w.children().filter(|c| c.kind() == K::WHERE_PRED) {
            let Some(t) = p.children().find_map(ast::Type::cast) else {
                continue;
            };
            let ty = resolve_type(&t, &inner, ds);
            out.extend(bounds(&p, &ty, &inner, ds));
        }
    }
    out
}

/// `Fn(A, B) -> R` (or `FnMut`, `FnOnce`) as a bound on `ty` (`expr.closure.param.1`): the trait
/// for the parameters' tuple, and `FnOnce` with its `Output` fixed (`()` if not written), and its
/// `Fails` too if the bound names its effects (`expr.closure.effects.2`).
#[allow(clippy::too_many_arguments)]
fn closure_bound(
    path: &SyntaxNode,
    sugar: &SyntaxNode,
    effects: Option<&SyntaxNode>,
    ty: &Ty,
    stored: bool,
    range: TextRange,
    scope: &Scope,
    diags: &mut Vec<Diagnostic>,
) -> Vec<Pred> {
    let tree = scope.tree;
    let segs: Vec<String> = path_segments(path)
        .unwrap_or_default()
        .into_iter()
        .map(|s| s.name)
        .collect();
    let names: Vec<&str> = segs.iter().map(String::as_str).collect();
    let found = match tree.path(scope.module, &names, scope.prelude_first) {
        Ok((Def::Trait(t), n)) if n == names.len() => Some(t),
        _ => None,
    };
    let closure_trait = |name| tree.lang(name);
    let Some(t) = found.filter(|t| {
        [
            closure_trait("Fn"),
            closure_trait("FnMut"),
            closure_trait("FnOnce"),
        ]
        .contains(&Some(*t))
    }) else {
        diags.push(Diagnostic::error(
            Code::E0401,
            range,
            "only the closure traits `Fn`, `FnMut` and `FnOnce` are written with `(…) -> …`",
        ));
        return Vec::new();
    };
    let args: Vec<Ty> = sugar
        .child(K::TYPE_LIST)
        .map(|l| {
            l.children()
                .filter_map(ast::Type::cast)
                .map(|t| resolve_type(&t, scope, diags))
                .collect()
        })
        .unwrap_or_default();
    let output = sugar
        .children()
        .filter(|c| c.kind() != K::TYPE_LIST)
        .find_map(ast::Type::cast)
        .map_or(Ty::Unit, |t| resolve_type(&t, scope, diags));
    let args: Arc<[Ty]> = vec![Ty::tuple(args)].into();
    let trait_ref = |t: u32| TraitRef {
        trait_: t,
        name: tree.traits[t as usize].name.as_str().into(),
        args: args.clone(),
    };
    let once = closure_trait("FnOnce").expect("the prelude's `FnOnce`");
    // The control effects the bound lists (`expr.closure.control.1`).
    let control = effects.map_or(0, |e| bound_effects(e, scope, &mut Vec::new()).control);
    vec![
        Pred {
            ty: ty.clone(),
            trait_ref: trait_ref(t),
            assoc: Vec::new(),
            control,
            range,
        },
        Pred {
            ty: ty.clone(),
            trait_ref: trait_ref(once),
            assoc: match effects.map(|e| bound_effects(e, scope, diags)) {
                Some(fx) => {
                    // A closure's needs are met where it is written, so neither a bound nor a
                    // stored closure's type lists them (`expr.closure.effects.1`,
                    // `expr.closure.stored.1`).
                    for (need, at) in &fx.needs {
                        let what = if stored {
                            "a stored closure owns what it captures, so it uses no handler: \
                             its type lists"
                        } else {
                            "a closure parameter's needs are met where the closure is \
                             written, so its bound lists"
                        };
                        diags.push(Diagnostic::error(
                            Code::E0609,
                            *at,
                            format!(
                                "{what} only `fail` and the control effects, not `{}`",
                                crate::check::show_effect(need)
                            ),
                        ));
                    }
                    vec![(0, output), (1, fx.fails)]
                }
                None => vec![(0, output)],
            },
            control,
            range,
        },
    ]
}

/// The effects a closure's bound or type names (`expr.closure.effects.2`).
pub(crate) struct BoundEffects {
    /// Its errors: those of `fail(…)`, and the effect variables (`with E`), which stand for
    /// error sets.
    pub fails: Ty,
    /// The control effects.
    pub control: crate::items::Control,
    /// The handlers' effects it names, which are errors (E0609).
    pub needs: Vec<(Ty, TextRange)>,
}

/// The effects of `with …` (a `TYPE_EFFECTS` node) on a closure's bound or type.
pub(crate) fn bound_effects(
    node: &SyntaxNode,
    scope: &Scope,
    diags: &mut Vec<Diagnostic>,
) -> BoundEffects {
    let mut members: Vec<Ty> = Vec::new();
    let mut control = 0;
    let mut needs = Vec::new();
    for e in node.children().filter(|c| c.kind() == K::EFFECT) {
        let bit = if e.token(K::SUSPEND_KW).is_some() {
            crate::items::SUSPEND
        } else if e.token(K::SPAWN_KW).is_some() {
            crate::items::SPAWN
        } else {
            0
        };
        if bit != 0 {
            if control & bit != 0 {
                diags.push(Diagnostic::error(
                    Code::E0606,
                    e.text_range(),
                    "an effect is listed once",
                ));
            }
            control |= bit;
            continue;
        }
        if e.token(K::FAIL_KW).is_some() {
            for t in e.child(K::ERROR_TYPES).into_iter().flat_map(|ts| {
                ts.children()
                    .filter_map(ast::Type::cast)
                    .collect::<Vec<_>>()
            }) {
                let ty = resolve_type(&t, scope, diags);
                if crate::collect::error_type(&ty, t.syntax().text_range(), &members, diags) {
                    members.push(ty);
                }
            }
            continue;
        }
        let Some(t) = e.children().find_map(ast::Type::cast) else {
            diags.push(Diagnostic::unsupported(e.text_range(), "this effect"));
            continue;
        };
        // An effect variable: a type parameter standing for the closure's errors.
        if let ast::Type::Path(_) = &t
            && let Some(path) = t.syntax().child(K::PATH)
            && let Some(segs) = path_segments(&path)
            && segs.len() == 1
            && let Some(p) = scope.param(&segs[0].name)
        {
            members.push(p);
            continue;
        }
        needs.push((effect_type(&t, scope, diags), e.text_range()));
    }
    BoundEffects {
        fails: Ty::errors(members),
        control,
        needs,
    }
}

/// The bounds `node` (a `TYPE_PARAM`, `WHERE_PRED` or trait item) puts on `ty`: its `BOUND`
/// children.
pub(crate) fn bounds(
    node: &SyntaxNode,
    ty: &Ty,
    scope: &Scope,
    diags: &mut Vec<Diagnostic>,
) -> Vec<Pred> {
    let mut out = Vec::new();
    for b in node.children().filter(|c| c.kind() == K::BOUND) {
        let Some(path) = b.child(K::PATH) else {
            continue;
        };
        if let Some(sugar) = b.child(K::FN_SUGAR) {
            // Its effects: in a parameter's or a `where` clause's bound, after the sugar; in
            // `impl Fn() with E` and `dyn Fn() with E`, the type's.
            let effects = sugar.child(K::TYPE_EFFECTS).or_else(|| {
                matches!(node.kind(), K::IMPL_TYPE | K::DYN_TYPE)
                    .then(|| node.parent())
                    .flatten()
                    .filter(|p| p.kind() == K::EFFECT_TYPE)
                    .and_then(|p| p.child(K::TYPE_EFFECTS))
            });
            // A returned closure is a stored one: its type lists no handler
            // (`expr.closure.result.1`).
            let returned =
                node.kind() == K::IMPL_TYPE && node.ancestors().any(|a| a.kind() == K::RET_TYPE);
            out.extend(closure_bound(
                &path,
                &sugar,
                effects.as_ref(),
                ty,
                returned,
                b.text_range(),
                scope,
                diags,
            ));
            continue;
        }
        // A default argument naming `Self` names what is bounded (`T: Add` is `T: Add<T>`).
        let bounded = Scope {
            default_self: Some(ty.clone()),
            ..scope.clone()
        };
        if let Some((trait_ref, assoc)) = trait_path(&path, &bounded, diags) {
            out.push(Pred {
                ty: ty.clone(),
                trait_ref,
                assoc,
                control: 0,
                range: b.text_range(),
            });
        }
    }
    out
}

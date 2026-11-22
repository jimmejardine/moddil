//! Checks on the impls: each against its trait, and impls that could apply to the same type.

use moddil_syntax::TextRange;

use super::Found;
use crate::items::{FnSig, ItemTree, Src};
use crate::traits::{Answer, Solver};
use crate::{Code, Diagnostic, Ty};

/// Checks each impl of a trait against the trait: it defines every required method and
/// associated type and nothing else, its methods have the trait's signatures, and the type
/// implements the trait's supertraits. Destructors may only be written for the impl's own
/// struct or enum, for all its type arguments.
pub(crate) fn check_impls(tree: &ItemTree) -> Vec<Found> {
    let mut found: Vec<(Src, Option<TextRange>, Diagnostic)> = Vec::new();
    let drop = tree.lang("Drop");
    for imp in &tree.impls {
        let push = |found: &mut Vec<_>, d: Diagnostic| found.push((imp.src, imp.origin, d));
        // A refined type has its base's methods and traits, and none of its own yet.
        if let Ty::Refined(_, name) = &imp.self_ty {
            push(
                &mut found,
                Diagnostic::error(
                    Code::E0414,
                    imp.range,
                    format!(
                        "`{name}` has the methods and traits of its base, and none of its own, for now"
                    ),
                ),
            );
            continue;
        }
        let Some(tr) = &imp.trait_ref else {
            // Inherent impls: only for the file's own structs and enums.
            match imp.self_ty.adt() {
                Some(a) if tree.adts[a as usize].src == imp.src => {}
                Some(_) | None if imp.self_ty == Ty::Error => {}
                // The prelude gives the built-in types their methods.
                None if imp.src == Src::Prelude
                    && matches!(
                        imp.self_ty,
                        Ty::Int
                            | Ty::Num(_)
                            | Ty::Bool
                            | Ty::Char
                            | Ty::Str
                            | Ty::Array(..)
                            | Ty::Slice(_)
                            | Ty::Buf(_)
                    ) => {}
                _ => push(
                    &mut found,
                    Diagnostic::error(
                        Code::E0414,
                        imp.range,
                        format!(
                            "methods can be added only to a struct or enum defined here, not \
                             to `{}`; define a trait for them instead",
                            imp.self_ty
                        ),
                    ),
                ),
            }
            continue;
        };
        let tdef = &tree.traits[tr.trait_ as usize];
        // Associated types.
        for (k, (n, _)) in tdef.assoc.iter().enumerate() {
            if imp.assoc.get(k).is_none_or(Option::is_none) {
                push(
                    &mut found,
                    Diagnostic::error(
                        Code::E0407,
                        imp.range,
                        format!("the impl of `{tr}` must define the associated type `{n}`"),
                    ),
                );
            }
        }
        // Methods.
        let mut args = vec![imp.self_ty.clone()];
        args.extend(tr.args.iter().cloned());
        for &tm in &tdef.methods {
            let tsig = &tree.fns[tm as usize];
            let Some(&im) = imp
                .methods
                .iter()
                .find(|&&m| tree.fns[m as usize].name == tsig.name)
            else {
                if !tsig.has_body {
                    push(
                        &mut found,
                        Diagnostic::error(
                            Code::E0407,
                            imp.range,
                            format!("the impl of `{tr}` must define the method `{}`", tsig.name),
                        ),
                    );
                }
                continue;
            };
            let isig = &tree.fns[im as usize];
            if let Some(d) = signature_mismatch(tree, &args, tsig, isig) {
                found.push((isig.src, isig.origin, d));
            }
        }
        for &im in &imp.methods {
            let isig = &tree.fns[im as usize];
            if !tdef
                .methods
                .iter()
                .any(|&m| tree.fns[m as usize].name == isig.name)
            {
                found.push((
                    isig.src,
                    isig.origin,
                    Diagnostic::error(
                        Code::E0408,
                        isig.name_range,
                        format!("`{tr}` has no method `{}`", isig.name),
                    ),
                ));
            }
        }
        // Supertraits.
        let mut solver = Solver::new(tree, &imp.generics.preds);
        for s in &tdef.supertraits {
            let s = s.subst(&args);
            if solver.implements(&imp.self_ty, &s) == Answer::No {
                push(
                    &mut found,
                    Diagnostic::error(
                        Code::E0402,
                        imp.range,
                        format!(
                            "`{}` must implement `{s}` to implement `{tr}`, and it does not",
                            imp.self_ty
                        ),
                    ),
                );
            }
        }
        // Destructors.
        if Some(tr.trait_) == drop {
            let own = imp.self_ty.adt().is_some_and(|a| {
                let def = &tree.adts[a as usize];
                def.src == imp.src && imp.self_ty.args() == imp.generics.identity().as_slice()
            });
            if !own && imp.self_ty != Ty::Error {
                push(
                    &mut found,
                    Diagnostic::error(
                        Code::E0411,
                        imp.range,
                        "a destructor is written for a struct or enum defined here, for all \
                         its type arguments: `impl<T> Drop for Name<T>`",
                    ),
                );
            } else if !imp.generics.preds.is_empty() {
                push(
                    &mut found,
                    Diagnostic::error(
                        Code::E0411,
                        imp.range,
                        "a destructor cannot have bounds: it runs for every value of the type",
                    ),
                );
            }
        }
    }
    found
}

/// Two impls of the same trait that could apply to the same type (E0406), and inherent
/// methods defined twice for the same type (E0103).
pub(crate) fn overlap(tree: &ItemTree) -> Vec<Found> {
    let mut found = Vec::new();
    for (j, b) in tree.impls.iter().enumerate() {
        for a in &tree.impls[..j] {
            if a.src == Src::Prelude && b.src == Src::Prelude {
                continue;
            }
            let same_trait = match (&a.trait_ref, &b.trait_ref) {
                (Some(x), Some(y)) => x.trait_ == y.trait_,
                (None, None) => true,
                _ => false,
            };
            if !same_trait || a.self_ty == Ty::Error || b.self_ty == Ty::Error {
                continue;
            }
            let mut s = Solver::new(tree, &[]);
            let xa = s.fresh_n(a.generics.params.len());
            let xb = s.fresh_n(b.generics.params.len());
            let mut overlaps = s.unify(&a.self_ty.subst(&xa), &b.self_ty.subst(&xb));
            if let (Some(x), Some(y)) = (&a.trait_ref, &b.trait_ref) {
                overlaps &= x
                    .subst(&xa)
                    .args
                    .iter()
                    .zip(y.subst(&xb).args.iter())
                    .all(|(p, q)| s.unify(p, q));
            }
            if !overlaps {
                continue;
            }
            match &b.trait_ref {
                Some(t) => found.push((
                    b.src,
                    b.origin,
                    Diagnostic::error(
                        Code::E0406,
                        b.range,
                        format!(
                            "`{t}` is already implemented for `{}`: a type has at most one \
                             impl of a trait",
                            b.self_ty
                        ),
                    )
                    .with_label(a.range, "the other impl"),
                )),
                None => {
                    for &m in &b.methods {
                        let name = &tree.fns[m as usize].name;
                        if let Some(&first) = a
                            .methods
                            .iter()
                            .find(|&&x| tree.fns[x as usize].name == *name)
                        {
                            found.push((
                                b.src,
                                b.origin,
                                Diagnostic::error(
                                    Code::E0103,
                                    tree.fns[m as usize].name_range,
                                    format!("`{}` already has a method called `{name}`", b.self_ty),
                                )
                                .with_label(
                                    tree.fns[first as usize].name_range,
                                    "first defined here",
                                ),
                            ));
                        }
                    }
                }
            }
        }
    }
    found
}

/// Why impl method `isig` does not have the signature of trait method `tsig`, if it does not.
/// `args` are `Self` and the trait's arguments.
fn signature_mismatch(
    tree: &ItemTree,
    args: &[Ty],
    tsig: &FnSig,
    isig: &FnSig,
) -> Option<Diagnostic> {
    let at = isig.name_range;
    let mismatch = |what: String| {
        Some(
            Diagnostic::error(
                Code::E0409,
                at,
                format!("`{}` does not match the trait: {what}", isig.name),
            )
            .with_label(tsig.name_range, "declared here"),
        )
    };
    if tsig.self_kind != isig.self_kind {
        return mismatch("it takes `self` differently".to_string());
    }
    if tsig.params.len() != isig.params.len() {
        return mismatch(format!(
            "it has {} parameters, and the trait's has {}",
            isig.params.len(),
            tsig.params.len()
        ));
    }
    let t_own = tsig.generics.params.len() - tsig.generics.parent as usize;
    let i_own = isig.generics.params.len() - isig.generics.parent as usize;
    if t_own != i_own {
        return mismatch(format!(
            "it has {i_own} type parameters, and the trait's has {t_own}"
        ));
    }
    // The trait's method with `Self` and the trait's parameters replaced, and its own type
    // parameters renamed to the impl method's.
    let mut subst = args.to_vec();
    for k in 0..t_own {
        let p = &isig.generics.params[isig.generics.parent as usize + k];
        subst.push(Ty::Param(
            isig.generics.parent + k as u32,
            p.name.as_str().into(),
        ));
    }
    let mut solver = Solver::new(tree, &isig.generics.preds);
    let mut same = |a: &Ty, b: &Ty| {
        let a = solver.deep(&a.subst(&subst));
        let b = solver.deep(b);
        a == b || a.has_error() || b.has_error()
    };
    for (k, (tp, ip)) in tsig.params.iter().zip(&isig.params).enumerate() {
        if !same(&tp.ty, &ip.ty) {
            let t = tp.ty.subst(&subst);
            return mismatch(format!(
                "parameter {} has type `{}`, and the trait's has `{t}`",
                k + 1,
                ip.ty
            ));
        }
    }
    if !same(&tsig.ret, &isig.ret) {
        return mismatch(format!(
            "it returns `{}`, and the trait's returns `{}`",
            isig.ret,
            tsig.ret.subst(&subst)
        ));
    }
    // An impl may fail with fewer errors than the trait declares, never more.
    let allowed: Vec<Ty> = tsig.fails.subst(&subst).members();
    if let Some(extra) = isig
        .fails
        .members()
        .into_iter()
        .find(|m| !allowed.contains(m))
    {
        return mismatch(format!(
            "it can fail with `{extra}`, which the trait's does not declare"
        ));
    }
    None
}

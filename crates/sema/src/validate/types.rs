//! Checks on the structs and enums: recursive types, `Copy` types and derived impls.

use moddil_syntax::TextRange;

use super::Found;
use crate::items::{COPY_CAP, ItemTree, Src};
use crate::traits::{Answer, Solver};
use crate::{Code, Diagnostic, Ty};

/// Reports every struct or enum that contains itself without indirection (E0211), and makes
/// the fields of each an error so that later passes need not care.
pub(crate) fn recursive_types(tree: &mut ItemTree) -> Vec<Found> {
    fn contains(t: &Ty, out: &mut Vec<u32>) {
        match t {
            Ty::Adt(i, _, args) => {
                out.push(*i);
                args.iter().for_each(|t| contains(t, out));
            }
            Ty::Tuple(ts) => ts.iter().for_each(|t| contains(t, out)),
            _ => {}
        }
    }
    let edges: Vec<Vec<u32>> = tree
        .adts
        .iter()
        .map(|a| {
            let mut out = Vec::new();
            for f in a.variants.iter().flat_map(|v| &v.fields) {
                contains(&f.ty, &mut out);
            }
            out
        })
        .collect();
    let mut cyclic = Vec::new();
    for start in 0..tree.adts.len() {
        let mut seen = vec![false; tree.adts.len()];
        let mut work: Vec<u32> = edges[start].clone();
        while let Some(n) = work.pop() {
            if n as usize == start {
                cyclic.push(start);
                break;
            }
            if !std::mem::replace(&mut seen[n as usize], true) {
                work.extend(&edges[n as usize]);
            }
        }
    }
    let mut found = Vec::new();
    for start in cyclic {
        let a = &mut tree.adts[start];
        let d = Diagnostic::error(
            Code::E0211,
            a.name_range,
            format!(
                "`{}` contains itself, so it would have infinite size",
                a.name
            ),
        );
        let src = a.src;
        for f in a.variants.iter_mut().flat_map(|v| &mut v.fields) {
            f.ty = Ty::Error;
        }
        found.push((src, d));
    }
    found.into_iter().map(|(src, d)| (src, None, d)).collect()
}

/// Checks the structs and enums that are `Copy` (derived, or by a hand-written impl): every
/// field must be `Copy`, the type at most 64 bytes, and it may not have a destructor. A
/// type parameter counts as `Copy`, since the impl requires it to be.
pub(crate) fn copy_types(tree: &ItemTree) -> Vec<Found> {
    let Some(copy) = tree.lang("Copy") else {
        return Vec::new();
    };
    let mut found: Vec<(Src, Option<TextRange>, Diagnostic)> = Vec::new();
    for imp in &tree.impls {
        if imp.trait_ref.as_ref().map(|t| t.trait_) != Some(copy) {
            continue;
        }
        let Some(a) = imp.self_ty.adt() else {
            continue;
        };
        let def = &tree.adts[a as usize];
        // A derived impl is reported at the type's name, a hand-written one at the impl.
        let (src, at) = if imp.origin.is_some() {
            (def.src, def.name_range)
        } else {
            (imp.src, imp.range)
        };
        let mut solver = Solver::new(tree, &imp.generics.preds);
        let bad: Vec<String> = (0..def.variants.len() as u32)
            .flat_map(|v| {
                let types = tree.field_types(&imp.self_ty, v);
                tree.variant(a, v)
                    .fields
                    .iter()
                    .zip(types)
                    .collect::<Vec<_>>()
            })
            .filter(|(_, t)| !matches!(t, Ty::Param(..)) && !solver.is_copy(t))
            .map(|(f, t)| format!("`{}: {t}`", f.name))
            .collect();
        if !bad.is_empty() {
            found.push((
                src,
                None,
                Diagnostic::error(
                    Code::E0311,
                    at,
                    format!(
                        "`{}` cannot be `Copy`: the field {} is not `Copy`",
                        def.name,
                        bad.join(", ")
                    ),
                ),
            ));
            continue;
        }
        if tree.has_drop_impl(a) {
            found.push((
                src,
                None,
                Diagnostic::error(
                    Code::E0411,
                    at,
                    format!(
                        "`{}` has a destructor, so it cannot be `Copy`: a copy would run it \
                         twice",
                        def.name
                    ),
                ),
            ));
            continue;
        }
        let size = tree.size_of(&imp.self_ty).0;
        if size > COPY_CAP {
            found.push((
                src,
                None,
                Diagnostic::error(
                    Code::E0312,
                    at,
                    format!(
                        "`{}` is {size} bytes, over the {COPY_CAP}-byte cap for `Copy` types; \
                         copy it with `.clone()` instead",
                        def.name
                    ),
                ),
            ));
        }
    }
    found
}

/// A derive needs every field to implement the trait derived (E0402, at the attribute). A
/// type parameter's field counts, since the impl requires it to. `Clone` with `Copy` is
/// left to the `Copy` check.
pub(crate) fn derived_fields(tree: &ItemTree) -> Vec<Diagnostic> {
    let mut found = Vec::new();
    for imp in &tree.impls {
        let (Some(origin), Some(tr), Some(a)) = (imp.origin, &imp.trait_ref, imp.self_ty.adt())
        else {
            continue;
        };
        let def = &tree.adts[a as usize];
        let name = tr.name.as_ref();
        if matches!(name, "Copy" | "Eq") || (name == "Clone" && def.copy) {
            continue;
        }
        let mut solver = Solver::new(tree, &imp.generics.preds);
        let bad = (0..def.variants.len() as u32)
            .flat_map(|v| tree.field_types(&imp.self_ty, v))
            .find(|t| !matches!(t, Ty::Param(..)) && solver.implements(t, tr) == Answer::No);
        if let Some(t) = bad {
            found.push(
                Diagnostic::error(
                    Code::E0402,
                    origin,
                    format!(
                        "`{}` cannot derive `{tr}`: its field of type `{t}` does not implement it",
                        def.name
                    ),
                )
                .in_file(def.src.file().unwrap_or(0)),
            );
        }
    }
    found
}

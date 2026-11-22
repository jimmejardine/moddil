//! Effects in signatures (`effects.md`): what a `with` clause lists must be an effect, listed
//! once; a trait's method and its impls' list only `fail`; and a `dyn` type in a signature must
//! be of a trait that can be used as one.

use moddil_syntax::TextRange;

use crate::items::{Container, ItemTree};
use crate::{Code, Diagnostic, TraitRef, Ty};

use super::Found;
use crate::check::show_effect;

/// Reports the effects of each function that are not effects (E0601), are on a trait's method
/// or an impl of one (E0604), or are listed twice (E0606), and makes them errors in the
/// signature so later passes need not care (a name they bind stays bound); and reports `dyn`
/// types of traits that cannot be used as `dyn` (E0605).
pub(crate) fn signature_effects(tree: &mut ItemTree) -> Vec<Found> {
    let mut found = Vec::new();
    for i in 0..tree.fns.len() {
        let f = &tree.fns[i];
        let in_trait = match f.container {
            Container::Trait(_) => true,
            Container::Impl(k) => tree.impls[k as usize].trait_ref.is_some(),
            Container::Free => false,
        };
        let mut bad = Vec::new();
        for (k, e) in f.effects.iter().enumerate() {
            let d = if let Some(why) = tree.not_an_effect(&e.ty) {
                Diagnostic::error(Code::E0601, e.range, why)
            } else if e.ty == Ty::Error {
                bad.push(k);
                continue;
            } else if in_trait {
                Diagnostic::error(
                    Code::E0604,
                    e.range,
                    "a trait's method lists only `fail`: a handler captures what else it needs \
                     when it is built",
                )
            } else if let Some(first) = f.effects[..k].iter().find(|x| x.ty == e.ty) {
                Diagnostic::error(
                    Code::E0606,
                    e.range,
                    format!("`{}` is listed twice", show_effect(&e.ty)),
                )
                .with_label(first.range, "first listed here")
            } else {
                continue;
            };
            found.push((f.src, f.origin, d));
            bad.push(k);
        }
        let mut types: Vec<(Ty, TextRange)> =
            f.params.iter().map(|p| (p.ty.clone(), p.range)).collect();
        if let Some(r) = f.ret_range {
            types.push((f.ret.clone(), r));
        }
        for (t, at) in types {
            if let Some(d) = dyn_problem(tree, &t, at) {
                found.push((f.src, f.origin, d));
            }
        }
        for k in bad {
            tree.fns[i].effects[k].ty = Ty::Error;
        }
    }
    found
}

/// E0605 for a `dyn` type in `t` whose trait cannot be used as `dyn`.
fn dyn_problem(tree: &ItemTree, t: &Ty, at: TextRange) -> Option<Diagnostic> {
    let mut out = None;
    find_dyn(t, &mut |tr| {
        if out.is_none()
            && let Some(why) = tree.not_dyn(tr.trait_, false)
        {
            out = Some(Diagnostic::error(
                Code::E0605,
                at,
                format!("`{}` cannot be used as `dyn`: {why}", tr.name),
            ));
        }
    });
    out
}

/// Calls `f` with the trait of each `dyn` type in `t`.
fn find_dyn(t: &Ty, f: &mut impl FnMut(&TraitRef)) {
    if let Ty::Dyn(tr, _) = t {
        f(tr);
    }
    let _ = t.map(&mut |x| {
        find_dyn(x, f);
        x.clone()
    });
}

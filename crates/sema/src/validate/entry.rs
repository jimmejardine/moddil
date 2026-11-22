//! Entry points: an error that reaches one has nowhere to go, so `main` cannot declare errors
//! (R3, `err.entry.1`). A test may fail, and a destructor's trait method declares none.

use moddil_syntax::TextRange;

use crate::items::ItemTree;
use crate::{Code, Diagnostic, Ty};

use super::Found;

/// `main` declaring `with fail(…)` (E0507), or taking what is not a root capability, or one
/// twice (E0608, `main.root.1`).
pub(crate) fn entry_points(tree: &ItemTree) -> Vec<Found> {
    let mut out = Vec::new();
    for (i, f) in tree.fns.iter().enumerate() {
        if !tree.is_main(i as u32) {
            continue;
        }
        if let Some(at) = f.fails_range {
            out.push((
                f.src,
                None,
                Diagnostic::error(
                    Code::E0507,
                    at,
                    "`main` is an entry point, so it cannot fail: handle its errors in it",
                ),
            ));
        }
        let mut seen: Vec<(u32, TextRange)> = Vec::new();
        for p in &f.params {
            let root = match &p.ty {
                Ty::Adt(a, ..) => tree.adts[*a as usize].root.then_some(*a),
                Ty::Error => continue,
                _ => None,
            };
            match root {
                None => out.push((
                    f.src,
                    None,
                    Diagnostic::error(
                        Code::E0608,
                        p.range,
                        format!(
                            "`main` takes only root capabilities (such as `Args` and `Stdio`), \
                             not `{}`",
                            p.ty
                        ),
                    ),
                )),
                Some(a) => {
                    if let Some(&(_, first)) = seen.iter().find(|(b, _)| *b == a) {
                        out.push((
                            f.src,
                            None,
                            Diagnostic::error(
                                Code::E0608,
                                p.range,
                                format!("`main` takes `{}` once", p.ty),
                            )
                            .with_label(first, "first taken here"),
                        ));
                    } else {
                        seen.push((a, p.range));
                    }
                }
            }
        }
    }
    out
}

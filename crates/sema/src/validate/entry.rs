//! Entry points: an error that reaches one has nowhere to go, so `main` cannot declare errors
//! (R3, `err.entry.1`). A test may fail, and a destructor's trait method declares none.

use crate::items::{Container, ItemTree, Src};
use crate::{Code, Diagnostic};

use super::Found;

/// `main` declaring `with fail(…)` (E0507).
pub(crate) fn entry_points(tree: &ItemTree) -> Vec<Found> {
    tree.fns
        .iter()
        .filter(|f| f.src == Src::File && f.container == Container::Free && f.name == "main")
        .filter_map(|f| {
            let at = f.fails_range?;
            Some((
                f.src,
                None,
                Diagnostic::error(
                    Code::E0507,
                    at,
                    "`main` is an entry point, so it cannot fail: handle its errors in it",
                ),
            ))
        })
        .collect()
}

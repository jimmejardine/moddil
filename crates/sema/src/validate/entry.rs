//! Entry points: an error that reaches one has nowhere to go, so `main` cannot declare errors
//! (R3, `err.entry.1`). A test may fail, and a destructor's trait method declares none.

use crate::items::ItemTree;
use crate::{Code, Diagnostic};

use super::Found;

/// `main` declaring `with fail(…)` (E0507).
pub(crate) fn entry_points(tree: &ItemTree) -> Vec<Found> {
    tree.fns
        .iter()
        .enumerate()
        .filter(|&(i, _)| tree.is_main(i as u32))
        .filter_map(|(_, f)| {
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

//! Compile-time code (`comptime.md`: Evaluation model): a const item's initialiser, a `const fn`'s
//! body and a `const { }` block call only `const fn`s (or the prelude's functions), and have no
//! effect and no error.

use crate::hir::ExprKind;
use crate::{Callee, Code, Db, Diagnostic, Item, Program, Ty};

/// The diagnostics of the compile-time code of `program`'s files (E1001, E1002).
pub(crate) fn diagnostics(db: &dyn Db, program: Program) -> Vec<Diagnostic> {
    let items = crate::item_tree(db, program);
    let fx = crate::effects(db, program);
    let mut out = Vec::new();
    for owner in crate::owners(db, program) {
        let item = owner.item(db);
        let whole = match item {
            Item::Fn(i) => {
                let sig = &items.fns[i as usize];
                if !sig.src.is_file() {
                    continue;
                }
                sig.is_const
            }
            _ => false,
        };
        let lowered = crate::body(db, owner);
        let body = &lowered.body;
        // The ranges of the `const { }` blocks in the body.
        let blocks: Vec<_> = body
            .exprs
            .iter()
            .filter(|(_, x)| matches!(x.kind, ExprKind::ConstBlock(_)))
            .map(|(_, x)| x.range)
            .collect();
        if !whole && blocks.is_empty() {
            continue;
        }
        let types = crate::infer(db, owner);
        let mut ds = Vec::new();
        for (e, x) in body.exprs.iter() {
            let at = x.range;
            if !whole
                && !blocks
                    .iter()
                    .any(|b| b.start <= at.start && at.end <= b.end)
            {
                continue;
            }
            let Some(Callee::Fn { func, .. }) = types.callee(e) else {
                continue;
            };
            let f = &items.fns[*func as usize];
            if f.src.is_file() && !f.is_const {
                ds.push(Diagnostic::error(
                    Code::E1001,
                    at,
                    format!(
                        "`{}` is not a `const fn`, so compile-time code cannot call it",
                        f.name
                    ),
                ));
            }
        }
        if whole
            && let Item::Fn(i) = item
            && let Some(effects) = fx.fns.get(i as usize)
            && (!effects.needs.is_empty() || effects.fails != Ty::Never)
        {
            let sig = &items.fns[i as usize];
            ds.push(Diagnostic::error(
                Code::E1002,
                sig.name_range,
                format!(
                    "`{}` runs when compiling, so it can have no effect and no error",
                    sig.name
                ),
            ));
        }
        out.extend(crate::in_file(items, item, &ds));
    }
    out
}

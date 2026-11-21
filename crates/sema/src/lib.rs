//! Semantic analysis: names and types (later effects and error sets). Produces the typed HIR.
//!
//! Every analysis is a Salsa query over [`SourceFile`] inputs, so results are memoised and
//! recomputed only when what they read changes:
//!
//! - [`parse`]: the syntax tree;
//! - [`item_tree`]: the program's items with their signatures: the prelude's, the file's, and the
//!   impls the derives expand to;
//! - [`body`]: one function's or test's body in HIR;
//! - [`infer`]: the types of a body's expressions, patterns and locals, and what its calls and
//!   operators run;
//! - [`diagnostics`]: everything wrong with a file.
//!
//! Slices S0 to S4 cover `fn`, `test`, `let`, `int`, `bool`, `()`, arithmetic, comparisons,
//! logic, `if`, blocks, calls and `return`; tuples, structs, enums, patterns, `match` (with
//! exhaustiveness checking), `if let`, `let … else`, loops and `assert`; borrow types, `*`,
//! binding modes, `Copy` and `defer`; generics, traits, impls, methods, operators through traits,
//! derives, destructors and the prelude; errors, `fail`, `?`, `try` and `catch`. Moves and
//! borrows are checked on MIR (`moddil-mir`).
//! Valid Moddil beyond that is reported as "not supported yet" (`I0001`). See this crate's
//! `README.md`.

mod check;
mod collect;
mod db;
mod derive;
mod diag;
mod exhaust;
pub mod hir;
mod items;
mod lower;
mod resolve;
mod traits;
mod ty;
mod validate;

pub use check::{Callee, Inference, Propagated, infer, tuple_positions};
pub use collect::{item_tree, parse};
pub use db::{Database, Db, Item, Owner, SourceFile};
pub use diag::{CODES, Code, Diagnostic, Severity};
pub use items::{
    AdtDef, Container, FieldDef, FnSig, GenericParam, Generics, ImplDef, ItemTree, ParamSig, Pred,
    SelfKind, Shape, Src, TestSig, TraitDef, VariantDef,
};
pub use lower::{Lowered, body};
pub use traits::{Adjust, Answer, Pick, Solver, Source, Table, elaborate};
pub use ty::{AssocTy, TraitRef, Ty};

use moddil_syntax::TextRange;

/// The prelude's source (`reference/2027/prelude.moddil`).
pub const PRELUDE: &str = collect::PRELUDE;

/// Every diagnostic for `file`, in source order. If the file has syntax errors, only those are
/// reported, since analysing a broken tree mostly produces noise. Problems in derived code are
/// reported at the `derive` attribute; the prelude has none.
#[salsa::tracked(returns(ref))]
pub fn diagnostics(db: &dyn Db, file: SourceFile) -> Vec<Diagnostic> {
    let parsed = parse(db, file);
    let mut out: Vec<Diagnostic> = parsed
        .errors()
        .iter()
        .map(|e| Diagnostic::error(Code::E0001, e.range, e.message.clone()))
        .collect();
    if out.is_empty() {
        let items = item_tree(db, file);
        out.extend(items.diagnostics.iter().cloned());
        for owner in owners(db, file) {
            let ds = body(db, owner)
                .diagnostics
                .iter()
                .chain(&infer(db, owner).diagnostics);
            out.extend(in_file(items, owner.item(db), ds));
        }
    }
    out.sort_by_key(|d| (d.range.start, d.range.end, d.code));
    out.dedup();
    out
}

/// The diagnostics `ds` of `item`'s body, as the file shows them: as they are for the file's
/// items, at the `derive` attribute for derived code, and not at all for the prelude.
pub fn in_file<'a>(
    items: &ItemTree,
    item: Item,
    ds: impl IntoIterator<Item = &'a Diagnostic>,
) -> Vec<Diagnostic> {
    let (src, origin, adt) = match item {
        Item::Fn(i) => {
            let sig = &items.fns[i as usize];
            let adt = match sig.container {
                Container::Impl(k) => items.impls[k as usize].self_ty.adt(),
                _ => None,
            };
            (sig.src, sig.origin, adt)
        }
        Item::Test(_) => (Src::File, None, None),
    };
    // A derive already reported as wrong, or for a type that is, would only report noise.
    let reported = |at: TextRange| {
        let name = adt.map(|a| items.adts[a as usize].name_range);
        items
            .diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error && (d.range == at || Some(d.range) == name))
    };
    match (src, origin) {
        (Src::File, _) => ds.into_iter().cloned().collect(),
        (Src::Derived, Some(at)) if reported(at) => Vec::new(),
        (Src::Derived, Some(at)) => ds
            .into_iter()
            .map(|d| Diagnostic {
                range: at,
                labels: Vec::new(),
                ..d.clone()
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Whether `file` has any errors.
pub fn has_errors(db: &dyn Db, file: SourceFile) -> bool {
    diagnostics(db, file)
        .iter()
        .any(|d| d.severity == Severity::Error)
}

/// Every function with a body and every test of the program (the prelude's, the file's and the
/// derived ones), functions first.
pub fn owners(db: &dyn Db, file: SourceFile) -> Vec<Owner<'_>> {
    let items = item_tree(db, file);
    let fns = (0..items.fns.len() as u32)
        .filter(|&i| items.fns[i as usize].has_body)
        .map(Item::Fn);
    let tests = (0..items.tests.len() as u32).map(Item::Test);
    fns.chain(tests)
        .map(|item| Owner::new(db, file, item))
        .collect()
}

/// This crate's version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests;

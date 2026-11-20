//! Semantic analysis: names and types (later effects and error sets). Produces the typed HIR.
//!
//! Every analysis is a Salsa query over [`SourceFile`] inputs, so results are memoised and
//! recomputed only when what they read changes:
//!
//! - [`parse`]: the syntax tree;
//! - [`item_tree`]: the file's functions, tests, structs and enums, with their signatures;
//! - [`body`]: one function's or test's body in HIR;
//! - [`infer`]: the types of a body's expressions, patterns and locals;
//! - [`diagnostics`]: everything wrong with a file.
//!
//! Slices S0 to S2 cover `fn`, `test`, `let`, `int`, `bool`, `()`, arithmetic, comparisons,
//! logic, `if`, blocks, calls and `return`; tuples, structs, enums, patterns, `match` (with
//! exhaustiveness checking), `if let`, `let … else`, loops and `assert`; borrow types, `*`,
//! binding modes, `Copy` and `defer`. Moves and borrows are checked on MIR (`moddil-mir`). Valid
//! Moddil beyond that is reported as "not supported yet" (`I0001`). See this crate's `README.md`.

mod check;
mod db;
mod diag;
mod exhaust;
pub mod hir;
mod items;
mod lower;
mod ty;

pub use check::{Inference, infer, tuple_positions};
pub use db::{Database, Db, Item, Owner, SourceFile};
pub use diag::{CODES, Code, Diagnostic, Severity};
pub use items::{
    AdtDef, FieldDef, FnSig, ItemTree, ParamSig, Shape, TestSig, VariantDef, item_tree, parse,
};
pub use lower::{Lowered, body};
pub use ty::Ty;

/// Every diagnostic for `file`, in source order. If the file has syntax errors, only those are
/// reported, since analysing a broken tree mostly produces noise.
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
            out.extend(body(db, owner).diagnostics.iter().cloned());
            out.extend(infer(db, owner).diagnostics.iter().cloned());
        }
    }
    out.sort_by_key(|d| (d.range.start, d.range.end, d.code));
    out.dedup();
    out
}

/// Whether `file` has any errors.
pub fn has_errors(db: &dyn Db, file: SourceFile) -> bool {
    diagnostics(db, file)
        .iter()
        .any(|d| d.severity == Severity::Error)
}

/// Every function and test in `file`, functions first.
pub fn owners(db: &dyn Db, file: SourceFile) -> Vec<Owner<'_>> {
    let items = item_tree(db, file);
    let fns = (0..items.fns.len() as u32).map(Item::Fn);
    let tests = (0..items.tests.len() as u32).map(Item::Test);
    fns.chain(tests)
        .map(|item| Owner::new(db, file, item))
        .collect()
}

/// This crate's version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests;

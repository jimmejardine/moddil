//! Semantic analysis: names, types, effects and error sets. Produces the typed HIR.
//!
//! Every analysis is a Salsa query over a [`Program`] input (its files and mods), so results are
//! memoised and recomputed only when what they read changes:
//!
//! - [`parse`]: the syntax tree;
//! - [`item_tree`]: the program's items with their signatures: the prelude's, the file's, and the
//!   impls the derives expand to;
//! - [`body`]: one function's or test's body in HIR;
//! - [`infer`]: the types of a body's expressions, patterns and locals, and what its calls and
//!   operators run;
//! - [`effects`]: every function's effects and errors, inferred for private functions;
//! - [`diagnostics`]: everything wrong with a file.
//!
//! Slices S0 to S7a cover `fn`, `test`, `let`, `int`, `bool`, `()`, arithmetic, comparisons,
//! logic, `if`, blocks, calls and `return`; tuples, structs, enums, patterns, `match` (with
//! exhaustiveness checking), `if let`, `let … else`, loops and `assert`; borrow types, `*`,
//! binding modes, `Copy` and `defer`; generics, traits, impls, methods, operators through traits,
//! derives, destructors and the prelude; errors, `fail`, `?`, `try` and `catch`; effects, `with`
//! and `dyn` borrows; modules, mods, `use` and visibility; closures and anonymous impls. Moves and
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
mod modules;
mod resolve;
mod traits;
mod ty;
mod validate;
mod wrap;

pub use check::{
    ArgSlot, Callee, Capture, CaptureMode, Effects, Evidence, FnEffects, Inference, Propagated,
    effects, infer, tuple_positions,
};
pub use collect::{item_tree, parse};
pub use db::{Database, Db, FileId, Item, ModInfo, Owner, Program, ProgramFile, SourceFile};
pub use diag::{CODES, Code, Diagnostic, Severity};
pub use items::{
    AdtDef, Container, Control, Def, EffectSig, Entry, FieldDef, FnSig, GenericParam, Generics,
    ImplDef, Import, ItemTree, ModDef, ModuleDef, PRELUDE_MOD, PRELUDE_MODULE, ParamSig, Pred,
    SPAWN, SUSPEND, SelfKind, Shape, Src, TestSig, TraitDef, VariantDef, Vis,
};
pub use lower::{Lowered, body};
pub use modules::{Member, PathError};
pub use traits::{Adjust, Answer, Lit, Pick, Solver, Source, Table, elaborate};
pub use ty::{AnonTy, AssocTy, ClosureKind, ClosureTy, Num, OpaqueTy, TraitRef, Ty};

use moddil_syntax::TextRange;

/// The prelude's source (`reference/2027/prelude.moddil`).
pub const PRELUDE: &str = collect::PRELUDE;

/// Every diagnostic for `program`, by file and then in source order. If a file has syntax errors,
/// only those are reported, since analysing a broken tree mostly produces noise. Problems in
/// derived code are reported at the `derive` attribute; the prelude has none.
#[salsa::tracked(returns(ref))]
pub fn diagnostics(db: &dyn Db, program: Program) -> Vec<Diagnostic> {
    let mut out: Vec<Diagnostic> = Vec::new();
    for (f, file) in program.files(db).iter().enumerate() {
        out.extend(
            parse(db, file.source).errors().iter().map(|e| {
                Diagnostic::error(Code::E0001, e.range, e.message.clone()).in_file(f as u32)
            }),
        );
    }
    if out.is_empty() {
        let items = item_tree(db, program);
        out.extend(items.diagnostics.iter().cloned());
        out.extend(effects(db, program).diagnostics.iter().cloned());
        out.extend(check::comptime::diagnostics(db, program));
        for owner in owners(db, program) {
            let ds = body(db, owner)
                .diagnostics
                .iter()
                .chain(&infer(db, owner).diagnostics);
            out.extend(in_file(items, owner.item(db), ds));
        }
    }
    out.sort_by_key(|d| (d.file, d.range.start, d.range.end, d.code));
    out.dedup();
    out
}

/// The diagnostics `ds` of `item`'s body, as the program's files show them: as they are for the
/// files' items, at the `derive` attribute for derived code, and not at all for the prelude.
pub fn in_file<'a>(
    items: &ItemTree,
    item: Item,
    ds: impl IntoIterator<Item = &'a Diagnostic>,
) -> Vec<Diagnostic> {
    let (src, origin, adt) = match item.owner() {
        Item::Closure(..) => unreachable!("an owner is a function or test"),
        Item::Fn(i) => {
            let sig = &items.fns[i as usize];
            let adt = match sig.container {
                Container::Impl(k) => items.impls[k as usize].self_ty.adt(),
                _ => None,
            };
            (sig.src, sig.origin, adt)
        }
        Item::Test(t) => (Src::File(items.tests[t as usize].file), None, None),
    };
    // Derived code is shown in the file of its type.
    let derived_file = adt.and_then(|a| items.adts[a as usize].src.file());
    // A derive already reported as wrong, or for a type that is, would only report noise.
    let reported = |at: TextRange| {
        let name = adt.map(|a| items.adts[a as usize].name_range);
        items.diagnostics.iter().any(|d| {
            d.severity == Severity::Error
                && Some(d.file) == derived_file
                && (d.range == at || Some(d.range) == name)
        })
    };
    match (src, origin) {
        (Src::File(f), _) => ds.into_iter().map(|d| d.clone().in_file(f)).collect(),
        (Src::Derived, Some(at)) if reported(at) => Vec::new(),
        (Src::Derived, Some(at)) => ds
            .into_iter()
            .map(|d| Diagnostic {
                range: at,
                labels: Vec::new(),
                file: derived_file.unwrap_or(0),
                ..d.clone()
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Whether attribute node `attr` (an `OUTER_ATTR` or `INNER_ATTR`) is a doc comment, `///` or
/// `//!` (`item.doc.1`): documentation, which the compiler does not read yet.
pub(crate) fn is_doc(attr: &moddil_syntax::SyntaxNode) -> bool {
    attr.child_tokens().any(|t| {
        matches!(
            t.kind(),
            moddil_syntax::SyntaxKind::DOC_OUTER | moddil_syntax::SyntaxKind::DOC_INNER
        )
    })
}

/// `t` with each opaque result type (`type.impl.ret.1`) replaced by the type its function's
/// body gives, for the MIR, which works with the types themselves.
pub fn reveal(db: &dyn Db, program: Program, t: &Ty) -> Ty {
    match t {
        Ty::Opaque(o) => {
            let owner = Owner::new(db, program, Item::Fn(o.func));
            let hidden = infer(db, owner).hidden().cloned().unwrap_or(Ty::Error);
            let args: Vec<Ty> = o.args.iter().map(|a| reveal(db, program, a)).collect();
            reveal(db, program, &hidden.subst(&args))
        }
        t => t.map(&mut |x| reveal(db, program, x)),
    }
}

/// Whether `program` has any errors.
pub fn has_errors(db: &dyn Db, program: Program) -> bool {
    diagnostics(db, program)
        .iter()
        .any(|d| d.severity == Severity::Error)
}

/// Every function with a body and every test of the program (the prelude's, the files' and the
/// derived ones), functions first.
pub fn owners(db: &dyn Db, program: Program) -> Vec<Owner<'_>> {
    let items = item_tree(db, program);
    let fns = (0..items.fns.len() as u32)
        .filter(|&i| items.fns[i as usize].has_body)
        .map(Item::Fn);
    let tests = (0..items.tests.len() as u32).map(Item::Test);
    fns.chain(tests)
        .map(|item| Owner::new(db, program, item))
        .collect()
}

/// This crate's version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests;

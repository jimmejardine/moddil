//! Compiling a program's entry points into one WASM module: the native lowering, built into a
//! [`WasmModule`], each function translated from its Cranelift IR.

use moddil_native::backend::{Entry, Site};
use moddil_native::layout::Layouts;
use moddil_sema::{Db, Item, ItemTree, Program, Ty};

use crate::module::{Layout, WasmModule};

/// A program's entry points compiled into one module.
pub struct Compiled<'db> {
    /// The module's bytes.
    pub bytes: Vec<u8>,
    /// Where its scratch is, and how much memory it starts with.
    pub layout: Layout,
    /// Each entry: its trampoline (exported as `moddil_entry`, or `moddil_entry_k` for the
    /// `k`th past the first), its result's drop (`moddil_drop_result`, likewise), its result type
    /// and its errors.
    pub entries: Vec<Entry>,
    /// Every site, by number.
    pub sites: Vec<Site>,
    /// The program's items.
    pub items: &'db ItemTree,
    /// The types `dyn` borrows point to, by the number their vtables start with.
    pub dyn_types: Vec<Ty>,
}

/// Compiles `entries` (tests, or `main`) of `program` into one module.
///
/// # Errors
///
/// What the program reaches that is not compiled to WASM yet.
pub fn compile<'db>(
    db: &'db dyn Db,
    program: Program,
    entries: &[Item],
) -> Result<Compiled<'db>, String> {
    let lowered = moddil_native::backend::lower(db, program, entries, WasmModule::new()?)?;
    // Room for any entry's result and any error, after its id.
    let mut layouts = Layouts::new(lowered.items);
    let mut out = 0;
    let mut err = 0;
    for e in &lowered.entries {
        out = out.max(layouts.of(&e.ret)?.size);
        for t in &e.errors {
            err = err.max(layouts.of(t)?.size);
        }
    }
    let (bytes, layout) = lowered
        .module
        .finish(out, moddil_native::backend::ERROR_VALUE + err)?;
    Ok(Compiled {
        bytes,
        layout,
        entries: lowered.entries,
        sites: lowered.sites,
        items: lowered.items,
        dyn_types: lowered.dyn_types,
    })
}

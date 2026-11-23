//! The native lowering, for other backends: MIR to Cranelift IR, built into any
//! [`cranelift_module::Module`]. `moddil-wasm` lowers through it and translates each finished
//! function to WebAssembly, so both backends run the same code.

use cranelift_codegen::isa::OwnedTargetIsa;
use cranelift_module::Module;
use moddil_sema::{Db, Item, ItemTree, Program, Ty};

use crate::Native;
pub use crate::lower::{ERROR_VALUE, Entry, Site};

/// What [`lower`] gives: the module, holding the code, and what running it needs.
pub struct Lowered<'db, M> {
    /// The module.
    pub module: M,
    /// Each entry point, in the order given.
    pub entries: Vec<Entry>,
    /// Every site, by number.
    pub sites: Vec<Site>,
    /// The program's items.
    pub items: &'db ItemTree,
    /// The types `dyn` borrows point to, by the number their vtables start with.
    pub dyn_types: Vec<Ty>,
    /// The program's files: each one's path and text.
    pub files: Vec<(String, String)>,
}

/// Lowers what `items` (each a test, or a function whose parameters are root capabilities)
/// reach into `module`. The first entry's trampoline is exported as `moddil_entry` and its drop
/// as `moddil_drop_result`; the `k`th other one's with `_k` after them.
///
/// # Errors
///
/// What the program reaches that is not lowered yet.
pub fn lower<'db, M: Module>(
    db: &'db dyn Db,
    program: Program,
    items: &[Item],
    module: M,
) -> Result<Lowered<'db, M>, String> {
    let b = crate::lower::build(db, program, items, module)?;
    Ok(Lowered {
        module: b.c.module,
        entries: b.entries,
        sites: b.c.sites,
        items: b.c.items,
        dyn_types: b.c.dyn_types,
        files: crate::lower::files_of(db, program),
    })
}

/// The host's target, for a module the code is not compiled for but whose IR is wanted: its
/// pointers are 64 bits, as the lowering's layouts are.
///
/// # Errors
///
/// If Cranelift does not support the host.
pub fn isa() -> Result<OwnedTargetIsa, String> {
    crate::lower::isa(false)
}

/// A trap as the interpreters report it ([`Native::Trap`]): where it happened, or, in tracked
/// code, at the innermost call untracked code made.
pub fn reported(sites: &[Site], trapped: &moddil_rt::Trapped) -> Native {
    crate::jit::reported(sites, trapped)
}

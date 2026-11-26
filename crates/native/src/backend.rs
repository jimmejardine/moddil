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
/// as `moddil_drop_result`; the `k`th other one's with `_k` after them. The site tables a trap's
/// text is read from without the compiler are data exported as `moddil_site_count`,
/// `moddil_sites` and `moddil_strings`, as `moddil build`'s objects have them.
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
    lower_with(db, program, items, module, false)
}

/// As [`lower`], for a built program's `main` (function `main`): with `moddil_exit_code` too,
/// `fn(out: *const u8) -> i32`, the process's exit code for `main`'s result (`main.exit.1`).
///
/// # Errors
///
/// As [`lower`], and if `main` gives something other than `()` or `ExitCode`.
pub fn lower_main<M: Module>(
    db: &dyn Db,
    program: Program,
    main: u32,
    module: M,
) -> Result<Lowered<'_, M>, String> {
    lower_with(db, program, &[Item::Fn(main)], module, true)
}

fn lower_with<'db, M: Module>(
    db: &'db dyn Db,
    program: Program,
    items: &[Item],
    module: M,
    exit: bool,
) -> Result<Lowered<'db, M>, String> {
    // The modules this lowers for run one thread (WASM): atomics are plain reads and writes.
    let mut b = crate::lower::build(
        db,
        program,
        items,
        module,
        false,
        crate::lower::Threads::One,
    )?;
    let files = crate::lower::files_of(db, program);
    b.c.site_tables(&files)?;
    if exit {
        b.c.exit_code(&b.entries[0].ret)?;
    }
    Ok(Lowered {
        module: b.c.module,
        entries: b.entries,
        sites: b.c.sites,
        items: b.c.items,
        dyn_types: b.c.dyn_types,
        files,
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

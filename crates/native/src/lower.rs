//! MIR to Cranelift IR: slices C0 and C1. Scalars, their operators and checks, control flow,
//! direct calls of non-generic functions (C0); tuples, structs, enums and arrays on the stack,
//! their places, and borrows of them (C1).
//!
//! **Values.** A scalar local nobody borrows is a Cranelift variable; an aggregate, and a local
//! something borrows, lives in a stack slot laid out as [`crate::layout`] says. A place is then
//! an address: its local's slot, moved by each field's offset, each element's stride and each
//! borrow it goes through.
//!
//! **The ABI.** Internal functions use Cranelift's `tail` convention, which returns several values
//! in registers. Each returns a status (`i8`: 0 for a return, 1 for a trap) and then its result.
//! A value crosses a call as nothing (`()`), as a scalar, as up to two words (an aggregate of up
//! to 16 bytes whose size splits into at most two 8-, 4-, 2- or 1-byte pieces), or indirectly: a
//! pointer to a copy the caller makes for an argument, and a pointer to where the caller wants it
//! for a result. A failed check calls `moddil-rt`'s `trap` with its site and returns status 1;
//! callers pass 1 on, so the host sees the trap without unwinding. Each call is wrapped in
//! `moddil-rt`'s `enter` and `leave`, which count the frames.
//!
//! **Sites.** Each check that can fail and each `// =>` observation is numbered; the number is
//! what the runtime is given, and [`Compiled::sites`] says what each one is.

#[path = "frozen.rs"]
mod frozen;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::{self, AbiParam, InstBuilder, Signature, types};
use cranelift_codegen::isa::CallConv;
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{DataDescription, DataId, FuncId, Linkage, Module};
use moddil_mir::{
    AggregateKind, BinOp, Body, Const, Local, LocalDecl, Operand, Place, Projection, Rvalue,
    Statement, Terminator, TrapClass, UnOp,
};
use moddil_sema::{
    Callee, ClosureTy, Db, Item, ItemTree, Num, Owner, Program, Solver, Src, TraitRef, Ty,
};
use moddil_syntax::TextRange;

use moddil_ffi::CType;
use moddil_ffi::abi::{CLayout, Part, Scalar as CScalar};

use crate::layout::{Layout, Layouts, Shape, Tag};

/// What a site is.
#[derive(Clone, Debug)]
pub enum Site {
    /// A check: its class, the file and where.
    Trap(TrapClass, u32, TextRange),
    /// A call: the file, where, and whether the code that makes it is tracked
    /// (`#[track_caller]`, the prelude's or derived code). Too deep a call traps here, and a
    /// trap in tracked code is reported at the innermost call untracked code makes.
    Call(u32, TextRange, bool),
    /// A `// =>` observation: the file, the expression, and its type.
    Observe(u32, TextRange, Ty),
    /// A `fail`: the file and where (the error of a run is reported at the last one).
    Fail(u32, TextRange),
    /// A part of a failed `assert`'s condition: where it is, its type if it was evaluated, and
    /// whether it is the first part.
    Part(TextRange, Option<Ty>, bool),
}

/// A program compiled for one entry point, ready to run.
pub(crate) struct Compiled<'db> {
    pub(crate) module: JITModule,
    /// The entry trampoline: `fn(out: *mut u8) -> i8`, which writes the result at `out`.
    pub(crate) entry: FuncId,
    /// `fn(out: *mut u8) -> i8`: drops the result at `out`, once the host has shown it.
    pub(crate) drop_result: FuncId,
    /// The entry's result type.
    pub(crate) ret: Ty,
    pub(crate) sites: Vec<Site>,
    pub(crate) items: &'db ItemTree,
    /// The types `dyn` borrows point to, by the number their vtables start with.
    pub(crate) dyn_types: Vec<Ty>,
    /// The types of the errors the entry can fail with.
    pub(crate) errors: Vec<Ty>,
    /// The program's files: each one's path and text.
    pub(crate) files: Vec<(String, String)>,
}

/// The Cranelift type of a scalar `ty`: `None` for `()`, an error for a type that is not a
/// scalar.
pub(crate) fn clif_type(ty: &Ty) -> Result<Option<ir::Type>, String> {
    Ok(Some(match ty {
        Ty::Unit | Ty::Never => return Ok(None),
        // A borrow is a pointer, as wide as an `int` on the 64-bit targets C1 runs on.
        Ty::Int | Ty::Ref(..) | Ty::Ptr(..) | Ty::FnPtr(..) => types::I64,
        Ty::Bool => types::I8,
        Ty::Char => types::I32,
        Ty::Num(n) => match n {
            Num::I8 | Num::U8 => types::I8,
            Num::I16 | Num::U16 => types::I16,
            Num::I32 | Num::U32 => types::I32,
            // As wide as a pointer: 64 bits on every target of this edition (`num.types.2`).
            Num::U64 | Num::Usize | Num::Isize => types::I64,
            Num::I128 | Num::U128 => types::I128,
            Num::F32 => types::F32,
            Num::F64 => types::F64,
        },
        t => return Err(format!("a value of type `{t}`")),
    }))
}

/// A stack slot of `size` bytes (at least one), aligned to `align` (at least 8): its address.
fn stack_slot(b: &mut FunctionBuilder<'_>, size: u32, align: u32) -> ir::Value {
    let slot = b.create_sized_stack_slot(ir::StackSlotData::new(
        ir::StackSlotKind::ExplicitSlot,
        size.max(1),
        align.max(8).trailing_zeros() as u8,
    ));
    b.ins().stack_addr(types::I64, slot, 0)
}

/// A slot of `size` bytes, 8-byte aligned: its address.
fn scratch(b: &mut FunctionBuilder<'_>, size: u32) -> ir::Value {
    stack_slot(b, size, 8)
}

/// Copies `size` bytes from `src` to `dest`, reading none past them.
fn copy_bytes(b: &mut FunctionBuilder<'_>, dest: ir::Value, src: ir::Value, size: u32) {
    let flags = ir::MemFlagsData::trusted();
    let mut at = 0u32;
    for (n, t) in [
        (8, types::I64),
        (4, types::I32),
        (2, types::I16),
        (1, types::I8),
    ] {
        while size - at >= n {
            let v = b.ins().load(t, flags, src, at as i32);
            b.ins().store(flags, v, dest, at as i32);
            at += n;
        }
    }
}

/// The values a Moddil call passes, as `pass` says, for the value at `at`.
fn load_pass(b: &mut FunctionBuilder<'_>, pass: &Pass, at: ir::Value) -> Vec<ir::Value> {
    let flags = ir::MemFlagsData::trusted();
    match pass {
        Pass::Nothing => Vec::new(),
        Pass::Scalar(t) => vec![b.ins().load(*t, flags, at, 0)],
        Pass::Words(ts) => {
            let mut off = 0;
            ts.iter()
                .map(|t| {
                    let v = b.ins().load(*t, flags, at, off);
                    off += t.bytes() as i32;
                    v
                })
                .collect()
        }
        Pass::Indirect => vec![at],
    }
}

/// Stores the values a Moddil call gave, as `pass` says, at `at`. (A result given through a
/// pointer is there already.)
fn store_pass(b: &mut FunctionBuilder<'_>, pass: &Pass, vals: &[ir::Value], at: ir::Value) {
    let flags = ir::MemFlagsData::trusted();
    match pass {
        Pass::Scalar(_) => {
            b.ins().store(flags, vals[0], at, 0);
        }
        Pass::Words(ts) => {
            let mut off = 0;
            for (t, v) in ts.iter().zip(vals) {
                b.ins().store(flags, *v, at, off);
                off += t.bytes() as i32;
            }
        }
        Pass::Nothing | Pass::Indirect => {}
    }
}

/// Whether `ty` is a signed integer type.
fn signed(ty: &Ty) -> bool {
    match ty {
        Ty::Int => true,
        Ty::Num(n) => n.is_signed() && !n.is_float(),
        _ => false,
    }
}

fn is_float(ty: &Ty) -> bool {
    matches!(ty, Ty::Num(Num::F32 | Num::F64))
}

/// How wide an integer type is, in bits.
fn bits(ty: &Ty) -> u32 {
    match ty {
        Ty::Bool => 8,
        Ty::Char => 32,
        Ty::Num(n) => n.bits(),
        // `int`.
        _ => 64,
    }
}

/// How a value crosses a call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Pass {
    /// It has no bytes.
    Nothing,
    /// A scalar.
    Scalar(ir::Type),
    /// Its bytes, as these pieces (each at the offset the ones before it end at).
    Words(Vec<ir::Type>),
    /// A pointer to it.
    Indirect,
}

/// How a value of layout `l` crosses a call.
pub(crate) fn pass_of(l: &Layout) -> Pass {
    match &l.shape {
        Shape::Empty => Pass::Nothing,
        Shape::Scalar(t) => Pass::Scalar(*t),
        _ if l.size == 0 => Pass::Nothing,
        _ if l.size <= 16 => {
            let mut pieces = Vec::new();
            let mut left = l.size;
            for (n, t) in [
                (8, types::I64),
                (4, types::I32),
                (2, types::I16),
                (1, types::I8),
            ] {
                while left >= n {
                    pieces.push(t);
                    left -= n;
                }
            }
            if pieces.len() <= 2 {
                Pass::Words(pieces)
            } else {
                Pass::Indirect
            }
        }
        _ => Pass::Indirect,
    }
}

/// Where an error's value is in the error channel and in an error set: after its id, a word.
pub const ERROR_VALUE: u32 = 8;

/// How many words a vtable starts with before its methods.
pub(crate) const VTABLE_HEADER: usize = 4;

/// The runtime functions the code calls.
struct Rt {
    trap: FuncId,
    trap_tracked: FuncId,
    observe: FuncId,
    enter: FuncId,
    leave: FuncId,
    fmod: FuncId,
    fmodf: FuncId,
    /// An `i128` division, remainder or overflow check, which Cranelift cannot compile for
    /// every target: the low and high words of its result (`moddil_rt::num::wide`).
    i128_lo: FuncId,
    i128_hi: FuncId,
    float_fn: FuncId,
    float_fn_f32: FuncId,
    char_at_byte: FuncId,
    is_between: FuncId,
    str_cmp: FuncId,
    char_is: FuncId,
    alloc: FuncId,
    free: FuncId,
    realloc: FuncId,
    memmove: FuncId,
    fail_at: FuncId,
    static_begin: FuncId,
    static_end: FuncId,
    convert: FuncId,
    text: FuncId,
    log: FuncId,
    trapped: FuncId,
    task: HashMap<&'static str, FuncId>,
    /// The running task's number, which code that may run on several threads reads with one
    /// load (`moddil_rt::RUNNING`); a one-thread build calls `task_id` instead.
    running: Option<DataId>,
    /// In code that may run on several threads, the word set when the running task is to give
    /// way (`moddil_rt::PREEMPT`), and the call that does.
    preempt: Option<(DataId, FuncId)>,
}

/// The runtime's task functions: each one's name, parameters and result.
const TASK_FNS: &[(&str, &[char], Option<char>)] = &[
    ("scope_open", &['b', 'w'], Some('w')),
    ("spawn", &['w', 'p', 'w'], Some('w')),
    ("executor_of_task", &[], Some('w')),
    (
        "executor_new",
        &['w', 'w', 'w', 'w', 'w', 'b', 'w'],
        Some('w'),
    ),
    ("scope_run", &['w', 'p', 'p', 'p', 's', 'b'], Some('b')),
    ("stop", &['w'], None),
    ("join", &['w', 'p', 'p', 's', 'b'], Some('b')),
    ("detach", &['w'], Some('b')),
    ("yield_now", &['s', 'b'], Some('b')),
    ("task_id", &[], Some('w')),
    ("install_runtime", &['w', 'w'], None),
    ("parallel", &[], Some('b')),
    ("hash_seed", &[], Some('w')),
    ("alloc_push", &['w', 'w'], None),
    ("alloc_pop", &[], None),
    ("alloc_top", &['p'], None),
    ("park", &['p', 'w', 's', 'b'], Some('b')),
    ("unpark", &['p', 'w'], Some('w')),
    ("stop_requested", &['s', 'b'], Some('b')),
    ("channel", &['w'], Some('w')),
    ("timer", &['w'], Some('w')),
    ("os_arg_count", &[], Some('w')),
    ("os_arg_len", &['w'], Some('w')),
    ("os_arg_copy", &['w', 'p', 'w'], None),
    ("os_task_tree_len", &[], Some('w')),
    ("os_task_tree_copy", &['p', 'w'], None),
    ("os_write_stderr", &['p', 'w'], None),
    ("os_random", &[], Some('w')),
    ("os_wall_clock", &[], Some('w')),
    ("os_listen", &['w'], Some('w')),
    ("os_connect", &['p', 'w', 'w'], Some('w')),
    ("os_channel", &['w', 'w'], Some('w')),
    ("os_take", &['w', 'p', 'w'], Some('w')),
    ("os_send", &['w', 'p', 'w'], Some('b')),
    ("os_close", &['w'], None),
    (
        "gpu_dispatch",
        &['p', 'w', 'p', 'w', 'w', 'w', 'w', 'w', 'p', 'w', 'w'],
        Some('w'),
    ),
    ("gpu_frame", &['p'], None),
    ("gpu_show", &['p', 'w', 'w', 'w'], None),
    ("clone_sender", &['w'], None),
    ("drop_sender", &['w'], None),
    ("drop_receiver", &['w', 'p'], Some('b')),
    ("send", &['w', 'p', 'w', 'p', 's', 'b'], Some('b')),
    ("recv", &['w', 'p', 's', 'b'], Some('b')),
    ("select", &['w', 'p', 's', 'b'], Some('w')),
    ("recv_now", &['w', 'p'], Some('b')),
    ("send_now", &['w', 'p', 'w', 'p', 's', 'b'], Some('b')),
    ("join_now", &['w', 'p', 'p'], Some('b')),
];

/// A function instance: a function (or a closure's body) and its (concrete) type arguments.
type Instance = (Item, Vec<Ty>);

pub(crate) struct Compiler<'db, M: Module> {
    db: &'db dyn Db,
    program: Program,
    pub(crate) items: &'db ItemTree,
    layouts: Layouts<'db>,
    pub(crate) module: M,
    rt: Rt,
    /// Each instance declared: its id, and its parameters' and result's types.
    fns: HashMap<Instance, (FuncId, Vec<Ty>, Ty)>,
    /// The instances declared and not yet compiled.
    queue: Vec<Instance>,
    pub(crate) sites: Vec<Site>,
    /// Each text constant's data.
    strs: HashMap<String, DataId>,
    /// Each vtable: for a type and a trait (with its arguments).
    vtables: HashMap<(Ty, TraitRef), DataId>,
    pub(crate) dyn_types: Vec<Ty>,
    /// Each type's drop glue: `fn(*mut T) -> i8`.
    glues: HashMap<Ty, FuncId>,
    /// The drop glue declared and not yet compiled.
    glue_queue: Vec<Ty>,
    /// Each `const` item's slot: a flag (whether it is computed yet), then its value; or, for
    /// one computed when compiling, its value.
    statics: HashMap<u32, DataId>,
    /// Each `const { }` block's value computed when compiling, by its item.
    blocks: HashMap<Item, DataId>,
    /// Each task closure's trampoline, by its type.
    tramps: HashMap<Ty, FuncId>,
    /// Each type's drop glue in the platform's C convention, for the runtime.
    c_drops: HashMap<Ty, FuncId>,
    /// Each C function called (`ffi.extern.1`), imported by its symbol.
    foreigns: HashMap<u32, FuncId>,
    /// Each Moddil function given C as a function pointer (`type.fnptr.1`): its C-ABI wrapper.
    callbacks: HashMap<u32, FuncId>,
    /// The places code comes from: a file and an offset in it, numbered by the source location
    /// (less one) the lowering gives that code.
    places: Vec<(u32, u32)>,
    /// Each place's number.
    place_ids: HashMap<(u32, u32), u32>,
    /// For a release build, each compiled function's lines (`None` when not kept).
    pub(crate) lines: Option<Vec<crate::dwarf::FnLines>>,
    /// Whether the code may run on several threads at once, and so lowers atomics as atomic
    /// instructions.
    threads: Threads,
    /// Whether the program installs an allocator (`alloc.with.1`): then a `Buf`'s storage is
    /// made, grown and freed through the prelude's `__heap_*`, after a header naming its
    /// allocator; if not, through the runtime directly, as before.
    uses_alloc: bool,
}

/// How many threads the compiled code may run on at once (`runtime.md`: Starting a program): a
/// one-thread build, as WASM is, reads and writes atomics plainly (`core.atomic.1`), checks for
/// preemption nowhere, and asks for the running task's number with a call.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Threads {
    /// One thread: under the deterministic runtime only.
    One,
    /// Several, as the parallel runtime runs tasks on.
    #[default]
    Many,
}

/// The host's target, with the flags the code needs: `pic` for an object to link.
pub(crate) fn isa(pic: bool) -> Result<cranelift_codegen::isa::OwnedTargetIsa, String> {
    let mut flags = settings::builder();
    flags.set("opt_level", "speed").map_err(|e| e.to_string())?;
    // `i128` and `u128` are passed and returned as two registers.
    flags
        .set("enable_llvm_abi_extensions", "true")
        .map_err(|e| e.to_string())?;
    if pic {
        flags.set("is_pic", "true").map_err(|e| e.to_string())?;
    }
    cranelift_native::builder()
        .map_err(|e| e.to_string())?
        .finish(settings::Flags::new(flags))
        .map_err(|e| e.to_string())
}

/// Compiles what `item` (a test, or a function without parameters) reaches, to run in process.
pub(crate) fn compile<'db>(
    db: &'db dyn Db,
    program: Program,
    item: Item,
    threads: Threads,
) -> Result<Compiled<'db>, String> {
    let mut builder = JITBuilder::with_isa(isa(false)?, cranelift_module::default_libcall_names());
    // The runtime's functions, by the names the code calls them by.
    for (name, at) in moddil_rt::symbols() {
        builder.symbol(name, at);
    }
    // C functions (`ffi.extern.1`): in the C libraries loaded for the run, then the C runtime.
    builder.symbol_lookup_fn(Box::new(moddil_ffi::lookup));
    let mut b = build(
        db,
        program,
        &[item],
        JITModule::new(builder),
        false,
        threads,
    )?;
    let mut module = b.c.module;
    module.finalize_definitions().map_err(|e| e.to_string())?;
    let e = b.entries.remove(0);
    Ok(Compiled {
        module,
        entry: e.entry,
        drop_result: e.drop_result,
        ret: e.ret,
        sites: b.c.sites,
        items: b.c.items,
        dyn_types: b.c.dyn_types,
        errors: e.errors,
        files: files_of(db, program),
    })
}

/// Compiles `main` (function `item`) into an object file to link with `moddil-rt-main`. It
/// exports `moddil_entry` and `moddil_drop_result` (as the JIT's), `moddil_exit_code` (the
/// process's exit code for `main`'s result), and the tables a trap is reported from:
/// `moddil_site_count`, `moddil_sites` (eight words a site: its kind, whether it is tracked, its
/// class's and its file's text in `moddil_strings`, and its line and column). With `lines`, its
/// functions' line tables too ([`crate::dwarf`]).
pub(crate) fn compile_object(
    db: &dyn Db,
    program: Program,
    item: Item,
    lines: bool,
    threads: Threads,
) -> Result<Vec<u8>, String> {
    let builder = cranelift_object::ObjectBuilder::new(
        isa(true)?,
        "moddil",
        cranelift_module::default_libcall_names(),
    )
    .map_err(|e| e.to_string())?;
    let b = build(
        db,
        program,
        &[item],
        cranelift_object::ObjectModule::new(builder),
        lines,
        threads,
    )?;
    let mut c = b.c;
    let files = files_of(db, program);
    c.exit_code(&b.entries[0].ret)?;
    c.site_tables(&files)?;
    let fns = c.lines.take().unwrap_or_default();
    let places = std::mem::take(&mut c.places);
    let mut product = c.module.finish();
    let symbols = product.functions.clone();
    crate::dwarf::write(
        &mut product.object,
        &fns,
        |f| symbols[f].expect("a function compiled").0,
        &places,
        &files,
    )?;
    product.emit().map_err(|e| e.to_string())
}

/// Compiles the `#[export]` functions `exports` (`ffi.export.1`) into an object: each one's code,
/// and a wrapper under its own name in the platform's C convention, which starts the runtime
/// (once), calls it, and ends the process if it traps. With the tables traps are reported from.
pub(crate) fn compile_library(
    db: &dyn Db,
    program: Program,
    exports: &[u32],
) -> Result<Vec<u8>, String> {
    let builder = cranelift_object::ObjectBuilder::new(
        isa(true)?,
        "moddil",
        cranelift_module::default_libcall_names(),
    )
    .map_err(|e| e.to_string())?;
    let b = build(
        db,
        program,
        &[],
        cranelift_object::ObjectModule::new(builder),
        false,
        Threads::Many,
    )?;
    let mut c = b.c;
    let mut targets = Vec::new();
    for &f in exports {
        let (id, params, ret) = c.declare_fn(f, &[])?;
        targets.push((f, id, params, ret));
    }
    loop {
        if let Some(inst) = c.queue.pop() {
            let (id, params, ret) = c.fns[&inst].clone();
            c.body_of(inst.0, &inst.1, id, &params, &ret)?;
        } else if let Some(ty) = c.glue_queue.pop() {
            c.glue_body(&ty)?;
        } else {
            break;
        }
    }
    for (f, id, params, ret) in targets {
        c.export_wrapper(f, id, &params, &ret)?;
    }
    let files = files_of(db, program);
    c.site_tables(&files)?;
    c.module.finish().emit().map_err(|e| e.to_string())
}

/// `ty` as C writes it, in a header.
fn c_type_name(items: &ItemTree, ty: &Ty) -> String {
    match ty {
        Ty::Unit => "void".to_string(),
        Ty::Bool => "bool".to_string(),
        Ty::Int => "int64_t".to_string(),
        Ty::Num(n) => match n {
            Num::I8 => "int8_t",
            Num::U8 => "uint8_t",
            Num::I16 => "int16_t",
            Num::U16 => "uint16_t",
            Num::I32 => "int32_t",
            Num::U32 => "uint32_t",
            Num::U64 => "uint64_t",
            Num::Usize => "size_t",
            Num::Isize => "ptrdiff_t",
            Num::F32 => "float",
            Num::F64 => "double",
            Num::I128 | Num::U128 => "__int128",
        }
        .to_string(),
        Ty::Ptr(m, inner) => {
            let t = c_type_name(items, inner);
            if *m {
                format!("{t} *")
            } else {
                format!("const {t} *")
            }
        }
        Ty::Adt(a, ..) => items.adts[*a as usize].name.clone(),
        // A function pointer (`type.fnptr.1`): `int32_t (*)(const void *, const void *)`; a
        // declaration puts its name after the `*`.
        Ty::FnPtr(ps, r) => {
            let ps: Vec<String> = ps.iter().map(|p| c_type_name(items, p)).collect();
            let ps = if ps.is_empty() {
                "void".to_string()
            } else {
                ps.join(", ")
            };
            format!("{} (*)({ps})", c_type_name(items, r))
        }
        t => format!("/* {t} */ void"),
    }
}

/// The structs `ty` names, behind its pointers: each once, in `out`.
fn structs_in(ty: &Ty, out: &mut Vec<u32>) {
    match ty {
        Ty::Ptr(_, inner) => structs_in(inner, out),
        Ty::FnPtr(ps, r) => {
            for p in ps.iter() {
                structs_in(p, out);
            }
            structs_in(r, out);
        }
        Ty::Adt(a, ..) if !out.contains(a) => out.push(*a),
        _ => {}
    }
}

/// As [`crate::header`].
pub(crate) fn header(db: &dyn Db, program: Program, exports: &[u32], name: &str) -> String {
    let items = moddil_sema::item_tree(db, program);
    let guard: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    // A type and a name: `int64_t n`, `const Point *p`.
    let decl = |t: &Ty, n: &str| {
        let t = c_type_name(items, t);
        if t.contains("(*)") {
            t.replacen("(*)", &format!("(*{n})"), 1)
        } else if t.ends_with('*') {
            format!("{t}{n}")
        } else {
            format!("{t} {n}")
        }
    };
    let mut out = format!(
        "/* {name}.h: the C ABI of the Moddil mod `{name}`, written by `moddil build --lib`. */\n\
         #ifndef {guard}_H\n#define {guard}_H\n\n#include <stdbool.h>\n#include <stddef.h>\n#include <stdint.h>\n\n"
    );
    let mut structs = Vec::new();
    for &f in exports {
        let sig = &items.fns[f as usize];
        for p in &sig.params {
            structs_in(&p.ty, &mut structs);
        }
        structs_in(&sig.ret, &mut structs);
    }
    for a in &structs {
        let adt = &items.adts[*a as usize];
        if adt.repr_c {
            out.push_str(&format!("typedef struct {} {{\n", adt.name));
            for f in &adt.variants[0].fields {
                out.push_str(&format!("    {};\n", decl(&f.ty, &f.name)));
            }
            out.push_str(&format!("}} {};\n\n", adt.name));
        } else {
            out.push_str(&format!("typedef struct {0} {0};\n\n", adt.name));
        }
    }
    for &f in exports {
        let sig = &items.fns[f as usize];
        let params: Vec<String> = sig
            .params
            .iter()
            .map(|p| decl(&p.ty, p.name.as_deref().unwrap_or("_")))
            .collect();
        let params = if params.is_empty() {
            "void".to_string()
        } else {
            params.join(", ")
        };
        out.push_str(&format!("{}({params});\n", decl(&sig.ret, &sig.name)));
    }
    out.push_str(
        "\n/* 1 if the last call into this library on this thread trapped (it then gave C its \
         result\n * type's zero, or its `on_trap` value), else 0. */\nint32_t moddil_trapped(void);\n",
    );
    out.push_str(&format!("\n#endif /* {guard}_H */\n"));
    out
}

/// The program's files: each one's path and text.
pub(crate) fn files_of(db: &dyn Db, program: Program) -> Vec<(String, String)> {
    program
        .files(db)
        .iter()
        .map(|f| (f.source.path(db).to_string(), f.source.text(db).to_string()))
        .collect()
}

/// An entry point compiled: its trampoline, `fn(out: *mut u8, err: *mut u8) -> i8` in the
/// platform's C convention, which calls it and stores its result's bytes at `out` or its error
/// at `err`; `fn(out: *mut u8) -> i8`, which drops the result at `out` once the host has shown it;
/// its result type; and the types of the errors it can fail with.
pub struct Entry {
    /// The trampoline.
    pub entry: FuncId,
    /// What drops the result.
    pub drop_result: FuncId,
    /// The result type.
    pub ret: Ty,
    /// The errors it can fail with.
    pub errors: Vec<Ty>,
}

/// What [`build`] gives: the compiler, with its module holding the code, and the entries.
pub(crate) struct Built<'db, M: Module> {
    pub(crate) c: Compiler<'db, M>,
    pub(crate) entries: Vec<Entry>,
}

/// The name an entry's function `base` is exported as: as it is for the first entry, and with
/// the entry's number after it for the others.
fn entry_name(base: &str, k: usize) -> String {
    if k == 0 {
        base.to_string()
    } else {
        format!("{base}_{k}")
    }
}

/// Compiles what `entries` reach into `module`, each an entry point (a test, or a function
/// whose parameters are root capabilities).
pub(crate) fn build<'db, M: Module>(
    db: &'db dyn Db,
    program: Program,
    entries: &[Item],
    mut module: M,
    lines: bool,
    threads: Threads,
) -> Result<Built<'db, M>, String> {
    let ptr = module.target_config().pointer_type();
    let mut import = |name: &str, params: &[ir::Type], ret: Option<ir::Type>| {
        let mut sig = module.make_signature();
        for &p in params {
            sig.params.push(AbiParam::new(p));
        }
        if let Some(r) = ret {
            sig.returns.push(AbiParam::new(r));
        }
        module
            .declare_function(name, Linkage::Import, &sig)
            .map_err(|e| e.to_string())
    };
    let mut rt = Rt {
        trap: import("moddil_rt_trap", &[types::I32], Some(types::I8))?,
        trap_tracked: import("moddil_rt_trap_tracked", &[types::I32], Some(types::I8))?,
        float_fn: import(
            "moddil_rt_float_fn",
            &[types::I32, types::F64, types::F64],
            Some(types::F64),
        )?,
        float_fn_f32: import(
            "moddil_rt_float_fn_f32",
            &[types::I32, types::F32, types::F32],
            Some(types::F32),
        )?,
        char_at_byte: import(
            "moddil_rt_char_at_byte",
            &[ptr, types::I64, types::I64],
            Some(types::I32),
        )?,
        is_between: import(
            "moddil_rt_is_between",
            &[ptr, types::I64, types::I64, types::I64],
            Some(types::I8),
        )?,
        str_cmp: import(
            "moddil_rt_str_cmp",
            &[ptr, types::I64, ptr, types::I64],
            Some(types::I8),
        )?,
        char_is: import(
            "moddil_rt_char_is",
            &[types::I32, types::I32],
            Some(types::I8),
        )?,
        alloc: import("moddil_rt_alloc", &[types::I64, types::I64], Some(ptr))?,
        free: import("moddil_rt_free", &[ptr, types::I64, types::I64], None)?,
        realloc: import(
            "moddil_rt_realloc",
            &[ptr, types::I64, types::I64, types::I64],
            Some(ptr),
        )?,
        memmove: import("moddil_rt_memmove", &[ptr, ptr, types::I64], None)?,
        fail_at: import("moddil_rt_fail_at", &[types::I32], None)?,
        static_begin: import("moddil_rt_static_begin", &[ptr], Some(types::I8))?,
        static_end: import("moddil_rt_static_end", &[ptr], None)?,
        convert: import(
            "moddil_rt_convert",
            &[
                types::I32,
                types::I32,
                types::I64,
                types::I64,
                types::I32,
                ptr,
            ],
            Some(types::I8),
        )?,
        text: import(
            "moddil_rt_text",
            &[types::I32, types::I64, types::I64, ptr, types::I64, ptr],
            Some(ptr),
        )?,
        log: import("moddil_rt_log", &[types::I32, ptr, types::I64], None)?,
        trapped: import("moddil_rt_trapped", &[types::I32, ptr], Some(ptr))?,
        task: {
            let mut m = HashMap::new();
            for (name, params, ret) in TASK_FNS {
                let ty = |c: char| match c {
                    'b' => types::I8,
                    's' => types::I32,
                    'p' => ptr,
                    _ => types::I64,
                };
                let ps: Vec<ir::Type> = params.iter().map(|c| ty(*c)).collect();
                m.insert(
                    *name,
                    import(&format!("moddil_rt_{name}"), &ps, ret.map(ty))?,
                );
            }
            m
        },
        observe: import("moddil_rt_observe", &[types::I32, ptr], None)?,
        enter: import("moddil_rt_enter", &[types::I32, types::I8], Some(types::I8))?,
        leave: import("moddil_rt_leave", &[], None)?,
        fmod: import(
            "moddil_rt_fmod",
            &[types::F64, types::F64],
            Some(types::F64),
        )?,
        fmodf: import(
            "moddil_rt_fmodf",
            &[types::F32, types::F32],
            Some(types::F32),
        )?,
        i128_lo: import(
            "moddil_rt_i128_lo",
            &[types::I32, types::I64, types::I64, types::I64, types::I64],
            Some(types::I64),
        )?,
        i128_hi: import(
            "moddil_rt_i128_hi",
            &[types::I32, types::I64, types::I64, types::I64, types::I64],
            Some(types::I64),
        )?,
        running: None,
        preempt: None,
    };
    if threads == Threads::Many {
        let data = |module: &mut M, name: &str| {
            module
                .declare_data(name, Linkage::Import, false, false)
                .map_err(|e| e.to_string())
        };
        rt.running = Some(data(&mut module, "moddil_rt_task")?);
        let word = data(&mut module, "moddil_rt_preempt")?;
        let sig = module.make_signature();
        let give_way = module
            .declare_function("moddil_rt_safe_point", Linkage::Import, &sig)
            .map_err(|e| e.to_string())?;
        rt.preempt = Some((word, give_way));
    }
    let items = moddil_sema::item_tree(db, program);
    let mut c = Compiler {
        db,
        program,
        items,
        layouts: Layouts::new(items),
        module,
        rt,
        fns: HashMap::new(),
        queue: Vec::new(),
        sites: Vec::new(),
        strs: HashMap::new(),
        vtables: HashMap::new(),
        dyn_types: Vec::new(),
        glues: HashMap::new(),
        glue_queue: Vec::new(),
        statics: HashMap::new(),
        blocks: HashMap::new(),
        tramps: HashMap::new(),
        c_drops: HashMap::new(),
        foreigns: HashMap::new(),
        callbacks: HashMap::new(),
        places: Vec::new(),
        place_ids: HashMap::new(),
        lines: lines.then(Vec::new),
        threads,
        uses_alloc: *moddil_sema::installs_alloc(db, program),
    };
    let mut pending = Vec::new();
    for (k, &item) in entries.iter().enumerate() {
        let (target, params, ret) = match item {
            Item::Test(_) => {
                let id = c.declare_item("test", &[], &Ty::Unit, false)?;
                c.body_of(item, &[], id, &[], &Ty::Unit)?;
                (id, Vec::new(), Ty::Unit)
            }
            Item::Fn(f) => {
                let (id, params, ret) = c.declare_fn(f, &[])?;
                // `main`'s root capabilities (`main.root.1`): each a struct of a stub mod, made with
                // its handle zero, as the interpreters make it.
                let roots = params
                    .iter()
                    .all(|p| matches!(p, Ty::Adt(a, ..) if c.items.adts[*a as usize].root));
                if !roots {
                    return Err("an entry point with parameters".to_string());
                }
                (id, params, ret)
            }
            Item::Closure(..) => return Err("a closure as an entry point".to_string()),
        };
        let drop_result = c.drop_result(&ret, k)?;
        let errors = c.errors_of(item)?;
        pending.push((target, params, ret, drop_result, errors));
    }
    loop {
        if let Some(inst) = c.queue.pop() {
            let (id, params, ret) = c.fns[&inst].clone();
            c.body_of(inst.0, &inst.1, id, &params, &ret)?;
        } else if let Some(ty) = c.glue_queue.pop() {
            c.glue_body(&ty)?;
        } else {
            break;
        }
    }
    let mut entries = Vec::new();
    for (k, (target, params, ret, drop_result, errors)) in pending.into_iter().enumerate() {
        let entry = c.trampoline(target, &params, &ret, k)?;
        entries.push(Entry {
            entry,
            drop_result,
            ret,
            errors,
        });
    }
    Ok(Built { c, entries })
}

impl<M: Module> Compiler<'_, M> {
    fn pass(&mut self, ty: &Ty) -> Result<Pass, String> {
        let l = self.layouts.of(ty)?;
        Ok(pass_of(&l))
    }

    /// The signature of a function with these parameters and result. A closure's body takes its
    /// captures (its first parameter) by pointer, whatever their size: so a call through a
    /// borrow works on the closure in place, and a `dyn` borrow's address is its captures'.
    fn signature(&mut self, params: &[Ty], ret: &Ty, closure: bool) -> Result<Signature, String> {
        let ptr = self.module.target_config().pointer_type();
        let mut sig = self.module.make_signature();
        sig.call_conv = CallConv::Tail;
        for (i, p) in params.iter().enumerate() {
            if closure && i == 0 {
                sig.params.push(AbiParam::new(ptr));
                continue;
            }
            match self.pass(p)? {
                Pass::Nothing => {}
                Pass::Scalar(t) => sig.params.push(AbiParam::new(t)),
                Pass::Words(ts) => sig.params.extend(ts.into_iter().map(AbiParam::new)),
                Pass::Indirect => sig.params.push(AbiParam::new(ptr)),
            }
        }
        sig.returns.push(AbiParam::new(types::I8));
        match self.pass(ret)? {
            Pass::Nothing => {}
            Pass::Scalar(t) => sig.returns.push(AbiParam::new(t)),
            Pass::Words(ts) => sig.returns.extend(ts.into_iter().map(AbiParam::new)),
            // Where the caller wants the result.
            Pass::Indirect => sig.params.push(AbiParam::new(ptr)),
        }
        // Where an error goes, last (null for a call that cannot fail).
        sig.params.push(AbiParam::new(ptr));
        Ok(sig)
    }

    /// The types of the errors `item`'s body raises with `fail`.
    fn errors_of(&self, item: Item) -> Result<Vec<Ty>, String> {
        let body = self.mir(item)?;
        let mut out = Vec::new();
        for bb in &body.blocks {
            if let Terminator::Fail(Operand::Move(p) | Operand::Copy(p)) = &bb.terminator
                && p.projection.is_empty()
            {
                match self.concrete(&body.locals[p.local.index()].ty, &[]) {
                    Ty::Errors(ms) => out.extend(ms.iter().cloned()),
                    t => out.push(t),
                }
            }
        }
        Ok(out)
    }

    fn declare_item(
        &mut self,
        name: &str,
        params: &[Ty],
        ret: &Ty,
        closure: bool,
    ) -> Result<FuncId, String> {
        let sig = self.signature(params, ret, closure)?;
        self.module
            .declare_anonymous_function(&sig)
            .map_err(|e| format!("{name}: {e}"))
    }

    /// `ty` with the type arguments `targs` for its parameters, its associated types resolved,
    /// and each refined type as its base (a refinement is checked, not represented).
    fn concrete(&self, ty: &Ty, targs: &[Ty]) -> Ty {
        Solver::new(self.items, &[]).deep(&ty.subst(targs))
    }

    /// Function `f` with type arguments `targs`: declared (and queued to be compiled) the first
    /// time it is named.
    fn declare_fn(&mut self, f: u32, targs: &[Ty]) -> Result<(FuncId, Vec<Ty>, Ty), String> {
        let sig = &self.items.fns[f as usize];
        if sig.intrinsic || !sig.has_body {
            return Err(format!("the built-in function `{}`", sig.name));
        }
        self.declare_inst(Item::Fn(f), targs)
    }

    /// C function `f` (`ffi.extern.1`), imported by its symbol with the platform's C signature
    /// (8- and 16-bit integers as 32, structs as [`moddil_ffi::abi`] says), the first time it is
    /// called.
    fn foreign_fn(&mut self, f: u32) -> Result<FuncId, String> {
        if let Some(&id) = self.foreigns.get(&f) {
            return Ok(id);
        }
        let sig = &self.items.fns[f as usize];
        let (name, params, ret) = (
            sig.name.clone(),
            sig.params.iter().map(|p| p.ty.clone()).collect::<Vec<_>>(),
            sig.ret.clone(),
        );
        let s = self.c_sig(&params, &ret)?;
        let id = self
            .module
            .declare_function(&name, Linkage::Import, &s)
            .map_err(|e| format!("Cranelift: {e:?}"))?;
        self.foreigns.insert(f, id);
        Ok(id)
    }

    /// `ty` as C takes or gives it (`ffi.types.1`, `ffi.types.2`).
    fn c_type(&mut self, ty: &Ty) -> Result<CType, String> {
        if matches!(ty, Ty::Adt(..)) {
            return Ok(CType::Struct(Arc::new(self.c_layout(ty)?)));
        }
        Ok(match clif_type(ty)? {
            None => CType::Void,
            Some(t) if t == types::I8 => CType::I8,
            Some(t) if t == types::I16 => CType::I16,
            Some(t) if t == types::I32 => CType::I32,
            Some(t) if t == types::F32 => CType::F32,
            Some(t) if t == types::F64 => CType::F64,
            Some(_) => CType::I64,
        })
    }

    /// The C layout of `#[repr(C)]` struct type `ty`: its scalars by offset, nested structs
    /// flattened.
    fn c_layout(&mut self, ty: &Ty) -> Result<CLayout, String> {
        let l = self.layouts.of(ty)?;
        CLayout::flatten(l.size, l.align, ty, |t| {
            Ok(match &self.layouts.of(t)?.shape {
                Shape::Fields(fs) => Part::Fields(fs.clone()),
                Shape::Scalar(c) if *c == types::F32 => Part::Scalar(CScalar::F32),
                Shape::Scalar(c) if *c == types::F64 => Part::Scalar(CScalar::F64),
                Shape::Scalar(c) => Part::Scalar(CScalar::Int(c.bytes() as u8)),
                _ => return Err(format!("a `{ty}` in a struct given to C by value")),
            })
        })
    }

    /// Builds function `id`, of signature `sig`, with `build`, and defines it in the module.
    fn define_fn<T>(
        &mut self,
        id: FuncId,
        sig: Signature,
        build: impl FnOnce(&mut Self, &mut FunctionBuilder<'_>) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut ctx = self.module.make_context();
        ctx.func.signature = sig;
        let mut fctx = FunctionBuilderContext::new();
        let mut b = FunctionBuilder::new(&mut ctx.func, &mut fctx);
        let out = build(self, &mut b)?;
        b.seal_all_blocks();
        b.finalize(self.module.target_config());
        self.module
            .define_function(id, &mut ctx)
            .map_err(|e| format!("Cranelift: {e:?}"))?;
        Ok(out)
    }

    /// The C signature of a function taking `params` and giving `ret`.
    fn c_sig(&mut self, params: &[Ty], ret: &Ty) -> Result<Signature, String> {
        let cparams = params
            .iter()
            .map(|p| self.c_type(p))
            .collect::<Result<Vec<_>, _>>()?;
        if cparams.contains(&CType::Void) {
            return Err("a C function's parameter of type `()`".to_string());
        }
        let cret = self.c_type(ret)?;
        let cc = self.module.isa().default_call_conv();
        Ok(moddil_ffi::c_signature(cc, &cparams, &cret))
    }

    /// The C-ABI wrapper of Moddil function `f`, through which C calls it when it is given C as
    /// a function pointer (`type.fnptr.1`): made the first time it is.
    fn callback_fn(&mut self, f: u32) -> Result<FuncId, String> {
        if let Some(&id) = self.callbacks.get(&f) {
            return Ok(id);
        }
        let (target, params, ret) = self.declare_fn(f, &[])?;
        let name = format!("moddil_callback_{f}");
        let id = self.c_wrapper(target, &params, &ret, &name, Linkage::Local, OnTrap::End)?;
        self.callbacks.insert(f, id);
        Ok(id)
    }

    /// The body of closure type `c`, declared (and queued) the first time it is called.
    fn declare_closure(&mut self, c: &ClosureTy) -> Result<(FuncId, Vec<Ty>, Ty), String> {
        self.declare_inst(Item::closure(c.owner, c.expr), &c.args)
    }

    fn declare_inst(&mut self, item: Item, targs: &[Ty]) -> Result<(FuncId, Vec<Ty>, Ty), String> {
        let key = (item, targs.to_vec());
        if let Some(d) = self.fns.get(&key) {
            return Ok(d.clone());
        }
        let closure = matches!(item, Item::Closure(..));
        let body = self.mir(item)?;
        let name = body.name.clone();
        let params: Vec<Ty> = body.locals[1..=body.arg_count]
            .iter()
            .map(|l| self.concrete(&l.ty, targs))
            .collect();
        let ret = self.concrete(&body.locals[0].ty, targs);
        let id = self.declare_item(&name, &params, &ret, closure)?;
        self.fns
            .insert(key.clone(), (id, params.clone(), ret.clone()));
        self.queue.push(key);
        Ok((id, params, ret))
    }

    /// The data holding text `t`'s bytes.
    fn text_data(&mut self, t: &str) -> Result<DataId, String> {
        if let Some(d) = self.strs.get(t) {
            return Ok(*d);
        }
        let id = self
            .module
            .declare_anonymous_data(false, false)
            .map_err(|e| e.to_string())?;
        let mut desc = DataDescription::new();
        // At least a byte, so that the text has an address of its own.
        let mut bytes = t.as_bytes().to_vec();
        bytes.push(0);
        desc.define(bytes.into_boxed_slice());
        self.module
            .define_data(id, &desc)
            .map_err(|e| e.to_string())?;
        self.strs.insert(t.to_string(), id);
        Ok(id)
    }

    /// The methods of `tr` and its supertraits, in vtable order, each with the trait (with
    /// `Self` as `self_ty`) it is declared in: each supertrait's first, depth first, then the
    /// trait's own. So the vtable of a trait starts with the methods of its first supertrait;
    /// its supertraits' own vtables follow the methods ([`Compiler::dyn_supertraits`]).
    fn dyn_methods(&self, self_ty: &Ty, tr: &TraitRef) -> Vec<(u32, TraitRef)> {
        let mut out = Vec::new();
        self.methods_into(self_ty, tr, &mut out, &mut HashSet::new());
        out
    }

    /// The supertraits of `tr`, every level, in the order [`Compiler::dyn_methods`] meets them
    /// (with `Self` as `self_ty`): after a vtable's methods, a word for each is the address of
    /// the type's vtable for that supertrait, which a `dyn` borrow of `tr` becomes when it is
    /// used as one of that supertrait (`type.dyn.coerce.1`).
    fn dyn_supertraits(&self, self_ty: &Ty, tr: &TraitRef) -> Vec<TraitRef> {
        fn walk(
            items: &ItemTree,
            self_ty: &Ty,
            tr: &TraitRef,
            out: &mut Vec<TraitRef>,
            seen: &mut HashSet<u32>,
        ) {
            if !seen.insert(tr.trait_) {
                return;
            }
            let mut args = vec![self_ty.clone()];
            args.extend(tr.args.iter().cloned());
            for sup in &items.traits[tr.trait_ as usize].supertraits {
                let sup = sup.subst(&args);
                walk(items, self_ty, &sup, out, seen);
                if !out.contains(&sup) {
                    out.push(sup);
                }
            }
        }
        let mut out = Vec::new();
        walk(self.items, self_ty, tr, &mut out, &mut HashSet::new());
        out
    }

    fn methods_into(
        &self,
        self_ty: &Ty,
        tr: &TraitRef,
        out: &mut Vec<(u32, TraitRef)>,
        seen: &mut HashSet<u32>,
    ) {
        if !seen.insert(tr.trait_) {
            return;
        }
        let def = &self.items.traits[tr.trait_ as usize];
        let mut args = vec![self_ty.clone()];
        args.extend(tr.args.iter().cloned());
        for sup in &def.supertraits {
            self.methods_into(self_ty, &sup.subst(&args), out, seen);
        }
        for &m in &def.methods {
            out.push((m, tr.clone()));
        }
    }

    /// The vtable of `ty` for `tr`: a header of [`VTABLE_HEADER`] words (its number in
    /// [`Compiled::dyn_types`], its drop glue, its size and its alignment), then each method's
    /// address (null for one a `dyn` borrow cannot call), then the address of `ty`'s vtable for
    /// each supertrait ([`Compiler::dyn_supertraits`]). A closure's methods (`call`,
    /// `call_mut`, `call_once`) are all its body.
    fn vtable(&mut self, ty: &Ty, tr: &TraitRef) -> Result<DataId, String> {
        let key = (ty.clone(), tr.clone());
        if let Some(d) = self.vtables.get(&key) {
            return Ok(*d);
        }
        let methods = self.dyn_methods(ty, tr);
        let supers = self.dyn_supertraits(ty, tr);
        let id = self
            .module
            .declare_anonymous_data(false, false)
            .map_err(|e| e.to_string())?;
        self.vtables.insert(key, id);
        let mut desc = DataDescription::new();
        let n = self.dyn_types.len() as u64;
        self.dyn_types.push(ty.clone());
        let l = self.layouts.of(ty)?;
        let mut bytes = n.to_le_bytes().to_vec();
        bytes.extend(0u64.to_le_bytes());
        bytes.extend(u64::from(l.size).to_le_bytes());
        bytes.extend(u64::from(l.align).to_le_bytes());
        bytes.resize(8 * (VTABLE_HEADER + methods.len() + supers.len()), 0);
        desc.define(bytes.into_boxed_slice());
        // The supertraits' vtables, for a `dyn` borrow used as one of them. Each is declared
        // (and cached) before it is filled, so building them cannot loop.
        for (k, sup) in supers.iter().enumerate() {
            let vt = self.vtable(ty, sup)?;
            let vref = self.module.declare_data_in_data(vt, &mut desc);
            let at = 8 * (VTABLE_HEADER + methods.len() + k) as u32;
            desc.write_data_addr(at, vref, 0);
        }
        let glue = self.glue(ty)?;
        let gref = self.module.declare_func_in_data(glue, &mut desc);
        desc.write_function_addr(8, gref);
        let closure_body = match ty {
            Ty::Closure(c) => Some(self.declare_closure(c)?.0),
            _ => None,
        };
        for (k, (m, t)) in methods.iter().enumerate() {
            let at = 8 * (VTABLE_HEADER + k) as u32;
            if let Some(fid) = closure_body {
                let fref = self.module.declare_func_in_data(fid, &mut desc);
                desc.write_function_addr(at, fref);
                continue;
            }
            // An anonymous impl's: each method it defines, through a thunk, or the trait's
            // default for it.
            if let Ty::Anon(a) = ty {
                let fid = match a.methods.iter().position(|(f, _)| f == m) {
                    Some(j) => Some(self.anon_thunk(ty, j)?),
                    None if self.items.fns[*m as usize].has_body => {
                        let mut args = vec![ty.clone()];
                        args.extend(t.args.iter().cloned());
                        Some(self.declare_fn(*m, &args)?.0)
                    }
                    None => None,
                };
                if let Some(fid) = fid {
                    let fref = self.module.declare_func_in_data(fid, &mut desc);
                    desc.write_function_addr(at, fref);
                }
                continue;
            }
            let sig = &self.items.fns[*m as usize];
            // Only a method that takes `self` by borrow, with no type parameters of its own, is
            // called through a `dyn` borrow.
            if sig.generics.params.len() > sig.generics.parent as usize {
                continue;
            }
            let Some((f, parent)) = Solver::new(self.items, &[]).impl_method(*m, ty, t) else {
                continue;
            };
            let fsig = &self.items.fns[f as usize];
            if fsig.intrinsic || !fsig.has_body {
                continue;
            }
            let body = self.mir(Item::Fn(f))?;
            if body.arg_count == 0 || !matches!(body.locals[1].ty, Ty::Ref(..)) {
                continue;
            }
            let (fid, ..) = self.declare_fn(f, &parent)?;
            let fref = self.module.declare_func_in_data(fid, &mut desc);
            desc.write_function_addr(at, fref);
        }
        self.module
            .define_data(id, &desc)
            .map_err(|e| e.to_string())?;
        Ok(id)
    }

    fn mir(&self, item: Item) -> Result<&Body, String> {
        let owner = Owner::new(self.db, self.program, item);
        moddil_mir::mir(self.db, owner)
            .as_ref()
            .ok_or_else(|| "a program with errors".to_string())
    }

    /// Compiles `item`'s body, with type arguments `targs`, as function `id`.
    fn body_of(
        &mut self,
        item: Item,
        targs: &[Ty],
        id: FuncId,
        params: &[Ty],
        ret: &Ty,
    ) -> Result<(), String> {
        let body = self.mir(item)?.clone();
        let file = body.src.file().unwrap_or(0);
        // Its traps are reported where untracked code called it.
        let tracked = body.track_caller || !body.src.is_file();
        let tys = body
            .locals
            .iter()
            .map(|l| self.concrete(&l.ty, targs))
            .collect();
        let closure = matches!(item, Item::Closure(..));
        let bounded = matches!(item, Item::Fn(f) if self.items.fns[f as usize].bounded);
        let safe_points = self.rt.preempt.is_some() && !bounded;
        let mut ctx = self.module.make_context();
        ctx.func.signature = self.signature(params, ret, closure)?;
        let mut fctx = FunctionBuilderContext::new();
        let mut lw = Lower {
            c: self,
            body: &body,
            targs,
            tys,
            file,
            tracked,
            closure,
            safe_points,
            span: TextRange::default(),
            b: FunctionBuilder::new(&mut ctx.func, &mut fctx),
            locs: Vec::new(),
            blocks: Vec::new(),
            flags: HashMap::new(),
            err_ptr: None,
            point: (0, 0),
            defs: HashMap::new(),
        };
        lw.function()?;
        let config = lw.c.module.target_config();
        lw.b.finalize(config);
        self.module
            .define_function(id, &mut ctx)
            .map_err(|e| format!("Cranelift: {e:?}"))?;
        if let (Some(lines), Some(code)) = (&mut self.lines, ctx.compiled_code()) {
            let base = ctx.func.params.base_srcloc();
            let runs = code
                .buffer
                .get_srclocs_sorted()
                .iter()
                .map(|r| {
                    let loc = if r.loc.is_rel() {
                        r.loc.relocate(base)
                    } else {
                        r.loc.as_abs()
                    };
                    (r.start, (!loc.is_default()).then(|| loc.bits() - 1))
                })
                .collect();
            lines.push(crate::dwarf::FnLines {
                id,
                name: body.name.clone(),
                size: code.buffer.data().len() as u32,
                runs,
            });
        }
        self.module.clear_context(&mut ctx);
        Ok(())
    }

    /// The source location of the code from offset `offset` of file `file`.
    fn srcloc(&mut self, file: u32, offset: u32) -> ir::SourceLoc {
        let n = self.places.len() as u32;
        let id = *self.place_ids.entry((file, offset)).or_insert(n);
        if id == n {
            self.places.push((file, offset));
        }
        ir::SourceLoc::new(id + 1)
    }

    /// `ty`'s drop glue, `fn(*mut T) -> i8`: declared (and queued) the first time it is needed.
    fn glue(&mut self, ty: &Ty) -> Result<FuncId, String> {
        if let Some(f) = self.glues.get(ty) {
            return Ok(*f);
        }
        let id = self.declare_item("drop", &[Ty::reference(true, Ty::Unit)], &Ty::Unit, false)?;
        self.glues.insert(ty.clone(), id);
        self.glue_queue.push(ty.clone());
        Ok(id)
    }

    /// Compiles `ty`'s drop glue.
    fn glue_body(&mut self, ty: &Ty) -> Result<(), String> {
        let id = self.glues[ty];
        let body = Body {
            name: format!("drop of `{ty}`"),
            src: Src::Prelude,
            track_caller: false,
            locals: vec![
                LocalDecl {
                    ty: Ty::Unit,
                    name: None,
                },
                LocalDecl {
                    ty: Ty::reference(true, Ty::Unit),
                    name: None,
                },
            ],
            arg_count: 1,
            blocks: Vec::new(),
            strs: Vec::new(),
            cleanups: Vec::new(),
            cleanup_locals: 0,
            late: Vec::new(),
        };
        let params = [Ty::reference(true, Ty::Unit)];
        let mut ctx = self.module.make_context();
        ctx.func.signature = self.signature(&params, &Ty::Unit, false)?;
        let mut fctx = FunctionBuilderContext::new();
        let mut lw = Lower {
            c: self,
            body: &body,
            targs: &[],
            tys: vec![Ty::Unit, Ty::reference(true, Ty::Unit)],
            file: 0,
            tracked: true,
            closure: false,
            safe_points: false,
            span: TextRange::default(),
            b: FunctionBuilder::new(&mut ctx.func, &mut fctx),
            locs: Vec::new(),
            blocks: Vec::new(),
            flags: HashMap::new(),
            err_ptr: None,
            point: (0, 0),
            defs: HashMap::new(),
        };
        let entry = lw.b.create_block();
        lw.b.append_block_params_for_function_params(entry);
        lw.b.switch_to_block(entry);
        let addr = lw.b.block_params(entry)[0];
        lw.err_ptr = Some(lw.b.block_params(entry)[1]);
        lw.drop_own(addr, ty)?;
        lw.return_status(0)?;
        lw.b.seal_all_blocks();
        let config = lw.c.module.target_config();
        lw.b.finalize(config);
        self.module
            .define_function(id, &mut ctx)
            .map_err(|e| format!("Cranelift: {e:?}"))?;
        self.module.clear_context(&mut ctx);
        Ok(())
    }

    /// `fn(*mut T) -> i8` in the platform's C convention, for the runtime: `ty`'s drop glue.
    fn c_drop(&mut self, ty: &Ty) -> Result<FuncId, String> {
        if let Some(f) = self.c_drops.get(ty) {
            return Ok(*f);
        }
        let ptr = self.module.target_config().pointer_type();
        let mut sig = self.module.make_signature();
        sig.params.push(AbiParam::new(ptr));
        sig.returns.push(AbiParam::new(types::I8));
        let id = self
            .module
            .declare_anonymous_function(&sig)
            .map_err(|e| e.to_string())?;
        self.c_drops.insert(ty.clone(), id);
        let glue = self.glue(ty)?;
        self.define_fn(id, sig, |this, b| {
            let block = b.create_block();
            b.append_block_params_for_function_params(block);
            b.switch_to_block(block);
            let at = b.block_params(block)[0];
            let f = this.module.declare_func_in_func(glue, b.func);
            let null = b.ins().iconst(ptr, 0);
            let call = b.ins().call(f, &[at, null]);
            let status = b.inst_results(call)[0];
            b.ins().return_(&[status]);
            Ok(id)
        })
    }

    /// A task's code, for closure type `c` ([`moddil_rt::TaskFn`]): its body, given the
    /// captures and the argument (a scope's borrow, for a scope's body), its result written to
    /// `out` and an error to `err`.
    fn task_tramp(&mut self, c: &ClosureTy) -> Result<FuncId, String> {
        let key = Ty::Closure(Arc::new(c.clone()));
        if let Some(f) = self.tramps.get(&key) {
            return Ok(*f);
        }
        let ptr = self.module.target_config().pointer_type();
        let mut sig = self.module.make_signature();
        for t in [ptr, types::I64, ptr, ptr] {
            sig.params.push(AbiParam::new(t));
        }
        sig.returns.push(AbiParam::new(types::I8));
        let id = self
            .module
            .declare_anonymous_function(&sig)
            .map_err(|e| e.to_string())?;
        self.tramps.insert(key, id);
        let (body, params, ret) = self.declare_closure(c)?;
        let arg_pass = self.pass(&params[1])?;
        let pass = self.pass(&ret)?;
        self.define_fn(id, sig, |this, b| {
            let block = b.create_block();
            b.append_block_params_for_function_params(block);
            b.switch_to_block(block);
            let ps = b.block_params(block).to_vec();
            let (captures, arg, out, err) = (ps[0], ps[1], ps[2], ps[3]);
            let mut vals = vec![captures];
            match arg_pass {
                Pass::Nothing => {}
                Pass::Scalar(_) => vals.push(arg),
                Pass::Words(ts) if ts == [types::I64] => vals.push(arg),
                _ => return Err("a task's argument".to_string()),
            }
            if pass == Pass::Indirect {
                vals.push(out);
            }
            vals.push(err);
            let f = this.module.declare_func_in_func(body, b.func);
            let call = b.ins().call(f, &vals);
            let results = b.inst_results(call).to_vec();
            store_pass(b, &pass, &results[1..], out);
            b.ins().return_(&[results[0]]);
            Ok(id)
        })
    }

    /// A trait method's code for method `k` of anonymous impl type `anon`, as a vtable calls it:
    /// given `self` and the method's other arguments, it calls the method's closure with its
    /// captures (`self`'s field) and the arguments as a tuple.
    fn anon_thunk(&mut self, anon: &Ty, k: usize) -> Result<FuncId, String> {
        let Ty::Anon(a) = anon else {
            return Err("an anonymous impl".to_string());
        };
        let Ty::Closure(c) = a.methods[k].1.clone() else {
            return Err("an anonymous impl's method".to_string());
        };
        let (body, cparams, ret) = self.declare_closure(&c)?;
        let elems: Vec<Ty> = match &cparams[1] {
            Ty::Tuple(ts) => ts.to_vec(),
            Ty::Unit => Vec::new(),
            t => vec![t.clone()],
        };
        let mut params = vec![Ty::reference(false, anon.clone())];
        params.extend(elems.iter().cloned());
        let sig = self.signature(&params, &ret, false)?;
        let id = self
            .module
            .declare_anonymous_function(&sig)
            .map_err(|e| e.to_string())?;
        let l = self.layouts.of(anon)?;
        let Shape::Fields(fs) = &l.shape else {
            return Err("an anonymous impl's methods".to_string());
        };
        let off = fs[k].0;
        let tl = self.layouts.of(&cparams[1])?;
        let toffs: Vec<u32> = match &tl.shape {
            Shape::Fields(fs) => fs.iter().map(|(o, _)| *o).collect(),
            _ => Vec::new(),
        };
        let tuple_pass = pass_of(&tl);
        let ret_pass = self.pass(&ret)?;
        let elem_passes: Vec<Pass> = elems
            .iter()
            .map(|t| self.pass(t))
            .collect::<Result<_, _>>()?;
        let elem_sizes: Vec<u32> = elems
            .iter()
            .map(|t| self.layouts.of(t).map(|l| l.size))
            .collect::<Result<_, _>>()?;
        self.define_fn(id, sig, |this, b| {
            let block = b.create_block();
            b.append_block_params_for_function_params(block);
            b.switch_to_block(block);
            let ps = b.block_params(block).to_vec();
            let flags = ir::MemFlagsData::trusted();
            let captures = b.ins().iadd_imm_s(ps[0], i64::from(off));
            // The arguments, as the tuple the closure takes.
            let slot = b.create_sized_stack_slot(ir::StackSlotData::new(
                ir::StackSlotKind::ExplicitSlot,
                tl.size.max(1),
                4,
            ));
            let ptr = this.module.target_config().pointer_type();
            let tuple = b.ins().stack_addr(ptr, slot, 0);
            let mut i = 1;
            for (e, (pass, size)) in elem_passes.iter().zip(&elem_sizes).enumerate() {
                let at = toffs.get(e).copied().unwrap_or(0) as i32;
                match pass {
                    Pass::Nothing => {}
                    Pass::Scalar(_) => {
                        b.ins().store(flags, ps[i], tuple, at);
                        i += 1;
                    }
                    Pass::Words(ts) => {
                        let mut o = at;
                        for t in ts {
                            b.ins().store(flags, ps[i], tuple, o);
                            o += t.bytes() as i32;
                            i += 1;
                        }
                    }
                    Pass::Indirect => {
                        let src = ps[i];
                        let mut o = 0;
                        while o < *size as i32 {
                            let v = b.ins().load(types::I8, flags, src, o);
                            b.ins().store(flags, v, tuple, at + o);
                            o += 1;
                        }
                        i += 1;
                    }
                }
            }
            let mut vals = vec![captures];
            vals.extend(load_pass(b, &tuple_pass, tuple));
            // The result's place (if the caller gives one), then the error's: as the caller gave.
            if ret_pass == Pass::Indirect {
                vals.push(ps[ps.len() - 2]);
            }
            vals.push(ps[ps.len() - 1]);
            let f = this.module.declare_func_in_func(body, b.func);
            let call = b.ins().call(f, &vals);
            let results = b.inst_results(call).to_vec();
            b.ins().return_(&results);
            Ok(id)
        })
    }

    /// `moddil_exit_code(out) -> i32`, for a built program: `main`'s result at `out` as the
    /// process's exit code (`main.exit.1`): 0 for `()`, an `ExitCode`'s `sysexits.h` number, or
    /// `Other(n)`'s `n`, and 0 for any other result, which the program prints as `moddil run`
    /// does: the data `moddil_result_size` (its size) and `moddil_result_types` (the length of
    /// a `moddil_show::Types` table, the result's index in it, and the table; a length of 0 for
    /// `()` and `ExitCode`, which are not printed).
    pub(crate) fn exit_code(&mut self, ret: &Ty) -> Result<(), String> {
        let exit = self.prelude_ty("ExitCode")?;
        let shown = *ret != Ty::Unit && ret.adt() != exit.adt();
        let size = self.layouts.of(ret).map_or(0, |l| u64::from(l.size));
        let mut types = Vec::new();
        if shown {
            let (table, root) = crate::show::Show::new(self.items, &self.dyn_types).table(ret);
            let bytes = table.encode();
            types.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
            types.extend_from_slice(&u64::from(root).to_le_bytes());
            types.extend_from_slice(&bytes);
        } else {
            types.extend_from_slice(&0u64.to_le_bytes());
        }
        for (name, bytes) in [
            ("moddil_result_size", size.to_le_bytes().to_vec()),
            ("moddil_result_types", types),
        ] {
            let id = self
                .module
                .declare_data(name, Linkage::Export, false, false)
                .map_err(|e| e.to_string())?;
            let mut desc = DataDescription::new();
            desc.define(bytes.into_boxed_slice());
            desc.set_align(8);
            self.module
                .define_data(id, &desc)
                .map_err(|e| e.to_string())?;
        }
        let ptr = self.module.target_config().pointer_type();
        let mut sig = self.module.make_signature();
        sig.params.push(AbiParam::new(ptr));
        sig.returns.push(AbiParam::new(types::I32));
        let id = self
            .module
            .declare_function("moddil_exit_code", Linkage::Export, &sig)
            .map_err(|e| e.to_string())?;
        let l = self.layouts.of(&exit)?;
        self.define_fn(id, sig, |_, b| {
            let block = b.create_block();
            b.append_block_params_for_function_params(block);
            b.switch_to_block(block);
            let out = b.block_params(block)[0];
            let code = if *ret == Ty::Unit || shown {
                b.ins().iconst(types::I32, 0)
            } else {
                let Shape::Enum {
                    tag: Tag::Direct { offset, ty },
                    variants,
                } = &l.shape
                else {
                    return Err("the prelude's `ExitCode`".to_string());
                };
                let flags = ir::MemFlagsData::trusted();
                let t = b.ins().load(*ty, flags, out, *offset as i32);
                let d = if *ty == types::I32 {
                    t
                } else {
                    b.ins().uextend(types::I32, t)
                };
                // `Ok` 0, then `Usage` 64 to `Config` 78; `Other(n)` its `n`.
                const CODES: [i64; 16] = [
                    0, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76, 77, 78,
                ];
                let other = variants.len() - 1;
                let at = variants[other].first().map_or(0, |f| f.0);
                let n = b.ins().load(types::I64, flags, out, at as i32);
                let mut code = b.ins().ireduce(types::I32, n);
                for (k, c) in CODES.iter().enumerate() {
                    let is = b.ins().icmp_imm_s(IntCC::Equal, d, k as i64);
                    let v = b.ins().iconst(types::I32, *c);
                    code = b.ins().select(is, v, code);
                }
                code
            };
            b.ins().return_(&[code]);
            Ok(())
        })
    }

    /// The tables a built program reports a trap from (see [`compile_object`]).
    pub(crate) fn site_tables(&mut self, files: &[(String, String)]) -> Result<(), String> {
        let mut strings: Vec<u8> = Vec::new();
        let mut intern = |s: &str| -> (u32, u32) {
            let at = strings.len() as u32;
            strings.extend_from_slice(s.as_bytes());
            (at, s.len() as u32)
        };
        let mut table: Vec<u8> = Vec::new();
        for site in &self.sites {
            let (kind, tracked, class, file, range) = match site {
                Site::Trap(class, file, range) => (0u32, false, class.to_string(), *file, *range),
                Site::Call(file, range, tracked) => (
                    1,
                    *tracked,
                    TrapClass::StackOverflow.to_string(),
                    *file,
                    *range,
                ),
                Site::Observe(file, range, _) => (2, false, String::new(), *file, *range),
                Site::Part(range, ..) => (2, false, String::new(), 0, *range),
                Site::Fail(file, range) => (3, false, String::new(), *file, *range),
            };
            let (path, text) = files
                .get(file as usize)
                .map_or(("", ""), |(p, t)| (p.as_str(), t.as_str()));
            let (line, col) = moddil_syntax::line_col(text, range.start);
            let (co, cl) = intern(&class);
            let (po, pl) = intern(path);
            for w in [kind, u32::from(tracked), co, cl, po, pl, line, col] {
                table.extend_from_slice(&w.to_le_bytes());
            }
        }
        let count = (self.sites.len() as u64).to_le_bytes().to_vec();
        for (name, bytes) in [
            ("moddil_site_count", count),
            ("moddil_sites", table),
            ("moddil_strings", strings),
        ] {
            let id = self
                .module
                .declare_data(name, Linkage::Export, false, false)
                .map_err(|e| e.to_string())?;
            let mut desc = DataDescription::new();
            let mut bytes = bytes;
            bytes.push(0);
            desc.define(bytes.into_boxed_slice());
            desc.set_align(8);
            self.module
                .define_data(id, &desc)
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// The prelude's struct or enum `name`, as a type without arguments.
    fn prelude_ty(&self, name: &str) -> Result<Ty, String> {
        let a = self
            .items
            .adts
            .iter()
            .position(|d| d.src == Src::Prelude && d.name == name)
            .ok_or_else(|| format!("the prelude's `{name}`"))?;
        Ok(Ty::Adt(a as u32, name.into(), Arc::from([])))
    }

    /// `fn(out: *mut u8) -> i8`, in the platform's C convention: drops the entry's result, of
    /// type `ret`, at `out`.
    fn drop_result(&mut self, ret: &Ty, k: usize) -> Result<FuncId, String> {
        let ptr = self.module.target_config().pointer_type();
        let mut sig = self.module.make_signature();
        sig.params.push(AbiParam::new(ptr));
        sig.returns.push(AbiParam::new(types::I8));
        let id = self
            .module
            .declare_function(&entry_name("moddil_drop_result", k), Linkage::Export, &sig)
            .map_err(|e| e.to_string())?;
        let needs = Solver::new(self.items, &[]).needs_drop(ret);
        let glue = if needs { Some(self.glue(ret)?) } else { None };
        self.define_fn(id, sig, |this, b| {
            let block = b.create_block();
            b.append_block_params_for_function_params(block);
            b.switch_to_block(block);
            let out = b.block_params(block)[0];
            let status = match glue {
                Some(g) => {
                    let f = this.module.declare_func_in_func(g, b.func);
                    let null = b.ins().iconst(ptr, 0);
                    let call = b.ins().call(f, &[out, null]);
                    b.inst_results(call)[0]
                }
                None => b.ins().iconst(types::I8, 0),
            };
            b.ins().return_(&[status]);
            Ok(id)
        })
    }

    /// `#[export]` function `f`'s wrapper (`ffi.export.1`): under its own name, with C's types
    /// in the platform's C convention ([`Compiler::c_wrapper`]).
    fn export_wrapper(
        &mut self,
        f: u32,
        target: FuncId,
        params: &[Ty],
        ret: &Ty,
    ) -> Result<FuncId, String> {
        let sig = &self.items.fns[f as usize];
        let name = sig.name.clone();
        let on_trap = if sig.export_status {
            OnTrap::Status
        } else {
            OnTrap::Give(sig.export_on_trap)
        };
        self.c_wrapper(target, params, ret, &name, Linkage::Export, on_trap)
    }

    /// A C-ABI function `name` that calls Moddil function `target` (taking `params`, giving
    /// `ret`): C's types (8- and 16-bit integers as 32, as C passes them; structs as
    /// [`moddil_ffi::abi`] says) in the platform's C convention. It starts the runtime, calls
    /// `target`, and if that traps, does as `on_trap` says.
    fn c_wrapper(
        &mut self,
        target: FuncId,
        params: &[Ty],
        ret: &Ty,
        name: &str,
        linkage: Linkage,
        on_trap: OnTrap,
    ) -> Result<FuncId, String> {
        let wide = |t: ir::Type| {
            if t == types::I8 || t == types::I16 {
                types::I32
            } else {
                t
            }
        };
        let on = moddil_ffi::abi::target();
        let sig = self.c_sig(params, ret)?;
        let id = self
            .module
            .declare_function(name, linkage, &sig)
            .map_err(|e| format!("Cranelift: {e:?}"))?;
        let import = |m: &mut M, n: &str| -> Result<FuncId, String> {
            let s = m.make_signature();
            m.declare_function(n, Linkage::Import, &s)
                .map_err(|e| e.to_string())
        };
        let begin = import(&mut self.module, "moddil_rt_export_begin")?;
        let trapped = import(
            &mut self.module,
            match on_trap {
                OnTrap::End => "moddil_rt_callback_trapped",
                _ => "moddil_rt_export_trapped",
            },
        )?;
        let ptr = self.module.target_config().pointer_type();
        // How each struct crosses, worked out before the builder borrows the context.
        let mut cparams = Vec::new();
        let mut passes = Vec::new();
        for p in params {
            cparams.push(self.c_type(p)?);
            passes.push(self.pass(p)?);
        }
        let cret = self.c_type(ret)?;
        let ret_pass = self.pass(ret)?;
        self.define_fn(id, sig, |this, b| {
            let block = b.create_block();
            b.append_block_params_for_function_params(block);
            b.switch_to_block(block);
            let mut args = b.block_params(block).to_vec();
            let flags = ir::MemFlagsData::trusted();
            // A struct result given through a hidden pointer: the first parameter.
            let sret = match &cret {
                CType::Struct(l)
                    if moddil_ffi::abi::ret(on, l) == moddil_ffi::abi::Ret::Indirect =>
                {
                    Some(args.remove(0))
                }
                _ => None,
            };
            let begin = this.module.declare_func_in_func(begin, b.func);
            b.ins().call(begin, &[]);
            let mut vals = Vec::new();
            let mut args = args.into_iter();
            for ((p, cp), pass) in params.iter().zip(&cparams).zip(&passes) {
                if let CType::Struct(l) = cp {
                    // The struct, assembled in a slot of its own, then passed as Moddil passes it.
                    let slot = scratch(b, l.padded() + 8);
                    match moddil_ffi::abi::arg(on, l) {
                        moddil_ffi::abi::Arg::Parts(parts) => {
                            for (off, _) in parts {
                                let v = args.next().ok_or("a struct's part")?;
                                b.ins().store(flags, v, slot, off as i32);
                            }
                        }
                        moddil_ffi::abi::Arg::Pointer | moddil_ffi::abi::Arg::Stack => {
                            let at = args.next().ok_or("a struct's address")?;
                            copy_bytes(b, slot, at, l.size);
                        }
                    }
                    vals.extend(load_pass(b, pass, slot));
                    continue;
                }
                let a = args.next().ok_or("a parameter")?;
                let t = clif_type(p)?.expect("a C scalar");
                vals.push(if wide(t) == t {
                    a
                } else {
                    b.ins().ireduce(t, a)
                });
            }
            // Where Moddil gives a struct result.
            let rslot = match &cret {
                CType::Struct(l) => Some(scratch(b, l.padded() + 8)),
                _ => None,
            };
            if let (Some(r), Pass::Indirect) = (rslot, &ret_pass) {
                vals.push(r);
            }
            // Room for an error, which an export cannot give.
            let slot = b.create_sized_stack_slot(ir::StackSlotData::new(
                ir::StackSlotKind::ExplicitSlot,
                64,
                3,
            ));
            let err = b.ins().stack_addr(ptr, slot, 0);
            vals.push(err);
            let target = this.module.declare_func_in_func(target, b.func);
            let call = b.ins().call(target, &vals);
            let results = b.inst_results(call).to_vec();
            let ok = b.create_block();
            let bad = b.create_block();
            b.ins().brif(results[0], bad, &[], ok, &[]);
            b.switch_to_block(bad);
            let trapped = this.module.declare_func_in_func(trapped, b.func);
            match on_trap {
                // `#[export(status)]` (`ffi.export.2`): the caller is told, with 2.
                OnTrap::Status => {
                    let two = b.ins().iconst(types::I32, 2);
                    b.ins().return_(&[two]);
                }
                // A function pointer's call has nothing to give (`type.fnptr.1`).
                OnTrap::End => {
                    b.ins().call(trapped, &[]);
                    b.ins().trap(ir::TrapCode::unwrap_user(1));
                }
                // An export gives its result type's zero, or `on_trap` (`ffi.export.3`), and
                // `moddil_trapped()` says it trapped.
                OnTrap::Give(v) => {
                    b.ins().call(trapped, &[]);
                    match &cret {
                        CType::Struct(l) => match moddil_ffi::abi::ret(on, l) {
                            moddil_ffi::abi::Ret::Parts(parts) => {
                                let out: Vec<ir::Value> =
                                    parts.iter().map(|(_, t)| zero(b, *t)).collect();
                                b.ins().return_(&out);
                            }
                            moddil_ffi::abi::Ret::Indirect => {
                                let at = sret.ok_or("the hidden result pointer")?;
                                let z = b.ins().iconst(types::I8, 0);
                                for off in 0..l.size {
                                    b.ins().store(flags, z, at, off as i32);
                                }
                                b.ins().return_(&[]);
                            }
                        },
                        _ => match clif_type(ret)? {
                            Some(t) => {
                                let t = wide(t);
                                let r = match v {
                                    // In the result's bits: a `u64` past `i64::MAX` too.
                                    Some(v) => b.ins().iconst(t, v as i64),
                                    None => zero(b, t),
                                };
                                b.ins().return_(&[r]);
                            }
                            None => {
                                b.ins().return_(&[]);
                            }
                        },
                    }
                }
            }
            b.switch_to_block(ok);
            if let (CType::Struct(l), Some(r)) = (&cret, rslot) {
                store_pass(b, &ret_pass, &results[1..], r);
                match moddil_ffi::abi::ret(on, l) {
                    moddil_ffi::abi::Ret::Parts(parts) => {
                        let out: Vec<ir::Value> = parts
                            .iter()
                            .map(|(off, t)| b.ins().load(*t, flags, r, *off as i32))
                            .collect();
                        b.ins().return_(&out);
                    }
                    moddil_ffi::abi::Ret::Indirect => {
                        let at = sret.ok_or("the hidden result pointer")?;
                        copy_bytes(b, at, r, l.size);
                        b.ins().return_(&[]);
                    }
                }
            } else {
                match clif_type(ret)? {
                    Some(t) if wide(t) != t => {
                        let v = if signed(ret) {
                            b.ins().sextend(wide(t), results[1])
                        } else {
                            b.ins().uextend(wide(t), results[1])
                        };
                        b.ins().return_(&[v]);
                    }
                    Some(_) => {
                        b.ins().return_(&[results[1]]);
                    }
                    None => {
                        b.ins().return_(&[]);
                    }
                }
            }
            Ok(id)
        })
    }

    /// `fn(out: *mut u8, err: *mut u8) -> i8`, in the platform's C convention: calls `target`,
    /// storing its result's bytes at `out`, or its error at `err`.
    fn trampoline(
        &mut self,
        target: FuncId,
        params: &[Ty],
        ret: &Ty,
        k: usize,
    ) -> Result<FuncId, String> {
        let ptr = self.module.target_config().pointer_type();
        let mut sig = self.module.make_signature();
        sig.params.push(AbiParam::new(ptr));
        sig.params.push(AbiParam::new(ptr));
        sig.returns.push(AbiParam::new(types::I8));
        let id = self
            .module
            .declare_function(&entry_name("moddil_entry", k), Linkage::Export, &sig)
            .map_err(|e| e.to_string())?;
        let pass = self.pass(ret)?;
        self.define_fn(id, sig, |this, b| {
            let block = b.create_block();
            b.append_block_params_for_function_params(block);
            b.switch_to_block(block);
            let out = b.block_params(block)[0];
            let err = b.block_params(block)[1];
            let f = this.module.declare_func_in_func(target, b.func);
            // Each parameter (a root capability) zero.
            let mut vals = Vec::new();
            for p in params {
                let ts = match this.pass(p)? {
                    Pass::Nothing => Vec::new(),
                    Pass::Scalar(t) => vec![t],
                    Pass::Words(ts) => ts,
                    Pass::Indirect => {
                        let l = this.layouts.of(p)?;
                        let slot = b.create_sized_stack_slot(ir::StackSlotData::new(
                            ir::StackSlotKind::ExplicitSlot,
                            l.size.max(1),
                            3,
                        ));
                        let a = b.ins().stack_addr(ptr, slot, 0);
                        let zero = b.ins().iconst(types::I8, 0);
                        for k in 0..l.size {
                            b.ins()
                                .store(ir::MemFlagsData::trusted(), zero, a, k as i32);
                        }
                        vals.push(a);
                        continue;
                    }
                };
                for t in ts {
                    vals.push(match t {
                        types::F32 => b.ins().f32const(0.0),
                        types::F64 => b.ins().f64const(0.0),
                        t => b.ins().iconst(t, 0),
                    });
                }
            }
            if pass == Pass::Indirect {
                vals.push(out);
            }
            vals.push(err);
            let call = b.ins().call(f, &vals);
            let results = b.inst_results(call).to_vec();
            store_pass(b, &pass, &results[1..], out);
            b.ins().return_(&[results[0]]);
            Ok(id)
        })
    }
}

/// The scheduler's operation that built-in function `f` of the prelude is, if it is one (as
/// the interpreter's `task_native`).
fn task_op(items: &ItemTree, f: u32) -> Option<&'static str> {
    moddil_mir::runtime_op(items, f)
}

/// The error a task operation's failure is, written into the buffer before its error edge
/// takes it: the runtime's own (already there), `Closed`, or (if the flag holds) `Trapped`.
#[derive(Clone, Copy)]
enum Before {
    Nothing,
    Closed,
    Trapped(ir::Value),
}

/// What a call runs: a function, or one through a pointer (a vtable's), with its parameters;
/// and its result type.
enum Run {
    Direct(FuncId, Ty),
    Indirect(ir::Value, Vec<Ty>, Ty),
}

/// Each operand of `r`.
fn rvalue_operands(r: &Rvalue, f: &mut impl FnMut(&Operand)) {
    match r {
        Rvalue::Use(a)
        | Rvalue::Unary(_, a)
        | Rvalue::NegOverflows(a)
        | Rvalue::Unsize(a, _)
        | Rvalue::Widen(a, _)
        | Rvalue::Repeat(a, _)
        | Rvalue::InBounds(a, _) => f(a),
        Rvalue::Binary(_, a, b) | Rvalue::Overflows(_, a, b) | Rvalue::SpanOk(a, b, _) => {
            f(a);
            f(b);
        }
        Rvalue::Aggregate(_, xs) => xs.iter().for_each(f),
        _ => {}
    }
}

/// Where a local is.
#[derive(Clone, Copy)]
enum Loc {
    /// It has no bytes.
    Nothing,
    /// A Cranelift variable: a scalar nothing borrows.
    Var(Variable),
    /// Memory, at this address.
    Mem(ir::Value),
}

/// A place, found: where it is and its type.
struct PlaceRef {
    /// A variable, for a whole local that is one.
    var: Option<Variable>,
    /// Its address otherwise.
    addr: Option<ir::Value>,
    ty: Ty,
    /// After a `Downcast`: the variant its fields are of.
    variant: Option<u32>,
    /// For an unsized place (`str`, `[T]`, `dyn Trait`): its length, or its vtable.
    meta: Option<ir::Value>,
    /// For a `[dyn Trait]` place: its elements' vtable.
    vtable: Option<ir::Value>,
}

impl PlaceRef {
    /// The place of a `ty` at `addr`.
    fn at(addr: ir::Value, ty: Ty) -> PlaceRef {
        PlaceRef {
            var: None,
            addr: Some(addr),
            ty,
            variant: None,
            meta: None,
            vtable: None,
        }
    }
}

/// A value, as an operand gives it.
#[derive(Clone, Copy)]
enum Val {
    /// `()` or another value without bytes.
    Unit,
    /// A scalar.
    Scalar(ir::Value),
    /// An aggregate, at this address.
    Mem(ir::Value),
}

/// One body being lowered.
struct Lower<'a, 'c, 'db, M: Module> {
    c: &'a mut Compiler<'db, M>,
    body: &'a Body,
    /// Its type arguments.
    targs: &'a [Ty],
    /// Its locals' types, concrete.
    tys: Vec<Ty>,
    file: u32,
    /// Whether it is `#[track_caller]`, the prelude's or derived code.
    tracked: bool,
    /// Whether it is a closure's body, which takes its captures by pointer.
    closure: bool,
    /// Whether its loops check for preemption at their back-edges: in code that may run on
    /// several threads, unless the function is `#[bounded]` (`attr.bounded.1`).
    safe_points: bool,
    /// Where the terminator being lowered is.
    span: TextRange,
    b: FunctionBuilder<'c>,
    locs: Vec<Loc>,
    /// Each MIR block's Cranelift block (`None` for cleanup code, which only unwinding runs).
    blocks: Vec<Option<ir::Block>>,
    /// The drop flags: for each local that may need dropping, and each part of one that is
    /// moved out of, whether it holds a value.
    flags: HashMap<Place, Variable>,
    /// Where an error this body raises goes.
    err_ptr: Option<ir::Value>,
    /// The block and statement being lowered (the statement past the last: the terminator),
    /// which says where unwinding from here goes.
    point: (usize, usize),
    /// Each local assigned once, and what it is assigned (to trace a `select`'s arms).
    defs: HashMap<u32, Rvalue>,
}

impl<M: Module> Lower<'_, '_, '_, M> {
    fn ptr(&self) -> ir::Type {
        self.c.module.target_config().pointer_type()
    }

    fn layout(&mut self, ty: &Ty) -> Result<Arc<Layout>, String> {
        self.c.layouts.of(ty)
    }

    /// A stack slot of layout `l`'s size, at least 8-byte aligned, and its address.
    fn slot(&mut self, l: &Layout) -> ir::Value {
        stack_slot(&mut self.b, l.size, l.align)
    }

    /// The locals some `&` or `&mut` takes the address of (not through a borrow they hold).
    fn borrowed(&self) -> HashSet<u32> {
        let mut out = HashSet::new();
        for bb in &self.body.blocks {
            for s in &bb.statements {
                if let Statement::Assign(_, Rvalue::Ref(_, p)) = s
                    && p.projection.first() != Some(&Projection::Deref)
                {
                    out.insert(p.local.0);
                }
            }
        }
        out
    }

    fn function(&mut self) -> Result<(), String> {
        let entry = self.b.create_block();
        self.b.append_block_params_for_function_params(entry);
        self.b.switch_to_block(entry);
        let params = self.b.block_params(entry).to_vec();
        let borrowed = self.borrowed();
        let mut k = 0;
        for i in 0..self.tys.len() {
            let layout = self.c.layouts.of(&self.tys[i])?;
            let pass = pass_of(&layout);
            let arg = (1..=self.body.arg_count).contains(&i);
            let loc = match (&layout.shape, borrowed.contains(&(i as u32))) {
                _ if self.closure && i == 1 => {
                    k += 1;
                    if layout.size == 0 {
                        Loc::Nothing
                    } else {
                        Loc::Mem(params[k - 1])
                    }
                }
                (Shape::Empty, _) => Loc::Nothing,
                _ if layout.size == 0 => Loc::Nothing,
                (Shape::Scalar(t), false) => {
                    let var = self.b.declare_var(*t);
                    let v = if arg {
                        k += 1;
                        params[k - 1]
                    } else {
                        self.zero(*t)
                    };
                    self.b.def_var(var, v);
                    Loc::Var(var)
                }
                _ if arg && (pass == Pass::Indirect || (self.closure && i == 1)) => {
                    // The caller's copy is the parameter's own (and a closure works on its
                    // captures where they are).
                    k += 1;
                    Loc::Mem(params[k - 1])
                }
                _ => {
                    let addr = self.slot(&layout);
                    if arg {
                        store_pass(&mut self.b, &pass, &params[k..], addr);
                        k += match &pass {
                            Pass::Scalar(_) => 1,
                            Pass::Words(ts) => ts.len(),
                            Pass::Nothing | Pass::Indirect => 0,
                        };
                    }
                    Loc::Mem(addr)
                }
            };
            self.locs.push(loc);
        }
        // A result returned indirectly is written where the caller says, the last parameter.
        let ret_layout = self.c.layouts.of(&self.tys[0])?;
        if pass_of(&ret_layout) == Pass::Indirect {
            self.locs[0] = Loc::Mem(params[params.len() - 2]);
        }
        self.err_ptr = params.last().copied();
        let mut counts: HashMap<u32, usize> = HashMap::new();
        for bb in &self.body.blocks {
            for st in &bb.statements {
                if let Statement::Assign(p, r) = st
                    && p.projection.is_empty()
                {
                    *counts.entry(p.local.0).or_default() += 1;
                    self.defs.insert(p.local.0, r.clone());
                }
            }
        }
        self.defs.retain(|l, _| counts[l] == 1);
        self.drop_flags();
        let reachable = self.reachable()?;
        for r in reachable {
            self.blocks.push(r.then(|| self.b.create_block()));
        }
        let first = self.block(0)?;
        self.b.ins().jump(first, &[]);
        for (i, bb) in self.body.blocks.iter().enumerate() {
            let Some(block) = self.blocks[i] else {
                continue;
            };
            self.b.switch_to_block(block);
            for (k, s) in bb.statements.iter().enumerate() {
                self.point = (i, k);
                if let Some(span) = bb.spans.get(k) {
                    let loc = self.c.srcloc(self.file, span.start);
                    self.b.set_srcloc(loc);
                }
                self.statement(s)?;
            }
            self.point = (i, bb.statements.len());
            let loc = self.c.srcloc(self.file, bb.terminator_span.start);
            self.b.set_srcloc(loc);
            self.terminator(&bb.terminator, bb.terminator_span)?;
        }
        self.b.seal_all_blocks();
        Ok(())
    }

    /// Which blocks control can reach: not cleanup code, and not the error edge of a call whose
    /// error is `!`.
    fn reachable(&mut self) -> Result<Vec<bool>, String> {
        let mut seen = vec![false; self.body.blocks.len()];
        // Cleanup code is reached by unwinding.
        let mut todo: Vec<usize> = (0..self.body.blocks.len())
            .filter(|&b| self.body.blocks[b].cleanup)
            .collect();
        todo.push(0);
        while let Some(b) = todo.pop() {
            if seen[b] {
                continue;
            }
            seen[b] = true;
            let t = &self.body.blocks[b].terminator;
            let next = match t {
                Terminator::Call {
                    target,
                    on_error: Some((p, _)),
                    ..
                } if self
                    .c
                    .concrete(&self.body.locals[p.local.index()].ty, self.targs)
                    == Ty::Never =>
                {
                    vec![*target]
                }
                t => t.successors(),
            };
            todo.extend(next.into_iter().map(|b| b.index()));
        }
        Ok(seen)
    }

    /// The value of type `ty` at `at`, taken out: a scalar loaded, or a copy in a slot of its
    /// own (the memory it was in is about to change).
    fn take_at(&mut self, at: ir::Value, ty: &Ty) -> Result<Val, String> {
        let r = PlaceRef::at(at, ty.clone());
        Ok(match self.read(&r)? {
            Val::Mem(a) => {
                let l = self.layout(ty)?;
                let copy = self.slot(&l);
                self.copy(copy, a, l.size);
                Val::Mem(copy)
            }
            v => v,
        })
    }

    fn block(&self, b: usize) -> Result<ir::Block, String> {
        self.blocks[b].ok_or_else(|| "unwinding".to_string())
    }

    fn zero(&mut self, t: ir::Type) -> ir::Value {
        match t {
            types::F32 => self.b.ins().f32const(0.0),
            types::F64 => self.b.ins().f64const(0.0),
            types::I128 => {
                let z = self.b.ins().iconst(types::I64, 0);
                self.b.ins().uextend(types::I128, z)
            }
            t => self.b.ins().iconst(t, 0),
        }
    }

    // ----- places ----------------------------------------------------------------------------

    /// Where place `p` is, and its type.
    fn place(&mut self, p: &Place) -> Result<PlaceRef, String> {
        let mut r = PlaceRef {
            var: None,
            addr: None,
            ty: self.tys[p.local.index()].clone(),
            variant: None,
            meta: None,
            vtable: None,
        };
        match self.locs[p.local.index()] {
            Loc::Var(v) => r.var = Some(v),
            Loc::Mem(a) => r.addr = Some(a),
            Loc::Nothing => {}
        }
        for proj in &p.projection {
            match proj {
                Projection::Deref => {
                    let (Ty::Ref(_, inner) | Ty::Ptr(_, inner)) = r.ty.clone() else {
                        return Err(format!("dereferencing a `{}`", r.ty));
                    };
                    let dyn_slice = matches!(&*inner, Ty::Slice(e) if matches!(**e, Ty::Dyn(..)));
                    let (ptr, meta, vtable) = match self.read(&r)? {
                        Val::Scalar(p) => (p, None, None),
                        // A wide borrow: the address, then the length or vtable (a borrow of
                        // a `[dyn Trait]` has both).
                        Val::Mem(a) => {
                            let flags = ir::MemFlagsData::trusted();
                            let p = self.b.ins().load(types::I64, flags, a, 0);
                            let m = self.b.ins().load(types::I64, flags, a, 8);
                            let v = dyn_slice.then(|| self.b.ins().load(types::I64, flags, a, 16));
                            (p, Some(m), v)
                        }
                        Val::Unit => return Err("a borrow of a value without bytes".to_string()),
                    };
                    r = PlaceRef {
                        meta,
                        vtable,
                        var: None,
                        addr: Some(ptr),
                        ty: (*inner).clone(),
                        variant: None,
                    };
                }
                // A member of an error set: the value after its id.
                Projection::Downcast(v) if matches!(r.ty, Ty::Errors(_)) => {
                    let Ty::Errors(ms) = r.ty.clone() else {
                        unreachable!()
                    };
                    let base = self.addr_of(&r)?;
                    r = PlaceRef::at(self.offset(base, ERROR_VALUE), ms[*v as usize].clone());
                }
                Projection::Downcast(v) => r.variant = Some(*v),
                Projection::Field(i) => {
                    let l = self.layout(&r.ty)?;
                    let (off, ty) = match (&l.shape, r.variant) {
                        (Shape::Fields(fs), _) => fs[*i as usize].clone(),
                        (Shape::Enum { variants, .. }, Some(v)) => {
                            variants[v as usize][*i as usize].clone()
                        }
                        (Shape::Enum { variants, .. }, None) if variants.len() == 1 => {
                            variants[0][*i as usize].clone()
                        }
                        _ => return Err(format!("a field of `{}`", r.ty)),
                    };
                    let base = self.addr_of(&r)?;
                    r = PlaceRef {
                        var: None,
                        addr: Some(self.offset(base, off)),
                        ty,
                        variant: None,
                        meta: None,
                        vtable: None,
                    };
                }
                Projection::Index(i) if r.vtable.is_some() => {
                    // An element of a `[dyn Trait]`: its size is in the vtable.
                    let vt = r.vtable.expect("a vtable");
                    let Ty::Slice(elem) = r.ty.clone() else {
                        return Err(format!("an element of `{}`", r.ty));
                    };
                    let base = self.addr_of(&r)?;
                    let Val::Scalar(index) = self.read_local(*i)? else {
                        return Err("an index without a value".to_string());
                    };
                    let size = self
                        .b
                        .ins()
                        .load(types::I64, ir::MemFlagsData::trusted(), vt, 16);
                    let off = self.b.ins().imul(index, size);
                    let addr = self.b.ins().iadd(base, off);
                    r = PlaceRef {
                        var: None,
                        addr: Some(addr),
                        ty: (*elem).clone(),
                        variant: None,
                        meta: Some(vt),
                        vtable: None,
                    };
                }
                Projection::Index(i) => {
                    let (elem, stride) = self.element(&r.ty)?;
                    let base = self.addr_of(&r)?;
                    let Val::Scalar(index) = self.read_local(*i)? else {
                        return Err("an index without a value".to_string());
                    };
                    let scaled = self.b.ins().imul_imm_s(index, i64::from(stride));
                    let addr = self.b.ins().iadd(base, scaled);
                    r = PlaceRef::at(addr, elem);
                }
                Projection::Subslice(from, to) => {
                    let (elem, stride) = self.element(&r.ty)?;
                    let base = self.addr_of(&r)?;
                    let (Val::Scalar(a), Val::Scalar(b)) =
                        (self.read_local(*from)?, self.read_local(*to)?)
                    else {
                        return Err("a part without bounds".to_string());
                    };
                    let scaled = self.b.ins().imul_imm_s(a, i64::from(stride));
                    let addr = self.b.ins().iadd(base, scaled);
                    let len = self.b.ins().isub(b, a);
                    r = PlaceRef {
                        var: None,
                        addr: Some(addr),
                        ty: Ty::Slice(Arc::new(elem)),
                        variant: None,
                        meta: Some(len),
                        vtable: None,
                    };
                }
            }
        }
        Ok(r)
    }

    /// The element type of an array or slice type, and its stride.
    fn element(&mut self, ty: &Ty) -> Result<(Ty, u32), String> {
        if let Ty::Slice(e) = ty {
            let l = self.layout(e)?;
            return Ok(((**e).clone(), l.size.div_ceil(l.align) * l.align));
        }
        let l = self.layout(ty)?;
        let Shape::Array { elem, stride, .. } = &l.shape else {
            return Err(format!("an element of `{ty}`"));
        };
        Ok((elem.clone(), *stride))
    }

    /// The length of the array or slice at a place, as an `int`.
    fn length(&mut self, r: &PlaceRef) -> Result<ir::Value, String> {
        if let Some(m) = r.meta {
            return Ok(m);
        }
        let l = self.layout(&r.ty)?;
        let Shape::Array { len, .. } = l.shape else {
            return Err(format!("the length of a `{}`", r.ty));
        };
        Ok(self.b.ins().iconst(types::I64, i64::from(len)))
    }

    /// `v`, of type `from`, as a `to`: a borrow of an array where a borrow of a slice is wanted
    /// becomes one, with the array's length (the interpreters treat the two alike).
    fn fit(&mut self, v: Val, from: &Ty, to: &Ty) -> Val {
        if let (Val::Scalar(p), Ty::Ref(_, a), Ty::Ref(_, b)) = (v, from, to)
            && let (Ty::Array(_, n), Ty::Slice(_)) = (&**a, &**b)
            && let Ty::Len(k) = **n
        {
            let n = self.b.ins().iconst(types::I64, k);
            return Val::Mem(self.wide(p, n));
        }
        v
    }

    /// A wide borrow, in a new slot: an address and its length or vtable.
    fn wide(&mut self, ptr: ir::Value, meta: ir::Value) -> ir::Value {
        let l = Layout {
            size: 16,
            align: 8,
            shape: Shape::Wide,
            niche: None,
        };
        let a = self.slot(&l);
        let flags = ir::MemFlagsData::trusted();
        self.b.ins().store(flags, ptr, a, 0);
        self.b.ins().store(flags, meta, a, 8);
        a
    }

    fn offset(&mut self, base: ir::Value, off: u32) -> ir::Value {
        if off == 0 {
            base
        } else {
            self.b.ins().iadd_imm_s(base, i64::from(off))
        }
    }

    /// The address of a place in memory, or, for a value without bytes, one that is never read.
    fn addr_or_dangling(&mut self, r: &PlaceRef) -> Result<ir::Value, String> {
        match (r.addr, r.var) {
            (Some(a), _) => Ok(a),
            (None, None) => {
                let ptr = self.ptr();
                Ok(self.b.ins().iconst(ptr, 8))
            }
            (None, Some(_)) => Err("dropping a value kept in a register".to_string()),
        }
    }

    /// The address of a place that is in memory.
    fn addr_of(&self, r: &PlaceRef) -> Result<ir::Value, String> {
        r.addr
            .ok_or_else(|| "the address of a value kept in a register".to_string())
    }

    fn read_local(&mut self, l: Local) -> Result<Val, String> {
        let r = self.place(&Place::local(l))?;
        self.read(&r)
    }

    /// The value at a place.
    fn read(&mut self, r: &PlaceRef) -> Result<Val, String> {
        if let Some(v) = r.var {
            return Ok(Val::Scalar(self.b.use_var(v)));
        }
        let l = self.layout(&r.ty)?;
        Ok(match (&l.shape, r.addr) {
            (Shape::Empty, _) | (_, None) => Val::Unit,
            _ if l.size == 0 => Val::Unit,
            (Shape::Scalar(t), Some(a)) => {
                Val::Scalar(self.b.ins().load(*t, ir::MemFlagsData::trusted(), a, 0))
            }
            (_, Some(a)) => Val::Mem(a),
        })
    }

    /// Stores `v` at the place.
    fn write(&mut self, r: &PlaceRef, v: Val) -> Result<(), String> {
        if let (Some(var), Val::Scalar(x)) = (r.var, v) {
            self.b.def_var(var, x);
            return Ok(());
        }
        let Some(dest) = r.addr else {
            return Ok(());
        };
        match v {
            Val::Unit => {}
            Val::Scalar(x) => {
                self.b.ins().store(ir::MemFlagsData::trusted(), x, dest, 0);
            }
            Val::Mem(src) => {
                let l = self.layout(&r.ty)?;
                self.copy(dest, src, l.size);
            }
        }
        Ok(())
    }

    /// Copies `size` bytes from `src` to `dest`, which do not overlap.
    fn copy(&mut self, dest: ir::Value, src: ir::Value, size: u32) {
        copy_bytes(&mut self.b, dest, src, size);
    }

    // ----- operands --------------------------------------------------------------------------

    fn operand_ty(&mut self, o: &Operand) -> Result<Ty, String> {
        Ok(match o {
            Operand::Copy(p) | Operand::Move(p) => self.place(p)?.ty,
            Operand::Const(Const::Int(_)) => Ty::Int,
            Operand::Const(Const::Num(n, _)) => Ty::Num(*n),
            Operand::Const(Const::Bool(_)) => Ty::Bool,
            Operand::Const(Const::Unit) => Ty::Unit,
            Operand::Const(Const::Char(_)) => Ty::Char,
            Operand::Const(Const::Str(_)) => Ty::reference(false, Ty::Str),
        })
    }

    fn operand(&mut self, o: &Operand) -> Result<Val, String> {
        Ok(match o {
            Operand::Copy(p) | Operand::Move(p) => {
                let r = self.place(p)?;
                let v = self.read(&r)?;
                if let Operand::Move(p) = o {
                    self.set_flag(p, false);
                }
                v
            }
            Operand::Const(c) => match *c {
                Const::Int(v) => Val::Scalar(self.b.ins().iconst(types::I64, v)),
                Const::Bool(v) => Val::Scalar(self.b.ins().iconst(types::I8, i64::from(v))),
                Const::Char(c) => {
                    Val::Scalar(self.b.ins().iconst(types::I32, i64::from(u32::from(c))))
                }
                Const::Unit => Val::Unit,
                Const::Num(n, bits) => Val::Scalar(self.num_const(n, bits)?),
                Const::Str(i) => {
                    let text = self.body.strs[i as usize].clone();
                    let data = self.c.text_data(&text)?;
                    let gv = self.c.module.declare_data_in_func(data, self.b.func);
                    let ptr = self.ptr();
                    let p = self.b.ins().symbol_value(ptr, gv);
                    let n = self.b.ins().iconst(types::I64, text.len() as i64);
                    Val::Mem(self.wide(p, n))
                }
            },
        })
    }

    fn num_const(&mut self, n: Num, bits: u128) -> Result<ir::Value, String> {
        let t = clif_type(&Ty::Num(n))?.expect("a number type");
        Ok(self.bits_const(t, bits))
    }

    fn bits_const(&mut self, t: ir::Type, bits: u128) -> ir::Value {
        match t {
            types::F32 => self
                .b
                .ins()
                .f32const(ir::immediates::Ieee32::with_bits(bits as u32)),
            types::F64 => self
                .b
                .ins()
                .f64const(ir::immediates::Ieee64::with_bits(bits as u64)),
            types::I128 => {
                let lo = self.b.ins().iconst(types::I64, bits as u64 as i64);
                let hi = self.b.ins().iconst(types::I64, (bits >> 64) as u64 as i64);
                self.b.ins().iconcat(lo, hi)
            }
            t => self.b.ins().iconst(t, bits as u64 as i64),
        }
    }

    fn value(&mut self, o: &Operand) -> Result<ir::Value, String> {
        match self.operand(o)? {
            Val::Scalar(v) => Ok(v),
            _ => Err("an operator on a value that is not a scalar".to_string()),
        }
    }

    // ----- sites and traps -------------------------------------------------------------------

    /// A new site, and its number as a constant.
    fn site(&mut self, s: Site) -> ir::Value {
        self.c.sites.push(s);
        let n = (self.c.sites.len() - 1) as i64;
        self.b.ins().iconst(types::I32, n)
    }

    /// Records a trap at `site` and returns the trapped status.
    fn trap(&mut self, class: TrapClass, range: TextRange) -> Result<(), String> {
        self.trap_in(class, self.file, range)
    }

    /// As [`Lower::trap`], at `range` of file `file`.
    fn trap_in(&mut self, class: TrapClass, file: u32, range: TextRange) -> Result<(), String> {
        let site = self.site(Site::Trap(class, file, range));
        // A broken precondition is the caller's, wherever it is checked.
        let rt = if self.tracked || class == TrapClass::Precondition {
            self.c.rt.trap_tracked
        } else {
            self.c.rt.trap
        };
        let f = self.c.module.declare_func_in_func(rt, self.b.func);
        let call = self.b.ins().call(f, &[site]);
        let status = self.b.inst_results(call)[0];
        self.not_returned(status)
    }

    /// Goes on after `status` if it is 0; otherwise passes it on, as [`Lower::not_returned`].
    fn passed_on(&mut self, status: ir::Value) -> Result<(), String> {
        let ok = self.b.create_block();
        let not = self.b.create_block();
        self.b.ins().brif(status, not, &[], ok, &[]);
        self.b.switch_to_block(not);
        self.not_returned(status)?;
        self.b.switch_to_block(ok);
        Ok(())
    }

    /// After a status other than 0 or 2: 3 unwinds from here, anything else (a trap) is passed
    /// on.
    fn not_returned(&mut self, status: ir::Value) -> Result<(), String> {
        let unwind = self.b.create_block();
        let trapped = self.b.create_block();
        let is = self.b.ins().icmp_imm_s(IntCC::Equal, status, 3);
        self.b.ins().brif(is, unwind, &[], trapped, &[]);
        self.b.switch_to_block(trapped);
        self.return_status(1)?;
        self.b.switch_to_block(unwind);
        self.unwind()
    }

    /// Unwinds from the point being lowered: its cleanup code, which ends by returning 3.
    fn unwind(&mut self) -> Result<(), String> {
        if self.body.cleanups.is_empty() {
            return self.return_status(3);
        }
        let (b, k) = self.point;
        let target = self.body.cleanup_at(b, k);
        match self.blocks.get(target.index()).copied().flatten() {
            Some(block) => {
                self.b.ins().jump(block, &[]);
                Ok(())
            }
            None => self.return_status(3),
        }
    }

    /// Returns `status`, with a zero result (the caller does not read it).
    fn return_status(&mut self, status: i64) -> Result<(), String> {
        let s = self.b.ins().iconst(types::I8, status);
        let mut rets = vec![s];
        let ret = self.tys[0].clone();
        match self.c.pass(&ret)? {
            Pass::Scalar(t) => rets.push(self.zero(t)),
            Pass::Words(ts) => {
                for t in ts {
                    rets.push(self.zero(t));
                }
            }
            Pass::Nothing | Pass::Indirect => {}
        }
        self.b.ins().return_(&rets);
        Ok(())
    }

    // ----- statements ------------------------------------------------------------------------

    fn statement(&mut self, s: &Statement) -> Result<(), String> {
        match s {
            Statement::Assign(place, r) => self.assign(place, r)?,
            Statement::Assert {
                cond,
                expected,
                trap,
                range,
                parts,
            } => {
                let c = self.value(cond)?;
                let ok = self.b.create_block();
                let fail = self.b.create_block();
                if *expected {
                    self.b.ins().brif(c, ok, &[], fail, &[]);
                } else {
                    self.b.ins().brif(c, fail, &[], ok, &[]);
                }
                self.b.switch_to_block(fail);
                // Each part of the condition, with its value if it was evaluated, for the
                // report.
                for (k, p) in parts.iter().enumerate() {
                    let Val::Scalar(evaluated) = self.read_local(p.evaluated)? else {
                        continue;
                    };
                    let yes = self.b.create_block();
                    let no = self.b.create_block();
                    let next = self.b.create_block();
                    self.b.ins().brif(evaluated, yes, &[], no, &[]);
                    self.b.switch_to_block(yes);
                    // Its own local, or where it lives if it was borrowed there.
                    let r = self.place(&p.value)?;
                    let ty = r.ty.clone();
                    let v = self.read(&r)?;
                    let addr = self.spilled(v, &ty)?;
                    let site = self.site(Site::Part(p.range, Some(ty), k == 0));
                    self.observe_call(site, addr);
                    self.b.ins().jump(next, &[]);
                    self.b.switch_to_block(no);
                    let ptr = self.ptr();
                    let null = self.b.ins().iconst(ptr, 0);
                    let site = self.site(Site::Part(p.range, None, k == 0));
                    self.observe_call(site, null);
                    self.b.ins().jump(next, &[]);
                    self.b.switch_to_block(next);
                }
                self.trap(*trap, *range)?;
                self.b.switch_to_block(ok);
            }
            Statement::Read(_) | Statement::StorageDead(_) | Statement::AllocTaint { .. } => {}
            Statement::Observe { range, value } => {
                let ty = self.operand_ty(value)?;
                let v = self.operand(value)?;
                let addr = self.spilled(v, &ty)?;
                let site = self.site(Site::Observe(self.file, *range, ty));
                self.observe_call(site, addr);
            }
        }
        Ok(())
    }

    /// The address of value `v` of type `ty`: its memory, a slot it is stored in, or null for
    /// one without bytes.
    fn spilled(&mut self, v: Val, ty: &Ty) -> Result<ir::Value, String> {
        Ok(match v {
            Val::Mem(a) => a,
            Val::Scalar(x) => {
                let l = self.layout(ty)?;
                let a = self.slot(&l);
                self.b.ins().store(ir::MemFlagsData::trusted(), x, a, 0);
                a
            }
            Val::Unit => {
                let ptr = self.ptr();
                self.b.ins().iconst(ptr, 0)
            }
        })
    }

    /// Hands the value at `addr` to the run's observer, for `site`.
    fn observe_call(&mut self, site: ir::Value, addr: ir::Value) {
        let f = self
            .c
            .module
            .declare_func_in_func(self.c.rt.observe, self.b.func);
        self.b.ins().call(f, &[site, addr]);
    }

    fn assign(&mut self, place: &Place, r: &Rvalue) -> Result<(), String> {
        self.assign_value(place, r)?;
        self.set_init(place);
        Ok(())
    }

    fn assign_value(&mut self, place: &Place, r: &Rvalue) -> Result<(), String> {
        let dest = self.place(place)?;
        match r {
            Rvalue::Use(o) => {
                let v = self.operand(o)?;
                let from = self.operand_ty(o)?;
                // An error moved between an error set and a set or type of its members (the
                // MIR gives a `try` block's or a caller's view of the errors as it is).
                if from != dest.ty
                    && (matches!(from, Ty::Errors(_)) || matches!(dest.ty, Ty::Errors(_)))
                {
                    return self.convert_error(v, &from, &dest);
                }
                let v = self.fit(v, &from, &dest.ty);
                self.write(&dest, v)
            }
            Rvalue::Aggregate(kind, ops) => self.aggregate(&dest, kind, ops),
            Rvalue::Ref(_, p) => {
                let target = self.place(p)?;
                if let (Some(a), Some(m)) = (target.addr, target.meta) {
                    let w = self.wide(a, m);
                    return self.write(&dest, Val::Mem(w));
                }
                let a = match target.addr {
                    Some(a) => a,
                    // A borrow of something without bytes points nowhere in particular.
                    None if target.var.is_none() => {
                        let ptr = self.ptr();
                        self.b.ins().iconst(ptr, 8)
                    }
                    None => return Err("a borrow of a value kept in a register".to_string()),
                };
                let from = Ty::reference(false, target.ty.clone());
                let v = self.fit(Val::Scalar(a), &from, &dest.ty);
                self.write(&dest, v)
            }
            // Whether the error is of struct or enum `n`: its id, or its type.
            Rvalue::ErrorIs(p, n) => {
                let target = self.place(p)?;
                let v = match &target.ty {
                    Ty::Errors(_) => {
                        let a = self.addr_of(&target)?;
                        let id = self
                            .b
                            .ins()
                            .load(types::I64, ir::MemFlagsData::trusted(), a, 0);
                        self.b.ins().icmp_imm_s(IntCC::Equal, id, i64::from(*n))
                    }
                    t => {
                        let is = t.adt() == Some(*n);
                        self.b.ins().iconst(types::I8, i64::from(is))
                    }
                };
                self.write(&dest, Val::Scalar(v))
            }
            // A function's address as a C function pointer (`type.fnptr.1`): a C function's
            // own, or a Moddil function's C-ABI wrapper's.
            Rvalue::FnPtr(f) => {
                let id = if self.c.items.fns[*f as usize].foreign {
                    self.c.foreign_fn(*f)?
                } else {
                    self.c.callback_fn(*f)?
                };
                let fref = self.c.module.declare_func_in_func(id, self.b.func);
                let a = self.b.ins().func_addr(types::I64, fref);
                self.write(&dest, Val::Scalar(a))
            }
            Rvalue::StaticRef(f) => {
                let a = self.static_ref(*f)?;
                self.write(&dest, Val::Scalar(a))
            }
            // A `const { }` block's value, computed when compiling (`comptime.block.2`).
            Rvalue::StaticBlock(item) => {
                let Ty::Ref(_, ty) = &dest.ty else {
                    return Err("a block's static data, not by a borrow".to_string());
                };
                let ty = (**ty).clone();
                let data = self.c.frozen_block(*item, &ty)?;
                let gv = self.c.module.declare_data_in_func(data, self.b.func);
                let ptr = self.ptr();
                let a = self.b.ins().symbol_value(ptr, gv);
                self.write(&dest, Val::Scalar(a))
            }
            Rvalue::Discriminant(p) => {
                let target = self.place(p)?;
                let d = self.discriminant(&target)?;
                self.write(&dest, Val::Scalar(d))
            }
            Rvalue::Repeat(o, n) => {
                let l = self.layout(&dest.ty)?;
                let Shape::Array { elem, stride, len } = &l.shape else {
                    return Err(format!("`[x; {n}]` as a `{}`", dest.ty));
                };
                let (elem, stride, len) = (elem.clone(), *stride, *len);
                let v = self.operand(o)?;
                let base = self.addr_of(&dest)?;
                for k in 0..len {
                    let addr = self.offset(base, k * stride);
                    let at = PlaceRef::at(addr, elem.clone());
                    self.write(&at, v)?;
                }
                Ok(())
            }
            Rvalue::Len(p) => {
                let target = self.place(p)?;
                let v = self.length(&target)?;
                self.write(&dest, Val::Scalar(v))
            }
            Rvalue::InBounds(i, p) => {
                let target = self.place(p)?;
                let len = self.length(&target)?;
                let i = self.value(i)?;
                // One unsigned comparison: a negative index is a large one.
                let ok = self.b.ins().icmp(IntCC::UnsignedLessThan, i, len);
                self.write(&dest, Val::Scalar(ok))
            }
            Rvalue::SpanOk(a, b, p) => {
                let target = self.place(p)?;
                let len = self.length(&target)?;
                let (a, b) = (self.value(a)?, self.value(b)?);
                // `0 <= a <= b <= len`, unsigned: a negative bound is a large one.
                let x = self.b.ins().icmp(IntCC::UnsignedLessThanOrEqual, a, b);
                let y = self.b.ins().icmp(IntCC::UnsignedLessThanOrEqual, b, len);
                let ok = self.b.ins().band(x, y);
                self.write(&dest, Val::Scalar(ok))
            }
            Rvalue::Length(n) => {
                let Ty::Len(k) = self.c.concrete(n, self.targs) else {
                    return Err(format!("the length `{n}`"));
                };
                let v = self.b.ins().iconst(types::I64, k);
                self.write(&dest, Val::Scalar(v))
            }
            Rvalue::Unsize(o, from)
                if matches!(self.c.concrete(from, self.targs),
                    Ty::Adt(b, ..) if Some(b) == self.c.items.lang("Box")) =>
            {
                // `Box<T>` as `Box<dyn Trait>`: its `Buf` takes the vtable of `T`.
                let from = self.c.concrete(from, self.targs);
                let Ty::Adt(_, _, args) = &from else {
                    unreachable!()
                };
                let inner = args.first().cloned().ok_or("a box without its type")?;
                let Ty::Adt(_, _, to) = &dest.ty else {
                    return Err(format!("a `{from}` as a `{}`", dest.ty));
                };
                let Some(Ty::Dyn(tr, _)) = to.first() else {
                    return Err(format!("a `{from}` as a `{}`", dest.ty));
                };
                let tr = tr.clone();
                let Val::Mem(src) = self.operand(o)? else {
                    return Err("a box without its value".to_string());
                };
                let a = self.addr_of(&dest)?;
                self.copy(a, src, 24);
                let vt = self.c.vtable(&inner, &tr)?;
                let gv = self.c.module.declare_data_in_func(vt, self.b.func);
                let ptr = self.ptr();
                let v = self.b.ins().symbol_value(ptr, gv);
                self.b.ins().store(ir::MemFlagsData::trusted(), v, a, 24);
                Ok(())
            }
            Rvalue::Unsize(o, from) => {
                let from = self.c.concrete(from, self.targs);
                let v = self.operand(o)?;
                let thin = match (v, &from) {
                    // Already a `dyn` borrow (of a subtrait): the value, with the supertrait's
                    // vtable (`type.dyn.coerce.1`).
                    (Val::Mem(_), Ty::Dyn(sub, _)) => {
                        let Ty::Ref(_, to) = &dest.ty else {
                            return Err(format!("a `{from}` as a `{}`", dest.ty));
                        };
                        let Ty::Dyn(sup, _) = &**to else {
                            return Err(format!("a `{from}` as a `{}`", dest.ty));
                        };
                        // The supertrait's own vtable, from the word after the methods that
                        // holds it, with the same value. (Even the first supertrait's: its
                        // methods are a prefix of the subtrait's, but its own supertraits'
                        // words are not where they are in the subtrait's vtable.)
                        let methods = self.c.dyn_methods(&from, sub).len();
                        let supers = self.c.dyn_supertraits(&from, sub);
                        let Some(k) = supers.iter().position(|t| t == sup) else {
                            return Err(format!("a `dyn {}` as a `dyn {}`", sub.name, sup.name));
                        };
                        let Val::Mem(w) = v else {
                            unreachable!("matched above")
                        };
                        let flags = ir::MemFlagsData::trusted();
                        let data = self.b.ins().load(types::I64, flags, w, 0);
                        let old = self.b.ins().load(types::I64, flags, w, 8);
                        let at = 8 * (VTABLE_HEADER + methods + k) as i32;
                        let vt = self.b.ins().load(types::I64, flags, old, at);
                        let wide = self.wide(data, vt);
                        return self.write(&dest, Val::Mem(wide));
                    }
                    (Val::Scalar(p), _) => p,
                    _ => return Err(format!("a `{from}` as a `{}`", dest.ty)),
                };
                let meta = match (&from, &dest.ty) {
                    (Ty::Array(_, n), _) => {
                        let Ty::Len(k) = **n else {
                            return Err(format!("an array of length `{n}`"));
                        };
                        self.b.ins().iconst(types::I64, k)
                    }
                    (_, Ty::Ref(_, to)) if matches!(**to, Ty::Dyn(..)) => {
                        let Ty::Dyn(tr, _) = &**to else {
                            unreachable!()
                        };
                        let vt = self.c.vtable(&from, tr)?;
                        let gv = self.c.module.declare_data_in_func(vt, self.b.func);
                        let ptr = self.ptr();
                        self.b.ins().symbol_value(ptr, gv)
                    }
                    _ => return Err(format!("a `{from}` as a `{}`", dest.ty)),
                };
                let w = self.wide(thin, meta);
                self.write(&dest, Val::Mem(w))
            }
            r => {
                let v = self.scalar_rvalue(r)?;
                self.write(&dest, v.map_or(Val::Unit, Val::Scalar))
            }
        }
    }

    /// A tuple, array, struct or variant built at `dest` from its fields.
    fn aggregate(
        &mut self,
        dest: &PlaceRef,
        kind: &AggregateKind,
        ops: &[Operand],
    ) -> Result<(), String> {
        let l = self.layout(&dest.ty)?;
        if l.size == 0 {
            for o in ops {
                self.operand(o)?;
            }
            return Ok(());
        }
        let base = self.addr_of(dest)?;
        let (fields, variant): (Vec<(u32, Ty)>, Option<u32>) = match (&l.shape, kind) {
            // A tuple, a struct, or a closure's captures.
            (Shape::Fields(fs), _) => (fs.clone(), None),
            (Shape::Array { elem, stride, len }, AggregateKind::Array) => (
                (0..*len).map(|k| (k * stride, elem.clone())).collect(),
                None,
            ),
            (Shape::Enum { variants, .. }, AggregateKind::Adt { variant, .. }) => {
                (variants[*variant as usize].clone(), Some(*variant))
            }
            _ => return Err(format!("building a `{}`", dest.ty)),
        };
        for ((off, ty), o) in fields.iter().zip(ops) {
            let from = self.operand_ty(o)?;
            let v = self.operand(o)?;
            let addr = self.offset(base, *off);
            let at = PlaceRef::at(addr, ty.clone());
            // An error given as a field of another view of the errors (`Err(e)`).
            if from != *ty && (matches!(from, Ty::Errors(_)) || matches!(ty, Ty::Errors(_))) {
                self.convert_error(v, &from, &at)?;
                continue;
            }
            self.write(&at, v)?;
        }
        if let (Some(v), Shape::Enum { tag, .. }) = (variant, &l.shape) {
            self.set_variant(base, tag, v);
        }
        Ok(())
    }

    /// Marks the enum at `base` as holding variant `v`.
    fn set_variant(&mut self, base: ir::Value, tag: &Tag, v: u32) {
        let flags = ir::MemFlagsData::trusted();
        match tag {
            Tag::None => {}
            Tag::Direct { offset, ty } => {
                let t = self.bits_const(*ty, u128::from(v));
                self.b.ins().store(flags, t, base, *offset as i32);
            }
            Tag::Niche { dataful, niche } => {
                if v != *dataful {
                    let rank = if v < *dataful { v } else { v - 1 };
                    let t = self.bits_const(niche.ty, niche.first + u128::from(rank));
                    self.b.ins().store(flags, t, base, niche.offset as i32);
                }
            }
        }
    }

    /// Which variant the enum at the place holds, as an `int`.
    fn discriminant(&mut self, r: &PlaceRef) -> Result<ir::Value, String> {
        let l = self.layout(&r.ty)?;
        let Shape::Enum { tag, variants } = &l.shape else {
            return Ok(self.b.ins().iconst(types::I64, 0));
        };
        let n = variants.len() as i64;
        let flags = ir::MemFlagsData::trusted();
        Ok(match tag {
            Tag::None => self.b.ins().iconst(types::I64, 0),
            Tag::Direct { offset, ty } => {
                let base = self.addr_of(r)?;
                let t = self.b.ins().load(*ty, flags, base, *offset as i32);
                self.b.ins().uextend(types::I64, t)
            }
            Tag::Niche { dataful, niche } => {
                let base = self.addr_of(r)?;
                let raw = self
                    .b
                    .ins()
                    .load(niche.ty, flags, base, niche.offset as i32);
                let first = self.bits_const(niche.ty, niche.first);
                let rel = self.b.ins().isub(raw, first);
                let rel = if niche.ty == types::I64 {
                    rel
                } else {
                    self.b.ins().uextend(types::I64, rel)
                };
                // One of the variants without a payload, or else the one with it.
                let other = self.b.ins().icmp_imm_s(IntCC::UnsignedLessThan, rel, n - 1);
                let past = self.b.ins().icmp_imm_s(
                    IntCC::UnsignedGreaterThanOrEqual,
                    rel,
                    i64::from(*dataful),
                );
                let past = self.b.ins().uextend(types::I64, past);
                let v = self.b.ins().iadd(rel, past);
                let d = self.b.ins().iconst(types::I64, i64::from(*dataful));
                self.b.ins().select(other, v, d)
            }
        })
    }

    fn scalar_rvalue(&mut self, r: &Rvalue) -> Result<Option<ir::Value>, String> {
        Ok(Some(match r {
            Rvalue::Unary(op, a) => {
                let ty = self.operand_ty(a)?;
                let v = self.value(a)?;
                match (op, &ty) {
                    (UnOp::Neg, t) if is_float(t) => self.b.ins().fneg(v),
                    (UnOp::Neg, _) => self.b.ins().ineg(v),
                    (UnOp::Not, Ty::Bool) => self.b.ins().bxor_imm_u(v, 1),
                    (UnOp::Not, _) => self.b.ins().bnot(v),
                }
            }
            // `()` equals itself.
            Rvalue::Binary(op, a, _) if self.operand_ty(a)? == Ty::Unit => {
                let eq = matches!(op, BinOp::Eq | BinOp::Le | BinOp::Ge);
                self.b.ins().iconst(types::I8, i64::from(eq))
            }
            Rvalue::Binary(op, a, b) if matches!(self.operand_ty(a)?, Ty::Ref(_, ref t) if **t == Ty::Str) =>
            {
                let (p, n) = self.text(a)?;
                let (q, m) = self.text(b)?;
                let c = self.rt_call(self.c.rt.str_cmp, &[p, n, q, m]);
                let cc = match op {
                    BinOp::Eq => IntCC::Equal,
                    BinOp::Ne => IntCC::NotEqual,
                    BinOp::Lt => IntCC::SignedLessThan,
                    BinOp::Le => IntCC::SignedLessThanOrEqual,
                    BinOp::Gt => IntCC::SignedGreaterThan,
                    BinOp::Ge => IntCC::SignedGreaterThanOrEqual,
                    op => return Err(format!("`{op:?}` on text")),
                };
                self.b.ins().icmp_imm_s(cc, c, 0)
            }
            Rvalue::Binary(op, a, b) => {
                let ty = self.operand_ty(a)?;
                let (x, y) = (self.value(a)?, self.value(b)?);
                self.binary(*op, &ty, x, y)?
            }
            Rvalue::Overflows(op, a, b) => {
                let ty = self.operand_ty(a)?;
                let (x, y) = (self.value(a)?, self.value(b)?);
                self.overflows(*op, &ty, x, y)?
            }
            Rvalue::NegOverflows(a) => {
                let ty = self.operand_ty(a)?;
                let v = self.value(a)?;
                if signed(&ty) {
                    let min = self.min(&ty)?;
                    self.b.ins().icmp(IntCC::Equal, v, min)
                } else {
                    self.b.ins().iconst(types::I8, 0)
                }
            }
            Rvalue::Widen(a, to) => {
                let from = self.operand_ty(a)?;
                let v = self.value(a)?;
                let to = self.c.concrete(to, self.targs);
                self.widen(&from, &to, v)?
            }
            _ => return Err(format!("this operation (`{r}`)")),
        }))
    }

    /// The least value of signed integer type `ty`.
    fn min(&mut self, ty: &Ty) -> Result<ir::Value, String> {
        let t = clif_type(ty)?.expect("an integer");
        Ok(if t == types::I128 {
            let lo = self.b.ins().iconst(types::I64, 0);
            let hi = self.b.ins().iconst(types::I64, i64::MIN);
            self.b.ins().iconcat(lo, hi)
        } else {
            let w = bits(ty);
            self.b.ins().iconst(t, (1i64 << (w - 1)).wrapping_neg())
        })
    }

    fn binary(
        &mut self,
        op: BinOp,
        ty: &Ty,
        x: ir::Value,
        y: ir::Value,
    ) -> Result<ir::Value, String> {
        let ins = self.b.ins();
        if is_float(ty) {
            return Ok(match op {
                BinOp::Add => ins.fadd(x, y),
                BinOp::Sub => ins.fsub(x, y),
                BinOp::Mul => ins.fmul(x, y),
                BinOp::Div => ins.fdiv(x, y),
                BinOp::Rem => {
                    let f = if *ty == Ty::Num(Num::F32) {
                        self.c.rt.fmodf
                    } else {
                        self.c.rt.fmod
                    };
                    let f = self.c.module.declare_func_in_func(f, self.b.func);
                    let call = self.b.ins().call(f, &[x, y]);
                    self.b.inst_results(call)[0]
                }
                BinOp::Eq => ins.fcmp(FloatCC::Equal, x, y),
                BinOp::Ne => ins.fcmp(FloatCC::NotEqual, x, y),
                BinOp::Lt => ins.fcmp(FloatCC::LessThan, x, y),
                BinOp::Le => ins.fcmp(FloatCC::LessThanOrEqual, x, y),
                BinOp::Gt => ins.fcmp(FloatCC::GreaterThan, x, y),
                BinOp::Ge => ins.fcmp(FloatCC::GreaterThanOrEqual, x, y),
                op => return Err(format!("`{op:?}` on floats")),
            });
        }
        let s = signed(ty);
        if matches!(op, BinOp::Div | BinOp::Rem) && clif_type(ty)? == Some(types::I128) {
            let code = match (op, s) {
                (BinOp::Div, true) => 0,
                (BinOp::Div, false) => 1,
                (_, true) => 2,
                (_, false) => 3,
            };
            return Ok(self.i128_op(code, x, y, true));
        }
        let ins = self.b.ins();
        let cmp = |s: bool, a: IntCC, b: IntCC| if s { a } else { b };
        Ok(match op {
            BinOp::Add => ins.iadd(x, y),
            BinOp::Sub => ins.isub(x, y),
            BinOp::Mul => ins.imul(x, y),
            BinOp::Div if s => ins.sdiv(x, y),
            BinOp::Div => ins.udiv(x, y),
            BinOp::Rem if s => ins.srem(x, y),
            BinOp::Rem => ins.urem(x, y),
            BinOp::BitAnd => ins.band(x, y),
            BinOp::BitOr => ins.bor(x, y),
            BinOp::BitXor => ins.bxor(x, y),
            BinOp::Shl => ins.ishl(x, y),
            BinOp::Shr if s => ins.sshr(x, y),
            BinOp::Shr => ins.ushr(x, y),
            BinOp::Eq => ins.icmp(IntCC::Equal, x, y),
            BinOp::Ne => ins.icmp(IntCC::NotEqual, x, y),
            BinOp::Lt => ins.icmp(cmp(s, IntCC::SignedLessThan, IntCC::UnsignedLessThan), x, y),
            BinOp::Le => ins.icmp(
                cmp(
                    s,
                    IntCC::SignedLessThanOrEqual,
                    IntCC::UnsignedLessThanOrEqual,
                ),
                x,
                y,
            ),
            BinOp::Gt => ins.icmp(
                cmp(s, IntCC::SignedGreaterThan, IntCC::UnsignedGreaterThan),
                x,
                y,
            ),
            BinOp::Ge => ins.icmp(
                cmp(
                    s,
                    IntCC::SignedGreaterThanOrEqual,
                    IntCC::UnsignedGreaterThanOrEqual,
                ),
                x,
                y,
            ),
        })
    }

    /// The runtime's `i128` operation `code` on `x` and `y` (`moddil_rt::num::wide`): its
    /// result as an `i128` if `both`, or just its low word.
    fn i128_op(&mut self, code: i64, x: ir::Value, y: ir::Value, both: bool) -> ir::Value {
        let (xl, xh) = self.b.ins().isplit(x);
        let (yl, yh) = self.b.ins().isplit(y);
        let op = self.b.ins().iconst(types::I32, code);
        let args = [op, xl, xh, yl, yh];
        let f = self
            .c
            .module
            .declare_func_in_func(self.c.rt.i128_lo, self.b.func);
        let call = self.b.ins().call(f, &args);
        let lo = self.b.inst_results(call)[0];
        if !both {
            return lo;
        }
        let f = self
            .c
            .module
            .declare_func_in_func(self.c.rt.i128_hi, self.b.func);
        let call = self.b.ins().call(f, &args);
        let hi = self.b.inst_results(call)[0];
        self.b.ins().iconcat(lo, hi)
    }

    /// Whether `x op y` overflows, or shifts by an amount out of range (as the interpreters'
    /// `overflows`).
    fn overflows(
        &mut self,
        op: BinOp,
        ty: &Ty,
        x: ir::Value,
        y: ir::Value,
    ) -> Result<ir::Value, String> {
        if is_float(ty) {
            return Ok(self.b.ins().iconst(types::I8, 0));
        }
        let s = signed(ty);
        if op == BinOp::Mul && clif_type(ty)? == Some(types::I128) {
            let flag = self.i128_op(if s { 4 } else { 5 }, x, y, false);
            return Ok(self.b.ins().icmp_imm_u(IntCC::NotEqual, flag, 0));
        }
        Ok(match op {
            BinOp::Add if s => self.b.ins().sadd_overflow(x, y).1,
            BinOp::Add => self.b.ins().uadd_overflow(x, y).1,
            BinOp::Sub if s => self.b.ins().ssub_overflow(x, y).1,
            BinOp::Sub => self.b.ins().usub_overflow(x, y).1,
            BinOp::Mul if s => self.b.ins().smul_overflow(x, y).1,
            BinOp::Mul => self.b.ins().umul_overflow(x, y).1,
            BinOp::Div => {
                // By zero, or the least value by -1.
                let zero = self.b.ins().icmp_imm_s(IntCC::Equal, y, 0);
                if s {
                    let min = self.min(ty)?;
                    let is_min = self.b.ins().icmp(IntCC::Equal, x, min);
                    let minus_one = self.b.ins().icmp_imm_s(IntCC::Equal, y, -1);
                    let both = self.b.ins().band(is_min, minus_one);
                    self.b.ins().bor(zero, both)
                } else {
                    zero
                }
            }
            BinOp::Shl | BinOp::Shr => {
                let w = i64::from(bits(ty));
                // Out of range: negative, or the width or more (one unsigned comparison).
                let out = self
                    .b
                    .ins()
                    .icmp_imm_s(IntCC::UnsignedGreaterThanOrEqual, y, w);
                if op == BinOp::Shr {
                    out
                } else {
                    // `<<` overflows when shifting back does not give the value (bits, or the
                    // sign, are lost).
                    let shifted = self.b.ins().ishl(x, y);
                    let back = if s {
                        self.b.ins().sshr(shifted, y)
                    } else {
                        self.b.ins().ushr(shifted, y)
                    };
                    let lost = self.b.ins().icmp(IntCC::NotEqual, back, x);
                    self.b.ins().bor(out, lost)
                }
            }
            _ => self.b.ins().iconst(types::I8, 0),
        })
    }

    /// `v` of type `from` as the wider `to` (`num.widen.1`).
    fn widen(&mut self, from: &Ty, to: &Ty, v: ir::Value) -> Result<ir::Value, String> {
        let (f, t) = (
            clif_type(from)?.expect("a number"),
            clif_type(to)?.expect("a number"),
        );
        if f == t && is_float(from) == is_float(to) {
            return Ok(v);
        }
        Ok(match (is_float(from), is_float(to)) {
            (true, true) => self.b.ins().fpromote(t, v),
            (false, true) if signed(from) => self.b.ins().fcvt_from_sint(t, v),
            (false, true) => self.b.ins().fcvt_from_uint(t, v),
            (false, false) if signed(from) => self.b.ins().sextend(t, v),
            (false, false) => self.b.ins().uextend(t, v),
            (true, false) => return Err("a float as an integer".to_string()),
        })
    }

    // ----- terminators -----------------------------------------------------------------------

    /// Before a jump to one of `targets`: if one is a loop's back-edge (a block at or before
    /// this one), a check of the preemption word, calling the runtime to give way when it is set
    /// (`runtime.md`: a long CPU loop never yields).
    fn safe_point(&mut self, targets: &[usize]) -> Result<(), String> {
        let Some((word, give_way)) = self.c.rt.preempt else {
            return Ok(());
        };
        if !self.safe_points || targets.iter().all(|&t| t > self.point.0) {
            return Ok(());
        }
        let ptr = self.ptr();
        let gv = self.c.module.declare_data_in_func(word, self.b.func);
        let at = self.b.ins().symbol_value(ptr, gv);
        let set = self
            .b
            .ins()
            .load(types::I8, ir::MemFlagsData::trusted(), at, 0);
        let slow = self.b.create_block();
        let on = self.b.create_block();
        self.b.set_cold_block(slow);
        self.b.ins().brif(set, slow, &[], on, &[]);
        self.b.switch_to_block(slow);
        let f = self.c.module.declare_func_in_func(give_way, self.b.func);
        self.b.ins().call(f, &[]);
        self.b.ins().jump(on, &[]);
        self.b.seal_block(slow);
        self.b.switch_to_block(on);
        self.b.seal_block(on);
        Ok(())
    }

    fn terminator(&mut self, t: &Terminator, span: TextRange) -> Result<(), String> {
        self.span = span;
        match t {
            Terminator::Goto(b) => {
                self.safe_point(&[b.index()])?;
                let b = self.block(b.index())?;
                self.b.ins().jump(b, &[]);
            }
            Terminator::SwitchBool { cond, then, else_ } => {
                self.safe_point(&[then.index(), else_.index()])?;
                let c = self.value(cond)?;
                let (y, n) = (self.block(then.index())?, self.block(else_.index())?);
                self.b.ins().brif(c, y, &[], n, &[]);
            }
            Terminator::Return => self.ret()?,
            Terminator::Trap(class, _) => self.trap(*class, span)?,
            Terminator::Unreachable => self.return_status(1)?,
            Terminator::Drop {
                place,
                target,
                glue_only,
            } => {
                self.drop_place(place, *glue_only)?;
                let b = self.block(target.index())?;
                self.b.ins().jump(b, &[]);
            }
            Terminator::Call {
                callee,
                args,
                dest,
                target,
                on_error,
                range,
                ..
            } => {
                // An error edge whose error is `!` is never taken.
                let catch = match on_error {
                    Some((p, b)) if self.place(p)?.ty != Ty::Never => Some((p.clone(), *b)),
                    _ => None,
                };
                self.call(callee, args, dest, *target, catch, *range)?;
            }
            Terminator::Fail(e) => {
                // The error, tagged, where the caller said; the site; the failed status.
                let ty = self.operand_ty(e)?;
                let v = self.operand(e)?;
                let to = self.err_ptr.ok_or("an error with nowhere to go")?;
                self.write_error(to, v, &ty)?;
                let site = self.site(Site::Fail(self.file, span));
                let f = self
                    .c
                    .module
                    .declare_func_in_func(self.c.rt.fail_at, self.b.func);
                self.b.ins().call(f, &[site]);
                self.return_status(2)?;
            }
            // Unwinding goes on in the caller.
            Terminator::Resume => self.return_status(3)?,
        }
        Ok(())
    }

    fn ret(&mut self) -> Result<(), String> {
        let s = self.b.ins().iconst(types::I8, 0);
        let mut rets = vec![s];
        let ty = self.tys[0].clone();
        let pass = self.c.pass(&ty)?;
        let r = self.place(&Place::local(Local(0)))?;
        match pass {
            Pass::Scalar(_) => match self.read(&r)? {
                Val::Scalar(v) => rets.push(v),
                _ => return Err("a result without a value".to_string()),
            },
            Pass::Words(ts) => {
                let a = self.addr_of(&r)?;
                let mut at = 0;
                for t in ts {
                    rets.push(self.b.ins().load(t, ir::MemFlagsData::trusted(), a, at));
                    at += t.bytes() as i32;
                }
            }
            // An indirect result is already where the caller wants it.
            Pass::Nothing | Pass::Indirect => {}
        }
        self.b.ins().return_(&rets);
        Ok(())
    }

    /// The arguments' values, as a function with these parameter types takes them.
    fn arguments(&mut self, args: &[Operand], params: &[Ty]) -> Result<Vec<ir::Value>, String> {
        let mut vals = Vec::new();
        for (a, pty) in args.iter().zip(params) {
            let from = self.operand_ty(a)?;
            let v = self.operand(a)?;
            let v = self.fit(v, &from, pty);
            vals.extend(self.pass_value(v, pty)?);
        }
        Ok(vals)
    }

    /// The values that pass `v` as a parameter of type `pty`.
    fn pass_value(&mut self, v: Val, pty: &Ty) -> Result<Vec<ir::Value>, String> {
        let flags = ir::MemFlagsData::trusted();
        let mut vals = Vec::new();
        match (self.c.pass(pty)?, v) {
            (Pass::Nothing, _) => {}
            (Pass::Scalar(_), Val::Scalar(x)) => vals.push(x),
            (Pass::Words(ts), Val::Mem(addr)) => {
                let mut at = 0;
                for t in ts {
                    vals.push(self.b.ins().load(t, flags, addr, at));
                    at += t.bytes() as i32;
                }
            }
            (Pass::Indirect, Val::Mem(addr)) => {
                // The callee's own copy.
                let l = self.layout(pty)?;
                let copy = self.slot(&l);
                self.copy(copy, addr, l.size);
                vals.push(copy);
            }
            _ => return Err(format!("an argument of an unexpected kind, a `{pty}`")),
        }
        Ok(vals)
    }

    fn call(
        &mut self,
        callee: &Callee,
        args: &[Operand],
        dest: &Place,
        target: moddil_mir::BlockId,
        catch: Option<(Place, moddil_mir::BlockId)>,
        range: TextRange,
    ) -> Result<(), String> {
        let cargs: Vec<Ty> = callee
            .args()
            .iter()
            .map(|t| self.c.concrete(t, self.targs))
            .collect();
        let d = self.place(dest)?;
        // After the call, memory to free: a `Box<dyn FnOnce…>` given up.
        let mut free_after = None;
        let (run, vals) = match callee {
            Callee::Fn { func, .. } => {
                // A C function (`ffi.extern.1`): a direct call in the platform's C convention.
                if self.c.items.fns[*func as usize].foreign {
                    self.foreign(*func, args, &d)?;
                    self.set_init(dest);
                    let b = self.block(target.index())?;
                    self.b.ins().jump(b, &[]);
                    return Ok(());
                }
                // A stub mod's function without a body (`mod.stub.1`).
                let sig = &self.c.items.fns[*func as usize];
                if !sig.intrinsic && !sig.has_body {
                    self.trap(TrapClass::Unimplemented, range)?;
                    let dead = self.b.create_block();
                    self.b.switch_to_block(dead);
                    return Ok(());
                }
                if self.c.threads == Threads::Many && self.atomic(*func, &cargs, args, &d)? {
                    self.set_init(dest);
                    let b = self.block(target.index())?;
                    self.b.ins().jump(b, &[]);
                    return Ok(());
                }
                if self.c.items.fns[*func as usize].intrinsic {
                    self.intrinsic(*func, &cargs, args, &d, range, &catch)?;
                    self.set_init(dest);
                    let b = self.block(target.index())?;
                    self.b.ins().jump(b, &[]);
                    return Ok(());
                }
                let (id, params, ret) = self.c.declare_fn(*func, &cargs)?;
                let vals = self.arguments(args, &params)?;
                (Run::Direct(id, ret), vals)
            }
            Callee::Trait { func, .. } => match &cargs[0] {
                // A C function pointer (`type.fnptr.1`): an indirect call in the platform's C
                // convention, of the value with the arguments' tuple.
                Ty::FnPtr(ps, r) => {
                    let (ps, r) = (ps.to_vec(), (**r).clone());
                    self.fn_ptr_call(&ps, &r, args, &d)?;
                    self.set_init(dest);
                    let b = self.block(target.index())?;
                    self.b.ins().jump(b, &[]);
                    return Ok(());
                }
                Ty::Dyn(tr, _) => {
                    let methods = self.c.dyn_methods(&cargs[0], tr);
                    let k = methods
                        .iter()
                        .position(|(m, _)| m == func)
                        .ok_or("a method missing from its vtable")?;
                    let self_ty = self.operand_ty(&args[0])?;
                    let Val::Mem(w) = self.operand(&args[0])? else {
                        return Err("a `dyn` call without a `dyn` borrow".to_string());
                    };
                    let flags = ir::MemFlagsData::trusted();
                    let (thin, vt) = if matches!(self_ty, Ty::Adt(..)) {
                        // A `Box<dyn FnOnce…>`, given up: the value its `Buf` holds, and its
                        // memory freed after.
                        let p = self.b.ins().load(types::I64, flags, w, 0);
                        let vt = self.b.ins().load(types::I64, flags, w, 24);
                        free_after = Some((p, vt));
                        (p, vt)
                    } else {
                        let thin = self.b.ins().load(types::I64, flags, w, 0);
                        (thin, self.b.ins().load(types::I64, flags, w, 8))
                    };
                    let at = 8 * (VTABLE_HEADER + k) as i32;
                    let fp = self.b.ins().load(types::I64, flags, vt, at);
                    // A closure's body takes its needs in its arguments' tuple, so a call
                    // through a `dyn Fn…` gives it the tuple, not the needs the trait's method
                    // is given after it (`expr.closure.stored.1`).
                    let closure = ["Fn", "FnMut", "FnOnce"]
                        .iter()
                        .any(|n| self.c.items.lang(n) == Some(tr.trait_));
                    let rest = if closure {
                        &args[1..args.len().min(2)]
                    } else {
                        &args[1..]
                    };
                    let mut params = vec![Ty::reference(false, Ty::Unit)];
                    for a in rest {
                        params.push(self.operand_ty(a)?);
                    }
                    let mut vals = vec![thin];
                    vals.extend(self.arguments(rest, &params[1..])?);
                    (Run::Indirect(fp, params, d.ty.clone()), vals)
                }
                // A closure called: its body, with its captures (by pointer) and its arguments.
                Ty::Closure(c) => {
                    let (id, params, ret) = self.c.declare_closure(c)?;
                    let captures = match self.operand(&args[0])? {
                        // Through a borrow (`call`, `call_mut`): the closure itself.
                        Val::Scalar(p) => p,
                        // By value (`call_once`): its own copy, which it gives up.
                        Val::Mem(a) => {
                            let l = self.layout(&params[0])?;
                            let copy = self.slot(&l);
                            self.copy(copy, a, l.size);
                            copy
                        }
                        Val::Unit => {
                            let ptr = self.ptr();
                            self.b.ins().iconst(ptr, 8)
                        }
                    };
                    let mut vals = vec![captures];
                    vals.extend(self.arguments(&args[1..], &params[1..])?);
                    (Run::Direct(id, ret), vals)
                }
                // An anonymous impl's method is a closure of its own: its captures are the
                // impl's field, its arguments but `self` a tuple (`expr.anon-impl.1`). One it
                // leaves out is the trait's default, for the impl's type.
                Ty::Anon(a) => {
                    let a = a.clone();
                    match a.methods.iter().position(|(m, _)| m == func) {
                        Some(k) => {
                            let Ty::Closure(c) = a.methods[k].1.clone() else {
                                return Err("an anonymous impl's method".to_string());
                            };
                            let l = self.layout(&cargs[0])?;
                            let Shape::Fields(fs) = &l.shape else {
                                return Err("an anonymous impl's methods".to_string());
                            };
                            let off = fs[k].0;
                            let base = match self.operand(&args[0])? {
                                Val::Scalar(p) | Val::Mem(p) => p,
                                Val::Unit => {
                                    let ptr = self.ptr();
                                    self.b.ins().iconst(ptr, 8)
                                }
                            };
                            let captures = self.offset(base, off);
                            let (id, params, ret) = self.c.declare_closure(&c)?;
                            let tuple = self.tuple_of(&args[1..], &params[1])?;
                            let mut vals = vec![captures];
                            vals.extend(self.pass_value(tuple, &params[1])?);
                            (Run::Direct(id, ret), vals)
                        }
                        None => {
                            let (id, params, ret) = self.c.declare_fn(*func, &cargs)?;
                            let vals = self.arguments(args, &params)?;
                            (Run::Direct(id, ret), vals)
                        }
                    }
                }
                // A method called on an error is the one of the error's own type: each member's,
                // by its id (`expr.closure.fails.2`).
                Ty::Errors(ms) => {
                    let ms = ms.clone();
                    self.error_set_call(*func, &cargs, &ms, args, &d, range, &catch)?;
                    self.set_init(dest);
                    let b = self.block(target.index())?;
                    self.b.ins().jump(b, &[]);
                    return Ok(());
                }
                // A borrow compares as what it points to (`trait.borrow.eq.1`): the method of the
                // type it borrows, each argument (a borrow of a borrow) read once.
                Ty::Ref(_, inner)
                    if ["PartialEq", "Eq", "PartialOrd", "Ord"].iter().any(|n| {
                        let items = self.c.items;
                        items.lang(n).is_some() && items.lang(n) == items.trait_of(*func)
                    }) =>
                {
                    let items = self.c.items;
                    let sig = &items.fns[*func as usize];
                    let t = items.trait_of(*func).ok_or("a trait's method")?;
                    let n = items.traits[t as usize].generics.params.len();
                    let tr = TraitRef {
                        trait_: t,
                        name: items.traits[t as usize].name.as_str().into(),
                        args: cargs[1..n].to_vec().into(),
                    };
                    let inner = (**inner).clone();
                    let (f, mut parent) = Solver::new(items, &[])
                        .impl_method(*func, &inner, &tr)
                        .ok_or("a comparison of what a borrow points to")?;
                    parent.extend(cargs[sig.generics.parent as usize..].iter().cloned());
                    // Each argument, `&&T`, read once: a `&T`.
                    let args: Vec<Operand> = args
                        .iter()
                        .map(|a| match a {
                            Operand::Copy(p) | Operand::Move(p) => Operand::Copy(p.clone().deref()),
                            c => c.clone(),
                        })
                        .collect();
                    if items.fns[f as usize].intrinsic {
                        self.intrinsic(f, &parent, &args, &d, range, &catch)?;
                        self.set_init(dest);
                        let b = self.block(target.index())?;
                        self.b.ins().jump(b, &[]);
                        return Ok(());
                    }
                    let (id, params, ret) = self.c.declare_fn(f, &parent)?;
                    let vals = self.arguments(&args, &params)?;
                    (Run::Direct(id, ret), vals)
                }
                self_ty => {
                    let items = self.c.items;
                    let sig = &items.fns[*func as usize];
                    let t = items.trait_of(*func).ok_or("a trait's method")?;
                    let n = items.traits[t as usize].generics.params.len();
                    let tr = TraitRef {
                        trait_: t,
                        name: items.traits[t as usize].name.as_str().into(),
                        args: cargs[1..n].to_vec().into(),
                    };
                    let found = Solver::new(items, &[]).impl_method(*func, self_ty, &tr);
                    let Some((f, mut parent)) = found else {
                        // The built-in `Clone` of a borrow or tuple: a copy.
                        let Val::Scalar(p) = self.operand(&args[0])? else {
                            return Err("`clone` of a value without bytes".to_string());
                        };
                        let from = PlaceRef::at(p, d.ty.clone());
                        let v = self.read(&from)?;
                        self.write(&d, v)?;
                        self.set_init(dest);
                        let b = self.block(target.index())?;
                        self.b.ins().jump(b, &[]);
                        return Ok(());
                    };
                    parent.extend(cargs[sig.generics.parent as usize..].iter().cloned());
                    if items.fns[f as usize].intrinsic {
                        self.intrinsic(f, &parent, args, &d, range, &catch)?;
                        self.set_init(dest);
                        let b = self.block(target.index())?;
                        self.b.ins().jump(b, &[]);
                        return Ok(());
                    }
                    let (id, params, ret) = self.c.declare_fn(f, &parent)?;
                    let vals = self.arguments(args, &params)?;
                    (Run::Direct(id, ret), vals)
                }
            },
        };
        self.emit_call_catching(run, vals, Some(&d), range, catch)?;
        if let Some((p, vt)) = free_after {
            self.free_dyn(p, vt, None)?;
        }
        self.set_init(dest);
        let b = self.block(target.index())?;
        self.b.ins().jump(b, &[]);
        Ok(())
    }

    /// Calls `run` with `vals`, which cannot fail, counting the frame and passing a trap on; its
    /// result goes to `dest` (or nowhere). The code goes on after a call that returned.
    fn emit_call(
        &mut self,
        run: Run,
        vals: Vec<ir::Value>,
        dest: Option<&PlaceRef>,
        range: TextRange,
    ) -> Result<(), String> {
        self.emit_call_catching(run, vals, dest, range, None)
    }

    /// As [`Lower::emit_call`], for a call whose error (if `catch` says where) goes to an error
    /// place and on to a block.
    fn emit_call_catching(
        &mut self,
        run: Run,
        mut vals: Vec<ir::Value>,
        dest: Option<&PlaceRef>,
        range: TextRange,
        catch: Option<(Place, moddil_mir::BlockId)>,
    ) -> Result<(), String> {
        let ret = match &run {
            Run::Direct(_, r) | Run::Indirect(_, _, r) => r.clone(),
        };
        let pass = self.c.pass(&ret)?;
        if pass == Pass::Indirect {
            let a = match dest {
                Some(d) => self.addr_of(d)?,
                None => {
                    let l = self.layout(&ret)?;
                    self.slot(&l)
                }
            };
            vals.push(a);
        }
        // Where its error goes: a buffer with room for any it can fail with, or nowhere.
        let buffer = match &catch {
            Some((p, _)) => {
                let ty = self.place(p)?.ty;
                let size = self.error_room(&ty)?;
                let l = Layout {
                    size,
                    align: 8,
                    shape: Shape::Wide,
                    niche: None,
                };
                Some(self.slot(&l))
            }
            None => None,
        };
        let err = match buffer {
            Some(b) => b,
            None => {
                let ptr = self.ptr();
                self.b.ins().iconst(ptr, 0)
            }
        };
        vals.push(err);
        // Count the frame, so that recursion without end traps.
        let site = self.site(Site::Call(self.file, range, self.tracked));
        let tracked = self.b.ins().iconst(types::I8, i64::from(self.tracked));
        let enter = self
            .c
            .module
            .declare_func_in_func(self.c.rt.enter, self.b.func);
        let e = self.b.ins().call(enter, &[site, tracked]);
        let deep = self.b.inst_results(e)[0];
        let call_block = self.b.create_block();
        let too_deep = self.b.create_block();
        self.b.ins().brif(deep, too_deep, &[], call_block, &[]);
        self.b.switch_to_block(too_deep);
        self.not_returned(deep)?;
        self.b.switch_to_block(call_block);
        let call = match run {
            Run::Direct(id, _) => {
                let f = self.c.module.declare_func_in_func(id, self.b.func);
                self.b.ins().call(f, &vals)
            }
            Run::Indirect(fp, params, ret) => {
                let sig = self.c.signature(&params, &ret, false)?;
                let sig = self.b.import_signature(sig);
                self.b.ins().call_indirect(sig, fp, &vals)
            }
        };
        let results = self.b.inst_results(call).to_vec();
        let leave = self
            .c
            .module
            .declare_func_in_func(self.c.rt.leave, self.b.func);
        self.b.ins().call(leave, &[]);
        let ok = self.b.create_block();
        let trapped = self.b.create_block();
        self.b.ins().brif(results[0], trapped, &[], ok, &[]);
        self.b.switch_to_block(trapped);
        if let (Some((p, edge)), Some(buf)) = (catch, buffer) {
            // Status 2: the error, into its place, then the error edge; else a trap.
            let failed = self.b.create_block();
            let trap = self.b.create_block();
            let is = self.b.ins().icmp_imm_s(IntCC::Equal, results[0], 2);
            self.b.ins().brif(is, failed, &[], trap, &[]);
            self.b.switch_to_block(failed);
            self.read_error(buf, &p)?;
            self.set_init(&p);
            let e = self.block(edge.index())?;
            self.b.ins().jump(e, &[]);
            self.b.switch_to_block(trap);
        }
        self.not_returned(results[0])?;
        self.b.switch_to_block(ok);
        let Some(d) = dest else {
            return Ok(());
        };
        match pass {
            Pass::Scalar(_) => self.write(d, Val::Scalar(results[1]))?,
            Pass::Words(_) => {
                let a = self.addr_of(d)?;
                store_pass(&mut self.b, &pass, &results[1..], a);
            }
            Pass::Nothing | Pass::Indirect => {}
        }
        Ok(())
    }

    // ----- errors ----------------------------------------------------------------------------

    /// How many bytes an error of type `ty` (a set, or one type) takes in the error channel.
    fn error_room(&mut self, ty: &Ty) -> Result<u32, String> {
        let l = self.layout(ty)?;
        Ok(match ty {
            Ty::Errors(_) => l.size,
            _ => ERROR_VALUE + l.size,
        })
    }

    /// Writes the error `v` of type `ty` at `to`, tagged: a set as it is, a value after its
    /// type's id.
    fn write_error(&mut self, to: ir::Value, v: Val, ty: &Ty) -> Result<(), String> {
        if let Ty::Errors(ms) = ty {
            // Only as much as the member it holds: the caller's buffer may be for fewer of
            // them than this set has (its view of the errors).
            if let Val::Mem(a) = v {
                let n = self.error_size(a, ms)?;
                let f = self
                    .c
                    .module
                    .declare_func_in_func(self.c.rt.memmove, self.b.func);
                self.b.ins().call(f, &[to, a, n]);
            }
            return Ok(());
        }
        let id = ty.adt().ok_or_else(|| format!("an error of type `{ty}`"))?;
        let tag = self.b.ins().iconst(types::I64, i64::from(id));
        self.b.ins().store(ir::MemFlagsData::trusted(), tag, to, 0);
        let at = self.offset(to, ERROR_VALUE);
        let place = PlaceRef::at(at, ty.clone());
        self.write(&place, v)
    }

    /// Error `v` of type `from` as `dest`'s type: through the tagged form, so a value of a set
    /// becomes its member's, a member's becomes the set's, and a set's another's.
    fn convert_error(&mut self, v: Val, from: &Ty, dest: &PlaceRef) -> Result<(), String> {
        let room = self.error_room(from)?.max(self.error_room(&dest.ty)?);
        let buf = self.slot(&Layout {
            size: room,
            align: 8,
            shape: Shape::Wide,
            niche: None,
        });
        self.write_error(buf, v, from)?;
        let l = self.layout(&dest.ty)?;
        let a = self.addr_or_dangling(dest)?;
        if let Ty::Errors(_) = dest.ty {
            self.copy(a, buf, l.size);
            return Ok(());
        }
        let at = self.offset(buf, ERROR_VALUE);
        let v = self.take_at(at, &dest.ty)?;
        self.write(dest, v)
    }

    /// How many bytes the error in the set (of members `ms`) at `at` takes, tagged: its id and
    /// the member its id names.
    fn error_size(&mut self, at: ir::Value, ms: &[Ty]) -> Result<ir::Value, String> {
        let id = self
            .b
            .ins()
            .load(types::I64, ir::MemFlagsData::trusted(), at, 0);
        let mut n = self.b.ins().iconst(types::I64, i64::from(ERROR_VALUE));
        for m in ms {
            let Some(a) = m.adt() else { continue };
            let size = self.layout(m)?.size;
            let is = self.b.ins().icmp_imm_s(IntCC::Equal, id, i64::from(a));
            let k = self
                .b
                .ins()
                .iconst(types::I64, i64::from(ERROR_VALUE + size));
            n = self.b.ins().select(is, k, n);
        }
        Ok(n)
    }

    /// The tagged error at `from` into error place `p`: all of it for a set, its value for one
    /// type.
    fn read_error(&mut self, from: ir::Value, p: &Place) -> Result<(), String> {
        let r = self.place(p)?;
        let l = self.layout(&r.ty)?;
        let a = self.addr_or_dangling(&r)?;
        if let Ty::Errors(_) = r.ty {
            self.copy(a, from, l.size);
            return Ok(());
        }
        let at = self.offset(from, ERROR_VALUE);
        if r.var.is_some() {
            let v = self.take_at(at, &r.ty)?;
            return self.write(&r, v);
        }
        self.copy(a, at, l.size);
        Ok(())
    }

    /// Splits scalar `v` of number type `ty` (`int` or a `Num`) into two words of bits.
    fn number_words(&mut self, v: ir::Value, ty: &Ty) -> (ir::Value, ir::Value) {
        let flags = ir::MemFlagsData::new();
        let zero = self.b.ins().iconst(types::I64, 0);
        match self.b.func.dfg.value_type(v) {
            types::I128 => {
                let (lo, hi) = self.b.ins().isplit(v);
                (lo, hi)
            }
            types::I64 => (v, zero),
            types::F64 => (self.b.ins().bitcast(types::I64, flags, v), zero),
            types::F32 => {
                let i = self.b.ins().bitcast(types::I32, flags, v);
                (self.b.ins().uextend(types::I64, i), zero)
            }
            _ => {
                let _ = ty;
                (self.b.ins().uextend(types::I64, v), zero)
            }
        }
    }

    /// The scalar of number type `ty` with the bits in words `lo` and `hi`.
    fn number_of_words(
        &mut self,
        lo: ir::Value,
        hi: ir::Value,
        ty: &Ty,
    ) -> Result<ir::Value, String> {
        let t = clif_type(ty)?.ok_or("a number")?;
        let flags = ir::MemFlagsData::new();
        Ok(match t {
            types::I128 => self.b.ins().iconcat(lo, hi),
            types::I64 => lo,
            types::F64 => self.b.ins().bitcast(types::F64, flags, lo),
            types::F32 => {
                let i = self.b.ins().ireduce(types::I32, lo);
                self.b.ins().bitcast(types::F32, flags, i)
            }
            t => self.b.ins().ireduce(t, lo),
        })
    }

    /// The code `host` knows a number type by.
    fn num_code(ty: &Ty) -> Option<u32> {
        match ty {
            Ty::Int => Some(moddil_mir::num_code(None)),
            Ty::Num(n) => Some(moddil_mir::num_code(Some(*n))),
            _ => None,
        }
    }

    /// A `&str` (in a new slot) of the text the host's `text` gives for `code`.
    fn host_text(
        &mut self,
        code: u32,
        lo: ir::Value,
        hi: ir::Value,
        p: ir::Value,
        n: ir::Value,
    ) -> ir::Value {
        let l = Layout {
            size: 8,
            align: 8,
            shape: Shape::Scalar(types::I64),
            niche: None,
        };
        let len = self.slot(&l);
        let code = self.b.ins().iconst(types::I32, i64::from(code));
        let at = self.rt_call(self.c.rt.text, &[code, lo, hi, p, n, len]);
        let n = self
            .b
            .ins()
            .load(types::I64, ir::MemFlagsData::trusted(), len, 0);
        self.wide(at, n)
    }

    /// The address of `const` item `f`'s value, computed the first time it is asked for.
    fn static_ref(&mut self, f: u32) -> Result<ir::Value, String> {
        // Computed when compiling, and written as data (`comptime.static.1`).
        let ty = self.c.items.fns[f as usize].ret.clone();
        if let Some(data) = self.c.frozen_static(f, &ty)? {
            let gv = self.c.module.declare_data_in_func(data, self.b.func);
            let ptr = self.ptr();
            return Ok(self.b.ins().symbol_value(ptr, gv));
        }
        let (id, _, ret) = self.c.declare_fn(f, &[])?;
        let l = self.layout(&ret)?;
        let data = match self.c.statics.get(&f) {
            Some(d) => *d,
            None => {
                let d = self
                    .c
                    .module
                    .declare_anonymous_data(true, false)
                    .map_err(|e| e.to_string())?;
                let mut desc = DataDescription::new();
                desc.define_zeroinit((16 + l.size) as usize);
                desc.set_align(16);
                self.c
                    .module
                    .define_data(d, &desc)
                    .map_err(|e| e.to_string())?;
                self.c.statics.insert(f, d);
                d
            }
        };
        let gv = self.c.module.declare_data_in_func(data, self.b.func);
        let ptr = self.ptr();
        let base = self.b.ins().symbol_value(ptr, gv);
        let value = self.offset(base, 16);
        // Its first byte: 0 before the value is computed, 1 while a task computes it, 2 once
        // it is; the runtime gives the computing to one task, and the others wait for it.
        let flags = ir::MemFlagsData::trusted();
        let state = match self.c.threads {
            Threads::Many => self.b.ins().atomic_load(types::I8, flags, base),
            Threads::One => self.b.ins().load(types::I8, flags, base, 0),
        };
        let done = self.b.ins().icmp_imm_s(IntCC::Equal, state, 2);
        let claim = self.b.create_block();
        let compute = self.b.create_block();
        let join = self.b.create_block();
        self.b.ins().brif(done, join, &[], claim, &[]);
        self.b.switch_to_block(claim);
        let begin = self
            .c
            .module
            .declare_func_in_func(self.c.rt.static_begin, self.b.func);
        let call = self.b.ins().call(begin, &[base]);
        let mine = self.b.inst_results(call)[0];
        self.b.ins().brif(mine, compute, &[], join, &[]);
        self.b.switch_to_block(compute);
        let at = PlaceRef::at(value, ret.clone());
        let span = self.span;
        self.emit_call(Run::Direct(id, ret), Vec::new(), Some(&at), span)?;
        let end = self
            .c
            .module
            .declare_func_in_func(self.c.rt.static_end, self.b.func);
        self.b.ins().call(end, &[base]);
        self.b.ins().jump(join, &[]);
        self.b.seal_block(claim);
        self.b.seal_block(compute);
        self.b.switch_to_block(join);
        Ok(value)
    }

    /// The arguments `args` as a tuple of type `ty`, in a new slot.
    fn tuple_of(&mut self, args: &[Operand], ty: &Ty) -> Result<Val, String> {
        let l = self.layout(ty)?;
        if l.size == 0 {
            for a in args {
                self.operand(a)?;
            }
            return Ok(Val::Unit);
        }
        let Shape::Fields(fs) = l.shape.clone() else {
            return Err(format!("arguments as a `{ty}`"));
        };
        let slot = self.slot(&l);
        for ((off, t), a) in fs.iter().zip(args) {
            let v = self.operand(a)?;
            let at = self.offset(slot, *off);
            let place = PlaceRef::at(at, t.clone());
            self.write(&place, v)?;
        }
        Ok(Val::Mem(slot))
    }

    /// A call of trait method `func` on an error of set `ms`: the method of the member its id
    /// names, given the member (or a borrow of it).
    #[allow(clippy::too_many_arguments)]
    fn error_set_call(
        &mut self,
        func: u32,
        cargs: &[Ty],
        ms: &[Ty],
        args: &[Operand],
        d: &PlaceRef,
        range: TextRange,
        catch: &Option<(Place, moddil_mir::BlockId)>,
    ) -> Result<(), String> {
        let items = self.c.items;
        let t = items.trait_of(func).ok_or("a trait's method")?;
        let n = items.traits[t as usize].generics.params.len();
        let tr = TraitRef {
            trait_: t,
            name: items.traits[t as usize].name.as_str().into(),
            args: cargs[1..n].to_vec().into(),
        };
        let sig = &items.fns[func as usize];
        let recv_ty = self.operand_ty(&args[0])?;
        let base = match self.operand(&args[0])? {
            Val::Scalar(p) | Val::Mem(p) => p,
            Val::Unit => return Err("an error without bytes".to_string()),
        };
        let id = self
            .b
            .ins()
            .load(types::I64, ir::MemFlagsData::trusted(), base, 0);
        let payload = self.offset(base, ERROR_VALUE);
        let join = self.b.create_block();
        for m in ms {
            let Some(adt) = m.adt() else { continue };
            let Some((f, mut parent)) = Solver::new(items, &[]).impl_method(func, m, &tr) else {
                continue;
            };
            parent.extend(cargs[sig.generics.parent as usize..].iter().cloned());
            let yes = self.b.create_block();
            let no = self.b.create_block();
            let is = self.b.ins().icmp_imm_s(IntCC::Equal, id, i64::from(adt));
            self.b.ins().brif(is, yes, &[], no, &[]);
            self.b.switch_to_block(yes);
            let (fid, params, ret) = self.c.declare_fn(f, &parent)?;
            let me = if matches!(recv_ty, Ty::Ref(..)) {
                Val::Scalar(payload)
            } else {
                let at = PlaceRef::at(payload, m.clone());
                self.read(&at)?
            };
            let mut vals = self.pass_value(me, &params[0])?;
            vals.extend(self.arguments(&args[1..], &params[1..])?);
            self.emit_call_catching(Run::Direct(fid, ret), vals, Some(d), range, catch.clone())?;
            self.b.ins().jump(join, &[]);
            self.b.switch_to_block(no);
        }
        // Its id names one of its members.
        self.return_status(1)?;
        self.b.switch_to_block(join);
        Ok(())
    }

    // ----- tasks -----------------------------------------------------------------------------

    /// Calls the runtime's task function `name` with `args`: its result, if it has one.
    fn task_call(&mut self, name: &str, args: &[ir::Value]) -> Option<ir::Value> {
        let f = self.c.rt.task[name];
        let f = self.c.module.declare_func_in_func(f, self.b.func);
        let call = self.b.ins().call(f, args);
        self.b.inst_results(call).first().copied()
    }

    /// The site a suspending operation deadlocks at, and whether its code is tracked.
    fn task_site(&mut self, range: TextRange) -> (ir::Value, ir::Value) {
        let site = self.site(Site::Trap(TrapClass::Deadlock, self.file, range));
        let tracked = self.b.ins().iconst(types::I8, i64::from(self.tracked));
        (site, tracked)
    }

    /// The handle's number at a borrow of a scope, task, sender or receiver (of type `ty`),
    /// through any borrows of it.
    fn handle_id(&mut self, v: Val, ty: &Ty) -> Result<ir::Value, String> {
        let flags = ir::MemFlagsData::trusted();
        let (mut p, mut t) = match (v, ty) {
            (Val::Scalar(p), Ty::Ref(_, inner)) => (p, (**inner).clone()),
            (Val::Mem(a), _) => return Ok(self.b.ins().load(types::I64, flags, a, 0)),
            _ => return Err(format!("a handle of type `{ty}`")),
        };
        while let Ty::Ref(_, inner) = t {
            p = self.b.ins().load(types::I64, flags, p, 0);
            t = (*inner).clone();
        }
        Ok(self.b.ins().load(types::I64, flags, p, 0))
    }

    /// A buffer for the error of a call with an error edge (`catch`), or null.
    fn error_buffer(
        &mut self,
        catch: &Option<(Place, moddil_mir::BlockId)>,
    ) -> Result<ir::Value, String> {
        Ok(match catch {
            Some((p, _)) => {
                let ty = self.place(p)?.ty;
                let size = self.error_room(&ty)?;
                let l = Layout {
                    size,
                    align: 8,
                    shape: Shape::Wide,
                    niche: None,
                };
                self.slot(&l)
            }
            None => {
                let ptr = self.ptr();
                self.b.ins().iconst(ptr, 0)
            }
        })
    }

    /// Goes on after a task operation's `status`: 0 returned; 2 failed, its error in `buf` (or
    /// to be written first, by `before`), to the error edge; 3 unwinds; anything else traps.
    fn task_status(
        &mut self,
        status: ir::Value,
        catch: &Option<(Place, moddil_mir::BlockId)>,
        buf: ir::Value,
        before: Before,
    ) -> Result<(), String> {
        let ok = self.b.create_block();
        let not = self.b.create_block();
        self.b.ins().brif(status, not, &[], ok, &[]);
        self.b.switch_to_block(not);
        if let Some((p, edge)) = catch {
            let failed = self.b.create_block();
            let other = self.b.create_block();
            let is = self.b.ins().icmp_imm_s(IntCC::Equal, status, 2);
            self.b.ins().brif(is, failed, &[], other, &[]);
            self.b.switch_to_block(failed);
            match before {
                Before::Nothing => {}
                Before::Closed => self.write_closed(buf)?,
                Before::Trapped(built) => {
                    let yes = self.b.create_block();
                    let done = self.b.create_block();
                    self.b.ins().brif(built, yes, &[], done, &[]);
                    self.b.switch_to_block(yes);
                    self.write_trapped(buf)?;
                    self.b.ins().jump(done, &[]);
                    self.b.switch_to_block(done);
                }
            }
            self.read_error(buf, p)?;
            self.set_init(p);
            let e = self.block(edge.index())?;
            self.b.ins().jump(e, &[]);
            self.b.switch_to_block(other);
        }
        self.not_returned(status)?;
        self.b.switch_to_block(ok);
        Ok(())
    }

    /// A task's description for the runtime ([`moddil_rt::TaskSpec`]), in a slot: closure `f`
    /// (of type `ty`), its captures copied to memory the task owns, with argument `arg`.
    fn task_spec(&mut self, f: &Operand, arg: ir::Value) -> Result<ir::Value, String> {
        let ty = self.operand_ty(f)?;
        let Ty::Closure(c) = &ty else {
            return Err(format!("a task of type `{ty}`"));
        };
        let c = c.clone();
        let tramp = self.c.task_tramp(&c)?;
        let (_, _, ret) = self.c.declare_closure(&c)?;
        let l = self.layout(&ty)?;
        let flags = ir::MemFlagsData::trusted();
        let size = self.b.ins().iconst(types::I64, i64::from(l.size));
        let align = self.b.ins().iconst(types::I64, i64::from(l.align.max(1)));
        let captures = self.rt_call(self.c.rt.alloc, &[size, align]);
        if let Val::Mem(a) = self.operand(f)? {
            self.copy(captures, a, l.size);
        }
        let out = self.layout(&ret)?.size;
        let err = if c.fails == Ty::Never {
            0
        } else {
            let fails = self.c.concrete(&c.fails, &[]);
            self.error_room(&fails)?
        };
        let ptr = self.ptr();
        let code = {
            let fref = self.c.module.declare_func_in_func(tramp, self.b.func);
            self.b.ins().func_addr(ptr, fref)
        };
        let drop = if self.needs_drop(&ret) {
            let d = self.c.c_drop(&ret)?;
            let fref = self.c.module.declare_func_in_func(d, self.b.func);
            self.b.ins().func_addr(ptr, fref)
        } else {
            self.b.ins().iconst(ptr, 0)
        };
        let spec = self.slot(&Layout {
            size: 64,
            align: 8,
            shape: Shape::Wide,
            niche: None,
        });
        let out = self.b.ins().iconst(types::I64, i64::from(out));
        let err = self.b.ins().iconst(types::I64, i64::from(err));
        for (k, v) in [code, captures, size, align, arg, out, err, drop]
            .into_iter()
            .enumerate()
        {
            self.b.ins().store(flags, v, spec, 8 * k as i32);
        }
        Ok(spec)
    }

    /// A C-ABI drop of `ty` for the runtime, or null if it needs no dropping.
    fn drop_fn(&mut self, ty: &Ty) -> Result<ir::Value, String> {
        let ptr = self.ptr();
        if !self.needs_drop(ty) {
            return Ok(self.b.ins().iconst(ptr, 0));
        }
        let d = self.c.c_drop(ty)?;
        let fref = self.c.module.declare_func_in_func(d, self.b.func);
        Ok(self.b.ins().func_addr(ptr, fref))
    }

    /// Writes the prelude's `Closed`, tagged, at `buf`.
    fn write_closed(&mut self, buf: ir::Value) -> Result<(), String> {
        let closed = self.c.prelude_ty("Closed")?;
        self.write_error(buf, Val::Unit, &closed)
    }

    /// Writes, tagged at `buf`, the prelude's `Trapped` for the trap an isolated scope failed
    /// with: its class and where it was, as `file:line:column`.
    fn write_trapped(&mut self, buf: ir::Value) -> Result<(), String> {
        let trapped = self.c.prelude_ty("Trapped")?;
        let l = self.layout(&trapped)?;
        let Shape::Fields(fs) = &l.shape else {
            return Err("the prelude's `Trapped`".to_string());
        };
        let fs = fs.clone();
        let items = self.c.items;
        let from_str = items
            .builtin_method(&items.string_ty(), "from_str")
            .ok_or("the prelude's `String::from_str`")?;
        let (id, params, ret) = self.c.declare_fn(from_str, &[])?;
        let tag = self
            .b
            .ins()
            .iconst(types::I64, i64::from(trapped.adt().unwrap_or(0)));
        let flags = ir::MemFlagsData::trusted();
        self.b.ins().store(flags, tag, buf, 0);
        for (which, (off, ty)) in fs.iter().enumerate() {
            let len = self.slot(&Layout {
                size: 8,
                align: 8,
                shape: Shape::Scalar(types::I64),
                niche: None,
            });
            let which = self.b.ins().iconst(types::I32, which as i64);
            let p = self.rt_call(self.c.rt.trapped, &[which, len]);
            let n = self.b.ins().load(types::I64, flags, len, 0);
            let w = self.wide(p, n);
            let at = self.offset(buf, ERROR_VALUE + off);
            let dest = PlaceRef::at(at, ty.clone());
            let vals = self.pass_value(Val::Mem(w), &params[0])?;
            let span = self.span;
            self.emit_call(Run::Direct(id, ret.clone()), vals, Some(&dest), span)?;
        }
        Ok(())
    }

    /// A destructor the scheduler runs: of a `Task` (given up without a `join`), a `Sender` or
    /// a `Receiver`, at `addr`.
    fn native_drop(&mut self, addr: ir::Value, ty: &Ty) -> Result<(), String> {
        let Ty::Adt(a, _, args) = ty else {
            return Err(format!("the built-in destructor of `{ty}`"));
        };
        let id = self
            .b
            .ins()
            .load(types::I64, ir::MemFlagsData::trusted(), addr, 0);
        match self.c.items.adts[*a as usize].name.as_str() {
            "Task" => {
                let st = self.task_call("detach", &[id]).expect("a status");
                self.passed_on(st)?;
            }
            "Sender" => {
                self.task_call("drop_sender", &[id]);
            }
            // What the channel holds is dropped (`task.channel.2`).
            "Receiver" => {
                let t = args.first().cloned().ok_or("a receiver without its type")?;
                let d = self.drop_fn(&t)?;
                let st = self.task_call("drop_receiver", &[id, d]).expect("a status");
                self.passed_on(st)?;
            }
            _ => return Err(format!("the built-in destructor of `{ty}`")),
        }
        Ok(())
    }

    /// The scheduler's operation `op` (`task.scope.1`), its result written to `d`.
    fn task(
        &mut self,
        op: &str,
        targs: &[Ty],
        args: &[Operand],
        d: &PlaceRef,
        range: TextRange,
        catch: &Option<(Place, moddil_mir::BlockId)>,
    ) -> Result<(), String> {
        let flags = ir::MemFlagsData::trusted();
        let ptr = self.ptr();
        if op.starts_with("os_") {
            return self.os(op, args, d);
        }
        match op {
            "install_runtime" => {
                let seed = self.value(&args[0])?;
                let workers = self.value(&args[1])?;
                self.task_call("install_runtime", &[seed, workers]);
                Ok(())
            }
            "parallel" => {
                let p = self.task_call(op, &[]).expect("whether");
                self.write(d, Val::Scalar(p))
            }
            "hash_seed" => {
                let s = self.task_call(op, &[]).expect("a seed");
                self.write(d, Val::Scalar(s))
            }
            // The allocator in scope (`alloc.with.1`): a `&dyn Alloc` is two words.
            "alloc_push" => {
                let Val::Mem(w) = self.operand(&args[0])? else {
                    return Err("an allocator not as a `&dyn Alloc`".to_string());
                };
                let data = self.b.ins().load(types::I64, flags, w, 0);
                let vt = self.b.ins().load(types::I64, flags, w, 8);
                self.task_call(op, &[data, vt]);
                Ok(())
            }
            "alloc_pop" => {
                self.task_call(op, &[]);
                Ok(())
            }
            "alloc_push_none" => {
                let zero = self.b.ins().iconst(types::I64, 0);
                self.task_call("alloc_push", &[zero, zero]);
                Ok(())
            }
            "alloc_top" => {
                let a = self.addr_of(d)?;
                self.task_call(op, &[a]);
                Ok(())
            }
            "raw_alloc" => {
                let size = self.value(&args[0])?;
                let align = self.value(&args[1])?;
                let p = self.rt_call(self.c.rt.alloc, &[size, align]);
                self.write(d, Val::Scalar(p))
            }
            "raw_free" => {
                let p = self.value(&args[0])?;
                let size = self.value(&args[1])?;
                let align = self.value(&args[2])?;
                let f = self
                    .c
                    .module
                    .declare_func_in_func(self.c.rt.free, self.b.func);
                self.b.ins().call(f, &[p, size, align]);
                Ok(())
            }
            "raw_move" => {
                let dst = self.value(&args[0])?;
                let src = self.value(&args[1])?;
                let n = self.value(&args[2])?;
                let f = self
                    .c
                    .module
                    .declare_func_in_func(self.c.rt.memmove, self.b.func);
                self.b.ins().call(f, &[dst, src, n]);
                Ok(())
            }
            "task_id" => {
                let t = match self.c.rt.running {
                    Some(data) => {
                        let gv = self.c.module.declare_data_in_func(data, self.b.func);
                        let at = self.b.ins().symbol_value(ptr, gv);
                        self.b.ins().atomic_load(types::I64, flags, at)
                    }
                    None => self.task_call("task_id", &[]).expect("a task's number"),
                };
                self.write(d, Val::Scalar(t))
            }
            "executor_of_task" => {
                let e = self.task_call(op, &[]).expect("an executor");
                self.write(d, Val::Scalar(e))
            }
            "executor_new" => {
                let vals = args
                    .iter()
                    .map(|a| self.value(a))
                    .collect::<Result<Vec<_>, _>>()?;
                let e = self.task_call(op, &vals).expect("an executor");
                self.write(d, Val::Scalar(e))
            }
            "__scope_open" => {
                let isolated = self.value(&args[0])?;
                let on = self.value(&args[1])?;
                let s = self
                    .task_call("scope_open", &[isolated, on])
                    .expect("a scope");
                self.write(d, Val::Scalar(s))
            }
            "__scope_run" => {
                let sty = self.operand_ty(&args[0])?;
                let scope = self.operand(&args[0])?;
                let Val::Scalar(sp) = scope else {
                    return Err("a scope not by borrow".to_string());
                };
                let id = self.handle_id(scope, &sty)?;
                let spec = self.task_spec(&args[1], sp)?;
                let out = self.result_room(d)?;
                let buf = self.error_buffer(catch)?;
                let (site, tracked) = self.task_site(range);
                let st = self
                    .task_call("scope_run", &[id, spec, out, buf, site, tracked])
                    .expect("a status");
                // 4: an isolated scope that trapped fails with `Trapped`.
                let trapped = self.b.create_block();
                let other = self.b.create_block();
                let is = self.b.ins().icmp_imm_s(IntCC::Equal, st, 4);
                self.b.ins().brif(is, trapped, &[], other, &[]);
                self.b.switch_to_block(trapped);
                let two = self.b.ins().iconst(types::I8, 2);
                let join = self.b.create_block();
                self.b.append_block_param(join, types::I8);
                self.b.ins().jump(join, &[ir::BlockArg::Value(two)]);
                self.b.switch_to_block(other);
                self.b.ins().jump(join, &[ir::BlockArg::Value(st)]);
                self.b.switch_to_block(join);
                let st2 = self.b.block_params(join)[0];
                // Whether the error is to be built (it was a trap).
                let built = self.b.ins().icmp_imm_s(IntCC::Equal, st, 4);
                self.task_status(st2, catch, buf, Before::Trapped(built))?;
                self.result_into(d, out)
            }
            "spawn" => {
                let sty = self.operand_ty(&args[0])?;
                let scope = self.operand(&args[0])?;
                let id = self.handle_id(scope, &sty)?;
                let zero = self.b.ins().iconst(types::I64, 0);
                let spec = self.task_spec(&args[1], zero)?;
                // The executor's number, its one field.
                let on = match self.operand(&args[2])? {
                    Val::Scalar(v) => v,
                    Val::Mem(a) => self.b.ins().load(types::I64, flags, a, 0),
                    _ => return Err("an executor not as a number".to_string()),
                };
                let t = self.task_call("spawn", &[id, spec, on]).expect("a task");
                let a = self.addr_of(d)?;
                self.b.ins().store(flags, t, a, 0);
                Ok(())
            }
            "stop" => {
                let sty = self.operand_ty(&args[0])?;
                let scope = self.operand(&args[0])?;
                let id = self.handle_id(scope, &sty)?;
                self.task_call("stop", &[id]);
                Ok(())
            }
            "join" => {
                let tty = self.operand_ty(&args[0])?;
                let task = self.operand(&args[0])?;
                let id = self.handle_id(task, &tty)?;
                let out = self.result_room(d)?;
                let buf = self.error_buffer(catch)?;
                let (site, tracked) = self.task_site(range);
                let st = self
                    .task_call("join", &[id, out, buf, site, tracked])
                    .expect("a status");
                self.task_status(st, catch, buf, Before::Nothing)?;
                self.result_into(d, out)
            }
            "yield_now" | "stop_requested" => {
                let (site, tracked) = self.task_site(range);
                let st = self.task_call(op, &[site, tracked]).expect("a status");
                let null = self.b.ins().iconst(ptr, 0);
                self.task_status(st, catch, null, Before::Nothing)
            }
            // A sync cell's wait (`task.park.1`).
            "park" => {
                let p = self.value(&args[0])?;
                let expected = self.value(&args[1])?;
                let (site, tracked) = self.task_site(range);
                let st = self
                    .task_call("park", &[p, expected, site, tracked])
                    .expect("a status");
                let null = self.b.ins().iconst(ptr, 0);
                self.task_status(st, catch, null, Before::Nothing)
            }
            "unpark" => {
                let p = self.value(&args[0])?;
                let n = self.value(&args[1])?;
                let woken = self.task_call("unpark", &[p, n]).expect("a count");
                self.write(d, Val::Scalar(woken))
            }
            // A timer's channel (`task.timer.1`): its id is the `Receiver`'s.
            "timer" => {
                let n = self.value(&args[0])?;
                let c = self.task_call("timer", &[n]).expect("a channel");
                let a = self.addr_of(d)?;
                self.b.ins().store(flags, c, a, 0);
                Ok(())
            }
            "channel" => {
                let n = self.value(&args[0])?;
                let zero = self.b.ins().iconst(types::I64, 0);
                let ok = self.b.ins().icmp(IntCC::SignedGreaterThanOrEqual, n, zero);
                self.check(ok, TrapClass::CapacityExceeded, range)?;
                let c = self.task_call("channel", &[n]).expect("a channel");
                let a = self.addr_of(d)?;
                let l = self.layout(&d.ty)?;
                if let Shape::Fields(fs) = &l.shape {
                    for (off, _) in fs.clone() {
                        self.b.ins().store(flags, c, a, off as i32);
                    }
                }
                Ok(())
            }
            "clone" => {
                let sty = self.operand_ty(&args[0])?;
                let sender = self.operand(&args[0])?;
                let id = self.handle_id(sender, &sty)?;
                self.task_call("clone_sender", &[id]);
                let a = self.addr_of(d)?;
                self.b.ins().store(flags, id, a, 0);
                Ok(())
            }
            "send" => {
                let sty = self.operand_ty(&args[0])?;
                let sender = self.operand(&args[0])?;
                let id = self.handle_id(sender, &sty)?;
                let t = targs.first().cloned().ok_or("a sender without its type")?;
                let l = self.layout(&t)?;
                let v = self.operand(&args[1])?;
                let tmp = self.slot(&l);
                let at = PlaceRef::at(tmp, t.clone());
                self.write(&at, v)?;
                let n = self.b.ins().iconst(types::I64, i64::from(l.size));
                let drop = self.drop_fn(&t)?;
                let buf = self.error_buffer(catch)?;
                let (site, tracked) = self.task_site(range);
                let st = self
                    .task_call("send", &[id, tmp, n, drop, site, tracked])
                    .expect("a status");
                self.task_status(st, catch, buf, Before::Closed)
            }
            "recv" => {
                let rty = self.operand_ty(&args[0])?;
                let receiver = self.operand(&args[0])?;
                let id = self.handle_id(receiver, &rty)?;
                let (site, tracked) = self.task_site(range);
                self.receive(d, |lw, out| {
                    lw.task_call("recv", &[id, out, site, tracked])
                        .expect("a status")
                })
            }
            _ => self.select(args, d, range),
        }
    }

    /// Memory for a result of `d`'s type: `d`'s own, or a slot to copy from after.
    fn result_room(&mut self, d: &PlaceRef) -> Result<ir::Value, String> {
        if let Some(a) = d.addr {
            return Ok(a);
        }
        let l = self.layout(&d.ty)?;
        Ok(self.slot(&l))
    }

    /// The result at `out` into `d`, if `d` is not in memory there already.
    fn result_into(&mut self, d: &PlaceRef, out: ir::Value) -> Result<(), String> {
        if d.addr == Some(out) {
            return Ok(());
        }
        let v = self.take_at(out, &d.ty)?;
        self.write(d, v)
    }

    /// An `Option<T>` into `d` from a receive: `get` writes a value at its argument and gives
    /// 0 (`Some`), 5 (`None`), 3 (unwind) or 1 (a trap).
    fn receive(
        &mut self,
        d: &PlaceRef,
        get: impl FnOnce(&mut Self, ir::Value) -> ir::Value,
    ) -> Result<(), String> {
        let l = self.layout(&d.ty)?;
        let Shape::Enum { tag, variants } = &l.shape else {
            return Err(format!("a receive into a `{}`", d.ty));
        };
        let (tag, payload) = (tag.clone(), variants[1][0].0);
        let a = self.addr_of(d)?;
        let out = self.offset(a, payload);
        let st = get(self, out);
        let some = self.b.create_block();
        let not = self.b.create_block();
        let none = self.b.create_block();
        let other = self.b.create_block();
        let join = self.b.create_block();
        self.b.ins().brif(st, not, &[], some, &[]);
        self.b.switch_to_block(some);
        self.set_variant(a, &tag, 1);
        self.b.ins().jump(join, &[]);
        self.b.switch_to_block(not);
        let is = self.b.ins().icmp_imm_s(IntCC::Equal, st, 5);
        self.b.ins().brif(is, none, &[], other, &[]);
        self.b.switch_to_block(none);
        self.set_variant(a, &tag, 0);
        self.b.ins().jump(join, &[]);
        self.b.switch_to_block(other);
        self.not_returned(st)?;
        self.b.switch_to_block(join);
        Ok(())
    }

    /// The local a temporary `l` borrows (`l = &x` or `&mut x`), if that is how it is defined.
    fn borrowed_local(&self, o: &Operand) -> Option<Place> {
        let (Operand::Move(p) | Operand::Copy(p)) = o else {
            return None;
        };
        match self.defs.get(&p.local.0) {
            Some(Rvalue::Ref(_, q)) => Some(q.clone()),
            _ => None,
        }
    }

    /// The operands an aggregate temporary is built from.
    fn parts_of(&self, o: &Operand) -> Option<Vec<Operand>> {
        let (Operand::Move(p) | Operand::Copy(p)) = o else {
            return None;
        };
        match self.defs.get(&p.local.0) {
            Some(Rvalue::Aggregate(_, ops)) => Some(ops.clone()),
            _ => None,
        }
    }

    /// `__select` (`task.select.1`): waits until an arm's operation is ready, does it (its
    /// result, or its error and that it failed, through the arm's borrows), and gives the
    /// arm's index.
    fn select(&mut self, args: &[Operand], d: &PlaceRef, range: TextRange) -> Result<(), String> {
        let flags = ir::MemFlagsData::trusted();
        let arms = self
            .parts_of(&args[0])
            .ok_or("a `select` whose arms are not known")?;
        struct Arm {
            kind: i64,
            handle: Operand,
            payload: Option<Place>,
            result: Place,
            error: Place,
            failed: Place,
        }
        let mut list = Vec::new();
        for a in &arms {
            let parts = self.parts_of(a).ok_or("a `select` arm that is not known")?;
            let Operand::Const(Const::Int(kind)) = parts[0] else {
                return Err("a `select` arm of an unknown kind".to_string());
            };
            let place = |lw: &Self, o: &Operand| {
                lw.borrowed_local(o)
                    .ok_or_else(|| "a `select` arm's place".to_string())
            };
            list.push(Arm {
                kind,
                handle: parts[1].clone(),
                payload: self.borrowed_local(&parts[2]),
                result: place(self, &parts[3])?,
                error: place(self, &parts[4])?,
                failed: place(self, &parts[5])?,
            });
        }
        // An arm on a `#[selectable]` function waits for the runtimes (`task.select.3`).
        if list.iter().any(|a| a.kind == 4) {
            self.trap(TrapClass::Unimplemented, range)?;
            // What follows is never reached.
            let dead = self.b.create_block();
            self.b.switch_to_block(dead);
            return Ok(());
        }
        let n = list.len() as u32;
        let ops = self.slot(&Layout {
            size: 16 * n.max(1),
            align: 8,
            shape: Shape::Wide,
            niche: None,
        });
        let mut ids = Vec::new();
        for (k, a) in list.iter().enumerate() {
            let kind = self.b.ins().iconst(types::I64, a.kind);
            let id = if a.kind == 3 {
                self.b.ins().iconst(types::I64, 0)
            } else {
                let ty = self.operand_ty(&a.handle)?;
                let v = self.operand(&a.handle)?;
                self.handle_id(v, &ty)?
            };
            self.b.ins().store(flags, kind, ops, 16 * k as i32);
            self.b.ins().store(flags, id, ops, 16 * k as i32 + 8);
            ids.push(id);
        }
        let count = self.b.ins().iconst(types::I64, i64::from(n));
        let (site, tracked) = self.task_site(range);
        let chosen = self
            .task_call("select", &[count, ops, site, tracked])
            .expect("an index");
        // -1: a trap; -3: unwind.
        let ok = self.b.create_block();
        let bad = self.b.create_block();
        let neg = self.b.ins().icmp_imm_s(IntCC::SignedLessThan, chosen, 0);
        self.b.ins().brif(neg, bad, &[], ok, &[]);
        self.b.switch_to_block(bad);
        let st = self.b.ins().ineg(chosen);
        let st = self.b.ins().ireduce(types::I8, st);
        self.not_returned(st)?;
        self.b.switch_to_block(ok);
        let join = self.b.create_block();
        for (k, a) in list.iter().enumerate() {
            let yes = self.b.create_block();
            let no = self.b.create_block();
            let is = self.b.ins().icmp_imm_s(IntCC::Equal, chosen, k as i64);
            self.b.ins().brif(is, yes, &[], no, &[]);
            self.b.switch_to_block(yes);
            let result = self.place(&a.result)?;
            let id = ids[k];
            let fail = |lw: &mut Self, buf: ir::Value, error: &Place, failed: &Place| {
                lw.read_error(buf, error)?;
                lw.set_init(error);
                let t = lw.b.ins().iconst(types::I8, 1);
                let f = lw.place(failed)?;
                lw.write(&f, Val::Scalar(t))
            };
            match a.kind {
                0 => {
                    self.receive(&result, |lw, out| {
                        lw.task_call("recv_now", &[id, out]).expect("a status")
                    })?;
                    self.set_init(&a.result);
                }
                1 => {
                    // The value is sent, or dropped: either way it is gone from its place.
                    let p = a.payload.clone().ok_or("a send arm's value")?;
                    let pr = self.place(&p)?;
                    let l = self.layout(&pr.ty)?;
                    let at = self.addr_or_dangling(&pr)?;
                    let size = self.b.ins().iconst(types::I64, i64::from(l.size));
                    let drop = self.drop_fn(&pr.ty)?;
                    let (site, tracked) = self.task_site(range);
                    let st = self
                        .task_call("send_now", &[id, at, size, drop, site, tracked])
                        .expect("a status");
                    self.set_flag(&p, false);
                    // 2: closed; 1 or 3: a trap, or unwinding, while it waited for its
                    // rendezvous.
                    let closed = self.b.create_block();
                    let sent = self.b.create_block();
                    let not = self.b.create_block();
                    let other = self.b.create_block();
                    self.b.ins().brif(st, not, &[], sent, &[]);
                    self.b.switch_to_block(not);
                    let is = self.b.ins().icmp_imm_s(IntCC::Equal, st, 2);
                    self.b.ins().brif(is, closed, &[], other, &[]);
                    self.b.switch_to_block(other);
                    self.not_returned(st)?;
                    self.b.switch_to_block(closed);
                    let er = self.place(&a.error)?;
                    let room = self.error_room(&er.ty)?;
                    let buf = self.slot(&Layout {
                        size: room,
                        align: 8,
                        shape: Shape::Wide,
                        niche: None,
                    });
                    self.write_closed(buf)?;
                    fail(self, buf, &a.error, &a.failed)?;
                    self.b.ins().jump(sent, &[]);
                    self.b.switch_to_block(sent);
                }
                2 => {
                    let out = self.result_room(&result)?;
                    let er = self.place(&a.error)?;
                    let room = self.error_room(&er.ty)?;
                    let buf = self.slot(&Layout {
                        size: room,
                        align: 8,
                        shape: Shape::Wide,
                        niche: None,
                    });
                    let st = self
                        .task_call("join_now", &[id, out, buf])
                        .expect("a status");
                    let done = self.b.create_block();
                    let not = self.b.create_block();
                    let failed = self.b.create_block();
                    let other = self.b.create_block();
                    self.b.ins().brif(st, not, &[], done, &[]);
                    self.b.switch_to_block(not);
                    let is = self.b.ins().icmp_imm_s(IntCC::Equal, st, 2);
                    self.b.ins().brif(is, failed, &[], other, &[]);
                    self.b.switch_to_block(failed);
                    fail(self, buf, &a.error, &a.failed)?;
                    let after = self.b.create_block();
                    self.b.ins().jump(after, &[]);
                    self.b.switch_to_block(other);
                    self.not_returned(st)?;
                    self.b.switch_to_block(done);
                    self.result_into(&result, out)?;
                    self.set_init(&a.result);
                    self.b.ins().jump(after, &[]);
                    self.b.switch_to_block(after);
                }
                _ => {}
            }
            self.b.ins().jump(join, &[]);
            self.b.switch_to_block(no);
        }
        self.b.ins().jump(join, &[]);
        self.b.switch_to_block(join);
        self.write(d, Val::Scalar(chosen))
    }

    // ----- drops -----------------------------------------------------------------------------

    fn needs_drop(&self, ty: &Ty) -> bool {
        Solver::new(self.c.items, &[]).needs_drop(ty)
    }

    /// The drop flags: one for each local whose type needs dropping, and one for each part of a
    /// local that is moved out of. A local's starts set if it is an argument; a part's starts
    /// set, as only its local's says whether there is anything there.
    fn drop_flags(&mut self) {
        let mut moved: Vec<Place> = Vec::new();
        let mut note = |o: &Operand| {
            if let Operand::Move(p) = o
                && !p.is_indirect()
                && !p.projection.is_empty()
                && !moved.contains(p)
            {
                moved.push(p.clone());
            }
        };
        for bb in &self.body.blocks {
            for st in &bb.statements {
                match st {
                    Statement::Assign(_, r) => rvalue_operands(r, &mut note),
                    Statement::Observe { value, .. } => note(value),
                    Statement::Assert { cond, .. } => note(cond),
                    _ => {}
                }
            }
            match &bb.terminator {
                Terminator::Call { args, .. } => args.iter().for_each(&mut note),
                Terminator::SwitchBool { cond, .. } => note(cond),
                _ => {}
            }
        }
        for i in 0..self.tys.len() {
            if self.needs_drop(&self.tys[i].clone()) {
                let set = (1..=self.body.arg_count).contains(&i);
                let v = self.b.declare_var(types::I8);
                let x = self.b.ins().iconst(types::I8, i64::from(set));
                self.b.def_var(v, x);
                self.flags.insert(Place::local(Local(i as u32)), v);
            }
        }
        for p in moved {
            if !self.flags.contains_key(&Place::local(p.local)) {
                continue;
            }
            let v = self.b.declare_var(types::I8);
            let x = self.b.ins().iconst(types::I8, 1);
            self.b.def_var(v, x);
            self.flags.insert(p, v);
        }
    }

    fn set_flag(&mut self, p: &Place, to: bool) {
        if let Some(&v) = self.flags.get(p) {
            let x = self.b.ins().iconst(types::I8, i64::from(to));
            self.b.def_var(v, x);
        }
    }

    /// After `place` is assigned: it, and each part of it, holds a value.
    fn set_init(&mut self, place: &Place) {
        if place.is_indirect() {
            return;
        }
        let parts: Vec<Place> = self
            .flags
            .keys()
            .filter(|p| p.local == place.local && p.projection.starts_with(&place.projection))
            .cloned()
            .collect();
        for p in parts {
            self.set_flag(&p, true);
        }
    }

    /// A `Drop` of `place`: what of it is still there, by its flags; only its fields if
    /// `glue_only`.
    fn drop_place(&mut self, place: &Place, glue_only: bool) -> Result<(), String> {
        let r = self.place(place)?;
        if !self.needs_drop(&r.ty) {
            return Ok(());
        }
        if place.is_indirect() {
            let a = self.addr_or_dangling(&r)?;
            return if glue_only {
                self.glue_fields(a, &r.ty)
            } else {
                self.drop_value(a, &r.ty)
            };
        }
        self.drop_path(place, glue_only)?;
        let parts: Vec<Place> = self
            .flags
            .keys()
            .filter(|p| p.local == place.local && p.projection.starts_with(&place.projection))
            .cloned()
            .collect();
        for p in parts {
            self.set_flag(&p, false);
        }
        Ok(())
    }

    fn drop_path(&mut self, place: &Place, glue_only: bool) -> Result<(), String> {
        let r = self.place(place)?;
        let ty = r.ty.clone();
        if !self.needs_drop(&ty) {
            return Ok(());
        }
        let join = self.b.create_block();
        if let Some(&flag) = self.flags.get(place) {
            let set = self.b.use_var(flag);
            let yes = self.b.create_block();
            self.b.ins().brif(set, yes, &[], join, &[]);
            self.b.switch_to_block(yes);
        }
        let addr = self.addr_or_dangling(&r)?;
        let parts: Vec<Place> = self
            .flags
            .keys()
            .filter(|p| {
                p.local == place.local
                    && p.projection.len() > place.projection.len()
                    && p.projection.starts_with(&place.projection)
            })
            .cloned()
            .collect();
        if parts.is_empty() {
            if glue_only {
                self.glue_fields(addr, &ty)?;
            } else {
                self.drop_value(addr, &ty)?;
            }
        } else {
            // Parts of it are moved out of: each field by its own flags.
            let l = self.layout(&ty)?;
            match &l.shape {
                Shape::Fields(fs) => {
                    for i in (0..fs.len()).rev() {
                        self.drop_path(&place.field(i as u32), false)?;
                    }
                }
                Shape::Enum { variants, .. } => {
                    let d = self.discriminant(&r)?;
                    for (v, fs) in variants.iter().enumerate() {
                        let down = place.downcast(v as u32);
                        if !parts
                            .iter()
                            .any(|p| p.projection.starts_with(&down.projection))
                        {
                            continue;
                        }
                        let yes = self.b.create_block();
                        let no = self.b.create_block();
                        let is = self.b.ins().icmp_imm_s(IntCC::Equal, d, v as i64);
                        self.b.ins().brif(is, yes, &[], no, &[]);
                        self.b.switch_to_block(yes);
                        for i in (0..fs.len()).rev() {
                            self.drop_path(&down.field(i as u32), false)?;
                        }
                        self.b.ins().jump(join, &[]);
                        self.b.switch_to_block(no);
                    }
                    // A variant none of whose parts is moved out of: all of it.
                    if glue_only {
                        self.glue_fields(addr, &ty)?;
                    } else {
                        self.drop_value(addr, &ty)?;
                    }
                }
                _ => self.drop_value(addr, &ty)?,
            }
        }
        self.b.ins().jump(join, &[]);
        self.b.switch_to_block(join);
        Ok(())
    }

    /// Drops the value of type `ty` at `addr`: calls its drop glue.
    fn drop_value(&mut self, addr: ir::Value, ty: &Ty) -> Result<(), String> {
        if !self.needs_drop(ty) {
            return Ok(());
        }
        let f = self.c.glue(ty)?;
        let span = self.span;
        self.emit_call(Run::Direct(f, Ty::Unit), vec![addr], None, span)
    }

    /// A type's drop glue, at `addr`: its `Drop` impl if it has one (which then drops the
    /// fields), or else its fields'.
    fn drop_own(&mut self, addr: ir::Value, ty: &Ty) -> Result<(), String> {
        if let Ty::Adt(a, ..) = ty
            && self.c.items.has_drop_impl(*a)
        {
            let items = self.c.items;
            let drop = items.lang_ref("Drop").ok_or("the prelude's `Drop`")?;
            let decl = items.traits[drop.trait_ as usize].methods[0];
            let (f, targs) = Solver::new(items, &[])
                .impl_method(decl, ty, &drop)
                .ok_or("a destructor")?;
            if items.fns[f as usize].intrinsic {
                return self.native_drop(addr, ty);
            }
            let (id, params, ret) = self.c.declare_fn(f, &targs)?;
            let at = PlaceRef::at(addr, ty.clone());
            let v = self.read(&at)?;
            let vals = self.pass_value(v, &params[0])?;
            let span = self.span;
            return self.emit_call(Run::Direct(id, ret), vals, None, span);
        }
        self.glue_fields(addr, ty)
    }

    /// Drops the fields of the value of type `ty` at `addr`: last to first (a variant's, for an
    /// enum), an array's and a `Buf`'s elements first to last, and then a `Buf`'s memory.
    fn glue_fields(&mut self, addr: ir::Value, ty: &Ty) -> Result<(), String> {
        let l = self.layout(ty)?;
        match &l.shape {
            Shape::Fields(fs) => {
                for (off, t) in fs.iter().rev() {
                    let a = self.offset(addr, *off);
                    self.drop_value(a, t)?;
                }
            }
            Shape::Enum { variants, .. } => {
                let at = PlaceRef::at(addr, ty.clone());
                let d = self.discriminant(&at)?;
                let join = self.b.create_block();
                for (v, fs) in variants.iter().enumerate() {
                    if !fs.iter().any(|(_, t)| self.needs_drop(t)) {
                        continue;
                    }
                    let yes = self.b.create_block();
                    let no = self.b.create_block();
                    let is = self.b.ins().icmp_imm_s(IntCC::Equal, d, v as i64);
                    self.b.ins().brif(is, yes, &[], no, &[]);
                    self.b.switch_to_block(yes);
                    for (off, t) in fs.iter().rev() {
                        let a = self.offset(addr, *off);
                        self.drop_value(a, t)?;
                    }
                    self.b.ins().jump(join, &[]);
                    self.b.switch_to_block(no);
                }
                self.b.ins().jump(join, &[]);
                self.b.switch_to_block(join);
            }
            Shape::Array { elem, stride, len } => {
                let (elem, stride, len) = (elem.clone(), *stride, *len);
                let n = self.b.ins().iconst(types::I64, i64::from(len));
                self.each(addr, n, stride, &elem)?;
            }
            Shape::Buf(elem) => {
                let elem = elem.clone();
                let flags = ir::MemFlagsData::trusted();
                let p = self.b.ins().load(types::I64, flags, addr, 0);
                let n = self.b.ins().load(types::I64, flags, addr, 8);
                let cap = self.b.ins().load(types::I64, flags, addr, 16);
                if matches!(elem, Ty::Dyn(..)) {
                    // A `Box<dyn Trait>`'s: its value is dropped by its vtable's glue.
                    let vt = self.b.ins().load(types::I64, flags, addr, 24);
                    let has = self.b.ins().icmp_imm_s(IntCC::NotEqual, n, 0);
                    let yes = self.b.create_block();
                    let join = self.b.create_block();
                    self.b.ins().brif(has, yes, &[], join, &[]);
                    self.b.switch_to_block(yes);
                    let glue = self.b.ins().load(types::I64, flags, vt, 8);
                    let params = vec![Ty::reference(true, Ty::Unit)];
                    let span = self.span;
                    self.emit_call(Run::Indirect(glue, params, Ty::Unit), vec![p], None, span)?;
                    self.b.ins().jump(join, &[]);
                    self.b.switch_to_block(join);
                    self.free_dyn(p, vt, Some(cap))?;
                } else {
                    let e = self.layout(&elem)?;
                    let stride = e.size.div_ceil(e.align) * e.align;
                    self.each(p, n, stride, &elem)?;
                    let bytes = self.b.ins().imul_imm_s(cap, i64::from(stride));
                    let align = self.b.ins().iconst(types::I64, i64::from(e.align));
                    self.heap_free(p, bytes, align)?;
                }
            }
            // An error set: the member it holds, by its id.
            Shape::Errors(ms) => {
                let ms = ms.clone();
                let id = self
                    .b
                    .ins()
                    .load(types::I64, ir::MemFlagsData::trusted(), addr, 0);
                let join = self.b.create_block();
                for m in &ms {
                    let Some(n) = m.adt() else { continue };
                    if !self.needs_drop(m) {
                        continue;
                    }
                    let yes = self.b.create_block();
                    let no = self.b.create_block();
                    let is = self.b.ins().icmp_imm_s(IntCC::Equal, id, i64::from(n));
                    self.b.ins().brif(is, yes, &[], no, &[]);
                    self.b.switch_to_block(yes);
                    let at = self.offset(addr, ERROR_VALUE);
                    self.drop_value(at, m)?;
                    self.b.ins().jump(join, &[]);
                    self.b.switch_to_block(no);
                }
                self.b.ins().jump(join, &[]);
                self.b.switch_to_block(join);
            }
            Shape::Scalar(_) | Shape::Empty | Shape::Wide => {}
        }
        Ok(())
    }

    /// Frees the memory of a `Buf<dyn Trait>` (of `cap` values, or one) at `p`, by its vtable's
    /// size and alignment.
    fn free_dyn(
        &mut self,
        p: ir::Value,
        vt: ir::Value,
        cap: Option<ir::Value>,
    ) -> Result<(), String> {
        let flags = ir::MemFlagsData::trusted();
        let size = self.b.ins().load(types::I64, flags, vt, 16);
        let align = self.b.ins().load(types::I64, flags, vt, 24);
        let bytes = match cap {
            Some(c) => self.b.ins().imul(size, c),
            None => size,
        };
        self.heap_free(p, bytes, align)
    }

    /// Drops each of the `n` values of type `elem` from `base`, `stride` apart, first to last.
    fn each(
        &mut self,
        base: ir::Value,
        n: ir::Value,
        stride: u32,
        elem: &Ty,
    ) -> Result<(), String> {
        if !self.needs_drop(elem) {
            return Ok(());
        }
        let head = self.b.create_block();
        let body = self.b.create_block();
        let done = self.b.create_block();
        self.b.append_block_param(head, types::I64);
        let zero = self.b.ins().iconst(types::I64, 0);
        self.b.ins().jump(head, &[ir::BlockArg::Value(zero)]);
        self.b.switch_to_block(head);
        let i = self.b.block_params(head)[0];
        let more = self.b.ins().icmp(IntCC::SignedLessThan, i, n);
        self.b.ins().brif(more, body, &[], done, &[]);
        self.b.switch_to_block(body);
        let off = self.b.ins().imul_imm_s(i, i64::from(stride));
        let a = self.b.ins().iadd(base, off);
        self.drop_value(a, elem)?;
        let next = self.b.ins().iadd_imm_s(i, 1);
        self.b.ins().jump(head, &[ir::BlockArg::Value(next)]);
        self.b.switch_to_block(done);
        Ok(())
    }

    /// A text's address and length, from a borrow of it (or a borrow of such a borrow).
    fn text(&mut self, o: &Operand) -> Result<(ir::Value, ir::Value), String> {
        let ty = self.operand_ty(o)?;
        let flags = ir::MemFlagsData::trusted();
        let w = match (self.operand(o)?, &ty) {
            (Val::Mem(w), _) => w,
            // `&&str`: the borrow it points to.
            (Val::Scalar(p), Ty::Ref(_, inner)) if matches!(**inner, Ty::Ref(..)) => p,
            _ => return Err(format!("text from a `{ty}`")),
        };
        let p = self.b.ins().load(types::I64, flags, w, 0);
        let n = self.b.ins().load(types::I64, flags, w, 8);
        Ok((p, n))
    }

    /// A call of C function `f` (`ffi.extern.1`) with `args`, its result into `d`. An 8- or
    /// 16-bit integer crosses widened to 32 bits, as C's callers pass them.
    fn foreign(&mut self, f: u32, args: &[Operand], d: &PlaceRef) -> Result<(), String> {
        let id = self.c.foreign_fn(f)?;
        let sig = &self.c.items.fns[f as usize];
        let (params, ret): (Vec<Ty>, Ty) = (
            sig.params.iter().map(|p| p.ty.clone()).collect(),
            sig.ret.clone(),
        );
        let on = moddil_ffi::abi::target();
        let flags = ir::MemFlagsData::trusted();
        let cret = self.c.c_type(&ret)?;
        let mut vals = Vec::new();
        // Room for a struct result; given as the hidden pointer if C writes it there.
        let rslot = match &cret {
            CType::Struct(l) => {
                let r = scratch(&mut self.b, l.padded() + 8);
                if moddil_ffi::abi::ret(on, l) == moddil_ffi::abi::Ret::Indirect {
                    vals.push(r);
                }
                Some(r)
            }
            _ => None,
        };
        for (a, ty) in args.iter().zip(&params) {
            if let CType::Struct(l) = self.c.c_type(ty)? {
                // A copy, padded, which the parts are read from or C is given.
                let Val::Mem(at) = self.operand(a)? else {
                    return Err("a struct not in memory".to_string());
                };
                let copy = scratch(&mut self.b, l.padded() + 8);
                copy_bytes(&mut self.b, copy, at, l.size);
                match moddil_ffi::abi::arg(on, &l) {
                    moddil_ffi::abi::Arg::Parts(parts) => {
                        for (off, t) in parts {
                            vals.push(self.b.ins().load(t, flags, copy, off as i32));
                        }
                    }
                    moddil_ffi::abi::Arg::Pointer | moddil_ffi::abi::Arg::Stack => vals.push(copy),
                }
                continue;
            }
            let v = self.value(a)?;
            let small = matches!(clif_type(ty)?, Some(t) if t == types::I8 || t == types::I16);
            vals.push(match (small, signed(ty)) {
                (true, true) => self.b.ins().sextend(types::I32, v),
                (true, false) => self.b.ins().uextend(types::I32, v),
                (false, _) => v,
            });
        }
        let fref = self.c.module.declare_func_in_func(id, self.b.func);
        let call = self.b.ins().call(fref, &vals);
        let results = self.b.inst_results(call).to_vec();
        if let (CType::Struct(l), Some(r)) = (&cret, rslot) {
            if let moddil_ffi::abi::Ret::Parts(parts) = moddil_ffi::abi::ret(on, l) {
                for ((off, _), v) in parts.iter().zip(&results) {
                    self.b.ins().store(flags, *v, r, *off as i32);
                }
            }
            let at = self.addr_of(d)?;
            copy_bytes(&mut self.b, at, r, l.size);
            return Ok(());
        }
        if let Some(&r) = results.first() {
            let r = match clif_type(&ret)? {
                Some(t) if t == types::I8 || t == types::I16 => self.b.ins().ireduce(t, r),
                _ => r,
            };
            self.write(d, Val::Scalar(r))?;
        }
        Ok(())
    }

    /// A call of a C function pointer (`type.fnptr.1`), `args` its value and its arguments'
    /// tuple, into `d`: C's signature for `params` and `ret`, small integers widened as C's
    /// callers do (`foreign`'s).
    fn fn_ptr_call(
        &mut self,
        params: &[Ty],
        ret: &Ty,
        args: &[Operand],
        d: &PlaceRef,
    ) -> Result<(), String> {
        let flags = ir::MemFlagsData::trusted();
        let f = self.value(&args[0])?;
        // The arguments: the tuple's fields, or its one scalar.
        let mut vals = Vec::new();
        if !params.is_empty() {
            let tuple = Ty::tuple(params.to_vec());
            let l = self.layout(&tuple)?;
            match (self.operand(&args[1])?, &l.shape) {
                (Val::Scalar(v), _) => vals.push(v),
                (Val::Mem(at), Shape::Fields(fs)) => {
                    for (off, t) in fs.clone() {
                        let ct = clif_type(&t)?.ok_or("a `()` argument of a `fn` value")?;
                        vals.push(self.b.ins().load(ct, flags, at, off as i32));
                    }
                }
                _ => return Err("the arguments of a `fn` value".to_string()),
            }
        }
        for (v, ty) in vals.iter_mut().zip(params) {
            let small = matches!(clif_type(ty)?, Some(t) if t == types::I8 || t == types::I16);
            if small {
                *v = if signed(ty) {
                    self.b.ins().sextend(types::I32, *v)
                } else {
                    self.b.ins().uextend(types::I32, *v)
                };
            }
        }
        let sig = self.c.c_sig(params, ret)?;
        let sref = self.b.import_signature(sig);
        let call = self.b.ins().call_indirect(sref, f, &vals);
        if let Some(&r) = self.b.inst_results(call).first() {
            let r = match clif_type(ret)? {
                Some(t) if t == types::I8 || t == types::I16 => self.b.ins().ireduce(t, r),
                _ => r,
            };
            self.write(d, Val::Scalar(r))?;
        }
        Ok(())
    }

    /// A call of the `core` mod's `ffi` module, `name`, into `d` (`ffi.core.1`): a borrow made
    /// of a pointer and a length, a borrow's pointer, and a box to its raw pointer and back. A
    /// box is its `Buf`: its memory's address, its length and its capacity (both 1).
    fn ffi(&mut self, name: &str, args: &[Operand], d: &PlaceRef) -> Result<(), String> {
        let flags = ir::MemFlagsData::trusted();
        match name {
            "slice" | "slice_mut" | "text" => {
                let p = self.value(&args[0])?;
                let n = self.value(&args[1])?;
                let w = self.wide(p, n);
                self.write(d, Val::Mem(w))
            }
            "ptr_of" | "text_ptr" => {
                let (p, _) = self.text(&args[0])?;
                self.write(d, Val::Scalar(p))
            }
            "into_raw" => {
                let Val::Mem(b) = self.operand(&args[0])? else {
                    return Err("a box not in memory".to_string());
                };
                let p = self.b.ins().load(types::I64, flags, b, 0);
                self.write(d, Val::Scalar(p))
            }
            "from_raw" => {
                let p = self.value(&args[0])?;
                let a = self.addr_of(d)?;
                let one = self.b.ins().iconst(types::I64, 1);
                self.b.ins().store(flags, p, a, 0);
                self.b.ins().store(flags, one, a, 8);
                self.b.ins().store(flags, one, a, 16);
                Ok(())
            }
            "c_len" => {
                // The bytes before the first NUL.
                let p = self.value(&args[0])?;
                let head = self.b.create_block();
                let more = self.b.create_block();
                let done = self.b.create_block();
                self.b.append_block_param(head, types::I64);
                self.b.append_block_param(done, types::I64);
                let zero = self.b.ins().iconst(types::I64, 0);
                self.b.ins().jump(head, &[zero.into()]);
                self.b.switch_to_block(head);
                let i = self.b.block_params(head)[0];
                let at = self.b.ins().iadd(p, i);
                let byte = self.b.ins().load(types::I8, flags, at, 0);
                self.b.ins().brif(byte, more, &[], done, &[i.into()]);
                self.b.switch_to_block(more);
                let next = self.b.ins().iadd_imm_s(i, 1);
                self.b.ins().jump(head, &[next.into()]);
                self.b.seal_block(head);
                self.b.seal_block(more);
                self.b.switch_to_block(done);
                self.b.seal_block(done);
                let n = self.b.block_params(done)[0];
                self.write(d, Val::Scalar(n))
            }
            _ => Err(format!("the built-in function `ffi::{name}`")),
        }
    }

    /// A call of the `core` mod's `atomic` module's `load`, `store`, `swap` or `cas`, as one
    /// atomic instruction (`core.atomic.1`), into `d`: whether it is one, for a value of a number
    /// type (not a float), `bool`, `char` or a pointer. Cranelift's atomics are all sequentially
    /// consistent, as strong as any ordering asks. Other calls run the module's code.
    fn atomic(
        &mut self,
        f: u32,
        targs: &[Ty],
        args: &[Operand],
        d: &PlaceRef,
    ) -> Result<bool, String> {
        let sig = &self.c.items.fns[f as usize];
        if sig.src != Src::Prelude || self.c.items.modules[sig.module as usize].path != ["atomic"] {
            return Ok(false);
        }
        let name = sig.name.as_str();
        if !matches!(name, "load" | "store" | "swap" | "cas") {
            return Ok(false);
        }
        let t = match targs.first().map(clif_type) {
            Some(Ok(Some(t))) if t.is_int() && t.bits() <= 64 => t,
            _ => return Ok(false),
        };
        let flags = ir::MemFlagsData::trusted();
        let p = self.value(&args[0])?;
        let v = match name {
            "load" => self.b.ins().atomic_load(t, flags, p),
            "store" => {
                let v = self.value(&args[1])?;
                self.b.ins().atomic_store(flags, v, p);
                return Ok(true);
            }
            "swap" => {
                let v = self.value(&args[1])?;
                self.b
                    .ins()
                    .atomic_rmw(t, flags, ir::AtomicRmwOp::Xchg, p, v)
            }
            _ => {
                let current = self.value(&args[1])?;
                let new = self.value(&args[2])?;
                self.b.ins().atomic_cas(flags, p, current, new)
            }
        };
        self.write(d, Val::Scalar(v))?;
        Ok(true)
    }

    /// A call of the `core` mod's `ptr` module, `name`, into `d` (`core.ptr.2`): null pointers,
    /// a pointer's distance on in values of its type, and casts, which keep the address.
    fn ptr_op(
        &mut self,
        name: &str,
        targs: &[Ty],
        args: &[Operand],
        d: &PlaceRef,
    ) -> Result<(), String> {
        let v = match name {
            "null" | "null_mut" => self.b.ins().iconst(types::I64, 0),
            "is_null" => {
                let p = self.value(&args[0])?;
                self.b.ins().icmp_imm_s(IntCC::Equal, p, 0)
            }
            "add" | "add_mut" => {
                let t = targs.first().ok_or("`ptr::add` without its type")?;
                let size = i64::from(self.layout(t)?.size);
                let p = self.value(&args[0])?;
                let n = self.value(&args[1])?;
                let by = self.b.ins().imul_imm_s(n, size);
                self.b.ins().iadd(p, by)
            }
            "addr_eq" => {
                let a = self.value(&args[0])?;
                let b = self.value(&args[1])?;
                self.b.ins().icmp(IntCC::Equal, a, b)
            }
            "cast" | "cast_mut_to" => self.value(&args[0])?,
            _ => return Err(format!("the built-in function `ptr::{name}`")),
        };
        self.write(d, Val::Scalar(v))
    }

    /// A call of the `core` mod's `gpu` module, `name`, into `d` (`gpu.1`): a kernel's WGSL
    /// (made here, kept as text), a value's bytes, and the runtime's dispatch and frame.
    fn gpu(
        &mut self,
        name: &str,
        targs: &[Ty],
        args: &[Operand],
        d: &PlaceRef,
    ) -> Result<(), String> {
        match name {
            "source" => {
                let (db, program) = (self.c.db, self.c.program);
                let k = moddil_gpu::kernel_of(db, program, &targs[0])
                    .ok_or("`gpu::source` of other than a `#[kernel]` named as a value")?;
                let layouts = &mut self.c.layouts;
                let mut offsets = |t: &Ty| match &layouts.of(t).ok()?.shape {
                    Shape::Fields(fs) => Some(fs.iter().map(|(o, _)| u64::from(*o)).collect()),
                    // An array's elements, one stride apart.
                    Shape::Array { stride, len, .. } => Some(
                        (0..u64::from(*len))
                            .map(|i| i * u64::from(*stride))
                            .collect(),
                    ),
                    _ => None,
                };
                let text = moddil_gpu::wgsl(db, program, k, &mut offsets)?;
                let data = self.c.text_data(&text)?;
                let gv = self.c.module.declare_data_in_func(data, self.b.func);
                let ptr = self.ptr();
                let p = self.b.ins().symbol_value(ptr, gv);
                let n = self.b.ins().iconst(types::I64, text.len() as i64);
                let w = self.wide(p, n);
                self.write(d, Val::Mem(w))
            }
            "bytes" => {
                let size = self.layout(&targs[0])?.size;
                let p = self.value(&args[0])?;
                let n = self.b.ins().iconst(types::I64, i64::from(size));
                let w = self.wide(p, n);
                self.write(d, Val::Mem(w))
            }
            "dispatch" => {
                let (sp, sn) = self.text(&args[0])?;
                let (pp, pn) = self.text(&args[1])?;
                let width = self.value(&args[2])?;
                let height = self.value(&args[3])?;
                let tile_width = self.value(&args[4])?;
                let tile_height = self.value(&args[5])?;
                let (op, on) = self.text(&args[6])?;
                let mode = self.value(&args[7])?;
                let r = self
                    .task_call(
                        "gpu_dispatch",
                        &[
                            sp,
                            sn,
                            pp,
                            pn,
                            width,
                            height,
                            tile_width,
                            tile_height,
                            op,
                            on,
                            mode,
                        ],
                    )
                    .expect("a code");
                self.write(d, Val::Scalar(r))
            }
            // A kernel's trap, reported by the GPU as its site (`gpu.trap.1`): trapped here, at
            // the check's own source.
            "trap" => {
                let (db, program) = (self.c.db, self.c.program);
                let k = moddil_gpu::kernel_of(db, program, &targs[0])
                    .ok_or("`gpu::trap` of other than a `#[kernel]` named as a value")?;
                let layouts = &mut self.c.layouts;
                let mut offsets = |t: &Ty| match &layouts.of(t).ok()?.shape {
                    Shape::Fields(fs) => Some(fs.iter().map(|(o, _)| u64::from(*o)).collect()),
                    Shape::Array { stride, len, .. } => Some(
                        (0..u64::from(*len))
                            .map(|i| i * u64::from(*stride))
                            .collect(),
                    ),
                    _ => None,
                };
                let (_, sites) = moddil_gpu::wgsl_sites(db, program, k, &mut offsets)?;
                let site = self.value(&args[1])?;
                let here = (self.file, self.span);
                for (i, (class, src, range)) in sites.iter().enumerate() {
                    let hit = self.b.create_block();
                    let next = self.b.create_block();
                    let is = self.b.ins().icmp_imm_s(IntCC::Equal, site, i as i64);
                    self.b.ins().brif(is, hit, &[], next, &[]);
                    self.b.switch_to_block(hit);
                    let (file, range) = match src {
                        Src::File(f) => (*f, *range),
                        _ => here,
                    };
                    self.trap_in(*class, file, range)?;
                    self.b.switch_to_block(next);
                }
                self.trap(TrapClass::Unreachable, here.1)?;
                // It does not return: what follows the call is never reached.
                let after = self.b.create_block();
                self.b.switch_to_block(after);
                Ok(())
            }
            "show" => {
                let (p, n) = self.text(&args[0])?;
                let width = self.value(&args[1])?;
                let height = self.value(&args[2])?;
                self.task_call("gpu_show", &[p, n, width, height]);
                Ok(())
            }
            "frame" => {
                let (p, _) = self.text(&args[0])?;
                self.task_call("gpu_frame", &[p]);
                Ok(())
            }
            _ => Err(format!("the built-in function `gpu::{name}`")),
        }
    }

    /// A call of the `core` mod's `os` module, `op`, into `d` (`os.1`): the runtime's function
    /// of the same name, with each `&str` and `&[u8]` as its address and length.
    fn os(&mut self, op: &str, args: &[Operand], d: &PlaceRef) -> Result<(), String> {
        let flags = ir::MemFlagsData::trusted();
        // A channel the OS feeds: its id is the `Receiver`'s.
        let channel = |this: &mut Self, kind: i64, id: ir::Value| -> Result<(), String> {
            let k = this.b.ins().iconst(types::I64, kind);
            let c = this.task_call("os_channel", &[k, id]).expect("a channel");
            let a = this.addr_of(d)?;
            this.b.ins().store(flags, c, a, 0);
            Ok(())
        };
        match op {
            "os_arg_count" | "os_random" | "os_wall_clock" | "os_task_tree_len" => {
                let r = self.task_call(op, &[]).expect("a number");
                self.write(d, Val::Scalar(r))
            }
            "os_arg_len" | "os_listen" => {
                let x = self.value(&args[0])?;
                let r = self.task_call(op, &[x]).expect("a number");
                self.write(d, Val::Scalar(r))
            }
            "os_arg_copy" => {
                let i = self.value(&args[0])?;
                let (p, n) = self.text(&args[1])?;
                self.task_call(op, &[i, p, n]);
                Ok(())
            }
            "os_write_stderr" | "os_task_tree_copy" => {
                let (p, n) = self.text(&args[0])?;
                self.task_call(op, &[p, n]);
                Ok(())
            }
            "os_connect" => {
                let (p, n) = self.text(&args[0])?;
                let port = self.value(&args[1])?;
                let r = self.task_call(op, &[p, n, port]).expect("a handle");
                self.write(d, Val::Scalar(r))
            }
            "os_take" => {
                let conn = self.value(&args[0])?;
                let (p, n) = self.text(&args[1])?;
                let r = self.task_call(op, &[conn, p, n]).expect("a count");
                self.write(d, Val::Scalar(r))
            }
            "os_send" => {
                let conn = self.value(&args[0])?;
                let (p, n) = self.text(&args[1])?;
                let r = self.task_call(op, &[conn, p, n]).expect("a flag");
                self.write(d, Val::Scalar(r))
            }
            "os_close" => {
                let x = self.value(&args[0])?;
                self.task_call(op, &[x]);
                Ok(())
            }
            "os_incoming" => {
                let x = self.value(&args[0])?;
                channel(self, 0, x)
            }
            "os_arrivals" => {
                let x = self.value(&args[0])?;
                channel(self, 1, x)
            }
            "os_signals" => {
                let zero = self.b.ins().iconst(types::I64, 0);
                channel(self, 2, zero)
            }
            _ => Err(format!("the `os` function `{op}`")),
        }
    }

    /// Calls the runtime's `f` with `args`: its one result.
    /// Calls the prelude's function `name` with scalar arguments `args`: its scalar result, if
    /// it has one.
    fn prelude_call(
        &mut self,
        name: &str,
        args: &[ir::Value],
    ) -> Result<Option<ir::Value>, String> {
        let f = self
            .c
            .items
            .prelude_fn(name)
            .ok_or_else(|| format!("the prelude's `{name}`"))?;
        let (id, params, ret) = self.c.declare_fn(f, &[])?;
        let mut vals = Vec::new();
        for (a, p) in args.iter().zip(&params) {
            vals.extend(self.pass_value(Val::Scalar(*a), p)?);
        }
        if ret == Ty::Unit {
            self.emit_call(Run::Direct(id, ret), vals, None, TextRange::default())?;
            return Ok(None);
        }
        let l = self.layout(&ret)?;
        let w = self.slot(&l);
        let at = PlaceRef::at(w, ret.clone());
        self.emit_call(Run::Direct(id, ret), vals, Some(&at), TextRange::default())?;
        let flags = ir::MemFlagsData::trusted();
        Ok(Some(self.b.ins().load(types::I64, flags, w, 0)))
    }

    /// New storage of `size` bytes aligned to `align`, for a `Buf`: from the allocator in scope
    /// (`alloc.make.1`).
    fn heap_alloc(&mut self, size: ir::Value, align: ir::Value) -> Result<ir::Value, String> {
        if self.c.uses_alloc {
            let p = self.prelude_call("__heap_alloc", &[size, align])?;
            return p.ok_or_else(|| "`__heap_alloc` without a result".to_string());
        }
        Ok(self.rt_call(self.c.rt.alloc, &[size, align]))
    }

    /// Frees a `Buf`'s storage, to the allocator it came from (`alloc.keep.1`).
    fn heap_free(&mut self, p: ir::Value, size: ir::Value, align: ir::Value) -> Result<(), String> {
        if self.c.uses_alloc {
            self.prelude_call("__heap_free", &[p, size, align])?;
            return Ok(());
        }
        let f = self
            .c
            .module
            .declare_func_in_func(self.c.rt.free, self.b.func);
        self.b.ins().call(f, &[p, size, align]);
        Ok(())
    }

    /// A `Buf`'s storage grown or shrunk, by the allocator it came from (`alloc.keep.1`).
    fn heap_realloc(
        &mut self,
        p: ir::Value,
        old: ir::Value,
        new: ir::Value,
        align: ir::Value,
    ) -> Result<ir::Value, String> {
        if self.c.uses_alloc {
            let q = self.prelude_call("__heap_realloc", &[p, old, new, align])?;
            return q.ok_or_else(|| "`__heap_realloc` without a result".to_string());
        }
        Ok(self.rt_call(self.c.rt.realloc, &[p, old, new, align]))
    }

    fn rt_call(&mut self, f: FuncId, args: &[ir::Value]) -> ir::Value {
        let f = self.c.module.declare_func_in_func(f, self.b.func);
        let call = self.b.ins().call(f, args);
        self.b.inst_results(call)[0]
    }

    /// Branches to `fail` (a trap of `class` at the call) unless `ok`.
    fn check(&mut self, ok: ir::Value, class: TrapClass, range: TextRange) -> Result<(), String> {
        let good = self.b.create_block();
        let bad = self.b.create_block();
        self.b.ins().brif(ok, good, &[], bad, &[]);
        self.b.switch_to_block(bad);
        self.trap(class, range)?;
        self.b.switch_to_block(good);
        Ok(())
    }

    /// One of the prelude's built-in functions, inline: its result written to `d`.
    fn intrinsic(
        &mut self,
        f: u32,
        targs: &[Ty],
        args: &[Operand],
        d: &PlaceRef,
        range: TextRange,
        catch: &Option<(Place, moddil_mir::BlockId)>,
    ) -> Result<(), String> {
        let items = self.c.items;
        if let Some(op) = task_op(items, f) {
            return self.task(op, targs, args, d, range, catch);
        }
        let sig = &items.fns[f as usize];
        if sig.src == Src::Prelude && items.modules[sig.module as usize].path == ["gpu"] {
            return self.gpu(sig.name.as_str(), targs, args, d);
        }
        if sig.src == Src::Prelude && items.modules[sig.module as usize].path == ["ffi"] {
            return self.ffi(sig.name.as_str(), args, d);
        }
        if sig.src == Src::Prelude
            && items.modules[sig.module as usize].path == ["ptr"]
            && sig.name != "cast_mut"
        {
            return self.ptr_op(sig.name.as_str(), targs, args, d);
        }
        let self_ty = match sig.container {
            moddil_sema::Container::Impl(k) => items.impls[k as usize].self_ty.clone(),
            _ => Ty::Unit,
        };
        let name = sig.name.as_str();
        let out = match (&self_ty, name) {
            (Ty::Buf(_), _) => return self.buf(name, targs, args, d, range),
            (Ty::Int | Ty::Num(_), "checked_to" | "truncate" | "wrap" | "saturate") => {
                self.convert(name, targs, args, d)?
            }
            (Ty::Int | Ty::Num(_) | Ty::Bool | Ty::Char, "text")
            | (Ty::Char, "quoted_text")
            | (Ty::Num(Num::F32 | Num::F64), "fixed_text" | "exp_text") => {
                self.display_text(&self_ty, name, args)?
            }
            (Ty::Str, _) => self.str_intrinsic(name, args, d, range)?,
            (Ty::Slice(_), _) => self.slice_intrinsic(name, args, d, range)?,
            (Ty::Num(n @ (Num::F32 | Num::F64)), _) => self.float_intrinsic(*n, name, args)?,
            (Ty::Char, _) => self.char_intrinsic(name, args, d)?,
            (Ty::Int, _) => self.int_intrinsic(name, args)?,
            (Ty::Unit, _) => self.mem_intrinsic(name, targs, args, d)?,
            (Ty::Adt(a, ..), "debug" | "info" | "warn" | "error")
                if items.adts[*a as usize].name == "Log" =>
            {
                self.log(name, args, range)?
            }
            _ => return unsupported(name),
        };
        self.write(d, out)
    }

    /// A built-in of `str` (`text.string.inline.1`, `text.pos.1`, `text.bytes.1`).
    fn str_intrinsic(
        &mut self,
        name: &str,
        args: &[Operand],
        d: &PlaceRef,
        range: TextRange,
    ) -> Result<Val, String> {
        let flags = ir::MemFlagsData::trusted();
        Ok(match name {
            "len" => {
                let (_, n) = self.text(&args[0])?;
                Val::Scalar(n)
            }
            // A byte of the text (`text.string.inline.1`).
            "byte_at" => {
                let (p, n) = self.text(&args[0])?;
                let i = self.value(&args[1])?;
                let ok = self.b.ins().icmp(IntCC::UnsignedLessThan, i, n);
                self.check(ok, TrapClass::IndexOutOfBounds, range)?;
                let at = self.b.ins().iadd(p, i);
                Val::Scalar(self.b.ins().load(types::I8, flags, at, 0))
            }
            // The character at a byte offset, if one starts there: an `Option<char>`, whose
            // `None` is the first value past the last character (its niche).
            "char_at_byte" => {
                self.char_option(&d.ty, name)?;
                let (p, n) = self.text(&args[0])?;
                let i = self.value(&args[1])?;
                Val::Scalar(self.rt_call(self.c.rt.char_at_byte, &[p, n, i]))
            }
            // Whether two texts are the same text: where they are, and how long (`text.pos.1`).
            "same_text" => {
                let (p, n) = self.text(&args[0])?;
                let (q, m) = self.text(&args[1])?;
                let at = self.b.ins().icmp(IntCC::Equal, p, q);
                let len = self.b.ins().icmp(IntCC::Equal, n, m);
                Val::Scalar(self.b.ins().band(at, len))
            }
            // The text between two byte offsets (`text.pos.1`).
            "bytes_between" => {
                let (p, n) = self.text(&args[0])?;
                let (a, b) = (self.value(&args[1])?, self.value(&args[2])?);
                let ok = self.rt_call(self.c.rt.is_between, &[p, n, a, b]);
                self.check(ok, TrapClass::IndexOutOfBounds, range)?;
                let start = self.b.ins().iadd(p, a);
                let len = self.b.ins().isub(b, a);
                Val::Mem(self.wide(start, len))
            }
            "eq" | "ne" | "cmp" | "lt" | "le" | "gt" | "ge" => {
                let (p, n) = self.text(&args[0])?;
                let (q, m) = self.text(&args[1])?;
                let c = self.rt_call(self.c.rt.str_cmp, &[p, n, q, m]);
                match name {
                    "cmp" => {
                        // `Ordering`'s variants are `Less`, `Equal` and `Greater`, in order.
                        let l = self.layout(&d.ty)?;
                        let Shape::Enum {
                            tag: Tag::Direct { offset, ty },
                            ..
                        } = &l.shape
                        else {
                            return unsupported(name);
                        };
                        let (offset, ty) = (*offset, *ty);
                        let v = self.b.ins().iadd_imm_s(c, 1);
                        let v = if ty == types::I8 {
                            v
                        } else {
                            self.b.ins().sextend(ty, v)
                        };
                        let a = self.addr_of(d)?;
                        self.b.ins().store(flags, v, a, offset as i32);
                        Val::Unit
                    }
                    _ => {
                        let cc = match name {
                            "eq" => IntCC::Equal,
                            "ne" => IntCC::NotEqual,
                            "lt" => IntCC::SignedLessThan,
                            "le" => IntCC::SignedLessThanOrEqual,
                            "gt" => IntCC::SignedGreaterThan,
                            _ => IntCC::SignedGreaterThanOrEqual,
                        };
                        Val::Scalar(self.b.ins().icmp_imm_s(cc, c, 0))
                    }
                }
            }
            "quoted_text" | "upper_text" | "lower_text" | "trim" => {
                let (p, n) = self.text(&args[0])?;
                let zero = self.b.ins().iconst(types::I64, 0);
                let code = match name {
                    "quoted_text" => 103,
                    "upper_text" => 104,
                    "lower_text" => 105,
                    _ => 106,
                };
                Val::Mem(self.host_text(code, zero, zero, p, n))
            }
            // Its bytes (`text.bytes.1`): the same address and length, as a slice.
            "as_bytes" => {
                let (p, n) = self.text(&args[0])?;
                Val::Mem(self.wide(p, n))
            }
            // The first `len` bytes, which are UTF-8, as text (`text.string.inline.1`).
            "view" => {
                let Val::Mem(w) = self.operand(&args[0])? else {
                    return unsupported(name);
                };
                let p = self.b.ins().load(types::I64, flags, w, 0);
                let n = self.value(&args[1])?;
                Val::Mem(self.wide(p, n))
            }
            _ => return unsupported(name),
        })
    }

    /// A built-in of a slice (`expr.slice.1`).
    fn slice_intrinsic(
        &mut self,
        name: &str,
        args: &[Operand],
        d: &PlaceRef,
        range: TextRange,
    ) -> Result<Val, String> {
        let flags = ir::MemFlagsData::trusted();
        Ok(match name {
            "len" => {
                let Val::Mem(w) = self.operand(&args[0])? else {
                    return Err("a slice without its length".to_string());
                };
                Val::Scalar(self.b.ins().load(types::I64, flags, w, 8))
            }
            // The two parts of an `&mut` slice borrow, disjoint (`expr.slice.1`).
            "split_at_mut_unchecked" => {
                // The element's type as the call has it (the impl's is a parameter).
                let Ty::Ref(_, slice) = self.operand_ty(&args[0])? else {
                    return unsupported(name);
                };
                let Ty::Slice(elem) = &*slice else {
                    return unsupported(name);
                };
                let (stride, _) = {
                    let e = self.layout(elem)?;
                    (e.size.div_ceil(e.align) * e.align, e.align)
                };
                let Val::Mem(w) = self.operand(&args[0])? else {
                    return unsupported(name);
                };
                let p = self.b.ins().load(types::I64, flags, w, 0);
                let at = self.value(&args[1])?;
                let n = self.value(&args[2])?;
                let zero = self.b.ins().iconst(types::I64, 0);
                let lo = self.b.ins().icmp(IntCC::SignedGreaterThanOrEqual, at, zero);
                let hi = self.b.ins().icmp(IntCC::SignedLessThanOrEqual, at, n);
                let ok = self.b.ins().band(lo, hi);
                self.check(ok, TrapClass::IndexOutOfBounds, range)?;
                let a = self.addr_of(d)?;
                let l = self.layout(&d.ty)?;
                let Shape::Fields(fs) = &l.shape else {
                    return unsupported(name);
                };
                let (oa, ob) = (fs[0].0, fs[1].0);
                let off = self.b.ins().imul_imm_s(at, i64::from(stride));
                let q = self.b.ins().iadd(p, off);
                let rest = self.b.ins().isub(n, at);
                let fa = self.offset(a, oa);
                let fb = self.offset(a, ob);
                self.b.ins().store(flags, p, fa, 0);
                self.b.ins().store(flags, at, fa, 8);
                self.b.ins().store(flags, q, fb, 0);
                self.b.ins().store(flags, rest, fb, 8);
                Val::Unit
            }
            _ => return unsupported(name),
        })
    }

    /// A float function (`num.float.fn.1`).
    fn float_intrinsic(&mut self, n: Num, name: &str, args: &[Operand]) -> Result<Val, String> {
        let x = self.value(&args[0])?;
        Ok(match name {
            "is_nan" => Val::Scalar(self.b.ins().fcmp(FloatCC::Unordered, x, x)),
            "is_finite" => {
                let a = self.b.ins().fabs(x);
                let inf = if n == Num::F32 {
                    self.b.ins().f32const(f32::INFINITY)
                } else {
                    self.b.ins().f64const(f64::INFINITY)
                };
                Val::Scalar(self.b.ins().fcmp(FloatCC::LessThan, a, inf))
            }
            _ => {
                let Some(k) = moddil_rt::FLOAT_FNS.iter().position(|f| *f == name) else {
                    return unsupported(name);
                };
                let y = match args.get(1) {
                    Some(a) => self.value(a)?,
                    None => x,
                };
                let k = self.b.ins().iconst(types::I32, k as i64);
                let rt = if n == Num::F32 {
                    self.c.rt.float_fn_f32
                } else {
                    self.c.rt.float_fn
                };
                Val::Scalar(self.rt_call(rt, &[k, x, y]))
            }
        })
    }

    /// Checks that `ty` is laid out as an `Option<char>` whose `None` is the first value past the
    /// last character (its niche), as the built-ins that give one write it.
    fn char_option(&mut self, ty: &Ty, name: &str) -> Result<(), String> {
        let l = self.layout(ty)?;
        let Shape::Enum {
            tag: Tag::Niche { niche, .. },
            ..
        } = &l.shape
        else {
            return unsupported(name);
        };
        if niche.first != 0x11_0000 || niche.offset != 0 {
            return unsupported(name);
        }
        Ok(())
    }

    /// A built-in of `char`.
    fn char_intrinsic(
        &mut self,
        name: &str,
        args: &[Operand],
        d: &PlaceRef,
    ) -> Result<Val, String> {
        Ok(match name {
            "is_alphabetic" | "is_numeric" | "is_whitespace" | "is_uppercase" | "is_lowercase" => {
                let which = match name {
                    "is_alphabetic" => 0,
                    "is_numeric" => 1,
                    "is_whitespace" => 2,
                    "is_uppercase" => 3,
                    _ => 4,
                };
                let c = self.value(&args[0])?;
                let w = self.b.ins().iconst(types::I32, which);
                Val::Scalar(self.rt_call(self.c.rt.char_is, &[w, c]))
            }
            // The `char` with this code, if there is one: an `Option<char>`, whose `None` is the
            // first value past the last character (its niche).
            "from_code" => {
                self.char_option(&d.ty, name)?;
                let c = self.value(&args[0])?;
                let below = self.b.ins().icmp_imm_s(IntCC::UnsignedLessThan, c, 0xD800);
                let past = self
                    .b
                    .ins()
                    .icmp_imm_s(IntCC::UnsignedGreaterThanOrEqual, c, 0xE000);
                let within = self
                    .b
                    .ins()
                    .icmp_imm_s(IntCC::UnsignedLessThan, c, 0x11_0000);
                let high = self.b.ins().band(past, within);
                let ok = self.b.ins().bor(below, high);
                let code = self.b.ins().ireduce(types::I32, c);
                let none = self.b.ins().iconst(types::I32, 0x11_0000);
                Val::Scalar(self.b.ins().select(ok, code, none))
            }
            "code" => {
                let c = self.value(&args[0])?;
                Val::Scalar(self.b.ins().uextend(types::I64, c))
            }
            "len_utf8" => {
                let c = self.value(&args[0])?;
                let mut n = self.b.ins().iconst(types::I64, 1);
                for (limit, bytes) in [(0x80, 2), (0x800, 3), (0x1_0000, 4)] {
                    let past = self
                        .b
                        .ins()
                        .icmp_imm_s(IntCC::UnsignedGreaterThanOrEqual, c, limit);
                    let k = self.b.ins().iconst(types::I64, bytes);
                    n = self.b.ins().select(past, k, n);
                }
                Val::Scalar(n)
            }
            _ => return unsupported(name),
        })
    }

    /// A built-in of `int`: rotation.
    fn int_intrinsic(&mut self, name: &str, args: &[Operand]) -> Result<Val, String> {
        Ok(match name {
            "rotate_left" => {
                let (x, y) = (self.value(&args[0])?, self.value(&args[1])?);
                let k = self.b.ins().band_imm_s(y, 63);
                Val::Scalar(self.b.ins().rotl(x, k))
            }
            _ => return unsupported(name),
        })
    }

    /// A number converted (`num.convert.1`), by `moddil_mir::convert` in the host.
    fn convert(
        &mut self,
        name: &str,
        targs: &[Ty],
        args: &[Operand],
        d: &PlaceRef,
    ) -> Result<Val, String> {
        let flags = ir::MemFlagsData::trusted();
        let from = self.operand_ty(&args[0])?;
        let to = targs.last().cloned().ok_or("the type converted to")?;
        let (Some(fc), Some(tc)) = (Self::num_code(&from), Self::num_code(&to)) else {
            return unsupported(name);
        };
        let x = self.value(&args[0])?;
        let (lo, hi) = self.number_words(x, &from);
        let how = moddil_rt::num::CONVERSIONS.iter().position(|h| *h == name);
        let how = self.b.ins().iconst(types::I32, how.unwrap_or(0) as i64);
        let fc = self.b.ins().iconst(types::I32, i64::from(fc));
        let tc = self.b.ins().iconst(types::I32, i64::from(tc));
        let l = Layout {
            size: 16,
            align: 8,
            shape: Shape::Wide,
            niche: None,
        };
        let out = self.slot(&l);
        let ok = self.rt_call(self.c.rt.convert, &[how, fc, lo, hi, tc, out]);
        let lo = self.b.ins().load(types::I64, flags, out, 0);
        let hi = self.b.ins().load(types::I64, flags, out, 8);
        let v = self.number_of_words(lo, hi, &to)?;
        Ok(if name != "checked_to" {
            Val::Scalar(v)
        } else {
            // An `Option`: `Some` of it, or `None`.
            let l = self.layout(&d.ty)?;
            let Shape::Enum { tag, variants } = &l.shape else {
                return unsupported(name);
            };
            let (tag, payload) = (tag.clone(), variants[1][0].0);
            let a = self.addr_of(d)?;
            let some = self.b.create_block();
            let none = self.b.create_block();
            let join = self.b.create_block();
            self.b.ins().brif(ok, some, &[], none, &[]);
            self.b.switch_to_block(none);
            self.set_variant(a, &tag, 0);
            self.b.ins().jump(join, &[]);
            self.b.switch_to_block(some);
            let at = self.offset(a, payload);
            self.b.ins().store(flags, v, at, 0);
            self.set_variant(a, &tag, 1);
            self.b.ins().jump(join, &[]);
            self.b.switch_to_block(join);
            Val::Unit
        })
    }

    /// What `Display` writes for a number, a `bool` or a `char` (`text.display.1`), a `char`
    /// quoted, or a float with fixed digits (`expr.fmt.spec.3`): made in the host.
    fn display_text(&mut self, self_ty: &Ty, name: &str, args: &[Operand]) -> Result<Val, String> {
        let ty = self.operand_ty(&args[0])?;
        // The impl's own type: a refined number's is its base (`Small` is an `int`).
        let inner = self_ty.clone();
        let x = match self.operand(&args[0])? {
            Val::Scalar(p) if matches!(ty, Ty::Ref(..)) => {
                let at = PlaceRef::at(p, inner.clone());
                match self.read(&at)? {
                    Val::Scalar(v) => v,
                    _ => return unsupported(name),
                }
            }
            Val::Scalar(v) => v,
            _ => return unsupported(name),
        };
        let code = match (&inner, name) {
            (Ty::Char, "quoted_text") => 102,
            (Ty::Bool, _) => 100,
            (Ty::Char, _) => 101,
            (Ty::Num(Num::F32), "fixed_text") => 107,
            (Ty::Num(Num::F64), "fixed_text") => 108,
            (Ty::Num(Num::F32), "exp_text") => 109,
            (Ty::Num(Num::F64), "exp_text") => 110,
            (t, _) => Self::num_code(t).ok_or("text of something not a number")?,
        };
        let (lo, mut hi) = self.number_words(x, &inner);
        // A float's fixed digits travel where a wide number's high word would; for its
        // exponent form, `(digits + 1) * 2 + upper`.
        if name == "fixed_text" {
            hi = self.value(&args[1])?;
        } else if name == "exp_text" {
            let digits = self.value(&args[1])?;
            let upper = self.value(&args[2])?;
            let d = self.b.ins().iadd_imm_s(digits, 1);
            let d = self.b.ins().ishl_imm_u(d, 1);
            let u = self.b.ins().uextend(types::I64, upper);
            hi = self.b.ins().bor(d, u);
        }
        let ptr = self.ptr();
        let null = self.b.ins().iconst(ptr, 0);
        let zero = self.b.ins().iconst(types::I64, 0);
        Ok(Val::Mem(self.host_text(code, lo, hi, null, zero)))
    }

    /// A free built-in on memory: `swap`, `replace`, and `cast_mut` (`core.ptr.1`).
    fn mem_intrinsic(
        &mut self,
        name: &str,
        targs: &[Ty],
        args: &[Operand],
        d: &PlaceRef,
    ) -> Result<Val, String> {
        Ok(match name {
            "swap" => {
                let (Val::Scalar(a), Val::Scalar(b)) =
                    (self.operand(&args[0])?, self.operand(&args[1])?)
                else {
                    return unsupported(name);
                };
                let Some(t) = targs.first().cloned() else {
                    return unsupported(name);
                };
                let l = self.layout(&t)?;
                let tmp = self.slot(&l);
                self.copy(tmp, a, l.size);
                self.copy(a, b, l.size);
                self.copy(b, tmp, l.size);
                Val::Unit
            }
            // The same pointer, to change through (`core.ptr.1`).
            "cast_mut" => self.operand(&args[0])?,
            // Compile-time code's error, reached when the program runs: a trap
            // (`comptime.error.1`).
            "compile_error" => {
                let span = self.span;
                self.trap(TrapClass::Unreachable, span)?;
                // What follows is never reached: in a block of its own.
                let dead = self.b.create_block();
                self.b.switch_to_block(dead);
                Val::Unit
            }
            "replace" => {
                let Val::Scalar(p) = self.operand(&args[0])? else {
                    return unsupported(name);
                };
                let at = PlaceRef::at(p, d.ty.clone());
                // The old value, out first, then the new one in.
                let old = self.read(&at)?;
                let l = self.layout(&d.ty)?;
                let old = match old {
                    Val::Mem(a) => {
                        let tmp = self.slot(&l);
                        self.copy(tmp, a, l.size);
                        Val::Mem(tmp)
                    }
                    v => v,
                };
                let new = self.operand(&args[1])?;
                self.write(&at, new)?;
                old
            }
            _ => return unsupported(name),
        })
    }

    /// The ambient log (`obs.log.1`): the message's text to standard error, then the message
    /// dropped.
    fn log(&mut self, name: &str, args: &[Operand], range: TextRange) -> Result<Val, String> {
        let items = self.c.items;
        let flags = ir::MemFlagsData::trusted();
        let msg_ty = self.operand_ty(&args[1])?;
        let Val::Mem(msg) = self.operand(&args[1])? else {
            return unsupported(name);
        };
        let as_str = items
            .builtin_method(&items.string_ty(), "as_str")
            .ok_or("the prelude's `String::as_str`")?;
        let (id, params, ret) = self.c.declare_fn(as_str, &[])?;
        let l = self.layout(&ret)?;
        let w = self.slot(&l);
        let at = PlaceRef::at(w, ret.clone());
        let vals = self.pass_value(Val::Scalar(msg), &params[0])?;
        self.emit_call(Run::Direct(id, ret), vals, Some(&at), range)?;
        let p = self.b.ins().load(types::I64, flags, w, 0);
        let n = self.b.ins().load(types::I64, flags, w, 8);
        let level = moddil_rt::host::LEVELS
            .iter()
            .position(|l| *l == name)
            .unwrap_or(0);
        let level = self.b.ins().iconst(types::I32, level as i64);
        let f = self
            .c
            .module
            .declare_func_in_func(self.c.rt.log, self.b.func);
        self.b.ins().call(f, &[level, p, n]);
        self.drop_value(msg, &msg_ty)?;
        Ok(Val::Unit)
    }

    /// A `Buf`'s built-in function (`text.string.inline.1`'s storage): its address, length and
    /// capacity, from a borrow of it.
    fn buf(
        &mut self,
        name: &str,
        targs: &[Ty],
        args: &[Operand],
        d: &PlaceRef,
        range: TextRange,
    ) -> Result<(), String> {
        let flags = ir::MemFlagsData::trusted();
        let elem = targs.first().cloned().ok_or("a `Buf` without its type")?;
        if matches!(elem, Ty::Dyn(..)) && !matches!(name, "len" | "as_slice" | "as_mut_slice") {
            return Err(format!("`Buf<{elem}>::{name}`"));
        }
        let (stride, align) = if let Ty::Dyn(..) = elem {
            (0, 8)
        } else {
            let e = self.layout(&elem)?;
            (e.size.div_ceil(e.align) * e.align, e.align)
        };
        let stride_v = |lw: &mut Self| lw.b.ins().iconst(types::I64, i64::from(stride));
        let elem_at = |lw: &mut Self, p: ir::Value, i: ir::Value| {
            let off = lw.b.ins().imul_imm_s(i, i64::from(stride));
            lw.b.ins().iadd(p, off)
        };
        // The new `Buf` itself, for the two that make one.
        if name == "with_capacity" || name == "from_array" {
            let a = self.addr_of(d)?;
            let (n, len, src) = if name == "with_capacity" {
                let n = self.value(&args[0])?;
                let zero = self.b.ins().iconst(types::I64, 0);
                let neg = self.b.ins().icmp(IntCC::SignedLessThan, n, zero);
                let n = self.b.ins().select(neg, zero, n);
                (n, zero, None)
            } else {
                let Ty::Array(_, k) = self.operand_ty(&args[0])? else {
                    return Err("`from_array` of a value that is not an array".to_string());
                };
                let Ty::Len(k) = *k else {
                    return Err("an array of unknown length".to_string());
                };
                let n = self.b.ins().iconst(types::I64, k);
                (n, n, Some(self.operand(&args[0])?))
            };
            let bytes = self.b.ins().imul_imm_s(n, i64::from(stride));
            let al = self.b.ins().iconst(types::I64, i64::from(align));
            let p = self.heap_alloc(bytes, al)?;
            if let Some(Val::Mem(src)) = src {
                let f = self
                    .c
                    .module
                    .declare_func_in_func(self.c.rt.memmove, self.b.func);
                self.b.ins().call(f, &[p, src, bytes]);
            }
            self.b.ins().store(flags, p, a, 0);
            self.b.ins().store(flags, len, a, 8);
            self.b.ins().store(flags, n, a, 16);
            return Ok(());
        }
        // The buffer, given up, holds exactly its array's elements (`repeat_array`): they move
        // into the array, and its storage is freed.
        if name == "into_array" {
            let Ty::Array(_, k) = &d.ty else {
                return Err("`into_array` into something that is not an array".to_string());
            };
            let Ty::Len(k) = **k else {
                return Err("an array of unknown length".to_string());
            };
            let Val::Mem(src) = self.operand(&args[0])? else {
                return Err("a `Buf` not in memory".to_string());
            };
            let p = self.b.ins().load(types::I64, flags, src, 0);
            let cap = self.b.ins().load(types::I64, flags, src, 16);
            let a = self.addr_of(d)?;
            let bytes = self.b.ins().iconst(types::I64, k * i64::from(stride));
            let f = self
                .c
                .module
                .declare_func_in_func(self.c.rt.memmove, self.b.func);
            self.b.ins().call(f, &[a, p, bytes]);
            let size = self.b.ins().imul_imm_s(cap, i64::from(stride));
            let al = self.b.ins().iconst(types::I64, i64::from(align));
            self.heap_free(p, size, al)?;
            return Ok(());
        }
        // The others take the `Buf` by borrow.
        let Val::Scalar(b) = self.operand(&args[0])? else {
            return Err("a `Buf` not by borrow".to_string());
        };
        let p = self.b.ins().load(types::I64, flags, b, 0);
        let n = self.b.ins().load(types::I64, flags, b, 8);
        let cap = self.b.ins().load(types::I64, flags, b, 16);
        let out = match name {
            "len" => Val::Scalar(n),
            "capacity" => Val::Scalar(cap),
            "as_slice" | "as_mut_slice" => {
                let w = self.wide(p, n);
                if let Ty::Dyn(..) = elem {
                    // A `[dyn Trait]`: its vtable too.
                    let l = Layout {
                        size: 24,
                        align: 8,
                        shape: Shape::Wide,
                        niche: None,
                    };
                    let w3 = self.slot(&l);
                    self.copy(w3, w, 16);
                    let vt = self.b.ins().load(types::I64, flags, b, 24);
                    self.b.ins().store(flags, vt, w3, 16);
                    Val::Mem(w3)
                } else {
                    Val::Mem(w)
                }
            }
            "set_capacity" => {
                let c = self.value(&args[1])?;
                let small = self.b.ins().icmp(IntCC::SignedLessThan, c, n);
                let c = self.b.ins().select(small, n, c);
                let s = stride_v(self);
                let old = self.b.ins().imul(cap, s);
                let new = self.b.ins().imul(c, s);
                let al = self.b.ins().iconst(types::I64, i64::from(align));
                let q = self.heap_realloc(p, old, new, al)?;
                self.b.ins().store(flags, q, b, 0);
                self.b.ins().store(flags, c, b, 16);
                Val::Unit
            }
            "push" | "insert" => {
                let room = self.b.ins().icmp(IntCC::SignedLessThan, n, cap);
                self.check(room, TrapClass::CapacityExceeded, range)?;
                let (i, x) = if name == "push" {
                    (n, &args[1])
                } else {
                    let i = self.value(&args[1])?;
                    let ok = self.b.ins().icmp(IntCC::UnsignedLessThanOrEqual, i, n);
                    self.check(ok, TrapClass::IndexOutOfBounds, range)?;
                    // Those from `i` on, one along.
                    let from = elem_at(self, p, i);
                    let to = self.b.ins().iadd_imm_s(from, i64::from(stride));
                    let k = self.b.ins().isub(n, i);
                    let bytes = self.b.ins().imul_imm_s(k, i64::from(stride));
                    let f = self
                        .c
                        .module
                        .declare_func_in_func(self.c.rt.memmove, self.b.func);
                    self.b.ins().call(f, &[to, from, bytes]);
                    (i, &args[2])
                };
                let at = elem_at(self, p, i);
                let v = self.operand(x)?;
                let place = PlaceRef::at(at, elem.clone());
                self.write(&place, v)?;
                let n1 = self.b.ins().iadd_imm_s(n, 1);
                self.b.ins().store(flags, n1, b, 8);
                Val::Unit
            }
            "pop" => {
                // An `Option<T>`: `None`, or `Some` of the last element, moved out.
                let l = self.layout(&d.ty)?;
                let Shape::Enum { tag, variants } = &l.shape else {
                    return Err(format!("`pop` into a `{}`", d.ty));
                };
                let (tag, payload) = (tag.clone(), variants[1][0].0);
                let a = self.addr_of(d)?;
                let empty = self.b.ins().icmp_imm_s(IntCC::Equal, n, 0);
                let none = self.b.create_block();
                let some = self.b.create_block();
                let join = self.b.create_block();
                self.b.ins().brif(empty, none, &[], some, &[]);
                self.b.switch_to_block(none);
                self.set_variant(a, &tag, 0);
                self.b.ins().jump(join, &[]);
                self.b.switch_to_block(some);
                let last = self.b.ins().iadd_imm_s(n, -1);
                let at = elem_at(self, p, last);
                let to = self.offset(a, payload);
                self.copy(to, at, stride);
                self.set_variant(a, &tag, 1);
                self.b.ins().store(flags, last, b, 8);
                self.b.ins().jump(join, &[]);
                self.b.switch_to_block(join);
                return Ok(());
            }
            "remove" => {
                let i = self.value(&args[1])?;
                let ok = self.b.ins().icmp(IntCC::UnsignedLessThan, i, n);
                self.check(ok, TrapClass::IndexOutOfBounds, range)?;
                let at = elem_at(self, p, i);
                let v = self.take_at(at, &elem)?;
                self.write(d, v)?;
                // Those after it, one back.
                let from = self.b.ins().iadd_imm_s(at, i64::from(stride));
                let k = self.b.ins().isub(n, i);
                let k = self.b.ins().iadd_imm_s(k, -1);
                let bytes = self.b.ins().imul_imm_s(k, i64::from(stride));
                let f = self
                    .c
                    .module
                    .declare_func_in_func(self.c.rt.memmove, self.b.func);
                self.b.ins().call(f, &[at, from, bytes]);
                let n1 = self.b.ins().iadd_imm_s(n, -1);
                self.b.ins().store(flags, n1, b, 8);
                return Ok(());
            }
            "swap" => {
                let i = self.value(&args[1])?;
                let j = self.value(&args[2])?;
                let ok = self.b.ins().icmp(IntCC::UnsignedLessThan, i, n);
                self.check(ok, TrapClass::IndexOutOfBounds, range)?;
                let ok = self.b.ins().icmp(IntCC::UnsignedLessThan, j, n);
                self.check(ok, TrapClass::IndexOutOfBounds, range)?;
                let x = elem_at(self, p, i);
                let y = elem_at(self, p, j);
                let e = self.layout(&elem)?;
                let tmp = self.slot(&e);
                self.copy(tmp, x, stride);
                let f = self
                    .c
                    .module
                    .declare_func_in_func(self.c.rt.memmove, self.b.func);
                let sz = stride_v(self);
                self.b.ins().call(f, &[x, y, sz]);
                self.copy(y, tmp, stride);
                Val::Unit
            }
            _ => return Err(format!("`Buf::{name}`")),
        };
        self.write(d, out)
    }
}

/// A built-in function native code does not compile yet.
fn unsupported<T>(name: &str) -> Result<T, String> {
    Err(format!("the built-in function `{name}`"))
}

/// What a C-ABI wrapper does when the Moddil function it calls traps.
#[derive(Clone, Copy)]
enum OnTrap {
    /// Ends the process: a function pointer's call (`type.fnptr.1`).
    End,
    /// Gives 2: `#[export(status)]` (`ffi.export.2`).
    Status,
    /// Gives the result type's zero, or the value (`#[export(on_trap = v)]`), and records the
    /// trap for `moddil_trapped()` (`ffi.export.3`).
    Give(Option<i128>),
}

/// A zero of type `t`.
fn zero(b: &mut FunctionBuilder, t: ir::Type) -> ir::Value {
    if t == types::F32 {
        b.ins().f32const(0.0)
    } else if t == types::F64 {
        b.ins().f64const(0.0)
    } else {
        b.ins().iconst(t, 0)
    }
}

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
    AggregateKind, BinOp, Body, Const, Local, Operand, Place, Projection, Rvalue, Statement,
    Terminator, TrapClass, UnOp,
};
use moddil_sema::{Callee, Db, Item, ItemTree, Num, Owner, Program, Solver, TraitRef, Ty};
use moddil_syntax::TextRange;

use crate::layout::{Layout, Layouts, Shape, Tag};

/// What a site is.
#[derive(Clone, Debug)]
pub(crate) enum Site {
    /// A check: its class, the file and where.
    Trap(TrapClass, u32, TextRange),
    /// A call: the file, where, and whether the code that makes it is tracked
    /// (`#[track_caller]`, the prelude's or derived code). Too deep a call traps here, and a
    /// trap in tracked code is reported at the innermost call untracked code makes.
    Call(u32, TextRange, bool),
    /// A `// =>` observation: the file, the expression, and its type.
    Observe(u32, TextRange, Ty),
}

/// A program compiled for one entry point, ready to run.
pub(crate) struct Compiled<'db> {
    pub(crate) module: JITModule,
    /// The entry trampoline: `fn(out: *mut u8) -> i8`, which writes the result at `out`.
    pub(crate) entry: FuncId,
    /// The entry's result type.
    pub(crate) ret: Ty,
    pub(crate) sites: Vec<Site>,
    pub(crate) items: &'db ItemTree,
    /// The types `dyn` borrows point to, by the number their vtables start with.
    pub(crate) dyn_types: Vec<Ty>,
}

/// The Cranelift type of a scalar `ty`: `None` for `()`, an error for a type that is not a
/// scalar.
pub(crate) fn clif_type(ty: &Ty) -> Result<Option<ir::Type>, String> {
    Ok(Some(match ty {
        Ty::Unit | Ty::Never => return Ok(None),
        // A borrow is a pointer, as wide as an `int` on the 64-bit targets C1 runs on.
        Ty::Int | Ty::Ref(..) => types::I64,
        Ty::Bool => types::I8,
        Ty::Char => types::I32,
        Ty::Num(n) => match n {
            Num::I8 | Num::U8 => types::I8,
            Num::I16 | Num::U16 => types::I16,
            Num::I32 | Num::U32 => types::I32,
            Num::U64 => types::I64,
            Num::I128 | Num::U128 => types::I128,
            Num::F32 => types::F32,
            Num::F64 => types::F64,
        },
        t => return Err(format!("a value of type `{t}`")),
    }))
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

/// The runtime functions the code calls.
struct Rt {
    trap: FuncId,
    trap_tracked: FuncId,
    observe: FuncId,
    enter: FuncId,
    leave: FuncId,
    fmod: FuncId,
    fmodf: FuncId,
    float_fn: FuncId,
    float_fn_f32: FuncId,
    char_at_byte: FuncId,
    is_between: FuncId,
    str_cmp: FuncId,
    char_is: FuncId,
}

/// A function instance: a function and its (concrete) type arguments.
type Instance = (u32, Vec<Ty>);

struct Compiler<'db> {
    db: &'db dyn Db,
    program: Program,
    items: &'db ItemTree,
    layouts: Layouts<'db>,
    module: JITModule,
    rt: Rt,
    /// Each instance declared: its id, and its parameters' and result's types.
    fns: HashMap<Instance, (FuncId, Vec<Ty>, Ty)>,
    /// The instances declared and not yet compiled.
    queue: Vec<Instance>,
    sites: Vec<Site>,
    /// Each text constant's data.
    strs: HashMap<String, DataId>,
    /// Each vtable: for a type and a trait (with its arguments).
    vtables: HashMap<(Ty, TraitRef), DataId>,
    dyn_types: Vec<Ty>,
}

/// Compiles what `item` (a test, or a function without parameters) reaches.
pub(crate) fn compile<'db>(
    db: &'db dyn Db,
    program: Program,
    item: Item,
) -> Result<Compiled<'db>, String> {
    let mut flags = settings::builder();
    flags.set("opt_level", "speed").map_err(|e| e.to_string())?;
    // `i128` and `u128` are passed and returned as two registers.
    flags
        .set("enable_llvm_abi_extensions", "true")
        .map_err(|e| e.to_string())?;
    let isa = cranelift_native::builder()
        .map_err(|e| e.to_string())?
        .finish(settings::Flags::new(flags))
        .map_err(|e| e.to_string())?;
    let mut builder = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
    builder.symbol("moddil_rt_trap", moddil_rt::trap as *const u8);
    builder.symbol(
        "moddil_rt_trap_tracked",
        moddil_rt::trap_tracked as *const u8,
    );
    builder.symbol("moddil_rt_float_fn", moddil_rt::float_fn as *const u8);
    builder.symbol(
        "moddil_rt_float_fn_f32",
        moddil_rt::float_fn_f32 as *const u8,
    );
    builder.symbol(
        "moddil_rt_char_at_byte",
        moddil_rt::char_at_byte as *const u8,
    );
    builder.symbol("moddil_rt_is_between", moddil_rt::is_between as *const u8);
    builder.symbol("moddil_rt_str_cmp", moddil_rt::str_cmp as *const u8);
    builder.symbol("moddil_rt_char_is", moddil_rt::char_is as *const u8);
    builder.symbol("moddil_rt_observe", moddil_rt::observe as *const u8);
    builder.symbol("moddil_rt_enter", moddil_rt::enter as *const u8);
    builder.symbol("moddil_rt_leave", moddil_rt::leave as *const u8);
    builder.symbol("moddil_rt_fmod", moddil_rt::fmod as *const u8);
    builder.symbol("moddil_rt_fmodf", moddil_rt::fmodf as *const u8);
    let mut module = JITModule::new(builder);
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
    let rt = Rt {
        trap: import("moddil_rt_trap", &[types::I32], None)?,
        trap_tracked: import("moddil_rt_trap_tracked", &[types::I32], None)?,
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
    };
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
    };
    let (target, ret) = match item {
        Item::Test(_) => {
            let id = c.declare_item("test", &[], &Ty::Unit)?;
            c.body_of(item, &[], id, &[], &Ty::Unit)?;
            (id, Ty::Unit)
        }
        Item::Fn(f) => {
            let (id, params, ret) = c.declare_fn(f, &[])?;
            if !params.is_empty() {
                return Err("an entry point with parameters".to_string());
            }
            (id, ret)
        }
        Item::Closure(..) => return Err("a closure as an entry point".to_string()),
    };
    while let Some(inst) = c.queue.pop() {
        let (id, params, ret) = c.fns[&inst].clone();
        c.body_of(Item::Fn(inst.0), &inst.1, id, &params, &ret)?;
    }
    let entry = c.trampoline(target, &ret)?;
    c.module.finalize_definitions().map_err(|e| e.to_string())?;
    Ok(Compiled {
        module: c.module,
        entry,
        ret,
        sites: c.sites,
        items,
        dyn_types: c.dyn_types,
    })
}

impl Compiler<'_> {
    fn pass(&mut self, ty: &Ty) -> Result<Pass, String> {
        let l = self.layouts.of(ty)?;
        Ok(pass_of(&l))
    }

    /// The signature of a function with these parameters and result.
    fn signature(&mut self, params: &[Ty], ret: &Ty) -> Result<Signature, String> {
        let ptr = self.module.target_config().pointer_type();
        let mut sig = self.module.make_signature();
        sig.call_conv = CallConv::Tail;
        for p in params {
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
            // Where the caller wants the result, last.
            Pass::Indirect => sig.params.push(AbiParam::new(ptr)),
        }
        Ok(sig)
    }

    fn declare_item(&mut self, name: &str, params: &[Ty], ret: &Ty) -> Result<FuncId, String> {
        let sig = self.signature(params, ret)?;
        self.module
            .declare_anonymous_function(&sig)
            .map_err(|e| format!("{name}: {e}"))
    }

    /// `ty` with the type arguments `targs` for its parameters, its associated types resolved.
    fn concrete(&self, ty: &Ty, targs: &[Ty]) -> Ty {
        Solver::new(self.items, &[]).deep(&ty.subst(targs))
    }

    /// Function `f` with type arguments `targs`: declared (and queued to be compiled) the first
    /// time it is named.
    fn declare_fn(&mut self, f: u32, targs: &[Ty]) -> Result<(FuncId, Vec<Ty>, Ty), String> {
        let key = (f, targs.to_vec());
        if let Some(d) = self.fns.get(&key) {
            return Ok(d.clone());
        }
        let sig = &self.items.fns[f as usize];
        if sig.intrinsic || !sig.has_body {
            return Err(format!("the built-in function `{}`", sig.name));
        }
        let name = sig.name.clone();
        let body = self.mir(Item::Fn(f))?;
        let params: Vec<Ty> = body.locals[1..=body.arg_count]
            .iter()
            .map(|l| self.concrete(&l.ty, targs))
            .collect();
        let ret = self.concrete(&body.locals[0].ty, targs);
        let id = self.declare_item(&name, &params, &ret)?;
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
    /// trait's own. So the vtable of a trait starts with that of its first supertrait, and a
    /// `dyn` borrow of it is one of that supertrait as it is.
    fn dyn_methods(&self, self_ty: &Ty, tr: &TraitRef) -> Vec<(u32, TraitRef)> {
        let mut out = Vec::new();
        self.methods_into(self_ty, tr, &mut out, &mut HashSet::new());
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

    /// The vtable of `ty` for `tr`: its number in [`Compiled::dyn_types`], then each method's
    /// address (null for one a `dyn` borrow cannot call).
    fn vtable(&mut self, ty: &Ty, tr: &TraitRef) -> Result<DataId, String> {
        let key = (ty.clone(), tr.clone());
        if let Some(d) = self.vtables.get(&key) {
            return Ok(*d);
        }
        let methods = self.dyn_methods(ty, tr);
        let id = self
            .module
            .declare_anonymous_data(false, false)
            .map_err(|e| e.to_string())?;
        self.vtables.insert(key, id);
        let mut desc = DataDescription::new();
        let n = self.dyn_types.len() as u64;
        self.dyn_types.push(ty.clone());
        let mut bytes = n.to_le_bytes().to_vec();
        bytes.resize(8 * (1 + methods.len()), 0);
        desc.define(bytes.into_boxed_slice());
        for (k, (m, t)) in methods.iter().enumerate() {
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
            desc.write_function_addr(8 * (1 + k as u32), fref);
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
        let mut ctx = self.module.make_context();
        ctx.func.signature = self.signature(params, ret)?;
        let mut fctx = FunctionBuilderContext::new();
        let mut lw = Lower {
            c: self,
            body: &body,
            targs,
            tys,
            file,
            tracked,
            b: FunctionBuilder::new(&mut ctx.func, &mut fctx),
            locs: Vec::new(),
            blocks: Vec::new(),
        };
        lw.function()?;
        let config = lw.c.module.target_config();
        lw.b.finalize(config);
        self.module
            .define_function(id, &mut ctx)
            .map_err(|e| format!("Cranelift: {e:?}"))?;
        self.module.clear_context(&mut ctx);
        Ok(())
    }

    /// `fn(out: *mut u8) -> i8`, in the platform's C convention: calls `target` and stores its
    /// result's bytes at `out`.
    fn trampoline(&mut self, target: FuncId, ret: &Ty) -> Result<FuncId, String> {
        let ptr = self.module.target_config().pointer_type();
        let mut sig = self.module.make_signature();
        sig.params.push(AbiParam::new(ptr));
        sig.returns.push(AbiParam::new(types::I8));
        let id = self
            .module
            .declare_function("entry", Linkage::Export, &sig)
            .map_err(|e| e.to_string())?;
        let pass = self.pass(ret)?;
        let mut ctx = self.module.make_context();
        ctx.func.signature = sig;
        let mut fctx = FunctionBuilderContext::new();
        let mut b = FunctionBuilder::new(&mut ctx.func, &mut fctx);
        let block = b.create_block();
        b.append_block_params_for_function_params(block);
        b.switch_to_block(block);
        let out = b.block_params(block)[0];
        let f = self.module.declare_func_in_func(target, b.func);
        let call = if pass == Pass::Indirect {
            b.ins().call(f, &[out])
        } else {
            b.ins().call(f, &[])
        };
        let results = b.inst_results(call).to_vec();
        let flags = ir::MemFlagsData::trusted();
        match pass {
            Pass::Scalar(_) => {
                b.ins().store(flags, results[1], out, 0);
            }
            Pass::Words(ts) => {
                let mut at = 0;
                for (k, t) in ts.iter().enumerate() {
                    b.ins().store(flags, results[1 + k], out, at);
                    at += t.bytes() as i32;
                }
            }
            Pass::Nothing | Pass::Indirect => {}
        }
        b.ins().return_(&[results[0]]);
        b.seal_all_blocks();
        b.finalize(self.module.target_config());
        self.module
            .define_function(id, &mut ctx)
            .map_err(|e| format!("Cranelift: {e:?}"))?;
        Ok(id)
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
struct Lower<'a, 'c, 'db> {
    c: &'a mut Compiler<'db>,
    body: &'a Body,
    /// Its type arguments.
    targs: &'a [Ty],
    /// Its locals' types, concrete.
    tys: Vec<Ty>,
    file: u32,
    /// Whether it is `#[track_caller]`, the prelude's or derived code.
    tracked: bool,
    b: FunctionBuilder<'c>,
    locs: Vec<Loc>,
    /// Each MIR block's Cranelift block (`None` for cleanup code, which only unwinding runs).
    blocks: Vec<Option<ir::Block>>,
}

impl Lower<'_, '_, '_> {
    fn ptr(&self) -> ir::Type {
        self.c.module.target_config().pointer_type()
    }

    fn layout(&mut self, ty: &Ty) -> Result<Arc<Layout>, String> {
        self.c.layouts.of(ty)
    }

    /// A stack slot of layout `l`'s size, at least 8-byte aligned, and its address.
    fn slot(&mut self, l: &Layout) -> ir::Value {
        let align = l.align.max(8);
        let slot = self.b.create_sized_stack_slot(ir::StackSlotData::new(
            ir::StackSlotKind::ExplicitSlot,
            l.size.max(1),
            align.trailing_zeros() as u8,
        ));
        let ptr = self.ptr();
        self.b.ins().stack_addr(ptr, slot, 0)
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
                _ if arg && pass == Pass::Indirect => {
                    // The caller's copy is the parameter's own.
                    k += 1;
                    Loc::Mem(params[k - 1])
                }
                _ => {
                    let addr = self.slot(&layout);
                    if arg {
                        match &pass {
                            Pass::Scalar(_) => {
                                self.b
                                    .ins()
                                    .store(ir::MemFlagsData::trusted(), params[k], addr, 0);
                                k += 1;
                            }
                            Pass::Words(ts) => {
                                let mut at = 0;
                                for t in ts {
                                    self.b.ins().store(
                                        ir::MemFlagsData::trusted(),
                                        params[k],
                                        addr,
                                        at,
                                    );
                                    at += t.bytes() as i32;
                                    k += 1;
                                }
                            }
                            Pass::Nothing | Pass::Indirect => {}
                        }
                    }
                    Loc::Mem(addr)
                }
            };
            self.locs.push(loc);
        }
        // A result returned indirectly is written where the caller says, the last parameter.
        let ret_layout = self.c.layouts.of(&self.tys[0])?;
        if pass_of(&ret_layout) == Pass::Indirect {
            self.locs[0] = Loc::Mem(*params.last().expect("the result's place"));
        }
        for bb in &self.body.blocks {
            self.blocks
                .push((!bb.cleanup).then(|| self.b.create_block()));
        }
        let first = self.block(0)?;
        self.b.ins().jump(first, &[]);
        for (i, bb) in self.body.blocks.iter().enumerate() {
            let Some(block) = self.blocks[i] else {
                continue;
            };
            self.b.switch_to_block(block);
            for s in &bb.statements {
                self.statement(s)?;
            }
            self.terminator(&bb.terminator, bb.terminator_span)?;
        }
        self.b.seal_all_blocks();
        Ok(())
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
        };
        match self.locs[p.local.index()] {
            Loc::Var(v) => r.var = Some(v),
            Loc::Mem(a) => r.addr = Some(a),
            Loc::Nothing => {}
        }
        for proj in &p.projection {
            match proj {
                Projection::Deref => {
                    let Ty::Ref(_, inner) = r.ty.clone() else {
                        return Err(format!("dereferencing a `{}`", r.ty));
                    };
                    let (ptr, meta) = match self.read(&r)? {
                        Val::Scalar(p) => (p, None),
                        // A wide borrow: the address, then the length or vtable.
                        Val::Mem(a) => {
                            let flags = ir::MemFlagsData::trusted();
                            let p = self.b.ins().load(types::I64, flags, a, 0);
                            let m = self.b.ins().load(types::I64, flags, a, 8);
                            (p, Some(m))
                        }
                        Val::Unit => return Err("a borrow of a value without bytes".to_string()),
                    };
                    r = PlaceRef {
                        meta,
                        var: None,
                        addr: Some(ptr),
                        ty: (*inner).clone(),
                        variant: None,
                    };
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
                    r = PlaceRef {
                        var: None,
                        addr: Some(addr),
                        ty: elem,
                        variant: None,
                        meta: None,
                    };
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
        let mut at = 0u32;
        let flags = ir::MemFlagsData::trusted();
        for (n, t) in [
            (8, types::I64),
            (4, types::I32),
            (2, types::I16),
            (1, types::I8),
        ] {
            while size - at >= n {
                let v = self.b.ins().load(t, flags, src, at as i32);
                self.b.ins().store(flags, v, dest, at as i32);
                at += n;
            }
        }
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
                self.read(&r)?
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
        let site = self.site(Site::Trap(class, self.file, range));
        // A broken precondition is the caller's, wherever it is checked.
        let rt = if self.tracked || class == TrapClass::Precondition {
            self.c.rt.trap_tracked
        } else {
            self.c.rt.trap
        };
        let f = self.c.module.declare_func_in_func(rt, self.b.func);
        self.b.ins().call(f, &[site]);
        self.return_status(1)
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
                ..
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
                self.trap(*trap, *range)?;
                self.b.switch_to_block(ok);
            }
            Statement::Read(_) | Statement::StorageDead(_) => {}
            Statement::Observe { range, value } => {
                let ty = self.operand_ty(value)?;
                let addr = match self.operand(value)? {
                    Val::Mem(a) => a,
                    Val::Scalar(v) => {
                        let l = self.layout(&ty)?;
                        let a = self.slot(&l);
                        self.b.ins().store(ir::MemFlagsData::trusted(), v, a, 0);
                        a
                    }
                    Val::Unit => {
                        let ptr = self.ptr();
                        self.b.ins().iconst(ptr, 0)
                    }
                };
                let site = self.site(Site::Observe(self.file, *range, ty));
                let f = self
                    .c
                    .module
                    .declare_func_in_func(self.c.rt.observe, self.b.func);
                self.b.ins().call(f, &[site, addr]);
            }
        }
        Ok(())
    }

    fn assign(&mut self, place: &Place, r: &Rvalue) -> Result<(), String> {
        let dest = self.place(place)?;
        match r {
            Rvalue::Use(o) => {
                let v = self.operand(o)?;
                let from = self.operand_ty(o)?;
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
                    let at = PlaceRef {
                        var: None,
                        addr: Some(addr),
                        ty: elem.clone(),
                        variant: None,
                        meta: None,
                    };
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
            Rvalue::Unsize(o, from) => {
                let from = self.c.concrete(from, self.targs);
                let v = self.operand(o)?;
                let thin = match (v, &from) {
                    // Already a `dyn` borrow (of a subtrait): as it is, if the supertrait's
                    // vtable is where the subtrait's starts.
                    (Val::Mem(_), Ty::Dyn(sub, _)) => {
                        let Ty::Ref(_, to) = &dest.ty else {
                            return Err(format!("a `{from}` as a `{}`", dest.ty));
                        };
                        let Ty::Dyn(sup, _) = &**to else {
                            return Err(format!("a `{from}` as a `{}`", dest.ty));
                        };
                        let ids = |ms: Vec<(u32, TraitRef)>| -> Vec<u32> {
                            ms.into_iter().map(|(m, _)| m).collect()
                        };
                        let a = ids(self.c.dyn_methods(&from, sub));
                        let b = ids(self.c.dyn_methods(&from, sup));
                        if !a.starts_with(&b) {
                            return Err(format!("a `dyn {}` as a `dyn {}`", sub.name, sup.name));
                        }
                        return self.write(&dest, v);
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
        if matches!(kind, AggregateKind::Closure(_)) {
            return Err("a closure".to_string());
        }
        let l = self.layout(&dest.ty)?;
        if l.size == 0 {
            for o in ops {
                self.operand(o)?;
            }
            return Ok(());
        }
        let base = self.addr_of(dest)?;
        let (fields, variant): (Vec<(u32, Ty)>, Option<u32>) = match (&l.shape, kind) {
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
            let v = self.operand(o)?;
            let addr = self.offset(base, *off);
            let at = PlaceRef {
                var: None,
                addr: Some(addr),
                ty: ty.clone(),
                variant: None,
                meta: None,
            };
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

    fn terminator(&mut self, t: &Terminator, span: TextRange) -> Result<(), String> {
        match t {
            Terminator::Goto(b) => {
                let b = self.block(b.index())?;
                self.b.ins().jump(b, &[]);
            }
            Terminator::SwitchBool { cond, then, else_ } => {
                let c = self.value(cond)?;
                let (y, n) = (self.block(then.index())?, self.block(else_.index())?);
                self.b.ins().brif(c, y, &[], n, &[]);
            }
            Terminator::Return => self.ret()?,
            Terminator::Trap(class, _) => self.trap(*class, span)?,
            Terminator::Unreachable => self.return_status(1)?,
            Terminator::Drop { place, target, .. } => {
                // C1's values have no destructors (their layouts say so).
                let r = self.place(place)?;
                self.layout(&r.ty)?;
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
            } => self.call(callee, args, dest, *target, on_error.is_some(), *range)?,
            Terminator::Fail(_) => return Err("an error leaving a function".to_string()),
            Terminator::Resume => return Err("unwinding".to_string()),
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
        let flags = ir::MemFlagsData::trusted();
        let mut vals = Vec::new();
        for (a, pty) in args.iter().zip(params) {
            let v = self.operand(a)?;
            let from = self.operand_ty(a)?;
            let v = self.fit(v, &from, pty);
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
        }
        Ok(vals)
    }

    fn call(
        &mut self,
        callee: &Callee,
        args: &[Operand],
        dest: &Place,
        target: moddil_mir::BlockId,
        can_fail: bool,
        range: TextRange,
    ) -> Result<(), String> {
        if can_fail {
            return Err("a call that can fail".to_string());
        }
        let cargs: Vec<Ty> = callee
            .args()
            .iter()
            .map(|t| self.c.concrete(t, self.targs))
            .collect();
        let d = self.place(dest)?;
        // What runs: a function instance, or a method through a vtable.
        enum Run {
            Direct(FuncId, Ty),
            Indirect(ir::Value, Vec<Ty>, Ty),
        }
        let mut args = args.to_vec();
        let (run, mut vals) = match callee {
            Callee::Fn { func, .. } => {
                if self.c.items.fns[*func as usize].intrinsic {
                    return self.intrinsic(*func, &cargs, &args, &d, target, range);
                }
                let (id, params, ret) = self.c.declare_fn(*func, &cargs)?;
                let vals = self.arguments(&args, &params)?;
                (Run::Direct(id, ret.clone()), vals)
            }
            Callee::Trait { func, .. } => match &cargs[0] {
                Ty::Dyn(tr, _) => {
                    let methods = self.c.dyn_methods(&cargs[0], tr);
                    let k = methods
                        .iter()
                        .position(|(m, _)| m == func)
                        .ok_or("a method missing from its vtable")?;
                    let Val::Mem(w) = self.operand(&args[0])? else {
                        return Err("a `dyn` call without a `dyn` borrow".to_string());
                    };
                    let flags = ir::MemFlagsData::trusted();
                    let thin = self.b.ins().load(types::I64, flags, w, 0);
                    let vt = self.b.ins().load(types::I64, flags, w, 8);
                    let fp = self.b.ins().load(types::I64, flags, vt, 8 * (1 + k as i32));
                    let mut params = vec![Ty::reference(false, Ty::Unit)];
                    for a in &args[1..] {
                        params.push(self.operand_ty(a)?);
                    }
                    let mut vals = vec![thin];
                    vals.extend(self.arguments(&args[1..], &params[1..])?);
                    args.clear();
                    (Run::Indirect(fp, params, d.ty.clone()), vals)
                }
                Ty::Anon(_) => return Err("an anonymous impl's method".to_string()),
                Ty::Errors(_) => return Err("a method of an error set".to_string()),
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
                        let from = PlaceRef {
                            var: None,
                            addr: Some(p),
                            ty: d.ty.clone(),
                            variant: None,
                            meta: None,
                        };
                        let v = self.read(&from)?;
                        self.write(&d, v)?;
                        let b = self.block(target.index())?;
                        self.b.ins().jump(b, &[]);
                        return Ok(());
                    };
                    parent.extend(cargs[sig.generics.parent as usize..].iter().cloned());
                    if items.fns[f as usize].intrinsic {
                        return self.intrinsic(f, &parent, &args, &d, target, range);
                    }
                    let (id, params, ret) = self.c.declare_fn(f, &parent)?;
                    let vals = self.arguments(&args, &params)?;
                    (Run::Direct(id, ret.clone()), vals)
                }
            },
        };
        let ret = match &run {
            Run::Direct(_, r) | Run::Indirect(_, _, r) => r.clone(),
        };
        let pass = self.c.pass(&ret)?;
        if pass == Pass::Indirect {
            vals.push(self.addr_of(&d)?);
        }
        let flags = ir::MemFlagsData::trusted();
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
        self.return_status(1)?;
        self.b.switch_to_block(call_block);
        let call = match run {
            Run::Direct(id, ..) => {
                let f = self.c.module.declare_func_in_func(id, self.b.func);
                self.b.ins().call(f, &vals)
            }
            Run::Indirect(fp, params, ret) => {
                let sig = self.c.signature(&params, &ret)?;
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
        self.return_status(1)?;
        self.b.switch_to_block(ok);
        match pass {
            Pass::Scalar(_) => self.write(&d, Val::Scalar(results[1]))?,
            Pass::Words(ts) => {
                let a = self.addr_of(&d)?;
                let mut at = 0;
                for (k, t) in ts.iter().enumerate() {
                    self.b.ins().store(flags, results[1 + k], a, at);
                    at += t.bytes() as i32;
                }
            }
            Pass::Nothing | Pass::Indirect => {}
        }
        let b = self.block(target.index())?;
        self.b.ins().jump(b, &[]);
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

    /// Calls the runtime's `f` with `args`: its one result.
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

    /// One of the prelude's built-in functions, inline.
    fn intrinsic(
        &mut self,
        f: u32,
        targs: &[Ty],
        args: &[Operand],
        d: &PlaceRef,
        target: moddil_mir::BlockId,
        range: TextRange,
    ) -> Result<(), String> {
        let items = self.c.items;
        let sig = &items.fns[f as usize];
        let self_ty = match sig.container {
            moddil_sema::Container::Impl(k) => items.impls[k as usize].self_ty.clone(),
            _ => Ty::Unit,
        };
        let name = sig.name.as_str();
        let unsupported = || Err(format!("the built-in function `{name}`"));
        let flags = ir::MemFlagsData::trusted();
        let out = match (&self_ty, name) {
            (Ty::Str, "len") => {
                let (_, n) = self.text(&args[0])?;
                Val::Scalar(n)
            }
            (Ty::Slice(_), "len") => {
                let Val::Mem(w) = self.operand(&args[0])? else {
                    return Err("a slice without its length".to_string());
                };
                Val::Scalar(self.b.ins().load(types::I64, flags, w, 8))
            }
            // A byte of the text (`text.string.inline.1`).
            (Ty::Str, "byte_at") => {
                let (p, n) = self.text(&args[0])?;
                let i = self.value(&args[1])?;
                let ok = self.b.ins().icmp(IntCC::UnsignedLessThan, i, n);
                self.check(ok, TrapClass::IndexOutOfBounds, range)?;
                let at = self.b.ins().iadd(p, i);
                Val::Scalar(self.b.ins().load(types::I8, flags, at, 0))
            }
            // The character at a byte offset, if one starts there: an `Option<char>`, whose
            // `None` is the first value past the last character (its niche).
            (Ty::Str, "char_at_byte") => {
                let l = self.layout(&d.ty)?;
                let Shape::Enum {
                    tag: Tag::Niche { niche, .. },
                    ..
                } = &l.shape
                else {
                    return unsupported();
                };
                if niche.first != 0x11_0000 || niche.offset != 0 {
                    return unsupported();
                }
                let (p, n) = self.text(&args[0])?;
                let i = self.value(&args[1])?;
                Val::Scalar(self.rt_call(self.c.rt.char_at_byte, &[p, n, i]))
            }
            // The text between two byte offsets (`text.pos.1`).
            (Ty::Str, "bytes_between") => {
                let (p, n) = self.text(&args[0])?;
                let (a, b) = (self.value(&args[1])?, self.value(&args[2])?);
                let ok = self.rt_call(self.c.rt.is_between, &[p, n, a, b]);
                self.check(ok, TrapClass::IndexOutOfBounds, range)?;
                let start = self.b.ins().iadd(p, a);
                let len = self.b.ins().isub(b, a);
                Val::Mem(self.wide(start, len))
            }
            (Ty::Str, "eq" | "ne" | "cmp" | "lt" | "le" | "gt" | "ge") => {
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
                            return unsupported();
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
            // The float functions (`num.float.fn.1`).
            (Ty::Num(n @ (Num::F32 | Num::F64)), _) => {
                let x = self.value(&args[0])?;
                match name {
                    "is_nan" => Val::Scalar(self.b.ins().fcmp(FloatCC::Unordered, x, x)),
                    "is_finite" => {
                        let a = self.b.ins().fabs(x);
                        let inf = if *n == Num::F32 {
                            self.b.ins().f32const(f32::INFINITY)
                        } else {
                            self.b.ins().f64const(f64::INFINITY)
                        };
                        Val::Scalar(self.b.ins().fcmp(FloatCC::LessThan, a, inf))
                    }
                    _ => {
                        let Some(k) = moddil_rt::FLOAT_FNS.iter().position(|f| *f == name) else {
                            return unsupported();
                        };
                        let y = match args.get(1) {
                            Some(a) => self.value(a)?,
                            None => x,
                        };
                        let k = self.b.ins().iconst(types::I32, k as i64);
                        let rt = if *n == Num::F32 {
                            self.c.rt.float_fn_f32
                        } else {
                            self.c.rt.float_fn
                        };
                        Val::Scalar(self.rt_call(rt, &[k, x, y]))
                    }
                }
            }
            (
                Ty::Char,
                "is_alphabetic" | "is_numeric" | "is_whitespace" | "is_uppercase" | "is_lowercase",
            ) => {
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
            (Ty::Char, "code") => {
                let c = self.value(&args[0])?;
                Val::Scalar(self.b.ins().uextend(types::I64, c))
            }
            (Ty::Char, "len_utf8") => {
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
            (Ty::Int, "wrapping_add" | "wrapping_sub" | "wrapping_mul" | "rotate_left") => {
                let (x, y) = (self.value(&args[0])?, self.value(&args[1])?);
                Val::Scalar(match name {
                    "wrapping_add" => self.b.ins().iadd(x, y),
                    "wrapping_sub" => self.b.ins().isub(x, y),
                    "wrapping_mul" => self.b.ins().imul(x, y),
                    _ => {
                        let k = self.b.ins().band_imm_s(y, 63);
                        self.b.ins().rotl(x, k)
                    }
                })
            }
            (Ty::Unit, "swap") => {
                let (Val::Scalar(a), Val::Scalar(b)) =
                    (self.operand(&args[0])?, self.operand(&args[1])?)
                else {
                    return unsupported();
                };
                let Some(t) = targs.first().cloned() else {
                    return unsupported();
                };
                let l = self.layout(&t)?;
                let tmp = self.slot(&l);
                self.copy(tmp, a, l.size);
                self.copy(a, b, l.size);
                self.copy(b, tmp, l.size);
                Val::Unit
            }
            (Ty::Unit, "replace") => {
                let Val::Scalar(p) = self.operand(&args[0])? else {
                    return unsupported();
                };
                let at = PlaceRef {
                    var: None,
                    addr: Some(p),
                    ty: d.ty.clone(),
                    variant: None,
                    meta: None,
                };
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
            _ => return unsupported(),
        };
        self.write(d, out)?;
        let b = self.block(target.index())?;
        self.b.ins().jump(b, &[]);
        Ok(())
    }
}

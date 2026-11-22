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
use cranelift_module::{FuncId, Linkage, Module};
use moddil_mir::{
    AggregateKind, BinOp, Body, Const, Local, Operand, Place, Projection, Rvalue, Statement,
    Terminator, TrapClass, UnOp,
};
use moddil_sema::{Callee, Db, Item, ItemTree, Num, Owner, Program, Src, Ty};
use moddil_syntax::TextRange;

use crate::layout::{Layout, Layouts, Shape, Tag};

/// What a site is.
#[derive(Clone, Debug)]
pub(crate) enum Site {
    /// A check: its class, the file and where.
    Trap(TrapClass, u32, TextRange),
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
    observe: FuncId,
    enter: FuncId,
    leave: FuncId,
    fmod: FuncId,
    fmodf: FuncId,
}

struct Compiler<'db> {
    db: &'db dyn Db,
    program: Program,
    items: &'db ItemTree,
    layouts: Layouts<'db>,
    module: JITModule,
    rt: Rt,
    /// Each function declared: its id, and its parameters' and result's types.
    fns: HashMap<u32, (FuncId, Vec<Ty>, Ty)>,
    /// The functions declared and not yet compiled.
    queue: Vec<u32>,
    sites: Vec<Site>,
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
        observe: import("moddil_rt_observe", &[types::I32, ptr], None)?,
        enter: import("moddil_rt_enter", &[types::I32], Some(types::I8))?,
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
    };
    let (target, ret) = match item {
        Item::Test(_) => {
            let id = c.declare_item(item, &[], &Ty::Unit)?;
            c.body_of(item, id, &[], &Ty::Unit)?;
            (id, Ty::Unit)
        }
        Item::Fn(f) => {
            let (id, params, ret) = c.declare_fn(f)?;
            if !params.is_empty() {
                return Err("an entry point with parameters".to_string());
            }
            (id, ret)
        }
        Item::Closure(..) => return Err("a closure as an entry point".to_string()),
    };
    while let Some(f) = c.queue.pop() {
        let (id, params, ret) = c.fns[&f].clone();
        c.body_of(Item::Fn(f), id, &params, &ret)?;
    }
    let entry = c.trampoline(target, &ret)?;
    c.module.finalize_definitions().map_err(|e| e.to_string())?;
    Ok(Compiled {
        module: c.module,
        entry,
        ret,
        sites: c.sites,
        items,
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

    fn declare_item(&mut self, item: Item, params: &[Ty], ret: &Ty) -> Result<FuncId, String> {
        let sig = self.signature(params, ret)?;
        let name = match item {
            Item::Fn(f) => format!("f{f}"),
            Item::Test(t) => format!("t{t}"),
            Item::Closure(..) => "closure".to_string(),
        };
        self.module
            .declare_function(&name, Linkage::Local, &sig)
            .map_err(|e| e.to_string())
    }

    /// Function `f`, declared (and queued to be compiled) the first time it is named.
    fn declare_fn(&mut self, f: u32) -> Result<(FuncId, Vec<Ty>, Ty), String> {
        if let Some(d) = self.fns.get(&f) {
            return Ok(d.clone());
        }
        let sig = &self.items.fns[f as usize];
        if sig.intrinsic || !sig.has_body {
            return Err(format!("the built-in function `{}`", sig.name));
        }
        let body = self.mir(Item::Fn(f))?;
        let params: Vec<Ty> = body.locals[1..=body.arg_count]
            .iter()
            .map(|l| l.ty.clone())
            .collect();
        let ret = body.locals[0].ty.clone();
        let id = self.declare_item(Item::Fn(f), &params, &ret)?;
        self.fns.insert(f, (id, params.clone(), ret.clone()));
        self.queue.push(f);
        Ok((id, params, ret))
    }

    fn mir(&self, item: Item) -> Result<&Body, String> {
        let owner = Owner::new(self.db, self.program, item);
        moddil_mir::mir(self.db, owner)
            .as_ref()
            .ok_or_else(|| "a program with errors".to_string())
    }

    /// Compiles `item`'s body as function `id`.
    fn body_of(&mut self, item: Item, id: FuncId, params: &[Ty], ret: &Ty) -> Result<(), String> {
        let body = self.mir(item)?.clone();
        let Src::File(file) = body.src else {
            return Err(format!("`{}`, of the prelude", body.name));
        };
        if body.track_caller {
            return Err(format!("`{}`, which is `#[track_caller]`", body.name));
        }
        let mut ctx = self.module.make_context();
        ctx.func.signature = self.signature(params, ret)?;
        let mut fctx = FunctionBuilderContext::new();
        let mut lw = Lower {
            c: self,
            body: &body,
            file,
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
    file: u32,
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
        for (i, l) in self.body.locals.iter().enumerate() {
            let layout = self.c.layouts.of(&l.ty)?;
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
        let ret_layout = self.c.layouts.of(&self.body.locals[0].ty)?;
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
            ty: self.body.locals[p.local.index()].ty.clone(),
            variant: None,
        };
        match self.locs[p.local.index()] {
            Loc::Var(v) => r.var = Some(v),
            Loc::Mem(a) => r.addr = Some(a),
            Loc::Nothing => {}
        }
        for proj in &p.projection {
            match proj {
                Projection::Deref => {
                    let Val::Scalar(ptr) = self.read(&r)? else {
                        return Err("a borrow of a value without bytes".to_string());
                    };
                    let Ty::Ref(_, inner) = r.ty.clone() else {
                        return Err(format!("dereferencing a `{}`", r.ty));
                    };
                    r = PlaceRef {
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
                    };
                }
                Projection::Index(i) => {
                    let l = self.layout(&r.ty)?;
                    let Shape::Array { elem, stride, .. } = &l.shape else {
                        return Err(format!("an element of `{}`", r.ty));
                    };
                    let (elem, stride) = (elem.clone(), *stride);
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
                    };
                }
                Projection::Subslice(..) => return Err("a part of an array".to_string()),
            }
        }
        Ok(r)
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
            Operand::Const(Const::Str(_)) => return Err("text".to_string()),
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
                Const::Str(_) => return Err("text".to_string()),
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
        let f = self
            .c
            .module
            .declare_func_in_func(self.c.rt.trap, self.b.func);
        self.b.ins().call(f, &[site]);
        self.return_status(1)
    }

    /// Returns `status`, with a zero result (the caller does not read it).
    fn return_status(&mut self, status: i64) -> Result<(), String> {
        let s = self.b.ins().iconst(types::I8, status);
        let mut rets = vec![s];
        let ret = self.body.locals[0].ty.clone();
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
                self.write(&dest, v)
            }
            Rvalue::Aggregate(kind, ops) => self.aggregate(&dest, kind, ops),
            Rvalue::Ref(_, p) => {
                let target = self.place(p)?;
                let a = match target.addr {
                    Some(a) => a,
                    // A borrow of something without bytes points nowhere in particular.
                    None if target.var.is_none() => {
                        let ptr = self.ptr();
                        self.b.ins().iconst(ptr, 8)
                    }
                    None => return Err("a borrow of a value kept in a register".to_string()),
                };
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
                    let at = PlaceRef {
                        var: None,
                        addr: Some(addr),
                        ty: elem.clone(),
                        variant: None,
                    };
                    self.write(&at, v)?;
                }
                Ok(())
            }
            Rvalue::Len(p) => {
                let target = self.place(p)?;
                let l = self.layout(&target.ty)?;
                let Shape::Array { len, .. } = l.shape else {
                    return Err(format!("the length of a `{}`", target.ty));
                };
                let v = self.b.ins().iconst(types::I64, i64::from(len));
                self.write(&dest, Val::Scalar(v))
            }
            Rvalue::InBounds(i, p) => {
                let target = self.place(p)?;
                let l = self.layout(&target.ty)?;
                let Shape::Array { len, .. } = l.shape else {
                    return Err(format!("indexing a `{}`", target.ty));
                };
                let i = self.value(i)?;
                // One unsigned comparison: a negative index is a large one.
                let ok = self
                    .b
                    .ins()
                    .icmp_imm_s(IntCC::UnsignedLessThan, i, i64::from(len));
                self.write(&dest, Val::Scalar(ok))
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
                self.widen(&from, to, v)?
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
        let ty = self.body.locals[0].ty.clone();
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
        let Callee::Fn { func, args: targs } = callee else {
            return Err("a trait's method".to_string());
        };
        if !targs.is_empty() {
            return Err("a generic function".to_string());
        }
        let (id, params, ret) = self.c.declare_fn(*func)?;
        let flags = ir::MemFlagsData::trusted();
        let mut vals = Vec::new();
        for (a, pty) in args.iter().zip(&params) {
            let v = self.operand(a)?;
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
                _ => return Err("an argument of an unexpected kind".to_string()),
            }
        }
        let d = self.place(dest)?;
        let pass = self.c.pass(&ret)?;
        if pass == Pass::Indirect {
            vals.push(self.addr_of(&d)?);
        }
        // Count the frame, so that recursion without end traps.
        let site = self.site(Site::Trap(TrapClass::StackOverflow, self.file, range));
        let enter = self
            .c
            .module
            .declare_func_in_func(self.c.rt.enter, self.b.func);
        let e = self.b.ins().call(enter, &[site]);
        let deep = self.b.inst_results(e)[0];
        let call_block = self.b.create_block();
        let too_deep = self.b.create_block();
        self.b.ins().brif(deep, too_deep, &[], call_block, &[]);
        self.b.switch_to_block(too_deep);
        self.return_status(1)?;
        self.b.switch_to_block(call_block);
        let f = self.c.module.declare_func_in_func(id, self.b.func);
        let call = self.b.ins().call(f, &vals);
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
}

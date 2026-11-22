//! MIR to Cranelift IR, slice C0: scalars, their operators and checks, control flow and direct
//! calls of non-generic functions.
//!
//! **The ABI.** Every compiled function takes its parameters (those of type `()` left out) and
//! returns a status (`i8`: 0 for a return, 1 for a trap) and then its result, if it is not `()`.
//! A failed check calls `moddil-rt`'s `trap` with its site and returns status 1, and every caller
//! returns 1 as soon as a callee does, so the host sees the trap without any unwinding. Each call
//! is wrapped in `moddil-rt`'s `enter` and `leave`, which count the frames, so that recursion
//! without end traps at the depth the interpreters' does.
//!
//! **Sites.** A check that can fail and a `// =>` observation are numbered; the number is what the
//! runtime records, and [`Compiled::sites`] says what each one is.

use std::collections::HashMap;

use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::{self, AbiParam, InstBuilder, Signature, types};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{FuncId, Linkage, Module};
use moddil_mir::{
    BinOp, Body, Const, Local, Operand, Place, Rvalue, Statement, Terminator, TrapClass, UnOp,
};
use moddil_sema::{Callee, Db, Item, Num, Owner, Program, Src, Ty};
use moddil_syntax::TextRange;

/// What a site is.
#[derive(Clone, Debug)]
pub(crate) enum Site {
    /// A check: its class, the file and where.
    Trap(TrapClass, u32, TextRange),
    /// A `// =>` observation: the file, the expression, and its type.
    Observe(u32, TextRange, Ty),
}

/// A program compiled for one entry point, ready to run.
pub(crate) struct Compiled {
    pub(crate) module: JITModule,
    /// The entry trampoline: `fn(out: *mut [u64; 2]) -> i8`.
    pub(crate) entry: FuncId,
    /// The entry's result type.
    pub(crate) ret: Ty,
    pub(crate) sites: Vec<Site>,
}

/// The Cranelift type of a value of `ty`: `None` for `()`, an error for a type C0 does not
/// compile.
pub(crate) fn clif_type(ty: &Ty) -> Result<Option<ir::Type>, String> {
    Ok(Some(match ty {
        Ty::Unit => return Ok(None),
        Ty::Int => types::I64,
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
    module: JITModule,
    rt: Rt,
    /// Each function declared: its id, and its parameters' and result's types.
    fns: HashMap<u32, (FuncId, Vec<Ty>, Ty)>,
    /// The functions declared and not yet compiled.
    queue: Vec<u32>,
    sites: Vec<Site>,
}

/// Compiles what `item` (a test, or a function without parameters) reaches.
pub(crate) fn compile(db: &dyn Db, program: Program, item: Item) -> Result<Compiled, String> {
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
        observe: import(
            "moddil_rt_observe",
            &[types::I32, types::I64, types::I64],
            None,
        )?,
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
    let mut c = Compiler {
        db,
        program,
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
    })
}

impl Compiler<'_> {
    /// The signature of a function with these parameters and result.
    fn signature(&self, params: &[Ty], ret: &Ty) -> Result<Signature, String> {
        let mut sig = self.module.make_signature();
        for p in params {
            if let Some(t) = clif_type(p)? {
                sig.params.push(AbiParam::new(t));
            }
        }
        sig.returns.push(AbiParam::new(types::I8));
        match clif_type(ret)? {
            // Too wide to come back in registers beside the status: the caller gives a place.
            Some(types::I128) => {
                let ptr = self.module.target_config().pointer_type();
                sig.params.push(AbiParam::new(ptr));
            }
            Some(t) => sig.returns.push(AbiParam::new(t)),
            None => {}
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
        let items = moddil_sema::item_tree(self.db, self.program);
        let sig = &items.fns[f as usize];
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
            vars: Vec::new(),
            blocks: Vec::new(),
            out: None,
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

    /// `fn(out: *mut [u64; 2]) -> i8`: calls `target` and stores its result's bits at `out`.
    fn trampoline(&mut self, target: FuncId, ret: &Ty) -> Result<FuncId, String> {
        let ptr = self.module.target_config().pointer_type();
        let mut sig = self.module.make_signature();
        sig.params.push(AbiParam::new(ptr));
        sig.returns.push(AbiParam::new(types::I8));
        let id = self
            .module
            .declare_function("entry", Linkage::Export, &sig)
            .map_err(|e| e.to_string())?;
        let mut ctx = self.module.make_context();
        ctx.func.signature = sig;
        let mut fctx = FunctionBuilderContext::new();
        let mut b = FunctionBuilder::new(&mut ctx.func, &mut fctx);
        let block = b.create_block();
        b.append_block_params_for_function_params(block);
        b.switch_to_block(block);
        let out = b.block_params(block)[0];
        let f = self.module.declare_func_in_func(target, b.func);
        let wide = clif_type(ret)? == Some(types::I128);
        let call = if wide {
            b.ins().call(f, &[out])
        } else {
            b.ins().call(f, &[])
        };
        let results = b.inst_results(call).to_vec();
        if let (false, Some(&v), Some(t)) = (wide, results.get(1), clif_type(ret)?) {
            let (lo, hi) = to_bits(&mut b, v, t);
            b.ins().store(ir::MemFlagsData::trusted(), lo, out, 0);
            b.ins().store(ir::MemFlagsData::trusted(), hi, out, 8);
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

/// A value's bits as two `i64`s, low and high, for the runtime: zero-extended, floats as their
/// bit patterns.
fn to_bits(b: &mut FunctionBuilder<'_>, v: ir::Value, t: ir::Type) -> (ir::Value, ir::Value) {
    let zero = || 0i64;
    match t {
        types::I128 => {
            let (lo, hi) = b.ins().isplit(v);
            (lo, hi)
        }
        types::F64 => {
            let lo = b.ins().bitcast(types::I64, ir::MemFlagsData::new(), v);
            (lo, b.ins().iconst(types::I64, zero()))
        }
        types::F32 => {
            let i = b.ins().bitcast(types::I32, ir::MemFlagsData::new(), v);
            let lo = b.ins().uextend(types::I64, i);
            (lo, b.ins().iconst(types::I64, zero()))
        }
        types::I64 => (v, b.ins().iconst(types::I64, zero())),
        _ => {
            let lo = b.ins().uextend(types::I64, v);
            (lo, b.ins().iconst(types::I64, zero()))
        }
    }
}

/// One body being lowered.
struct Lower<'a, 'c, 'db> {
    c: &'a mut Compiler<'db>,
    body: &'a Body,
    file: u32,
    b: FunctionBuilder<'c>,
    /// Each local's variable, if it has a value (not `()`).
    vars: Vec<Option<Variable>>,
    /// Each MIR block's Cranelift block (`None` for cleanup code, which only unwinding runs).
    blocks: Vec<Option<ir::Block>>,
    /// Where a 128-bit result goes: the pointer the caller gives.
    out: Option<ir::Value>,
}

impl Lower<'_, '_, '_> {
    fn function(&mut self) -> Result<(), String> {
        let entry = self.b.create_block();
        self.b.append_block_params_for_function_params(entry);
        self.b.switch_to_block(entry);
        for l in &self.body.locals {
            let v = match clif_type(&l.ty)? {
                Some(t) => {
                    let var = self.b.declare_var(t);
                    let zero = self.zero(t);
                    self.b.def_var(var, zero);
                    Some(var)
                }
                None => None,
            };
            self.vars.push(v);
        }
        let params = self.b.block_params(entry).to_vec();
        if clif_type(&self.body.locals[0].ty)? == Some(types::I128) {
            self.out = params.last().copied();
        }
        let mut k = 0;
        for i in 1..=self.body.arg_count {
            if let Some(var) = self.vars[i] {
                self.b.def_var(var, params[k]);
                k += 1;
            }
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
            self.terminator(&bb.terminator)?;
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

    fn local_ty(&self, l: Local) -> &Ty {
        &self.body.locals[l.index()].ty
    }

    /// The local a place is: C0 has no projections.
    fn whole(place: &Place) -> Result<Local, String> {
        if place.projection.is_empty() {
            Ok(place.local)
        } else {
            Err("a part of a value (a field, an element or a borrow)".to_string())
        }
    }

    fn operand_ty(&self, o: &Operand) -> Result<Ty, String> {
        Ok(match o {
            Operand::Copy(p) | Operand::Move(p) => self.local_ty(Self::whole(p)?).clone(),
            Operand::Const(Const::Int(_)) => Ty::Int,
            Operand::Const(Const::Num(n, _)) => Ty::Num(*n),
            Operand::Const(Const::Bool(_)) => Ty::Bool,
            Operand::Const(Const::Unit) => Ty::Unit,
            Operand::Const(Const::Char(_)) => Ty::Char,
            Operand::Const(Const::Str(_)) => return Err("text".to_string()),
        })
    }

    /// The value of `o`, or `None` for `()`.
    fn operand(&mut self, o: &Operand) -> Result<Option<ir::Value>, String> {
        Ok(match o {
            Operand::Copy(p) | Operand::Move(p) => {
                let l = Self::whole(p)?;
                self.vars[l.index()].map(|v| self.b.use_var(v))
            }
            Operand::Const(c) => match *c {
                Const::Int(v) => Some(self.b.ins().iconst(types::I64, v)),
                Const::Bool(v) => Some(self.b.ins().iconst(types::I8, i64::from(v))),
                Const::Char(c) => Some(self.b.ins().iconst(types::I32, i64::from(u32::from(c)))),
                Const::Unit => None,
                Const::Num(n, bits) => Some(self.num_const(n, bits)?),
                Const::Str(_) => return Err("text".to_string()),
            },
        })
    }

    fn num_const(&mut self, n: Num, bits: u128) -> Result<ir::Value, String> {
        let t = clif_type(&Ty::Num(n))?.expect("a number type");
        Ok(match t {
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
        })
    }

    fn value(&mut self, o: &Operand) -> Result<ir::Value, String> {
        self.operand(o)?
            .ok_or_else(|| "an operator on `()`".to_string())
    }

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
        if let Some(t) = clif_type(&self.body.locals[0].ty)?
            && self.out.is_none()
        {
            rets.push(self.zero(t));
        }
        self.b.ins().return_(&rets);
        Ok(())
    }

    fn statement(&mut self, s: &Statement) -> Result<(), String> {
        match s {
            Statement::Assign(place, r) => {
                let l = Self::whole(place)?;
                let v = self.rvalue(r)?;
                if let (Some(var), Some(v)) = (self.vars[l.index()], v) {
                    self.b.def_var(var, v);
                }
            }
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
                let site = self.site(Site::Observe(self.file, *range, ty.clone()));
                let (lo, hi) = match (self.operand(value)?, clif_type(&ty)?) {
                    (Some(v), Some(t)) => to_bits(&mut self.b, v, t),
                    _ => {
                        let z = self.b.ins().iconst(types::I64, 0);
                        (z, z)
                    }
                };
                let f = self
                    .c
                    .module
                    .declare_func_in_func(self.c.rt.observe, self.b.func);
                self.b.ins().call(f, &[site, lo, hi]);
            }
        }
        Ok(())
    }

    fn rvalue(&mut self, r: &Rvalue) -> Result<Option<ir::Value>, String> {
        Ok(Some(match r {
            Rvalue::Use(o) => return self.operand(o),
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

    fn terminator(&mut self, t: &Terminator) -> Result<(), String> {
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
            Terminator::Return => {
                let s = self.b.ins().iconst(types::I8, 0);
                let mut rets = vec![s];
                if let Some(var) = self.vars[0] {
                    let v = self.b.use_var(var);
                    match self.out {
                        Some(out) => {
                            self.b.ins().store(ir::MemFlagsData::trusted(), v, out, 0);
                        }
                        None => rets.push(v),
                    }
                }
                self.b.ins().return_(&rets);
            }
            Terminator::Trap(class, _) => {
                let range =
                    self.body.blocks.iter().find_map(|bb| {
                        std::ptr::eq(&bb.terminator, t).then_some(bb.terminator_span)
                    });
                self.trap(*class, range.unwrap_or_default())?;
            }
            Terminator::Unreachable => self.return_status(1)?,
            Terminator::Drop { place, target, .. } => {
                let l = Self::whole(place)?;
                clif_type(self.local_ty(l))?;
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
                if on_error.is_some() {
                    return Err("a call that can fail".to_string());
                }
                let Callee::Fn { func, args: targs } = callee else {
                    return Err("a trait's method".to_string());
                };
                if !targs.is_empty() {
                    return Err("a generic function".to_string());
                }
                let (id, _, _) = self.c.declare_fn(*func)?;
                let mut vals = Vec::new();
                for a in args {
                    if let Some(v) = self.operand(a)? {
                        vals.push(v);
                    }
                }
                // Count the frame, so that recursion without end traps.
                let site = self.site(Site::Trap(TrapClass::StackOverflow, self.file, *range));
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
                let l = Self::whole(dest)?;
                let wide = clif_type(self.local_ty(l))? == Some(types::I128);
                let slot = wide.then(|| {
                    self.b.create_sized_stack_slot(ir::StackSlotData::new(
                        ir::StackSlotKind::ExplicitSlot,
                        16,
                        4,
                    ))
                });
                if let Some(slot) = slot {
                    let ptr = self.c.module.target_config().pointer_type();
                    let addr = self.b.ins().stack_addr(ptr, slot, 0);
                    vals.push(addr);
                }
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
                if let (Some(var), Some(slot)) = (self.vars[l.index()], slot) {
                    let ptr = self.c.module.target_config().pointer_type();
                    let v = self.b.ins().stack_load(ptr, types::I128, slot, 0);
                    self.b.def_var(var, v);
                } else if let (Some(var), Some(&v)) = (self.vars[l.index()], results.get(1)) {
                    self.b.def_var(var, v);
                }
                let b = self.block(target.index())?;
                self.b.ins().jump(b, &[]);
            }
            Terminator::Fail(_) => return Err("an error leaving a function".to_string()),
            Terminator::Resume => return Err("unwinding".to_string()),
        }
        Ok(())
    }
}

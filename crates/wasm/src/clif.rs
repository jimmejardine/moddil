//! A finished Cranelift IR function as a WASM function body.
//!
//! - **Values.** Each CLIF value is a wasm local: `i8`, `i16` and `i32` an `i32` (the narrow
//!   ones kept zero-extended, so their bits above are always clear), `i64` an `i64`, `i128` two
//!   `i64`s (low, then high), and the floats as themselves. A pointer is an `i64` (the lowering's
//!   layouts are 64-bit ones), wrapped to an `i32` address where memory is accessed.
//! - **Blocks** are structured by [`crate::cfg`]; a branch's arguments are stored in the target's
//!   parameters on the edge, all read before any is written.
//! - **Stack slots** are a frame on the shadow stack (global [`SP`], growing down), made on entry
//!   and given back before each return.
//! - **Calls** to the runtime's functions are adapted to their wasm32 signatures: an address
//!   crosses as an `i32`, so the code's `i64` is wrapped going in and extended coming out.

use std::collections::HashMap;

use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::{
    self, BlockArg, ExternalName, Function, GlobalValueData, InstructionData, Opcode, Type, Value,
    types,
};
use cranelift_module::{DataId, FuncId};
use wasm_encoder::{BlockType, Instruction as I, MemArg, ValType};

use crate::cfg::{self, Exit};

/// The shadow stack's pointer (mutable).
pub(crate) const SP: u32 = 0;
/// The lowest address the shadow stack may reach.
pub(crate) const STACK_BASE: u32 = 1;
/// Where the runtime's `i128` operations put their results (two words).
pub(crate) const I128_OUT: u32 = 2;

/// An instruction of a body, some resolved only once the module is laid out.
#[derive(Clone, Debug)]
pub(crate) enum Op {
    I(I<'static>),
    /// A call of a function (one the module defines, or imports).
    Call(FuncId),
    /// A call of the runtime's function of this name.
    CallRuntime(&'static str),
    /// `i64.const` of the function's slot in the table (its address).
    FuncAddr(FuncId),
    /// `i64.const` of the data object's address plus this much.
    DataAddr(DataId, i64),
}

/// A translated function: its type and its code.
pub(crate) struct Body {
    pub(crate) params: Vec<ValType>,
    pub(crate) results: Vec<ValType>,
    /// Its locals past the parameters.
    pub(crate) locals: Vec<ValType>,
    pub(crate) code: Vec<Op>,
}

/// How a function is called.
pub(crate) enum Callee {
    /// As its signature says: one the module defines (or the host's, which takes what it is
    /// given).
    AsDeclared,
    /// With these wasm types, which the code's are adapted to: the runtime's.
    Adapted(Vec<ValType>, Vec<ValType>),
}

/// What a translation asks of the module.
pub(crate) trait Env {
    /// How function `f` is called, or why it cannot be.
    fn callee(&mut self, f: FuncId) -> Result<Callee, String>;
    /// The index of the function type with these parameters and results.
    fn type_index(&mut self, params: &[ValType], results: &[ValType]) -> u32;
}

/// The wasm values a CLIF value of type `t` is.
pub(crate) fn vals(t: Type) -> Result<&'static [ValType], String> {
    Ok(match t {
        types::I8 | types::I16 | types::I32 => &[ValType::I32],
        types::I64 => &[ValType::I64],
        types::I128 => &[ValType::I64, ValType::I64],
        types::F32 => &[ValType::F32],
        types::F64 => &[ValType::F64],
        t => return Err(format!("a value of Cranelift type `{t}`")),
    })
}

/// A signature's wasm parameters and results.
pub(crate) fn signature(sig: &ir::Signature) -> Result<(Vec<ValType>, Vec<ValType>), String> {
    let mut ps = Vec::new();
    for p in &sig.params {
        ps.extend_from_slice(vals(p.value_type)?);
    }
    let mut rs = Vec::new();
    for r in &sig.returns {
        rs.extend_from_slice(vals(r.value_type)?);
    }
    Ok((ps, rs))
}

/// The mask that keeps a narrow integer's bits, if `t` is one.
fn mask(t: Type) -> Option<i32> {
    match t {
        types::I8 => Some(0xFF),
        types::I16 => Some(0xFFFF),
        _ => None,
    }
}

fn mem(offset: u64, align: u32) -> MemArg {
    MemArg {
        offset,
        align,
        memory_index: 0,
    }
}

/// Translates `func` into a body.
pub(crate) fn translate(func: &Function, env: &mut dyn Env) -> Result<Body, String> {
    let (params, results) = signature(&func.signature)?;
    let mut t = Translator {
        func,
        env,
        nparams: params.len() as u32,
        locals: Vec::new(),
        code: Vec::new(),
        values: HashMap::new(),
        order: Vec::new(),
        index: HashMap::new(),
        fp: 0,
        frame: 0,
        slots: HashMap::new(),
    };
    t.body()?;
    Ok(Body {
        params,
        results,
        locals: t.locals,
        code: t.code,
    })
}

struct Translator<'a> {
    func: &'a Function,
    env: &'a mut dyn Env,
    nparams: u32,
    locals: Vec<ValType>,
    code: Vec<Op>,
    /// Each value's local, and for an `i128` its high word's.
    values: HashMap<Value, (u32, Option<u32>)>,
    /// The blocks, entry first, and each one's number.
    order: Vec<ir::Block>,
    index: HashMap<ir::Block, usize>,
    /// The local holding the frame's address.
    fp: u32,
    frame: u32,
    /// Each stack slot's offset in the frame.
    slots: HashMap<ir::StackSlot, u32>,
}

impl Translator<'_> {
    fn e(&mut self, i: I<'static>) {
        self.code.push(Op::I(i));
    }

    fn local(&mut self, vt: ValType) -> u32 {
        self.locals.push(vt);
        self.nparams + (self.locals.len() - 1) as u32
    }

    fn ty(&self, v: Value) -> Type {
        self.func.dfg.value_type(v)
    }

    /// Value `v`'s locals, made the first time it is seen.
    fn slot_of(&mut self, v: Value) -> Result<(u32, Option<u32>), String> {
        let v = self.func.dfg.resolve_aliases(v);
        if let Some(l) = self.values.get(&v) {
            return Ok(*l);
        }
        let vs = vals(self.ty(v))?;
        let lo = self.local(vs[0]);
        let hi = if vs.len() == 2 {
            Some(self.local(vs[1]))
        } else {
            None
        };
        self.values.insert(v, (lo, hi));
        Ok((lo, hi))
    }

    /// Pushes `v` (an `i128`'s low word, then its high one).
    fn get(&mut self, v: Value) -> Result<(), String> {
        let (lo, hi) = self.slot_of(v)?;
        self.e(I::LocalGet(lo));
        if let Some(h) = hi {
            self.e(I::LocalGet(h));
        }
        Ok(())
    }

    /// Pushes an `i128`'s low word or high one.
    fn get_lo(&mut self, v: Value) -> Result<(), String> {
        let (lo, _) = self.slot_of(v)?;
        self.e(I::LocalGet(lo));
        Ok(())
    }

    fn get_hi(&mut self, v: Value) -> Result<(), String> {
        let (_, hi) = self.slot_of(v)?;
        self.e(I::LocalGet(hi.ok_or("the high word of a narrow value")?));
        Ok(())
    }

    /// Stores what is on the stack in `v`.
    fn set(&mut self, v: Value) -> Result<(), String> {
        let (lo, hi) = self.slot_of(v)?;
        if let Some(h) = hi {
            self.e(I::LocalSet(h));
        }
        self.e(I::LocalSet(lo));
        Ok(())
    }

    /// Pushes narrow `v` sign-extended to 32 bits (an `i32`, as it is).
    fn get_signed(&mut self, v: Value) -> Result<(), String> {
        self.get(v)?;
        match self.ty(v) {
            types::I8 => self.e(I::I32Extend8S),
            types::I16 => self.e(I::I32Extend16S),
            _ => {}
        }
        Ok(())
    }

    /// Masks the `i32` on the stack to type `t`'s bits, if it is narrow.
    fn canon(&mut self, t: Type) {
        if let Some(m) = mask(t) {
            self.e(I::I32Const(m));
            self.e(I::I32And);
        }
    }

    fn body(&mut self) -> Result<(), String> {
        let entry = self
            .func
            .layout
            .entry_block()
            .ok_or("a function without code")?;
        self.order.push(entry);
        for b in self.func.layout.blocks() {
            if b != entry {
                self.order.push(b);
            }
        }
        for (k, b) in self.order.iter().enumerate() {
            self.index.insert(*b, k);
        }
        // The parameters are the entry block's.
        let mut n = 0u32;
        for &p in self.func.dfg.block_params(entry) {
            let w = vals(self.ty(p))?.len() as u32;
            let hi = (w == 2).then_some(n + 1);
            self.values.insert(p, (n, hi));
            n += w;
        }
        self.fp = self.local(ValType::I32);
        // The frame: each stack slot at its alignment.
        for (slot, data) in &self.func.sized_stack_slots {
            let align = 1u32 << data.align_shift.min(4);
            let at = self.frame.div_ceil(align) * align;
            self.slots.insert(slot, at);
            self.frame = at + data.size;
        }
        self.frame = self.frame.div_ceil(16) * 16;
        if self.frame > 0 {
            self.e(I::GlobalGet(SP));
            self.e(I::I32Const(self.frame as i32));
            self.e(I::I32Sub);
            self.e(I::LocalTee(self.fp));
            // The shadow stack is full: the run stops (the calls are counted, so this is a
            // frame larger than the stack has room for).
            self.e(I::GlobalGet(STACK_BASE));
            self.e(I::I32LtU);
            self.e(I::If(BlockType::Empty));
            self.e(I::Unreachable);
            self.e(I::End);
            self.e(I::LocalGet(self.fp));
            self.e(I::GlobalSet(SP));
        }
        let mut succs = Vec::new();
        for &b in &self.order {
            succs.push(self.successors(b)?);
        }
        cfg::structure(&succs, self)?;
        self.e(I::Unreachable);
        Ok(())
    }

    /// The blocks block `b`'s terminator goes on at.
    fn successors(&self, b: ir::Block) -> Result<Vec<usize>, String> {
        let Some(last) = self.func.layout.last_inst(b) else {
            return Ok(Vec::new());
        };
        let pool = &self.func.dfg.value_lists;
        Ok(match &self.func.dfg.insts[last] {
            InstructionData::Jump { destination, .. } => {
                vec![self.index[&destination.block(pool)]]
            }
            InstructionData::Brif { blocks, .. } => vec![
                self.index[&blocks[0].block(pool)],
                self.index[&blocks[1].block(pool)],
            ],
            _ => Vec::new(),
        })
    }

    /// Gives the frame back.
    fn epilogue(&mut self) {
        if self.frame > 0 {
            self.e(I::LocalGet(self.fp));
            self.e(I::I32Const(self.frame as i32));
            self.e(I::I32Add);
            self.e(I::GlobalSet(SP));
        }
    }

    /// Pushes the address `p` (an `i64`) plus `offset`, as an `i32`, and the offset left for
    /// the access.
    fn address(&mut self, p: Value, offset: i32) -> Result<u64, String> {
        self.get(p)?;
        if self.ty(p) == types::I64 {
            self.e(I::I32WrapI64);
        }
        Ok(if offset >= 0 {
            offset as u64
        } else {
            self.e(I::I32Const(offset));
            self.e(I::I32Add);
            0
        })
    }

    /// The address `p` plus `offset` in a new local, for more than one access.
    fn address_local(&mut self, p: Value, offset: i32) -> Result<u32, String> {
        let off = self.address(p, offset)?;
        if off > 0 {
            self.e(I::I32Const(off as i32));
            self.e(I::I32Add);
        }
        let a = self.local(ValType::I32);
        self.e(I::LocalSet(a));
        Ok(a)
    }

    fn inst(&mut self, inst: ir::Inst) -> Result<(), String> {
        let dfg = &self.func.dfg;
        let data = dfg.insts[inst];
        let op = data.opcode();
        let results: Vec<Value> = dfg.inst_results(inst).to_vec();
        let args: Vec<Value> = dfg
            .inst_args(inst)
            .iter()
            .map(|v| dfg.resolve_aliases(*v))
            .collect();
        let rty = results.first().map(|r| self.ty(*r));
        match (op, data) {
            (Opcode::Nop, _) => {}
            (Opcode::Iconst, InstructionData::UnaryImm { imm, .. }) => {
                let v = imm.bits();
                match rty.ok_or("a constant without a result")? {
                    types::I64 => self.e(I::I64Const(v)),
                    types::I128 => {
                        self.e(I::I64Const(v));
                        self.e(I::I64Const(v >> 63));
                    }
                    t => self.e(I::I32Const((v as i32) & mask(t).unwrap_or(-1))),
                }
                self.set(results[0])?;
            }
            (Opcode::F32const, InstructionData::UnaryIeee32 { imm, .. }) => {
                self.e(I::F32Const(f32::from_bits(imm.bits()).into()));
                self.set(results[0])?;
            }
            (Opcode::F64const, InstructionData::UnaryIeee64 { imm, .. }) => {
                self.e(I::F64Const(f64::from_bits(imm.bits()).into()));
                self.set(results[0])?;
            }
            (
                Opcode::Iadd
                | Opcode::Isub
                | Opcode::Imul
                | Opcode::Udiv
                | Opcode::Sdiv
                | Opcode::Urem
                | Opcode::Srem
                | Opcode::Band
                | Opcode::Bor
                | Opcode::Bxor
                | Opcode::Smin
                | Opcode::Smax
                | Opcode::Umin
                | Opcode::Umax,
                _,
            ) => self.int_binary(op, &args, results[0])?,
            (Opcode::Ishl | Opcode::Ushr | Opcode::Sshr | Opcode::Rotl | Opcode::Rotr, _) => {
                self.shift(op, &args, results[0])?;
            }
            (Opcode::Ineg, _) => {
                let t = self.ty(args[0]);
                match t {
                    types::I128 => {
                        // 0 - x: lo = -x.lo; hi = -x.hi - (x.lo != 0).
                        self.e(I::I64Const(0));
                        self.get_lo(args[0])?;
                        self.e(I::I64Sub);
                        self.e(I::I64Const(0));
                        self.get_hi(args[0])?;
                        self.e(I::I64Sub);
                        self.get_lo(args[0])?;
                        self.e(I::I64Const(0));
                        self.e(I::I64Ne);
                        self.e(I::I64ExtendI32U);
                        self.e(I::I64Sub);
                    }
                    types::I64 => {
                        self.e(I::I64Const(0));
                        self.get(args[0])?;
                        self.e(I::I64Sub);
                    }
                    t => {
                        self.e(I::I32Const(0));
                        self.get(args[0])?;
                        self.e(I::I32Sub);
                        self.canon(t);
                    }
                }
                self.set(results[0])?;
            }
            (Opcode::Bnot, _) => {
                let t = self.ty(args[0]);
                match t {
                    types::I128 => {
                        self.get_lo(args[0])?;
                        self.e(I::I64Const(-1));
                        self.e(I::I64Xor);
                        self.get_hi(args[0])?;
                        self.e(I::I64Const(-1));
                        self.e(I::I64Xor);
                    }
                    types::I64 => {
                        self.get(args[0])?;
                        self.e(I::I64Const(-1));
                        self.e(I::I64Xor);
                    }
                    t => {
                        self.get(args[0])?;
                        self.e(I::I32Const(-1));
                        self.e(I::I32Xor);
                        self.canon(t);
                    }
                }
                self.set(results[0])?;
            }
            (Opcode::Popcnt | Opcode::Clz | Opcode::Ctz, _) => {
                let t = self.ty(args[0]);
                let w = t.bits() as i32;
                match (op, t) {
                    (_, types::I128) => return Err(format!("`{op}` on an `i128`")),
                    (Opcode::Popcnt, types::I64) => {
                        self.get(args[0])?;
                        self.e(I::I64Popcnt);
                    }
                    (Opcode::Clz, types::I64) => {
                        self.get(args[0])?;
                        self.e(I::I64Clz);
                    }
                    (_, types::I64) => {
                        self.get(args[0])?;
                        self.e(I::I64Ctz);
                    }
                    (Opcode::Popcnt, _) => {
                        self.get(args[0])?;
                        self.e(I::I32Popcnt);
                    }
                    (Opcode::Clz, _) => {
                        self.get(args[0])?;
                        self.e(I::I32Clz);
                        self.e(I::I32Const(32 - w));
                        self.e(I::I32Sub);
                    }
                    _ => {
                        // A bit just past the value's, so that 0 counts its width.
                        self.get(args[0])?;
                        if w < 32 {
                            self.e(I::I32Const(1 << w));
                            self.e(I::I32Or);
                        }
                        self.e(I::I32Ctz);
                    }
                }
                self.set(results[0])?;
            }
            (Opcode::Icmp, InstructionData::IntCompare { cond, .. }) => {
                self.icmp(cond, args[0], args[1])?;
                self.set(results[0])?;
            }
            (Opcode::Fcmp, InstructionData::FloatCompare { cond, .. }) => {
                self.fcmp(cond, args[0], args[1])?;
                self.set(results[0])?;
            }
            (Opcode::Select | Opcode::SelectSpectreGuard, _) => {
                let t = self.ty(args[1]);
                if t == types::I128 {
                    self.get_lo(args[1])?;
                    self.get_lo(args[2])?;
                    self.cond(args[0])?;
                    self.e(I::Select);
                    self.get_hi(args[1])?;
                    self.get_hi(args[2])?;
                    self.cond(args[0])?;
                    self.e(I::Select);
                } else {
                    self.get(args[1])?;
                    self.get(args[2])?;
                    self.cond(args[0])?;
                    self.e(I::Select);
                }
                self.set(results[0])?;
            }
            (Opcode::Uextend, _) => {
                let (from, to) = (self.ty(args[0]), rty.ok_or("an extension")?);
                self.get(args[0])?;
                match (from, to) {
                    (types::I64, types::I128) => self.e(I::I64Const(0)),
                    (_, types::I128) => {
                        self.e(I::I64ExtendI32U);
                        self.e(I::I64Const(0));
                    }
                    (types::I64, _) => {}
                    (_, types::I64) => self.e(I::I64ExtendI32U),
                    // Zero-extended already.
                    _ => {}
                }
                self.set(results[0])?;
            }
            (Opcode::Sextend, _) => {
                let (from, to) = (self.ty(args[0]), rty.ok_or("an extension")?);
                self.get_signed(args[0])?;
                match (from, to) {
                    (_, types::I128) => {
                        if from != types::I64 {
                            self.e(I::I64ExtendI32S);
                        }
                        let lo = self.local(ValType::I64);
                        self.e(I::LocalTee(lo));
                        self.e(I::LocalGet(lo));
                        self.e(I::I64Const(63));
                        self.e(I::I64ShrS);
                    }
                    (types::I64, _) => {}
                    (_, types::I64) => self.e(I::I64ExtendI32S),
                    (_, t) => self.canon(t),
                }
                self.set(results[0])?;
            }
            (Opcode::Ireduce, _) => {
                let (from, to) = (self.ty(args[0]), rty.ok_or("a reduction")?);
                if from == types::I128 {
                    self.get_lo(args[0])?;
                } else {
                    self.get(args[0])?;
                }
                if matches!(from, types::I64 | types::I128) && to != types::I64 {
                    self.e(I::I32WrapI64);
                }
                self.canon(to);
                self.set(results[0])?;
            }
            (Opcode::Iconcat, _) => {
                self.get(args[0])?;
                self.get(args[1])?;
                self.set(results[0])?;
            }
            (Opcode::Isplit, _) => {
                self.get_lo(args[0])?;
                self.set(results[0])?;
                self.get_hi(args[0])?;
                self.set(results[1])?;
            }
            (Opcode::Bitcast, _) => {
                let (from, to) = (self.ty(args[0]), rty.ok_or("a bit cast")?);
                self.get(args[0])?;
                match (from, to) {
                    (types::I64, types::F64) => self.e(I::F64ReinterpretI64),
                    (types::F64, types::I64) => self.e(I::I64ReinterpretF64),
                    (types::I32, types::F32) => self.e(I::F32ReinterpretI32),
                    (types::F32, types::I32) => self.e(I::I32ReinterpretF32),
                    (a, b) if a == b => {}
                    (a, b) => return Err(format!("a bit cast of `{a}` to `{b}`")),
                }
                self.set(results[0])?;
            }
            (
                Opcode::Fadd
                | Opcode::Fsub
                | Opcode::Fmul
                | Opcode::Fdiv
                | Opcode::Fmin
                | Opcode::Fmax
                | Opcode::Fcopysign,
                _,
            ) => {
                let f64_ = rty == Some(types::F64);
                self.get(args[0])?;
                self.get(args[1])?;
                self.e(match (op, f64_) {
                    (Opcode::Fadd, true) => I::F64Add,
                    (Opcode::Fsub, true) => I::F64Sub,
                    (Opcode::Fmul, true) => I::F64Mul,
                    (Opcode::Fdiv, true) => I::F64Div,
                    (Opcode::Fmin, true) => I::F64Min,
                    (Opcode::Fmax, true) => I::F64Max,
                    (_, true) => I::F64Copysign,
                    (Opcode::Fadd, false) => I::F32Add,
                    (Opcode::Fsub, false) => I::F32Sub,
                    (Opcode::Fmul, false) => I::F32Mul,
                    (Opcode::Fdiv, false) => I::F32Div,
                    (Opcode::Fmin, false) => I::F32Min,
                    (Opcode::Fmax, false) => I::F32Max,
                    _ => I::F32Copysign,
                });
                self.set(results[0])?;
            }
            (
                Opcode::Fneg
                | Opcode::Fabs
                | Opcode::Sqrt
                | Opcode::Ceil
                | Opcode::Floor
                | Opcode::Trunc
                | Opcode::Nearest,
                _,
            ) => {
                let f64_ = rty == Some(types::F64);
                self.get(args[0])?;
                self.e(match (op, f64_) {
                    (Opcode::Fneg, true) => I::F64Neg,
                    (Opcode::Fabs, true) => I::F64Abs,
                    (Opcode::Sqrt, true) => I::F64Sqrt,
                    (Opcode::Ceil, true) => I::F64Ceil,
                    (Opcode::Floor, true) => I::F64Floor,
                    (Opcode::Trunc, true) => I::F64Trunc,
                    (_, true) => I::F64Nearest,
                    (Opcode::Fneg, false) => I::F32Neg,
                    (Opcode::Fabs, false) => I::F32Abs,
                    (Opcode::Sqrt, false) => I::F32Sqrt,
                    (Opcode::Ceil, false) => I::F32Ceil,
                    (Opcode::Floor, false) => I::F32Floor,
                    (Opcode::Trunc, false) => I::F32Trunc,
                    _ => I::F32Nearest,
                });
                self.set(results[0])?;
            }
            (Opcode::Fpromote, _) => {
                self.get(args[0])?;
                self.e(I::F64PromoteF32);
                self.set(results[0])?;
            }
            (Opcode::Fdemote, _) => {
                self.get(args[0])?;
                self.e(I::F32DemoteF64);
                self.set(results[0])?;
            }
            (Opcode::FcvtFromSint | Opcode::FcvtFromUint, _) => {
                let from = self.ty(args[0]);
                let to64 = rty == Some(types::F64);
                let s = op == Opcode::FcvtFromSint;
                if s {
                    self.get_signed(args[0])?;
                } else {
                    self.get(args[0])?;
                }
                self.e(match (from, to64, s) {
                    (types::I128, ..) => return Err("an `i128` as a float".to_string()),
                    (types::I64, true, true) => I::F64ConvertI64S,
                    (types::I64, true, false) => I::F64ConvertI64U,
                    (types::I64, false, true) => I::F32ConvertI64S,
                    (types::I64, false, false) => I::F32ConvertI64U,
                    (_, true, true) => I::F64ConvertI32S,
                    (_, true, false) => I::F64ConvertI32U,
                    (_, false, true) => I::F32ConvertI32S,
                    (_, false, false) => I::F32ConvertI32U,
                });
                self.set(results[0])?;
            }
            (
                Opcode::FcvtToSint
                | Opcode::FcvtToUint
                | Opcode::FcvtToSintSat
                | Opcode::FcvtToUintSat,
                _,
            ) => {
                let from64 = self.ty(args[0]) == types::F64;
                let to = rty.ok_or("a conversion")?;
                let s = matches!(op, Opcode::FcvtToSint | Opcode::FcvtToSintSat);
                let sat = matches!(op, Opcode::FcvtToSintSat | Opcode::FcvtToUintSat);
                self.get(args[0])?;
                let i = match (to, from64, s, sat) {
                    (types::I64, true, true, false) => I::I64TruncF64S,
                    (types::I64, true, false, false) => I::I64TruncF64U,
                    (types::I64, false, true, false) => I::I64TruncF32S,
                    (types::I64, false, false, false) => I::I64TruncF32U,
                    (types::I64, true, true, true) => I::I64TruncSatF64S,
                    (types::I64, true, false, true) => I::I64TruncSatF64U,
                    (types::I64, false, true, true) => I::I64TruncSatF32S,
                    (types::I64, false, false, true) => I::I64TruncSatF32U,
                    (types::I32, true, true, false) => I::I32TruncF64S,
                    (types::I32, true, false, false) => I::I32TruncF64U,
                    (types::I32, false, true, false) => I::I32TruncF32S,
                    (types::I32, false, false, false) => I::I32TruncF32U,
                    (types::I32, true, true, true) => I::I32TruncSatF64S,
                    (types::I32, true, false, true) => I::I32TruncSatF64U,
                    (types::I32, false, true, true) => I::I32TruncSatF32S,
                    (types::I32, false, false, true) => I::I32TruncSatF32U,
                    (t, ..) => return Err(format!("a float as a `{t}`")),
                };
                self.e(i);
                self.set(results[0])?;
            }
            (
                Opcode::SaddOverflow
                | Opcode::UaddOverflow
                | Opcode::SsubOverflow
                | Opcode::UsubOverflow
                | Opcode::SmulOverflow
                | Opcode::UmulOverflow,
                _,
            ) => self.overflow(op, args[0], args[1], results[0], results[1])?,
            (
                Opcode::Load
                | Opcode::Uload8
                | Opcode::Sload8
                | Opcode::Uload16
                | Opcode::Sload16
                | Opcode::Uload32
                | Opcode::Sload32,
                InstructionData::Load { offset, .. },
            ) => {
                let t = rty.ok_or("a load")?;
                let offset = i32::from(offset);
                if t == types::I128 {
                    let a = self.address_local(args[0], offset)?;
                    self.e(I::LocalGet(a));
                    self.e(I::I64Load(mem(0, 3)));
                    self.e(I::LocalGet(a));
                    self.e(I::I64Load(mem(8, 3)));
                } else {
                    let off = self.address(args[0], offset)?;
                    let i = match (op, t) {
                        (Opcode::Uload8, types::I64) => I::I64Load8U(mem(off, 0)),
                        (Opcode::Sload8, types::I64) => I::I64Load8S(mem(off, 0)),
                        (Opcode::Uload16, types::I64) => I::I64Load16U(mem(off, 1)),
                        (Opcode::Sload16, types::I64) => I::I64Load16S(mem(off, 1)),
                        (Opcode::Load, types::I8) | (Opcode::Uload8, _) => {
                            I::I32Load8U(mem(off, 0))
                        }
                        (Opcode::Load, types::I16) | (Opcode::Uload16, _) => {
                            I::I32Load16U(mem(off, 1))
                        }
                        (Opcode::Load, types::I32) => I::I32Load(mem(off, 2)),
                        (Opcode::Load, types::I64) => I::I64Load(mem(off, 3)),
                        (Opcode::Load, types::F32) => I::F32Load(mem(off, 2)),
                        (Opcode::Load, types::F64) => I::F64Load(mem(off, 3)),
                        (Opcode::Sload8, _) => I::I32Load8S(mem(off, 0)),
                        (Opcode::Sload16, _) => I::I32Load16S(mem(off, 1)),
                        (Opcode::Uload32, _) => I::I64Load32U(mem(off, 2)),
                        (Opcode::Sload32, _) => I::I64Load32S(mem(off, 2)),
                        (op, t) => return Err(format!("`{op}` of a `{t}`")),
                    };
                    self.e(i);
                    if matches!(op, Opcode::Sload8 | Opcode::Sload16) {
                        self.canon(t);
                    }
                }
                self.set(results[0])?;
            }
            (
                Opcode::Store | Opcode::Istore8 | Opcode::Istore16 | Opcode::Istore32,
                InstructionData::Store { offset, .. },
            ) => {
                let (x, p) = (args[0], args[1]);
                let t = self.ty(x);
                let offset = i32::from(offset);
                if t == types::I128 {
                    let a = self.address_local(p, offset)?;
                    self.e(I::LocalGet(a));
                    self.get_lo(x)?;
                    self.e(I::I64Store(mem(0, 3)));
                    self.e(I::LocalGet(a));
                    self.get_hi(x)?;
                    self.e(I::I64Store(mem(8, 3)));
                } else {
                    let off = self.address(p, offset)?;
                    self.get(x)?;
                    let i = match (op, t) {
                        (Opcode::Istore8, types::I64) => I::I64Store8(mem(off, 0)),
                        (Opcode::Istore16, types::I64) => I::I64Store16(mem(off, 1)),
                        (Opcode::Store, types::I8) | (Opcode::Istore8, _) => {
                            I::I32Store8(mem(off, 0))
                        }
                        (Opcode::Store, types::I16) | (Opcode::Istore16, _) => {
                            I::I32Store16(mem(off, 1))
                        }
                        (Opcode::Store, types::I32) => I::I32Store(mem(off, 2)),
                        (Opcode::Store, types::I64) => I::I64Store(mem(off, 3)),
                        (Opcode::Store, types::F32) => I::F32Store(mem(off, 2)),
                        (Opcode::Store, types::F64) => I::F64Store(mem(off, 3)),
                        (Opcode::Istore32, _) => I::I64Store32(mem(off, 2)),
                        (op, t) => return Err(format!("`{op}` of a `{t}`")),
                    };
                    self.e(i);
                }
            }
            (
                Opcode::StackAddr,
                InstructionData::StackAddr {
                    stack_slot, offset, ..
                },
            ) => {
                let at = self.slots[&stack_slot] as i64 + i64::from(i32::from(offset));
                self.e(I::LocalGet(self.fp));
                self.e(I::I64ExtendI32U);
                if at != 0 {
                    self.e(I::I64Const(at));
                    self.e(I::I64Add);
                }
                self.set(results[0])?;
            }
            (Opcode::SymbolValue, InstructionData::UnaryGlobalValue { global_value, .. }) => {
                let GlobalValueData::Symbol { name, offset, .. } =
                    &self.func.global_values[global_value]
                else {
                    return Err("a global value that is not a symbol".to_string());
                };
                let (ns, index) = self.user_name(name)?;
                let off = offset.bits();
                self.code.push(match ns {
                    1 => Op::DataAddr(DataId::from_u32(index), off),
                    _ => Op::FuncAddr(FuncId::from_u32(index)),
                });
                self.set(results[0])?;
            }
            (Opcode::FuncAddr, InstructionData::FuncAddr { func_ref, .. }) => {
                let name = self.func.dfg.ext_funcs[func_ref].name.clone();
                let (_, index) = self.user_name(&name)?;
                self.code.push(Op::FuncAddr(FuncId::from_u32(index)));
                self.set(results[0])?;
            }
            (Opcode::Call, InstructionData::Call { func_ref, .. }) => {
                let ext = &self.func.dfg.ext_funcs[func_ref];
                let name = ext.name.clone();
                let sig = self.func.dfg.signatures[ext.signature].clone();
                let (_, index) = self.user_name(&name)?;
                let f = FuncId::from_u32(index);
                match self.env.callee(f)? {
                    Callee::AsDeclared => {
                        for &a in &args {
                            self.get(a)?;
                        }
                        self.code.push(Op::Call(f));
                        for &r in results.iter().rev() {
                            self.set(r)?;
                        }
                    }
                    Callee::Adapted(ps, rs) => {
                        let mut k = 0;
                        for &a in &args {
                            let t = self.ty(a);
                            for (j, have) in vals(t)?.iter().enumerate() {
                                let (lo, hi) = self.slot_of(a)?;
                                self.e(I::LocalGet(if j == 0 { lo } else { hi.unwrap_or(lo) }));
                                let want = *ps.get(k).ok_or("a runtime call's arguments")?;
                                self.convert(*have, want);
                                k += 1;
                            }
                        }
                        self.code.push(Op::Call(f));
                        // The results, last first, each as the code wants it.
                        let mut wants = Vec::new();
                        for r in &sig.returns {
                            for w in vals(r.value_type)? {
                                wants.push((*w, r.value_type));
                            }
                        }
                        if wants.len() != rs.len() {
                            return Err("a runtime call's results".to_string());
                        }
                        let mut k = rs.len();
                        for &r in results.iter().rev() {
                            let n = vals(self.ty(r))?.len();
                            let (lo, hi) = self.slot_of(r)?;
                            let locs = [lo, hi.unwrap_or(lo)];
                            for j in (0..n).rev() {
                                k -= 1;
                                self.convert(rs[k], wants[k].0);
                                self.canon(wants[k].1);
                                self.e(I::LocalSet(locs[j]));
                            }
                        }
                    }
                }
            }
            (Opcode::CallIndirect, InstructionData::CallIndirect { sig_ref, .. }) => {
                let sig = self.func.dfg.signatures[sig_ref].clone();
                let (ps, rs) = signature(&sig)?;
                let ty = self.env.type_index(&ps, &rs);
                for &a in &args[1..] {
                    self.get(a)?;
                }
                self.get(args[0])?;
                self.e(I::I32WrapI64);
                self.e(I::CallIndirect {
                    type_index: ty,
                    table_index: 0,
                });
                for &r in results.iter().rev() {
                    self.set(r)?;
                }
            }
            (Opcode::Trapz | Opcode::Trapnz, _) => {
                self.cond(args[0])?;
                if op == Opcode::Trapz {
                    self.e(I::I32Eqz);
                }
                self.e(I::If(BlockType::Empty));
                self.e(I::Unreachable);
                self.e(I::End);
            }
            (op, _) => return Err(format!("the Cranelift instruction `{op}`")),
        }
        Ok(())
    }

    /// A user name's namespace (0 for functions, 1 for data) and index.
    fn user_name(&self, name: &ExternalName) -> Result<(u32, u32), String> {
        let ExternalName::User(r) = name else {
            return Err(format!("a call of `{name:?}`"));
        };
        let n = &self.func.params.user_named_funcs()[*r];
        Ok((n.namespace, n.index))
    }

    /// Converts the value on the stack from wasm type `have` to `want` (an address: an `i64`
    /// and an `i32`, unsigned).
    fn convert(&mut self, have: ValType, want: ValType) {
        match (have, want) {
            (ValType::I64, ValType::I32) => self.e(I::I32WrapI64),
            (ValType::I32, ValType::I64) => self.e(I::I64ExtendI32U),
            _ => {}
        }
    }

    /// Pushes condition `v` as an `i32` that is not zero when it holds.
    fn cond(&mut self, v: Value) -> Result<(), String> {
        match self.ty(v) {
            types::I64 => {
                self.get(v)?;
                self.e(I::I64Const(0));
                self.e(I::I64Ne);
            }
            types::I128 => {
                self.get_lo(v)?;
                self.get_hi(v)?;
                self.e(I::I64Or);
                self.e(I::I64Const(0));
                self.e(I::I64Ne);
            }
            _ => self.get(v)?,
        }
        Ok(())
    }

    fn int_binary(&mut self, op: Opcode, args: &[Value], r: Value) -> Result<(), String> {
        let t = self.ty(args[0]);
        let (x, y) = (args[0], args[1]);
        if matches!(
            op,
            Opcode::Smin | Opcode::Smax | Opcode::Umin | Opcode::Umax
        ) {
            let cc = match op {
                Opcode::Smin => IntCC::SignedLessThan,
                Opcode::Smax => IntCC::SignedGreaterThan,
                Opcode::Umin => IntCC::UnsignedLessThan,
                _ => IntCC::UnsignedGreaterThan,
            };
            if t == types::I128 {
                return Err(format!("`{op}` on an `i128`"));
            }
            self.get(x)?;
            self.get(y)?;
            self.icmp(cc, x, y)?;
            self.e(I::Select);
            return self.set(r);
        }
        if t == types::I128 {
            match op {
                Opcode::Iadd => {
                    self.add128(x, y)?;
                    return self.set(r);
                }
                Opcode::Isub => {
                    self.sub128(x, y)?;
                    return self.set(r);
                }
                Opcode::Band | Opcode::Bor | Opcode::Bxor => {
                    let i = match op {
                        Opcode::Band => I::I64And,
                        Opcode::Bor => I::I64Or,
                        _ => I::I64Xor,
                    };
                    self.get_lo(x)?;
                    self.get_lo(y)?;
                    self.e(i.clone());
                    self.get_hi(x)?;
                    self.get_hi(y)?;
                    self.e(i);
                    return self.set(r);
                }
                _ => {
                    let code = match op {
                        Opcode::Imul => 0,
                        Opcode::Sdiv => 1,
                        Opcode::Udiv => 2,
                        Opcode::Srem => 3,
                        _ => 4,
                    };
                    self.i128_call(code, x, y)?;
                    self.e(I::Drop);
                    self.i128_result()?;
                    return self.set(r);
                }
            }
        }
        let wide = t == types::I64;
        let signed = matches!(op, Opcode::Sdiv | Opcode::Srem);
        if signed {
            self.get_signed(x)?;
            self.get_signed(y)?;
        } else {
            self.get(x)?;
            self.get(y)?;
        }
        self.e(match (op, wide) {
            (Opcode::Iadd, true) => I::I64Add,
            (Opcode::Isub, true) => I::I64Sub,
            (Opcode::Imul, true) => I::I64Mul,
            (Opcode::Udiv, true) => I::I64DivU,
            (Opcode::Sdiv, true) => I::I64DivS,
            (Opcode::Urem, true) => I::I64RemU,
            (Opcode::Srem, true) => I::I64RemS,
            (Opcode::Band, true) => I::I64And,
            (Opcode::Bor, true) => I::I64Or,
            (_, true) => I::I64Xor,
            (Opcode::Iadd, false) => I::I32Add,
            (Opcode::Isub, false) => I::I32Sub,
            (Opcode::Imul, false) => I::I32Mul,
            (Opcode::Udiv, false) => I::I32DivU,
            (Opcode::Sdiv, false) => I::I32DivS,
            (Opcode::Urem, false) => I::I32RemU,
            (Opcode::Srem, false) => I::I32RemS,
            (Opcode::Band, false) => I::I32And,
            (Opcode::Bor, false) => I::I32Or,
            _ => I::I32Xor,
        });
        self.canon(t);
        self.set(r)
    }

    /// Pushes the shift amount `v` as a value of `x`'s wasm type.
    fn amount(&mut self, v: Value, wide: bool) -> Result<(), String> {
        let t = self.ty(v);
        self.get_lo(v)?;
        let have = if t == types::I128 || t == types::I64 {
            ValType::I64
        } else {
            ValType::I32
        };
        self.convert(have, if wide { ValType::I64 } else { ValType::I32 });
        Ok(())
    }

    fn shift(&mut self, op: Opcode, args: &[Value], r: Value) -> Result<(), String> {
        let (x, y) = (args[0], args[1]);
        let t = self.ty(x);
        match t {
            types::I128 => {
                let code = match op {
                    Opcode::Ishl => 5,
                    Opcode::Ushr => 6,
                    Opcode::Sshr => 7,
                    Opcode::Rotl => 8,
                    _ => return Err("rotating an `i128` right".to_string()),
                };
                // The amount, as an `i128`.
                let b = self.local(ValType::I64);
                self.amount(y, true)?;
                self.e(I::LocalSet(b));
                self.e(I::I32Const(code));
                self.get(x)?;
                self.e(I::LocalGet(b));
                self.e(I::I64Const(0));
                self.e(I::GlobalGet(I128_OUT));
                self.code.push(Op::CallRuntime("moddil_wasm_i128"));
                self.e(I::Drop);
                self.i128_result()?;
            }
            types::I64 | types::I32 => {
                let wide = t == types::I64;
                self.get(x)?;
                self.amount(y, wide)?;
                self.e(match (op, wide) {
                    (Opcode::Ishl, true) => I::I64Shl,
                    (Opcode::Ushr, true) => I::I64ShrU,
                    (Opcode::Sshr, true) => I::I64ShrS,
                    (Opcode::Rotl, true) => I::I64Rotl,
                    (_, true) => I::I64Rotr,
                    (Opcode::Ishl, false) => I::I32Shl,
                    (Opcode::Ushr, false) => I::I32ShrU,
                    (Opcode::Sshr, false) => I::I32ShrS,
                    (Opcode::Rotl, false) => I::I32Rotl,
                    _ => I::I32Rotr,
                });
            }
            t => {
                // Narrow: by the amount modulo the width, in 32 bits.
                let w = t.bits() as i32;
                let k = self.local(ValType::I32);
                self.amount(y, false)?;
                self.e(I::I32Const(w - 1));
                self.e(I::I32And);
                self.e(I::LocalSet(k));
                match op {
                    Opcode::Ishl => {
                        self.get(x)?;
                        self.e(I::LocalGet(k));
                        self.e(I::I32Shl);
                    }
                    Opcode::Ushr => {
                        self.get(x)?;
                        self.e(I::LocalGet(k));
                        self.e(I::I32ShrU);
                    }
                    Opcode::Sshr => {
                        self.get_signed(x)?;
                        self.e(I::LocalGet(k));
                        self.e(I::I32ShrS);
                    }
                    _ => {
                        // Both ways, by `k` and by the rest of the width.
                        let (first, second) = if op == Opcode::Rotl {
                            (I::I32Shl, I::I32ShrU)
                        } else {
                            (I::I32ShrU, I::I32Shl)
                        };
                        self.get(x)?;
                        self.e(I::LocalGet(k));
                        self.e(first);
                        self.get(x)?;
                        self.e(I::I32Const(w));
                        self.e(I::LocalGet(k));
                        self.e(I::I32Sub);
                        self.e(I::I32Const(w - 1));
                        self.e(I::I32And);
                        self.e(second);
                        self.e(I::I32Or);
                    }
                }
                self.canon(t);
            }
        }
        self.set(r)
    }

    /// Calls the runtime's `i128` operation `code` on `x` and `y`: its flag on the stack, its
    /// result at [`I128_OUT`].
    fn i128_call(&mut self, code: i32, x: Value, y: Value) -> Result<(), String> {
        self.e(I::I32Const(code));
        self.get(x)?;
        self.get(y)?;
        self.e(I::GlobalGet(I128_OUT));
        self.code.push(Op::CallRuntime("moddil_wasm_i128"));
        Ok(())
    }

    /// Pushes the `i128` at [`I128_OUT`].
    fn i128_result(&mut self) -> Result<(), String> {
        self.e(I::GlobalGet(I128_OUT));
        self.e(I::I64Load(mem(0, 3)));
        self.e(I::GlobalGet(I128_OUT));
        self.e(I::I64Load(mem(8, 3)));
        Ok(())
    }

    /// Pushes `x + y` (`i128`s).
    fn add128(&mut self, x: Value, y: Value) -> Result<(), String> {
        // lo = x.lo + y.lo; hi = x.hi + y.hi + (lo < x.lo).
        let lo = self.local(ValType::I64);
        self.get_lo(x)?;
        self.get_lo(y)?;
        self.e(I::I64Add);
        self.e(I::LocalTee(lo));
        self.get_hi(x)?;
        self.get_hi(y)?;
        self.e(I::I64Add);
        self.e(I::LocalGet(lo));
        self.get_lo(x)?;
        self.e(I::I64LtU);
        self.e(I::I64ExtendI32U);
        self.e(I::I64Add);
        Ok(())
    }

    /// Pushes `x - y` (`i128`s).
    fn sub128(&mut self, x: Value, y: Value) -> Result<(), String> {
        // lo = x.lo - y.lo; hi = x.hi - y.hi - (x.lo < y.lo).
        self.get_lo(x)?;
        self.get_lo(y)?;
        self.e(I::I64Sub);
        self.get_hi(x)?;
        self.get_hi(y)?;
        self.e(I::I64Sub);
        self.get_lo(x)?;
        self.get_lo(y)?;
        self.e(I::I64LtU);
        self.e(I::I64ExtendI32U);
        self.e(I::I64Sub);
        Ok(())
    }

    /// Pushes whether `x cond y` holds, as an `i32` 0 or 1.
    fn icmp(&mut self, cond: IntCC, x: Value, y: Value) -> Result<(), String> {
        let t = self.ty(x);
        let signed = matches!(
            cond,
            IntCC::SignedLessThan
                | IntCC::SignedLessThanOrEqual
                | IntCC::SignedGreaterThan
                | IntCC::SignedGreaterThanOrEqual
        );
        if t == types::I128 {
            return self.icmp128(cond, x, y);
        }
        let wide = t == types::I64;
        if signed {
            self.get_signed(x)?;
            self.get_signed(y)?;
        } else {
            self.get(x)?;
            self.get(y)?;
        }
        self.e(cmp(cond, wide));
        Ok(())
    }

    fn icmp128(&mut self, cond: IntCC, x: Value, y: Value) -> Result<(), String> {
        match cond {
            IntCC::Equal | IntCC::NotEqual => {
                self.get_lo(x)?;
                self.get_lo(y)?;
                self.e(I::I64Xor);
                self.get_hi(x)?;
                self.get_hi(y)?;
                self.e(I::I64Xor);
                self.e(I::I64Or);
                self.e(if cond == IntCC::Equal {
                    I::I64Eqz
                } else {
                    I::I64Const(0)
                });
                if cond == IntCC::NotEqual {
                    self.e(I::I64Ne);
                }
            }
            _ => {
                // By the high words, or, if they are equal, the low ones (unsigned).
                let low = match cond {
                    IntCC::SignedLessThan | IntCC::UnsignedLessThan => IntCC::UnsignedLessThan,
                    IntCC::SignedLessThanOrEqual | IntCC::UnsignedLessThanOrEqual => {
                        IntCC::UnsignedLessThanOrEqual
                    }
                    IntCC::SignedGreaterThan | IntCC::UnsignedGreaterThan => {
                        IntCC::UnsignedGreaterThan
                    }
                    _ => IntCC::UnsignedGreaterThanOrEqual,
                };
                let strict = match cond {
                    IntCC::SignedLessThan | IntCC::SignedLessThanOrEqual => IntCC::SignedLessThan,
                    IntCC::UnsignedLessThan | IntCC::UnsignedLessThanOrEqual => {
                        IntCC::UnsignedLessThan
                    }
                    IntCC::SignedGreaterThan | IntCC::SignedGreaterThanOrEqual => {
                        IntCC::SignedGreaterThan
                    }
                    _ => IntCC::UnsignedGreaterThan,
                };
                self.get_lo(x)?;
                self.get_lo(y)?;
                self.e(cmp(low, true));
                self.get_hi(x)?;
                self.get_hi(y)?;
                self.e(cmp(strict, true));
                self.get_hi(x)?;
                self.get_hi(y)?;
                self.e(I::I64Eq);
                self.e(I::Select);
            }
        }
        Ok(())
    }

    fn fcmp(&mut self, cond: FloatCC, x: Value, y: Value) -> Result<(), String> {
        let f64_ = self.ty(x) == types::F64;
        let op = |c: FloatCC| -> I<'static> {
            match (c, f64_) {
                (FloatCC::Equal, true) => I::F64Eq,
                (FloatCC::NotEqual, true) => I::F64Ne,
                (FloatCC::LessThan, true) => I::F64Lt,
                (FloatCC::LessThanOrEqual, true) => I::F64Le,
                (FloatCC::GreaterThan, true) => I::F64Gt,
                (_, true) => I::F64Ge,
                (FloatCC::Equal, false) => I::F32Eq,
                (FloatCC::NotEqual, false) => I::F32Ne,
                (FloatCC::LessThan, false) => I::F32Lt,
                (FloatCC::LessThanOrEqual, false) => I::F32Le,
                (FloatCC::GreaterThan, false) => I::F32Gt,
                _ => I::F32Ge,
            }
        };
        let simple = |s: &mut Self, c: FloatCC| -> Result<(), String> {
            s.get(x)?;
            s.get(y)?;
            s.e(op(c));
            Ok(())
        };
        match cond {
            FloatCC::Equal
            | FloatCC::NotEqual
            | FloatCC::LessThan
            | FloatCC::LessThanOrEqual
            | FloatCC::GreaterThan
            | FloatCC::GreaterThanOrEqual => simple(self, cond)?,
            FloatCC::Ordered | FloatCC::Unordered => {
                // Each is itself (not NaN).
                self.get(x)?;
                self.get(x)?;
                self.e(op(FloatCC::Equal));
                self.get(y)?;
                self.get(y)?;
                self.e(op(FloatCC::Equal));
                self.e(I::I32And);
                if cond == FloatCC::Unordered {
                    self.e(I::I32Eqz);
                }
            }
            FloatCC::OrderedNotEqual | FloatCC::UnorderedOrEqual => {
                simple(self, FloatCC::LessThan)?;
                simple(self, FloatCC::GreaterThan)?;
                self.e(I::I32Or);
                if cond == FloatCC::UnorderedOrEqual {
                    self.e(I::I32Eqz);
                }
            }
            // Unordered, or the comparison: not the opposite ordered one.
            FloatCC::UnorderedOrLessThan => {
                simple(self, FloatCC::GreaterThanOrEqual)?;
                self.e(I::I32Eqz);
            }
            FloatCC::UnorderedOrLessThanOrEqual => {
                simple(self, FloatCC::GreaterThan)?;
                self.e(I::I32Eqz);
            }
            FloatCC::UnorderedOrGreaterThan => {
                simple(self, FloatCC::LessThanOrEqual)?;
                self.e(I::I32Eqz);
            }
            FloatCC::UnorderedOrGreaterThanOrEqual => {
                simple(self, FloatCC::LessThan)?;
                self.e(I::I32Eqz);
            }
        }
        Ok(())
    }

    /// `x op y` and whether it overflows, into `r` and `flag`.
    fn overflow(
        &mut self,
        op: Opcode,
        x: Value,
        y: Value,
        r: Value,
        flag: Value,
    ) -> Result<(), String> {
        let t = self.ty(x);
        let signed = matches!(
            op,
            Opcode::SaddOverflow | Opcode::SsubOverflow | Opcode::SmulOverflow
        );
        match t {
            types::I128 => self.overflow128(op, x, y, r, flag),
            types::I64 => self.overflow64(op, x, y, r, flag),
            _ => {
                // In 64 bits, from the operands extended as the operation's signedness says,
                // then whether the result is in range.
                let w = t.bits();
                if signed {
                    self.get_signed(x)?;
                    self.e(I::I64ExtendI32S);
                    self.get_signed(y)?;
                    self.e(I::I64ExtendI32S);
                } else {
                    self.get(x)?;
                    self.e(I::I64ExtendI32U);
                    self.get(y)?;
                    self.e(I::I64ExtendI32U);
                }
                self.e(match op {
                    Opcode::SaddOverflow | Opcode::UaddOverflow => I::I64Add,
                    Opcode::SsubOverflow | Opcode::UsubOverflow => I::I64Sub,
                    _ => I::I64Mul,
                });
                let wide = self.local(ValType::I64);
                self.e(I::LocalTee(wide));
                self.e(I::I32WrapI64);
                self.canon(t);
                self.set(r)?;
                if signed {
                    let lo = (-1i64) << (w - 1);
                    self.e(I::LocalGet(wide));
                    self.e(I::I64Const(lo));
                    self.e(I::I64LtS);
                    self.e(I::LocalGet(wide));
                    self.e(I::I64Const(-lo - 1));
                    self.e(I::I64GtS);
                    self.e(I::I32Or);
                } else {
                    // Below zero (a difference), or past the largest value.
                    self.e(I::LocalGet(wide));
                    self.e(I::I64Const(((1u128 << w) - 1) as i64));
                    self.e(I::I64GtU);
                }
                self.set(flag)
            }
        }
    }

    fn overflow64(
        &mut self,
        op: Opcode,
        x: Value,
        y: Value,
        r: Value,
        flag: Value,
    ) -> Result<(), String> {
        match op {
            Opcode::SaddOverflow
            | Opcode::UaddOverflow
            | Opcode::SsubOverflow
            | Opcode::UsubOverflow => {
                let add = matches!(op, Opcode::SaddOverflow | Opcode::UaddOverflow);
                self.get(x)?;
                self.get(y)?;
                self.e(if add { I::I64Add } else { I::I64Sub });
                self.set(r)?;
                match op {
                    Opcode::UaddOverflow => {
                        // It wrapped below `x`.
                        self.get(r)?;
                        self.get(x)?;
                        self.e(I::I64LtU);
                    }
                    Opcode::UsubOverflow => {
                        self.get(x)?;
                        self.get(y)?;
                        self.e(I::I64LtU);
                    }
                    _ => {
                        // The result's sign is wrong: ((x ^ r) & (y ^ r)) < 0 for a sum,
                        // ((x ^ y) & (x ^ r)) < 0 for a difference.
                        self.get(x)?;
                        if add {
                            self.get(r)?;
                        } else {
                            self.get(y)?;
                        }
                        self.e(I::I64Xor);
                        if add {
                            self.get(y)?;
                        } else {
                            self.get(x)?;
                        }
                        self.get(r)?;
                        self.e(I::I64Xor);
                        self.e(I::I64And);
                        self.e(I::I64Const(0));
                        self.e(I::I64LtS);
                    }
                }
                self.set(flag)
            }
            _ => {
                let signed = op == Opcode::SmulOverflow;
                self.get(x)?;
                self.get(y)?;
                self.e(I::I64Mul);
                self.set(r)?;
                // `x` is not 0 and `x * y / x` is not `y`; signed, `-1 * MIN` (which division
                // cannot check) overflows too.
                self.get(x)?;
                self.e(I::I64Eqz);
                self.e(I::If(BlockType::Result(ValType::I32)));
                self.e(I::I32Const(0));
                self.e(I::Else);
                if signed {
                    self.get(x)?;
                    self.e(I::I64Const(-1));
                    self.e(I::I64Eq);
                    self.get(y)?;
                    self.e(I::I64Const(i64::MIN));
                    self.e(I::I64Eq);
                    self.e(I::I32And);
                    self.e(I::If(BlockType::Result(ValType::I32)));
                    self.e(I::I32Const(1));
                    self.e(I::Else);
                }
                self.get(r)?;
                self.get(x)?;
                self.e(if signed { I::I64DivS } else { I::I64DivU });
                self.get(y)?;
                self.e(I::I64Ne);
                if signed {
                    self.e(I::End);
                }
                self.e(I::End);
                self.set(flag)
            }
        }
    }

    fn overflow128(
        &mut self,
        op: Opcode,
        x: Value,
        y: Value,
        r: Value,
        flag: Value,
    ) -> Result<(), String> {
        match op {
            Opcode::SmulOverflow | Opcode::UmulOverflow => {
                let code = if op == Opcode::SmulOverflow { 9 } else { 10 };
                self.i128_call(code, x, y)?;
                self.set(flag)?;
                self.i128_result()?;
                self.set(r)
            }
            _ => {
                let add = matches!(op, Opcode::SaddOverflow | Opcode::UaddOverflow);
                if add {
                    self.add128(x, y)?;
                } else {
                    self.sub128(x, y)?;
                }
                self.set(r)?;
                match op {
                    Opcode::UaddOverflow => self.icmp128(IntCC::UnsignedLessThan, r, x)?,
                    Opcode::UsubOverflow => self.icmp128(IntCC::UnsignedLessThan, x, y)?,
                    _ => {
                        // As for 64 bits, on the high words.
                        self.get_hi(x)?;
                        if add {
                            self.get_hi(r)?;
                        } else {
                            self.get_hi(y)?;
                        }
                        self.e(I::I64Xor);
                        if add {
                            self.get_hi(y)?;
                        } else {
                            self.get_hi(x)?;
                        }
                        self.get_hi(r)?;
                        self.e(I::I64Xor);
                        self.e(I::I64And);
                        self.e(I::I64Const(0));
                        self.e(I::I64LtS);
                    }
                }
                self.set(flag)
            }
        }
    }
}

/// The comparison `cond` on two `i32`s, or two `i64`s.
fn cmp(cond: IntCC, wide: bool) -> I<'static> {
    match (cond, wide) {
        (IntCC::Equal, true) => I::I64Eq,
        (IntCC::NotEqual, true) => I::I64Ne,
        (IntCC::SignedLessThan, true) => I::I64LtS,
        (IntCC::SignedLessThanOrEqual, true) => I::I64LeS,
        (IntCC::SignedGreaterThan, true) => I::I64GtS,
        (IntCC::SignedGreaterThanOrEqual, true) => I::I64GeS,
        (IntCC::UnsignedLessThan, true) => I::I64LtU,
        (IntCC::UnsignedLessThanOrEqual, true) => I::I64LeU,
        (IntCC::UnsignedGreaterThan, true) => I::I64GtU,
        (_, true) => I::I64GeU,
        (IntCC::Equal, false) => I::I32Eq,
        (IntCC::NotEqual, false) => I::I32Ne,
        (IntCC::SignedLessThan, false) => I::I32LtS,
        (IntCC::SignedLessThanOrEqual, false) => I::I32LeS,
        (IntCC::SignedGreaterThan, false) => I::I32GtS,
        (IntCC::SignedGreaterThanOrEqual, false) => I::I32GeS,
        (IntCC::UnsignedLessThan, false) => I::I32LtU,
        (IntCC::UnsignedLessThanOrEqual, false) => I::I32LeU,
        (IntCC::UnsignedGreaterThan, false) => I::I32GtU,
        _ => I::I32GeU,
    }
}

impl cfg::Emit for Translator<'_> {
    fn node(&mut self, n: usize) -> Result<Exit, String> {
        let b = self.order[n];
        let insts: Vec<ir::Inst> = self.func.layout.block_insts(b).collect();
        let Some((&last, rest)) = insts.split_last() else {
            return Ok(Exit::Done);
        };
        for &i in rest {
            self.inst(i)?;
        }
        let dfg = &self.func.dfg;
        let pool = &dfg.value_lists;
        match dfg.insts[last] {
            InstructionData::Jump { destination, .. } => {
                Ok(Exit::Goto(self.index[&destination.block(pool)]))
            }
            InstructionData::Brif { arg, blocks, .. } => {
                let (t, f) = (
                    self.index[&blocks[0].block(pool)],
                    self.index[&blocks[1].block(pool)],
                );
                let arg = dfg.resolve_aliases(arg);
                self.cond(arg)?;
                Ok(Exit::Branch(t, f))
            }
            InstructionData::MultiAry {
                opcode: Opcode::Return,
                ..
            } => {
                let args: Vec<Value> = dfg
                    .inst_args(last)
                    .iter()
                    .map(|v| dfg.resolve_aliases(*v))
                    .collect();
                self.epilogue();
                for a in args {
                    self.get(a)?;
                }
                self.e(I::Return);
                Ok(Exit::Done)
            }
            InstructionData::Trap { .. } => {
                self.e(I::Unreachable);
                Ok(Exit::Done)
            }
            _ => {
                let op = dfg.insts[last].opcode();
                Err(format!("a block that ends in `{op}`"))
            }
        }
    }

    fn edge(&mut self, from: usize, k: usize) -> Result<(), String> {
        let b = self.order[from];
        let last = self
            .func
            .layout
            .last_inst(b)
            .ok_or("an edge from nothing")?;
        let dfg = &self.func.dfg;
        let pool = &dfg.value_lists;
        let call = match dfg.insts[last] {
            InstructionData::Jump { destination, .. } => destination,
            InstructionData::Brif { blocks, .. } => blocks[k],
            _ => return Ok(()),
        };
        let target = call.block(pool);
        let mut args = Vec::new();
        for a in call.args(pool) {
            match a {
                BlockArg::Value(v) => args.push(dfg.resolve_aliases(v)),
                _ => return Err("a block argument of an exception".to_string()),
            }
        }
        let params: Vec<Value> = dfg.block_params(target).to_vec();
        // All read, then all written: an argument may be another parameter.
        for &a in &args {
            self.get(a)?;
        }
        for &p in params.iter().rev() {
            self.set(p)?;
        }
        Ok(())
    }

    fn block(&mut self) {
        self.e(I::Block(BlockType::Empty));
    }
    fn loop_(&mut self) {
        self.e(I::Loop(BlockType::Empty));
    }
    fn if_(&mut self) {
        self.e(I::If(BlockType::Empty));
    }
    fn else_(&mut self) {
        self.e(I::Else);
    }
    fn end(&mut self) {
        self.e(I::End);
    }
    fn br(&mut self, depth: u32) {
        self.e(I::Br(depth));
    }
}

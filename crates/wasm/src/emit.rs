//! MIR to WebAssembly (slice W0): scalars, their operators and checks, control flow, direct calls
//! of non-generic functions, and tuples, structs, enums and arrays in linear memory with borrows
//! of them.
//!
//! **Values.** `int`, `i64`, `u64` and `f64` are wasm's `i64` and `f64`; the narrower integers,
//! `bool` and `char` are `i32`, kept sign- or zero-extended as their type says; `f32` is `f32`;
//! a borrow is an `i32` address. A scalar local nothing borrows is a wasm local; an aggregate, or
//! a local something borrows, lives in the function's frame on a shadow stack in linear memory,
//! which grows down from [`STACK_TOP`].
//!
//! **The ABI.** Each function returns a status (0 returned, 1 trapped) and then its result, if it
//! is a scalar. A scalar argument passes by value, an aggregate as the address of a copy the
//! caller makes; an aggregate result goes where an address the caller passes last says. A failed
//! check calls the host's `trap` with its site and returns 1, which callers pass on. Calls are
//! counted in a global, so recursion without end traps at the interpreters' depth.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use cranelift_codegen::ir::{self, types};
use moddil_mir::{
    AggregateKind, BinOp, Body, Const, Local, Operand, Place, Projection, Rvalue, Statement,
    Terminator, TrapClass, UnOp,
};
use moddil_native::layout::{Layout, Layouts, Shape, Tag};
use moddil_sema::{Callee, Db, Item, ItemTree, Num, Owner, Program, Src, Ty};
use moddil_syntax::TextRange;
use wasm_encoder::{
    BlockType, CodeSection, ConstExpr, EntityType, ExportKind, ExportSection, Function,
    FunctionSection, GlobalSection, GlobalType, ImportSection, Instruction as I, MemArg,
    MemorySection, MemoryType, Module, TypeSection, ValType,
};

use crate::cfg::{self, Exit};

/// Where the shadow stack starts, growing down: 2 MiB up the memory.
pub const STACK_TOP: u32 = 1 << 21;
/// Where `main`'s result goes, below the stack.
pub const OUT: u32 = 1024;
/// How many calls deep a run may go: as the interpreters' limit.
const MAX_DEPTH: i32 = 10_000;

// The host's functions, by index.
const TRAP: u32 = 0;
const OBSERVE: u32 = 1;
const FMOD: u32 = 2;
const FMODF: u32 = 3;
const IMPORTS: u32 = 4;
// The globals.
const SP: u32 = 0;
const DEPTH: u32 = 1;

/// What a site is.
#[derive(Clone, Debug)]
pub enum Site {
    /// A check: its class, the file and where.
    Trap(TrapClass, u32, TextRange),
    /// A `// =>` observation: the file, the expression and its type.
    Observe(u32, TextRange, Ty),
}

/// A program's entries compiled into one module.
pub struct Compiled<'db> {
    /// The module's bytes.
    pub bytes: Vec<u8>,
    /// Every site, by number.
    pub sites: Vec<Site>,
    /// The program's items.
    pub items: &'db ItemTree,
    /// `main`'s result type, if the module has `main` (exported as `main`, its result written
    /// at [`OUT`]).
    pub main: Option<Ty>,
}

/// How a value of some type is held.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    /// It has no bytes.
    Nothing,
    /// A scalar, of this wasm type.
    Scalar(ValType),
    /// In memory.
    Mem,
}

/// A scalar type's wasm type, width in bits, and whether it is signed, or a float.
#[derive(Clone, Copy)]
struct Info {
    vt: ValType,
    bits: u32,
    signed: bool,
    float: bool,
}

fn info(ty: &Ty) -> Option<Info> {
    let i = |vt, bits, signed| Info {
        vt,
        bits,
        signed,
        float: false,
    };
    Some(match ty {
        Ty::Int => i(ValType::I64, 64, true),
        Ty::Bool => i(ValType::I32, 8, false),
        // A borrow is a 32-bit address.
        Ty::Char | Ty::Ref(..) => i(ValType::I32, 32, false),
        Ty::Num(n) => match n {
            Num::I8 => i(ValType::I32, 8, true),
            Num::U8 => i(ValType::I32, 8, false),
            Num::I16 => i(ValType::I32, 16, true),
            Num::U16 => i(ValType::I32, 16, false),
            Num::I32 => i(ValType::I32, 32, true),
            Num::U32 => i(ValType::I32, 32, false),
            Num::U64 => i(ValType::I64, 64, false),
            Num::I128 | Num::U128 => return None,
            Num::F32 => Info {
                vt: ValType::F32,
                bits: 32,
                signed: true,
                float: true,
            },
            Num::F64 => Info {
                vt: ValType::F64,
                bits: 64,
                signed: true,
                float: true,
            },
        },
        _ => return None,
    })
}

fn mem(offset: u64, align: u32) -> MemArg {
    MemArg {
        offset,
        align,
        memory_index: 0,
    }
}

/// The instruction that loads a scalar of type `ty` from the address on the stack.
fn load(ty: &Ty) -> Result<I<'static>, String> {
    let n = info(ty).ok_or_else(|| format!("a value of type `{ty}`"))?;
    Ok(match (n.vt, n.bits, n.signed) {
        (ValType::F32, ..) => I::F32Load(mem(0, 2)),
        (ValType::F64, ..) => I::F64Load(mem(0, 3)),
        (ValType::I64, ..) => I::I64Load(mem(0, 3)),
        (_, 8, true) => I::I32Load8S(mem(0, 0)),
        (_, 8, false) => I::I32Load8U(mem(0, 0)),
        (_, 16, true) => I::I32Load16S(mem(0, 1)),
        (_, 16, false) => I::I32Load16U(mem(0, 1)),
        _ => I::I32Load(mem(0, 2)),
    })
}

/// The instruction that stores a scalar of type `ty`: the address, then the value, on the stack.
fn store(ty: &Ty) -> Result<I<'static>, String> {
    let n = info(ty).ok_or_else(|| format!("a value of type `{ty}`"))?;
    Ok(match (n.vt, n.bits) {
        (ValType::F32, _) => I::F32Store(mem(0, 2)),
        (ValType::F64, _) => I::F64Store(mem(0, 3)),
        (ValType::I64, _) => I::I64Store(mem(0, 3)),
        (_, 8) => I::I32Store8(mem(0, 0)),
        (_, 16) => I::I32Store16(mem(0, 1)),
        _ => I::I32Store(mem(0, 2)),
    })
}

/// A function's wasm signature, for parameters and a result of these kinds.
fn signature(params: &[Kind], ret: Kind) -> (Vec<ValType>, Vec<ValType>) {
    let mut ps = Vec::new();
    for p in params {
        match p {
            Kind::Nothing => {}
            Kind::Scalar(vt) => ps.push(*vt),
            Kind::Mem => ps.push(ValType::I32),
        }
    }
    let mut rs = vec![ValType::I32];
    match ret {
        Kind::Nothing => {}
        Kind::Scalar(vt) => rs.push(vt),
        // Where the caller wants it, last.
        Kind::Mem => ps.push(ValType::I32),
    }
    (ps, rs)
}

struct Compiler<'db> {
    db: &'db dyn Db,
    program: Program,
    items: &'db ItemTree,
    layouts: Layouts<'db>,
    types: Vec<(Vec<ValType>, Vec<ValType>)>,
    /// Each function declared: its index, its parameters' and result's types.
    fns: HashMap<u32, (u32, Vec<Ty>, Ty)>,
    queue: Vec<u32>,
    /// Each function's type and code, by its index past the imports.
    code: Vec<Option<(u32, Function)>>,
    sites: Vec<Site>,
    exports: Vec<(String, u32)>,
}

/// Compiles the program's tests `tests` (exported as `test<n>`), and `main` if given, into one
/// module.
pub fn compile<'db>(
    db: &'db dyn Db,
    program: Program,
    tests: &[u32],
    main: Option<u32>,
) -> Result<Compiled<'db>, String> {
    let items = moddil_sema::item_tree(db, program);
    let mut c = Compiler {
        db,
        program,
        items,
        layouts: Layouts::with_pointer(items, types::I32),
        types: Vec::new(),
        fns: HashMap::new(),
        queue: Vec::new(),
        code: Vec::new(),
        sites: Vec::new(),
        exports: Vec::new(),
    };
    for &t in tests {
        let index = c.reserve();
        let ty = c.type_of(&[], &[ValType::I32]);
        let f = c.body_of(Item::Test(t), &[], &Ty::Unit)?;
        c.code[(index - IMPORTS) as usize] = Some((ty, f));
        c.exports.push((format!("test{t}"), index));
    }
    let mut main_ret = None;
    if let Some(m) = main {
        let (index, params, ret) = c.declare_fn(m)?;
        if !params.is_empty() {
            return Err("a `main` with root capabilities".to_string());
        }
        let wrapper = c.main_wrapper(index, &ret)?;
        c.exports.push(("main".to_string(), wrapper));
        main_ret = Some(ret);
    }
    while let Some(f) = c.queue.pop() {
        let (index, params, ret) = c.fns[&f].clone();
        let ty = {
            let ks: Vec<Kind> = params.iter().map(|p| c.kind(p)).collect::<Result<_, _>>()?;
            let k = c.kind(&ret)?;
            let (ps, rs) = signature(&ks, k);
            c.type_of(&ps, &rs)
        };
        let body = c.body_of(Item::Fn(f), &params, &ret)?;
        c.code[(index - IMPORTS) as usize] = Some((ty, body));
    }
    let bytes = c.module()?;
    Ok(Compiled {
        bytes,
        sites: c.sites,
        items,
        main: main_ret,
    })
}

impl Compiler<'_> {
    fn kind(&mut self, ty: &Ty) -> Result<Kind, String> {
        let l = self.layouts.of(ty)?;
        kind_of(&l, ty)
    }

    fn type_of(&mut self, ps: &[ValType], rs: &[ValType]) -> u32 {
        let key = (ps.to_vec(), rs.to_vec());
        if let Some(k) = self.types.iter().position(|t| *t == key) {
            return k as u32;
        }
        self.types.push(key);
        (self.types.len() - 1) as u32
    }

    /// A function index, its code to come.
    fn reserve(&mut self) -> u32 {
        self.code.push(None);
        IMPORTS + (self.code.len() - 1) as u32
    }

    fn mir(&self, item: Item) -> Result<&Body, String> {
        let owner = Owner::new(self.db, self.program, item);
        moddil_mir::mir(self.db, owner)
            .as_ref()
            .ok_or_else(|| "a program with errors".to_string())
    }

    /// Function `f`, declared (and queued) the first time it is named.
    fn declare_fn(&mut self, f: u32) -> Result<(u32, Vec<Ty>, Ty), String> {
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
        if params.iter().chain([&ret]).any(Ty::has_param) {
            return Err("a generic function".to_string());
        }
        let index = self.reserve();
        self.fns.insert(f, (index, params.clone(), ret.clone()));
        self.queue.push(f);
        Ok((index, params, ret))
    }

    /// `main`'s entry: `(out: i32) -> i32`, writing `main`'s result at `out`.
    fn main_wrapper(&mut self, main: u32, ret: &Ty) -> Result<u32, String> {
        let k = self.kind(ret)?;
        let index = self.reserve();
        let ty = self.type_of(&[ValType::I32], &[ValType::I32]);
        let mut locals = vec![(1, ValType::I32)];
        if let Kind::Scalar(vt) = k {
            locals.push((1, vt));
        }
        let mut f = Function::new(locals);
        if k == Kind::Mem {
            f.instruction(&I::LocalGet(0));
        }
        f.instruction(&I::Call(main));
        if let Kind::Scalar(_) = k {
            f.instruction(&I::LocalSet(2));
            f.instruction(&I::LocalSet(1));
            f.instruction(&I::LocalGet(0));
            f.instruction(&I::LocalGet(2));
            f.instruction(&store(ret)?);
            f.instruction(&I::LocalGet(1));
        }
        f.instruction(&I::End);
        self.code[(index - IMPORTS) as usize] = Some((ty, f));
        Ok(index)
    }

    /// Compiles `item`'s body.
    fn body_of(&mut self, item: Item, params: &[Ty], ret: &Ty) -> Result<Function, String> {
        let body = self.mir(item)?.clone();
        let Src::File(file) = body.src else {
            return Err(format!("`{}`, of the prelude", body.name));
        };
        if body.track_caller {
            return Err(format!("`{}`, which is `#[track_caller]`", body.name));
        }
        let mut lw = Lower {
            c: self,
            body: &body,
            file,
            code: Vec::new(),
            locals: Vec::new(),
            nparams: 0,
            homes: Vec::new(),
            frame: 0,
            fp: 0,
            ret: ret.clone(),
            out: None,
            patches: Vec::new(),
        };
        lw.function(params)?;
        let Lower {
            mut code,
            locals,
            frame,
            fp,
            patches,
            ..
        } = lw;
        // The frame's size, now known, where each return gives it back.
        for at in patches {
            code[at] = I::I32Const(frame as i32);
        }
        let mut f = Function::new(locals.iter().map(|v| (1, *v)));
        // The frame, below the caller's.
        f.instruction(&I::GlobalGet(SP));
        f.instruction(&I::I32Const(frame as i32));
        f.instruction(&I::I32Sub);
        f.instruction(&I::LocalTee(fp));
        f.instruction(&I::GlobalSet(SP));
        for i in &code {
            f.instruction(i);
        }
        f.instruction(&I::Unreachable);
        f.instruction(&I::End);
        Ok(f)
    }

    fn module(&mut self) -> Result<Vec<u8>, String> {
        let mut m = Module::new();
        let mut ts = TypeSection::new();
        // The host's functions' types come first: trap, observe, fmod, fmodf.
        let trap = self.type_of(&[ValType::I32], &[]);
        let observe = self.type_of(&[ValType::I32, ValType::I32], &[]);
        let fmod = self.type_of(&[ValType::F64, ValType::F64], &[ValType::F64]);
        let fmodf = self.type_of(&[ValType::F32, ValType::F32], &[ValType::F32]);
        for (ps, rs) in &self.types {
            ts.ty().function(ps.iter().copied(), rs.iter().copied());
        }
        m.section(&ts);
        let mut imports = ImportSection::new();
        imports.import("env", "trap", EntityType::Function(trap));
        imports.import("env", "observe", EntityType::Function(observe));
        imports.import("env", "fmod", EntityType::Function(fmod));
        imports.import("env", "fmodf", EntityType::Function(fmodf));
        m.section(&imports);
        let mut fs = FunctionSection::new();
        let mut code = CodeSection::new();
        for c in &self.code {
            let (ty, f) = c.as_ref().ok_or("a function without its code")?;
            fs.function(*ty);
            code.function(f);
        }
        m.section(&fs);
        let mut ms = MemorySection::new();
        ms.memory(MemoryType {
            minimum: u64::from(STACK_TOP >> 16) + 1,
            maximum: None,
            memory64: false,
            shared: false,
            page_size_log2: None,
        });
        m.section(&ms);
        let mut gs = GlobalSection::new();
        for init in [STACK_TOP as i32, 0] {
            gs.global(
                GlobalType {
                    val_type: ValType::I32,
                    mutable: true,
                    shared: false,
                },
                &ConstExpr::i32_const(init),
            );
        }
        m.section(&gs);
        let mut es = ExportSection::new();
        es.export("memory", ExportKind::Memory, 0);
        es.export("sp", ExportKind::Global, SP);
        es.export("depth", ExportKind::Global, DEPTH);
        for (name, index) in &self.exports {
            es.export(name, ExportKind::Func, *index);
        }
        m.section(&es);
        m.section(&code);
        Ok(m.finish())
    }
}

/// How a value of layout `l` and type `ty` is held.
fn kind_of(l: &Layout, ty: &Ty) -> Result<Kind, String> {
    Ok(match &l.shape {
        Shape::Empty => Kind::Nothing,
        _ if l.size == 0 => Kind::Nothing,
        Shape::Scalar(_) => Kind::Scalar(
            info(ty)
                .ok_or_else(|| format!("a value of type `{ty}`"))?
                .vt,
        ),
        _ => Kind::Mem,
    })
}

/// Where a local is.
#[derive(Clone, Copy)]
enum Home {
    Nothing,
    /// A wasm local.
    Local(u32),
    /// In the frame, at this offset.
    Frame(u32),
    /// In memory at the address this wasm local holds.
    Ptr(u32),
}

/// A place, found: a wasm local, or in memory at the address the code has just pushed.
enum PlaceR {
    Local(u32, Ty),
    Mem(Ty, Option<u32>),
    Nothing(Ty),
}

impl PlaceR {
    fn ty(&self) -> &Ty {
        match self {
            PlaceR::Local(_, t) | PlaceR::Mem(t, _) | PlaceR::Nothing(t) => t,
        }
    }
}

struct Lower<'a, 'db> {
    c: &'a mut Compiler<'db>,
    body: &'a Body,
    file: u32,
    code: Vec<I<'static>>,
    /// The wasm locals past the parameters.
    locals: Vec<ValType>,
    nparams: u32,
    homes: Vec<Home>,
    /// The frame's size.
    frame: u32,
    /// The wasm local holding the frame's address.
    fp: u32,
    ret: Ty,
    /// For a result in memory: the wasm local holding where.
    out: Option<u32>,
    /// Where the code adds the frame's size, which is known only at the end.
    patches: Vec<usize>,
}

impl Lower<'_, '_> {
    fn emit(&mut self, i: I<'static>) {
        self.code.push(i);
    }

    fn local(&mut self, vt: ValType) -> u32 {
        self.locals.push(vt);
        self.nparams + (self.locals.len() - 1) as u32
    }

    fn layout(&mut self, ty: &Ty) -> Result<Arc<Layout>, String> {
        self.c.layouts.of(ty)
    }

    fn kind(&mut self, ty: &Ty) -> Result<Kind, String> {
        self.c.kind(ty)
    }

    /// Room in the frame for a value of layout `l`: its offset.
    fn slot(&mut self, l: &Layout) -> u32 {
        let align = l.align.max(1);
        let at = self.frame.div_ceil(align) * align;
        self.frame = (at + l.size.max(1)).div_ceil(16) * 16;
        at
    }

    /// Pushes the address `off` into the frame.
    fn frame_addr(&mut self, off: u32) {
        self.emit(I::LocalGet(self.fp));
        if off > 0 {
            self.emit(I::I32Const(off as i32));
            self.emit(I::I32Add);
        }
    }

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

    fn function(&mut self, params: &[Ty]) -> Result<(), String> {
        let _ = params;
        let borrowed = self.borrowed();
        // The parameters: each argument by its kind, then where an aggregate result goes.
        let mut wasm_params = Vec::new();
        for l in 1..=self.body.arg_count {
            let ty = self.body.locals[l].ty.clone();
            let k = self.kind(&ty)?;
            wasm_params.push((l, k));
        }
        let ret_kind = self.kind(&self.ret.clone())?;
        let mut n = 0u32;
        let mut param_of = HashMap::new();
        for (l, k) in &wasm_params {
            if *k != Kind::Nothing {
                param_of.insert(*l, n);
                n += 1;
            }
        }
        if ret_kind == Kind::Mem {
            self.out = Some(n);
            n += 1;
        }
        self.nparams = n;
        self.fp = self.local(ValType::I32);
        let mut copy_in = Vec::new();
        for (i, l) in self.body.locals.clone().iter().enumerate() {
            let layout = self.layout(&l.ty)?;
            let k = kind_of(&layout, &l.ty)?;
            let arg = (1..=self.body.arg_count).contains(&i);
            let home = match k {
                Kind::Nothing => Home::Nothing,
                Kind::Scalar(vt) if !borrowed.contains(&(i as u32)) => match param_of.get(&i) {
                    Some(&p) => Home::Local(p),
                    None => Home::Local(self.local(vt)),
                },
                Kind::Scalar(_) => {
                    let off = self.slot(&layout);
                    if let Some(&p) = param_of.get(&i) {
                        copy_in.push((off, p, l.ty.clone()));
                    }
                    Home::Frame(off)
                }
                Kind::Mem if arg => Home::Ptr(param_of[&i]),
                Kind::Mem if i == 0 => Home::Ptr(self.out.expect("the result's place")),
                Kind::Mem => Home::Frame(self.slot(&layout)),
            };
            self.homes.push(home);
        }
        // A scalar argument something borrows goes into the frame.
        for (off, p, ty) in copy_in {
            self.frame_addr(off);
            self.emit(I::LocalGet(p));
            self.emit(store(&ty)?);
        }
        let mut succs = Vec::new();
        for bb in &self.body.blocks {
            succs.push(match &bb.terminator {
                Terminator::Goto(t) => vec![t.index()],
                Terminator::SwitchBool { then, else_, .. } => vec![then.index(), else_.index()],
                Terminator::Call { target, .. } | Terminator::Drop { target, .. } => {
                    vec![target.index()]
                }
                _ => Vec::new(),
            });
        }
        cfg::structure(&succs, self)
    }

    // ----- places --------------------------------------------------------------------------

    /// Finds place `p`: a local, or memory (its address pushed).
    fn place(&mut self, p: &Place) -> Result<PlaceR, String> {
        let ty = self.body.locals[p.local.index()].ty.clone();
        let mut r = match self.homes[p.local.index()] {
            Home::Nothing => PlaceR::Nothing(ty),
            Home::Local(l) => PlaceR::Local(l, ty),
            Home::Frame(off) => {
                self.frame_addr(off);
                PlaceR::Mem(ty, None)
            }
            Home::Ptr(l) => {
                self.emit(I::LocalGet(l));
                PlaceR::Mem(ty, None)
            }
        };
        for proj in &p.projection {
            r = match (proj, r) {
                (Projection::Deref, r) => {
                    let Ty::Ref(_, inner) = r.ty().clone() else {
                        return Err(format!("dereferencing a `{}`", r.ty()));
                    };
                    match r {
                        PlaceR::Local(l, _) => self.emit(I::LocalGet(l)),
                        PlaceR::Mem(..) => self.emit(I::I32Load(mem(0, 2))),
                        PlaceR::Nothing(_) => return Err("a borrow without bytes".to_string()),
                    }
                    PlaceR::Mem((*inner).clone(), None)
                }
                (Projection::Downcast(v), PlaceR::Mem(t, _)) => PlaceR::Mem(t, Some(*v)),
                (Projection::Downcast(_), r) => r,
                (Projection::Field(i), PlaceR::Mem(t, variant)) => {
                    let l = self.layout(&t)?;
                    let (off, fty) = match (&l.shape, variant) {
                        (Shape::Fields(fs), _) => fs[*i as usize].clone(),
                        (Shape::Enum { variants, .. }, Some(v)) => {
                            variants[v as usize][*i as usize].clone()
                        }
                        (Shape::Enum { variants, .. }, None) if variants.len() == 1 => {
                            variants[0][*i as usize].clone()
                        }
                        _ => return Err(format!("a field of `{t}`")),
                    };
                    if off > 0 {
                        self.emit(I::I32Const(off as i32));
                        self.emit(I::I32Add);
                    }
                    PlaceR::Mem(fty, None)
                }
                (Projection::Field(_), PlaceR::Nothing(t)) => {
                    // A field of a value without bytes has none either.
                    let _ = t;
                    return Err("a field of a value without bytes".to_string());
                }
                (Projection::Index(i), PlaceR::Mem(t, _)) => {
                    let l = self.layout(&t)?;
                    let Shape::Array { elem, stride, .. } = &l.shape else {
                        return Err(format!("an element of `{t}`"));
                    };
                    let (elem, stride) = (elem.clone(), *stride);
                    self.read_local(*i)?;
                    self.emit(I::I32WrapI64);
                    self.emit(I::I32Const(stride as i32));
                    self.emit(I::I32Mul);
                    self.emit(I::I32Add);
                    PlaceR::Mem(elem, None)
                }
                (Projection::Subslice(..), _) => return Err("a part of an array".to_string()),
                (proj, r) => return Err(format!("`{proj:?}` of a `{}`", r.ty())),
            };
        }
        Ok(r)
    }

    /// Pushes the value of local `l` (a scalar).
    fn read_local(&mut self, l: Local) -> Result<(), String> {
        let r = self.place(&Place::local(l))?;
        self.read(r)
    }

    /// Pushes the value at a place just found: a scalar's value, or an aggregate's address.
    fn read(&mut self, r: PlaceR) -> Result<(), String> {
        match r {
            PlaceR::Local(l, _) => self.emit(I::LocalGet(l)),
            PlaceR::Mem(ty, _) => match self.kind(&ty)? {
                Kind::Scalar(_) => self.emit(load(&ty)?),
                // The address stays.
                Kind::Mem => {}
                Kind::Nothing => self.emit(I::Drop),
            },
            PlaceR::Nothing(_) => {}
        }
        Ok(())
    }

    /// The value of operand `o`: a scalar pushed, an aggregate's address pushed, or nothing.
    fn operand(&mut self, o: &Operand) -> Result<Kind, String> {
        match o {
            Operand::Copy(p) | Operand::Move(p) => {
                let r = self.place(p)?;
                let k = self.kind(&r.ty().clone())?;
                self.read(r)?;
                Ok(k)
            }
            Operand::Const(c) => {
                match *c {
                    Const::Int(v) => self.emit(I::I64Const(v)),
                    Const::Bool(v) => self.emit(I::I32Const(i32::from(v))),
                    Const::Char(c) => self.emit(I::I32Const(u32::from(c) as i32)),
                    Const::Unit => return Ok(Kind::Nothing),
                    Const::Num(n, bits) => self.num_const(n, bits)?,
                    Const::Str(_) => return Err("text".to_string()),
                }
                Ok(Kind::Scalar(ValType::I32))
            }
        }
    }

    fn num_const(&mut self, n: Num, bits: u128) -> Result<(), String> {
        let i = info(&Ty::Num(n)).ok_or_else(|| format!("a value of type `{}`", Ty::Num(n)))?;
        match i.vt {
            ValType::F32 => self.emit(I::F32Const(f32::from_bits(bits as u32).into())),
            ValType::F64 => self.emit(I::F64Const(f64::from_bits(bits as u64).into())),
            ValType::I64 => self.emit(I::I64Const(bits as u64 as i64)),
            _ => {
                // Canonical: sign-extended for a signed type, zero-extended else.
                let v = if i.signed {
                    let shift = 128 - i.bits;
                    (((bits as i128) << shift) >> shift) as i32
                } else {
                    bits as u32 as i32
                };
                self.emit(I::I32Const(v));
            }
        }
        Ok(())
    }

    fn operand_ty(&self, o: &Operand) -> Result<Ty, String> {
        Ok(match o {
            Operand::Copy(p) | Operand::Move(p) => self.place_ty(p)?,
            Operand::Const(Const::Int(_)) => Ty::Int,
            Operand::Const(Const::Num(n, _)) => Ty::Num(*n),
            Operand::Const(Const::Bool(_)) => Ty::Bool,
            Operand::Const(Const::Unit) => Ty::Unit,
            Operand::Const(Const::Char(_)) => Ty::Char,
            Operand::Const(Const::Str(_)) => return Err("text".to_string()),
        })
    }

    /// The type of place `p`, without code.
    fn place_ty(&self, p: &Place) -> Result<Ty, String> {
        let mut t = self.body.locals[p.local.index()].ty.clone();
        let mut variant = None;
        for proj in &p.projection {
            t = match proj {
                Projection::Deref => match t {
                    Ty::Ref(_, inner) => (*inner).clone(),
                    t => return Err(format!("dereferencing a `{t}`")),
                },
                Projection::Downcast(v) => {
                    variant = Some(*v);
                    continue;
                }
                Projection::Field(i) => {
                    let ts = match &t {
                        Ty::Tuple(ts) => ts.to_vec(),
                        Ty::Adt(..) => self.c.items.field_types(&t, variant.unwrap_or(0)),
                        t => return Err(format!("a field of `{t}`")),
                    };
                    ts.get(*i as usize).cloned().ok_or("a field out of range")?
                }
                Projection::Index(_) => match t {
                    Ty::Array(e, _) => (*e).clone(),
                    t => return Err(format!("an element of `{t}`")),
                },
                Projection::Subslice(..) => return Err("a part of an array".to_string()),
            };
            variant = None;
        }
        Ok(t)
    }

    // ----- sites and traps -----------------------------------------------------------------

    fn site(&mut self, s: Site) -> i32 {
        self.c.sites.push(s);
        (self.c.sites.len() - 1) as i32
    }

    /// Restores the caller's stack, then returns `status` with a zero result.
    fn return_status(&mut self, status: i32) -> Result<(), String> {
        self.epilogue();
        self.emit(I::I32Const(status));
        if let Kind::Scalar(vt) = self.kind(&self.ret.clone())? {
            self.zero(vt);
        }
        self.emit(I::Return);
        Ok(())
    }

    fn zero(&mut self, vt: ValType) {
        self.emit(match vt {
            ValType::I64 => I::I64Const(0),
            ValType::F32 => I::F32Const(0.0f32.into()),
            ValType::F64 => I::F64Const(0.0f64.into()),
            _ => I::I32Const(0),
        });
    }

    /// Restores the caller's stack pointer (the frame is given back).
    fn epilogue(&mut self) {
        // The frame's size is known only at the end; the stack pointer is the frame's address
        // plus it, and it is the same as what it was on entry, which `fp` was made from.
        self.emit(I::LocalGet(self.fp));
        self.patches.push(self.code.len());
        self.emit(I::I32Const(0));
        self.emit(I::I32Add);
        self.emit(I::GlobalSet(SP));
    }

    fn trap(&mut self, class: TrapClass, range: TextRange) -> Result<(), String> {
        let site = self.site(Site::Trap(class, self.file, range));
        self.emit(I::I32Const(site));
        self.emit(I::Call(TRAP));
        self.return_status(1)
    }

    // ----- statements ----------------------------------------------------------------------

    fn statement(&mut self, s: &Statement) -> Result<(), String> {
        match s {
            Statement::Assign(place, r) => self.assign(place, r),
            Statement::Assert {
                cond,
                expected,
                trap,
                range,
                ..
            } => {
                self.operand(cond)?;
                if *expected {
                    self.emit(I::I32Eqz);
                }
                self.emit(I::If(BlockType::Empty));
                self.trap(*trap, *range)?;
                self.emit(I::End);
                Ok(())
            }
            Statement::Read(_) | Statement::StorageDead(_) => Ok(()),
            Statement::Observe { range, value } => {
                let ty = self.operand_ty(value)?;
                let site = self.site(Site::Observe(self.file, *range, ty.clone()));
                self.emit(I::I32Const(site));
                match self.kind(&ty)? {
                    Kind::Nothing => {
                        self.operand(value)?;
                        self.emit(I::I32Const(0));
                    }
                    Kind::Mem => {
                        self.operand(value)?;
                    }
                    Kind::Scalar(_) => {
                        // Into the frame, where the host can read it.
                        let l = self.layout(&ty)?;
                        let off = self.slot(&l);
                        self.frame_addr(off);
                        self.operand(value)?;
                        self.emit(store(&ty)?);
                        self.frame_addr(off);
                    }
                }
                self.emit(I::Call(OBSERVE));
                Ok(())
            }
        }
    }

    /// Stores the value `value` pushes into `place`.
    fn assign(&mut self, place: &Place, r: &Rvalue) -> Result<(), String> {
        let dest = self.place(place)?;
        let ty = dest.ty().clone();
        match dest {
            PlaceR::Nothing(_) => {
                self.rvalue(r, &ty)?;
                Ok(())
            }
            PlaceR::Local(l, _) => {
                self.rvalue(r, &ty)?;
                self.emit(I::LocalSet(l));
                Ok(())
            }
            PlaceR::Mem(..) => match self.kind(&ty)? {
                Kind::Scalar(_) => {
                    self.rvalue(r, &ty)?;
                    self.emit(store(&ty)?);
                    Ok(())
                }
                Kind::Nothing => {
                    self.emit(I::Drop);
                    self.rvalue(r, &ty)?;
                    Ok(())
                }
                Kind::Mem => {
                    // Its address is on the stack: kept in a local while the value is built.
                    let at = self.local(ValType::I32);
                    self.emit(I::LocalSet(at));
                    self.aggregate_into(at, r, &ty)
                }
            },
        }
    }

    /// Builds the aggregate `r` (of type `ty`) at the address local `at` holds.
    fn aggregate_into(&mut self, at: u32, r: &Rvalue, ty: &Ty) -> Result<(), String> {
        let l = self.layout(ty)?;
        match r {
            Rvalue::Use(o) => {
                self.emit(I::LocalGet(at));
                self.operand(o)?;
                self.emit(I::I32Const(l.size as i32));
                self.emit(I::MemoryCopy {
                    src_mem: 0,
                    dst_mem: 0,
                });
                Ok(())
            }
            Rvalue::Aggregate(kind, ops) => {
                if matches!(kind, AggregateKind::Closure(_)) {
                    return Err("a closure".to_string());
                }
                let (fields, variant): (Vec<(u32, Ty)>, Option<u32>) = match (&l.shape, kind) {
                    (Shape::Fields(fs), _) => (fs.clone(), None),
                    (Shape::Array { elem, stride, len }, AggregateKind::Array) => (
                        (0..*len).map(|k| (k * stride, elem.clone())).collect(),
                        None,
                    ),
                    (Shape::Enum { variants, .. }, AggregateKind::Adt { variant, .. }) => {
                        (variants[*variant as usize].clone(), Some(*variant))
                    }
                    _ => return Err(format!("building a `{ty}`")),
                };
                for ((off, fty), o) in fields.iter().zip(ops) {
                    self.store_at(at, *off, fty, o)?;
                }
                if let (Some(v), Shape::Enum { tag, .. }) = (variant, &l.shape) {
                    self.set_variant(at, tag, v)?;
                }
                Ok(())
            }
            Rvalue::Repeat(o, _) => {
                let Shape::Array { elem, stride, len } = &l.shape else {
                    return Err(format!("`[x; n]` as a `{ty}`"));
                };
                let (elem, stride, len) = (elem.clone(), *stride, *len);
                for k in 0..len {
                    self.store_at(at, k * stride, &elem, o)?;
                }
                Ok(())
            }
            r => Err(format!("this operation (`{r}`) into a `{ty}`")),
        }
    }

    /// Stores operand `o`, of type `ty`, at offset `off` from the address local `at` holds.
    fn store_at(&mut self, at: u32, off: u32, ty: &Ty, o: &Operand) -> Result<(), String> {
        match self.kind(ty)? {
            Kind::Nothing => Ok(()),
            Kind::Scalar(_) => {
                self.emit(I::LocalGet(at));
                if off > 0 {
                    self.emit(I::I32Const(off as i32));
                    self.emit(I::I32Add);
                }
                self.operand(o)?;
                self.emit(store(ty)?);
                Ok(())
            }
            Kind::Mem => {
                let l = self.layout(ty)?;
                self.emit(I::LocalGet(at));
                if off > 0 {
                    self.emit(I::I32Const(off as i32));
                    self.emit(I::I32Add);
                }
                self.operand(o)?;
                self.emit(I::I32Const(l.size as i32));
                self.emit(I::MemoryCopy {
                    src_mem: 0,
                    dst_mem: 0,
                });
                Ok(())
            }
        }
    }

    /// Marks the enum at the address local `at` holds as variant `v`.
    fn set_variant(&mut self, at: u32, tag: &Tag, v: u32) -> Result<(), String> {
        let (offset, ty, value) = match tag {
            Tag::None => return Ok(()),
            Tag::Direct { offset, ty } => (*offset, *ty, u128::from(v)),
            Tag::Niche { dataful, niche } => {
                if v == *dataful {
                    return Ok(());
                }
                let rank = if v < *dataful { v } else { v - 1 };
                (niche.offset, niche.ty, niche.first + u128::from(rank))
            }
        };
        self.emit(I::LocalGet(at));
        if offset > 0 {
            self.emit(I::I32Const(offset as i32));
            self.emit(I::I32Add);
        }
        self.raw_store(ty, value);
        Ok(())
    }

    /// Stores `value` as a raw scalar of type `t` (a tag), the address on the stack.
    fn raw_store(&mut self, t: ir::Type, value: u128) {
        match t {
            types::I8 => {
                self.emit(I::I32Const(value as i32));
                self.emit(I::I32Store8(mem(0, 0)));
            }
            types::I16 => {
                self.emit(I::I32Const(value as i32));
                self.emit(I::I32Store16(mem(0, 1)));
            }
            types::I64 => {
                self.emit(I::I64Const(value as u64 as i64));
                self.emit(I::I64Store(mem(0, 3)));
            }
            _ => {
                self.emit(I::I32Const(value as u32 as i32));
                self.emit(I::I32Store(mem(0, 2)));
            }
        }
    }

    /// Loads a raw scalar of type `t` (a tag) as an `i64`, the address on the stack.
    fn raw_load(&mut self, t: ir::Type) {
        match t {
            types::I8 => {
                self.emit(I::I32Load8U(mem(0, 0)));
                self.emit(I::I64ExtendI32U);
            }
            types::I16 => {
                self.emit(I::I32Load16U(mem(0, 1)));
                self.emit(I::I64ExtendI32U);
            }
            types::I64 => self.emit(I::I64Load(mem(0, 3))),
            _ => {
                self.emit(I::I32Load(mem(0, 2)));
                self.emit(I::I64ExtendI32U);
            }
        }
    }

    /// Pushes the scalar value `r` gives, of type `dest` (nothing for one without bytes).
    fn rvalue(&mut self, r: &Rvalue, dest: &Ty) -> Result<(), String> {
        match r {
            Rvalue::Use(o) => {
                self.operand(o)?;
            }
            Rvalue::Ref(_, p) => match self.place(p)? {
                PlaceR::Mem(..) => {}
                PlaceR::Nothing(_) => self.emit(I::I32Const(8)),
                PlaceR::Local(..) => {
                    return Err("a borrow of a value kept in a register".to_string());
                }
            },
            Rvalue::Discriminant(p) => self.discriminant(p)?,
            Rvalue::Len(p) => {
                let ty = self.place_ty(p)?;
                let l = self.layout(&ty)?;
                let Shape::Array { len, .. } = l.shape else {
                    return Err(format!("the length of a `{ty}`"));
                };
                self.emit(I::I64Const(i64::from(len)));
            }
            Rvalue::InBounds(i, p) => {
                let ty = self.place_ty(p)?;
                let l = self.layout(&ty)?;
                let Shape::Array { len, .. } = l.shape else {
                    return Err(format!("indexing a `{ty}`"));
                };
                self.operand(i)?;
                self.emit(I::I64Const(i64::from(len)));
                // Unsigned: a negative index is a large one.
                self.emit(I::I64LtU);
            }
            Rvalue::Unary(op, a) => {
                let ty = self.operand_ty(a)?;
                let n = info(&ty).ok_or_else(|| format!("`{op:?}` on a `{ty}`"))?;
                match (op, n.vt) {
                    (UnOp::Neg, ValType::F32) => {
                        self.operand(a)?;
                        self.emit(I::F32Neg);
                    }
                    (UnOp::Neg, ValType::F64) => {
                        self.operand(a)?;
                        self.emit(I::F64Neg);
                    }
                    (UnOp::Neg, ValType::I64) => {
                        self.emit(I::I64Const(0));
                        self.operand(a)?;
                        self.emit(I::I64Sub);
                    }
                    (UnOp::Neg, _) => {
                        self.emit(I::I32Const(0));
                        self.operand(a)?;
                        self.emit(I::I32Sub);
                        self.canon(n);
                    }
                    (UnOp::Not, _) if ty == Ty::Bool => {
                        self.operand(a)?;
                        self.emit(I::I32Const(1));
                        self.emit(I::I32Xor);
                    }
                    (UnOp::Not, ValType::I64) => {
                        self.operand(a)?;
                        self.emit(I::I64Const(-1));
                        self.emit(I::I64Xor);
                    }
                    (UnOp::Not, _) => {
                        self.operand(a)?;
                        self.emit(I::I32Const(-1));
                        self.emit(I::I32Xor);
                        self.canon(n);
                    }
                }
            }
            Rvalue::Binary(op, a, b) => {
                let ty = self.operand_ty(a)?;
                if ty == Ty::Unit {
                    let eq = matches!(op, BinOp::Eq | BinOp::Le | BinOp::Ge);
                    self.emit(I::I32Const(i32::from(eq)));
                    return Ok(());
                }
                let n = info(&ty).ok_or_else(|| format!("`{op:?}` on a `{ty}`"))?;
                self.operand(a)?;
                self.operand_as(b, n)?;
                self.binary(*op, n)?;
            }
            Rvalue::Overflows(op, a, b) => {
                let ty = self.operand_ty(a)?;
                let n = info(&ty).ok_or_else(|| format!("`{op:?}` on a `{ty}`"))?;
                self.overflows(*op, n, a, b)?;
            }
            Rvalue::NegOverflows(a) => {
                let ty = self.operand_ty(a)?;
                let n = info(&ty).ok_or_else(|| format!("negating a `{ty}`"))?;
                if n.signed && !n.float {
                    self.operand(a)?;
                    self.min(n);
                    self.eq(n.vt);
                } else {
                    self.emit(I::I32Const(0));
                }
            }
            Rvalue::Widen(a, to) => {
                let from = self.operand_ty(a)?;
                let (f, t) = (info(&from).ok_or("a number")?, info(to).ok_or("a number")?);
                self.operand(a)?;
                match (f.float, t.float, f.vt, t.vt) {
                    (true, true, ValType::F32, ValType::F64) => self.emit(I::F64PromoteF32),
                    (false, true, fv, tv) => self.emit(match (fv, tv, f.signed) {
                        (ValType::I32, ValType::F32, true) => I::F32ConvertI32S,
                        (ValType::I32, ValType::F32, false) => I::F32ConvertI32U,
                        (ValType::I32, _, true) => I::F64ConvertI32S,
                        (ValType::I32, _, false) => I::F64ConvertI32U,
                        (_, ValType::F32, true) => I::F32ConvertI64S,
                        (_, ValType::F32, false) => I::F32ConvertI64U,
                        (_, _, true) => I::F64ConvertI64S,
                        (_, _, false) => I::F64ConvertI64U,
                    }),
                    (false, false, ValType::I32, ValType::I64) => self.emit(if f.signed {
                        I::I64ExtendI32S
                    } else {
                        I::I64ExtendI32U
                    }),
                    // A float as itself, or an integer as a wider one of the same width.
                    (true, true, ..) | (false, false, ..) => {}
                    (true, false, ..) => return Err("a float as an integer".to_string()),
                }
            }
            Rvalue::Aggregate(..) | Rvalue::Repeat(..) if self.c.kind(dest)? == Kind::Nothing => {
                if let Rvalue::Aggregate(_, ops) = r {
                    for o in ops {
                        if self.operand(o)? != Kind::Nothing {
                            self.emit(I::Drop);
                        }
                    }
                }
            }
            r => return Err(format!("this operation (`{r}`)")),
        }
        Ok(())
    }

    /// Pushes operand `o` as a value of type `n` (a shift amount of another integer type is
    /// converted).
    fn operand_as(&mut self, o: &Operand, n: Info) -> Result<(), String> {
        let ty = self.operand_ty(o)?;
        let m = info(&ty).ok_or_else(|| format!("an operand of type `{ty}`"))?;
        self.operand(o)?;
        match (m.vt, n.vt) {
            (ValType::I64, ValType::I32) => self.emit(I::I32WrapI64),
            (ValType::I32, ValType::I64) => self.emit(I::I64ExtendI32U),
            _ => {}
        }
        Ok(())
    }

    /// Keeps a narrow integer result canonical: sign-extended or zero-extended to 32 bits.
    fn canon(&mut self, n: Info) {
        if n.vt != ValType::I32 || n.bits >= 32 || n.float {
            return;
        }
        match (n.bits, n.signed) {
            (8, true) => self.emit(I::I32Extend8S),
            (16, true) => self.emit(I::I32Extend16S),
            (b, _) => {
                self.emit(I::I32Const(((1u64 << b) - 1) as i32));
                self.emit(I::I32And);
            }
        }
    }

    /// Pushes the least value of signed integer type `n`.
    fn min(&mut self, n: Info) {
        if n.vt == ValType::I64 {
            self.emit(I::I64Const(i64::MIN));
        } else {
            self.emit(I::I32Const(((-1i64) << (n.bits - 1)) as i32));
        }
    }

    fn eq(&mut self, vt: ValType) {
        self.emit(if vt == ValType::I64 {
            I::I64Eq
        } else {
            I::I32Eq
        });
    }

    fn binary(&mut self, op: BinOp, n: Info) -> Result<(), String> {
        use BinOp as B;
        let s = n.signed;
        let i = match (n.vt, op) {
            (ValType::F32, B::Add) => I::F32Add,
            (ValType::F32, B::Sub) => I::F32Sub,
            (ValType::F32, B::Mul) => I::F32Mul,
            (ValType::F32, B::Div) => I::F32Div,
            (ValType::F32, B::Rem) => I::Call(FMODF),
            (ValType::F32, B::Eq) => I::F32Eq,
            (ValType::F32, B::Ne) => I::F32Ne,
            (ValType::F32, B::Lt) => I::F32Lt,
            (ValType::F32, B::Le) => I::F32Le,
            (ValType::F32, B::Gt) => I::F32Gt,
            (ValType::F32, B::Ge) => I::F32Ge,
            (ValType::F64, B::Add) => I::F64Add,
            (ValType::F64, B::Sub) => I::F64Sub,
            (ValType::F64, B::Mul) => I::F64Mul,
            (ValType::F64, B::Div) => I::F64Div,
            (ValType::F64, B::Rem) => I::Call(FMOD),
            (ValType::F64, B::Eq) => I::F64Eq,
            (ValType::F64, B::Ne) => I::F64Ne,
            (ValType::F64, B::Lt) => I::F64Lt,
            (ValType::F64, B::Le) => I::F64Le,
            (ValType::F64, B::Gt) => I::F64Gt,
            (ValType::F64, B::Ge) => I::F64Ge,
            (ValType::F32 | ValType::F64, op) => return Err(format!("`{op:?}` on floats")),
            (ValType::I64, B::Add) => I::I64Add,
            (ValType::I64, B::Sub) => I::I64Sub,
            (ValType::I64, B::Mul) => I::I64Mul,
            (ValType::I64, B::Div) => {
                if s {
                    I::I64DivS
                } else {
                    I::I64DivU
                }
            }
            (ValType::I64, B::Rem) => {
                if s {
                    I::I64RemS
                } else {
                    I::I64RemU
                }
            }
            (ValType::I64, B::BitAnd) => I::I64And,
            (ValType::I64, B::BitOr) => I::I64Or,
            (ValType::I64, B::BitXor) => I::I64Xor,
            (ValType::I64, B::Shl) => I::I64Shl,
            (ValType::I64, B::Shr) => {
                if s {
                    I::I64ShrS
                } else {
                    I::I64ShrU
                }
            }
            (ValType::I64, B::Eq) => I::I64Eq,
            (ValType::I64, B::Ne) => I::I64Ne,
            (ValType::I64, B::Lt) => {
                if s {
                    I::I64LtS
                } else {
                    I::I64LtU
                }
            }
            (ValType::I64, B::Le) => {
                if s {
                    I::I64LeS
                } else {
                    I::I64LeU
                }
            }
            (ValType::I64, B::Gt) => {
                if s {
                    I::I64GtS
                } else {
                    I::I64GtU
                }
            }
            (ValType::I64, B::Ge) => {
                if s {
                    I::I64GeS
                } else {
                    I::I64GeU
                }
            }
            (_, B::Add) => I::I32Add,
            (_, B::Sub) => I::I32Sub,
            (_, B::Mul) => I::I32Mul,
            (_, B::Div) => {
                if s {
                    I::I32DivS
                } else {
                    I::I32DivU
                }
            }
            (_, B::Rem) => {
                if s {
                    I::I32RemS
                } else {
                    I::I32RemU
                }
            }
            (_, B::BitAnd) => I::I32And,
            (_, B::BitOr) => I::I32Or,
            (_, B::BitXor) => I::I32Xor,
            (_, B::Shl) => I::I32Shl,
            (_, B::Shr) => {
                if s {
                    I::I32ShrS
                } else {
                    I::I32ShrU
                }
            }
            (_, B::Eq) => I::I32Eq,
            (_, B::Ne) => I::I32Ne,
            (_, B::Lt) => {
                if s {
                    I::I32LtS
                } else {
                    I::I32LtU
                }
            }
            (_, B::Le) => {
                if s {
                    I::I32LeS
                } else {
                    I::I32LeU
                }
            }
            (_, B::Gt) => {
                if s {
                    I::I32GtS
                } else {
                    I::I32GtU
                }
            }
            (_, B::Ge) => {
                if s {
                    I::I32GeS
                } else {
                    I::I32GeU
                }
            }
        };
        self.emit(i);
        if matches!(op, B::Add | B::Sub | B::Mul | B::Shl) {
            self.canon(n);
        }
        Ok(())
    }

    /// Pushes whether `a op b` overflows (or shifts by an amount out of range), as the
    /// interpreters' `overflows`.
    fn overflows(&mut self, op: BinOp, n: Info, a: &Operand, b: &Operand) -> Result<(), String> {
        use BinOp as B;
        if n.float {
            self.emit(I::I32Const(0));
            return Ok(());
        }
        // The operands, each once, into locals.
        let x = self.local(n.vt);
        let y = self.local(n.vt);
        self.operand(a)?;
        self.emit(I::LocalSet(x));
        self.operand_as(b, n)?;
        self.emit(I::LocalSet(y));
        let wide = n.vt == ValType::I64;
        let s = n.signed;
        match op {
            B::Add | B::Sub | B::Mul if !wide => {
                // In 64 bits, then in range: the narrower types cannot overflow `i64`, but for
                // the unsigned 32-bit product, compared unsigned.
                let ext = if s {
                    I::I64ExtendI32S
                } else {
                    I::I64ExtendI32U
                };
                self.emit(I::LocalGet(x));
                self.emit(ext.clone());
                self.emit(I::LocalGet(y));
                self.emit(ext);
                self.emit(match op {
                    B::Add => I::I64Add,
                    B::Sub => I::I64Sub,
                    _ => I::I64Mul,
                });
                let r = self.local(ValType::I64);
                self.emit(I::LocalTee(r));
                if s {
                    let lo = (-1i64) << (n.bits - 1);
                    let hi = -lo - 1;
                    self.emit(I::I64Const(lo));
                    self.emit(I::I64LtS);
                    self.emit(I::LocalGet(r));
                    self.emit(I::I64Const(hi));
                    self.emit(I::I64GtS);
                    self.emit(I::I32Or);
                } else {
                    let hi = ((1u128 << n.bits) - 1) as i64;
                    self.emit(I::I64Const(hi));
                    self.emit(I::I64GtU);
                }
            }
            B::Add | B::Sub if s => {
                // Signed, 64 bits: the sign of the result is wrong.
                self.emit(I::LocalGet(x));
                self.emit(I::LocalGet(y));
                self.emit(if op == B::Add { I::I64Add } else { I::I64Sub });
                let r = self.local(ValType::I64);
                self.emit(I::LocalSet(r));
                if op == B::Add {
                    // ((x ^ r) & (y ^ r)) < 0
                    self.emit(I::LocalGet(x));
                    self.emit(I::LocalGet(r));
                    self.emit(I::I64Xor);
                    self.emit(I::LocalGet(y));
                    self.emit(I::LocalGet(r));
                    self.emit(I::I64Xor);
                } else {
                    // ((x ^ y) & (x ^ r)) < 0
                    self.emit(I::LocalGet(x));
                    self.emit(I::LocalGet(y));
                    self.emit(I::I64Xor);
                    self.emit(I::LocalGet(x));
                    self.emit(I::LocalGet(r));
                    self.emit(I::I64Xor);
                }
                self.emit(I::I64And);
                self.emit(I::I64Const(0));
                self.emit(I::I64LtS);
            }
            B::Add => {
                // Unsigned, 64 bits: it wrapped below `x`.
                self.emit(I::LocalGet(x));
                self.emit(I::LocalGet(y));
                self.emit(I::I64Add);
                self.emit(I::LocalGet(x));
                self.emit(I::I64LtU);
            }
            B::Sub => {
                self.emit(I::LocalGet(x));
                self.emit(I::LocalGet(y));
                self.emit(I::I64LtU);
            }
            B::Mul => {
                // 64 bits: `x` is not 0 and `x * y / x` is not `y` (or, signed, the one product
                // division cannot check, `-1 * MIN`).
                self.emit(I::LocalGet(x));
                self.emit(I::I64Eqz);
                self.emit(I::If(BlockType::Result(ValType::I32)));
                self.emit(I::I32Const(0));
                self.emit(I::Else);
                if s {
                    self.emit(I::LocalGet(x));
                    self.emit(I::I64Const(-1));
                    self.emit(I::I64Eq);
                    self.emit(I::LocalGet(y));
                    self.emit(I::I64Const(i64::MIN));
                    self.emit(I::I64Eq);
                    self.emit(I::I32And);
                    self.emit(I::If(BlockType::Result(ValType::I32)));
                    self.emit(I::I32Const(1));
                    self.emit(I::Else);
                }
                self.emit(I::LocalGet(x));
                self.emit(I::LocalGet(y));
                self.emit(I::I64Mul);
                self.emit(I::LocalGet(x));
                self.emit(if s { I::I64DivS } else { I::I64DivU });
                self.emit(I::LocalGet(y));
                self.emit(I::I64Ne);
                if s {
                    self.emit(I::End);
                }
                self.emit(I::End);
            }
            B::Div | B::Rem => {
                // By zero, or the least value by -1.
                self.emit(I::LocalGet(y));
                self.emit(if wide { I::I64Eqz } else { I::I32Eqz });
                if s {
                    self.emit(I::LocalGet(x));
                    self.min(n);
                    self.eq(n.vt);
                    self.emit(I::LocalGet(y));
                    self.emit(if wide {
                        I::I64Const(-1)
                    } else {
                        I::I32Const(-1)
                    });
                    self.eq(n.vt);
                    self.emit(I::I32And);
                    self.emit(I::I32Or);
                }
            }
            B::Shl | B::Shr => {
                // Out of range: negative, or the width or more (unsigned).
                self.emit(I::LocalGet(y));
                if wide {
                    self.emit(I::I64Const(i64::from(n.bits)));
                    self.emit(I::I64GeU);
                } else {
                    self.emit(I::I32Const(n.bits as i32));
                    self.emit(I::I32GeU);
                }
                if op == B::Shl {
                    // Shifting back does not give the value: bits, or the sign, are lost.
                    self.emit(I::LocalGet(x));
                    self.emit(I::LocalGet(y));
                    self.emit(if wide { I::I64Shl } else { I::I32Shl });
                    self.canon(n);
                    self.emit(I::LocalGet(y));
                    self.emit(match (wide, s) {
                        (true, true) => I::I64ShrS,
                        (true, false) => I::I64ShrU,
                        (false, true) => I::I32ShrS,
                        (false, false) => I::I32ShrU,
                    });
                    self.emit(I::LocalGet(x));
                    self.emit(if wide { I::I64Ne } else { I::I32Ne });
                    self.emit(I::I32Or);
                }
            }
            _ => self.emit(I::I32Const(0)),
        }
        Ok(())
    }

    /// Pushes which variant the enum at a place holds, as an `int`.
    fn discriminant(&mut self, p: &Place) -> Result<(), String> {
        let ty = self.place_ty(p)?;
        let l = self.layout(&ty)?;
        let Shape::Enum { tag, variants } = &l.shape else {
            self.emit(I::I64Const(0));
            return Ok(());
        };
        let n = variants.len() as i64;
        match tag.clone() {
            Tag::None => self.emit(I::I64Const(0)),
            Tag::Direct { offset, ty: t } => {
                let PlaceR::Mem(..) = self.place(p)? else {
                    return Err("an enum not in memory".to_string());
                };
                if offset > 0 {
                    self.emit(I::I32Const(offset as i32));
                    self.emit(I::I32Add);
                }
                self.raw_load(t);
            }
            Tag::Niche { dataful, niche } => {
                let PlaceR::Mem(..) = self.place(p)? else {
                    return Err("an enum not in memory".to_string());
                };
                if niche.offset > 0 {
                    self.emit(I::I32Const(niche.offset as i32));
                    self.emit(I::I32Add);
                }
                self.raw_load(niche.ty);
                // rel = raw - first: one of the others if below their count, else the dataful.
                self.emit(I::I64Const(niche.first as u64 as i64));
                self.emit(I::I64Sub);
                let rel = self.local(ValType::I64);
                self.emit(I::LocalTee(rel));
                // v = rel + (rel >= dataful)
                self.emit(I::LocalGet(rel));
                self.emit(I::I64Const(i64::from(dataful)));
                self.emit(I::I64GeU);
                self.emit(I::I64ExtendI32U);
                self.emit(I::I64Add);
                self.emit(I::I64Const(i64::from(dataful)));
                self.emit(I::LocalGet(rel));
                self.emit(I::I64Const(n - 1));
                self.emit(I::I64LtU);
                self.emit(I::Select);
            }
        }
        Ok(())
    }

    // ----- terminators ---------------------------------------------------------------------

    fn ret(&mut self) -> Result<(), String> {
        let k = self.kind(&self.ret.clone())?;
        self.epilogue();
        self.emit(I::I32Const(0));
        if let Kind::Scalar(_) = k {
            let r = self.place(&Place::local(Local(0)))?;
            self.read(r)?;
        }
        self.emit(I::Return);
        Ok(())
    }

    fn call(
        &mut self,
        callee: &Callee,
        args: &[Operand],
        dest: &Place,
        range: TextRange,
    ) -> Result<(), String> {
        let Callee::Fn { func, args: targs } = callee else {
            return Err("a trait's method".to_string());
        };
        if !targs.is_empty() {
            return Err("a generic function".to_string());
        }
        let (index, params, ret) = self.c.declare_fn(*func)?;
        // Count the frame, so that recursion without end traps.
        self.emit(I::GlobalGet(DEPTH));
        self.emit(I::I32Const(1));
        self.emit(I::I32Add);
        self.emit(I::GlobalSet(DEPTH));
        self.emit(I::GlobalGet(DEPTH));
        self.emit(I::I32Const(MAX_DEPTH));
        self.emit(I::I32GeS);
        self.emit(I::If(BlockType::Empty));
        self.trap(TrapClass::StackOverflow, range)?;
        self.emit(I::End);
        for (a, pty) in args.iter().zip(&params) {
            match self.kind(pty)? {
                Kind::Nothing => {}
                Kind::Scalar(_) => {
                    self.operand(a)?;
                }
                Kind::Mem => {
                    // The callee's own copy, in this frame.
                    let l = self.layout(pty)?;
                    let off = self.slot(&l);
                    self.frame_addr(off);
                    self.operand(a)?;
                    self.emit(I::I32Const(l.size as i32));
                    self.emit(I::MemoryCopy {
                        src_mem: 0,
                        dst_mem: 0,
                    });
                    self.frame_addr(off);
                }
            }
        }
        let rk = self.kind(&ret)?;
        // An aggregate result: where the destination is (or a slot, for one without a home).
        if rk == Kind::Mem {
            match self.place(dest)? {
                PlaceR::Mem(..) => {}
                _ => {
                    let l = self.layout(&ret)?;
                    let off = self.slot(&l);
                    self.frame_addr(off);
                }
            }
        }
        self.emit(I::Call(index));
        let result = match rk {
            Kind::Scalar(vt) => {
                let r = self.local(vt);
                self.emit(I::LocalSet(r));
                Some(r)
            }
            _ => None,
        };
        let status = self.local(ValType::I32);
        self.emit(I::LocalSet(status));
        self.emit(I::GlobalGet(DEPTH));
        self.emit(I::I32Const(1));
        self.emit(I::I32Sub);
        self.emit(I::GlobalSet(DEPTH));
        // A trap: passed on.
        self.emit(I::LocalGet(status));
        self.emit(I::If(BlockType::Empty));
        self.return_status(1)?;
        self.emit(I::End);
        if let Some(r) = result {
            match self.place(dest)? {
                PlaceR::Local(l, _) => {
                    self.emit(I::LocalGet(r));
                    self.emit(I::LocalSet(l));
                }
                PlaceR::Mem(ty, _) => {
                    self.emit(I::LocalGet(r));
                    self.emit(store(&ty)?);
                }
                PlaceR::Nothing(_) => {}
            }
        }
        Ok(())
    }
}

impl cfg::Emit for Lower<'_, '_> {
    fn node(&mut self, n: usize) -> Result<Exit, String> {
        let bb = &self.body.blocks[n];
        for s in &bb.statements {
            self.statement(s)?;
        }
        Ok(match &bb.terminator {
            Terminator::Goto(t) => Exit::Goto(t.index()),
            Terminator::SwitchBool { cond, then, else_ } => {
                self.operand(cond)?;
                Exit::Branch(then.index(), else_.index())
            }
            Terminator::Return => {
                self.ret()?;
                Exit::Done
            }
            Terminator::Trap(class, _) => {
                self.trap(*class, bb.terminator_span)?;
                Exit::Done
            }
            Terminator::Unreachable => {
                self.return_status(1)?;
                Exit::Done
            }
            Terminator::Drop { place, target, .. } => {
                let ty = self.place_ty(place)?;
                if moddil_sema::Solver::new(self.c.items, &[]).needs_drop(&ty) {
                    return Err(format!("dropping a `{ty}`"));
                }
                Exit::Goto(target.index())
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
                if let Some((p, _)) = on_error
                    && self.place_ty(p)? != Ty::Never
                {
                    return Err("a call that can fail".to_string());
                }
                self.call(callee, args, dest, *range)?;
                Exit::Goto(target.index())
            }
            Terminator::Fail(_) => return Err("an error leaving a function".to_string()),
            Terminator::Resume => return Err("unwinding".to_string()),
        })
    }
    fn block(&mut self) {
        self.emit(I::Block(BlockType::Empty));
    }
    fn loop_(&mut self) {
        self.emit(I::Loop(BlockType::Empty));
    }
    fn if_(&mut self) {
        self.emit(I::If(BlockType::Empty));
    }
    fn else_(&mut self) {
        self.emit(I::Else);
    }
    fn end(&mut self) {
        self.emit(I::End);
    }
    fn br(&mut self, depth: u32) {
        self.emit(I::Br(depth));
    }
}

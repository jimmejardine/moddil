//! Lowers a Moddil kernel (`#[kernel]`, `reference/2027/gpu.md`) to naga's IR, validates it, and
//! writes it as WGSL: a compute entry point `main`, a workgroup of 8 x 8 invocations per 8 x 8
//! tile.
//!
//! - **One thread per pixel:** a kernel that draws with `Tile::each` runs on every thread of its
//!   tile's workgroup (it is pure, so each computes the same values), and each thread runs the
//!   closure `each` is given for its own pixel. A kernel that only uses `Tile::set` runs on the
//!   workgroup's first thread.
//! - **From the MIR**, specialised: the kernel and each function (and the `each` closure) it
//!   calls, with concrete type arguments, become naga functions. Their control flow is rebuilt as
//!   `if` and `loop` (`structure`); a body whose shape does not fit is a loop over a switch on
//!   its current block, which is structured whatever the MIR's control flow.
//! - **Values:** `int` is `i32` (WGSL has no 64-bit integers), `f32` and `bool` themselves, `u8`
//!   to `u32` as `u32`; structs and tuples are structs, an enum a struct of its tag and every
//!   variant's fields; a borrow is the value it borrows (kernel code only reads through shared
//!   borrows), and the tile (`&mut Tile`) is its position.
//! - **Traps are not checked** on the GPU (the MIR's `Assert`s are dropped), and an error edge is
//!   never taken: interim limits.
//! - **Bindings:** the parameters (`@binding(0)`, words in Moddil's own layout, which the
//!   generated code reads), the image's pixels (`@binding(1)`, packed RGBA8 row by row), and its
//!   width and height (`@binding(2)`).

mod structure;

use std::collections::HashMap;

use structure::Exit;

use moddil_mir::{
    AggregateKind, BinOp, Body, Const, Operand, Place, Projection, Rvalue, Statement, Terminator,
    UnOp,
};
use moddil_sema::{
    Callee, ClosureTy, Db, Item, ItemTree, Num, Owner, Program, Solver, TraitRef, Ty,
};
use naga::{
    AddressSpace, ArraySize, BinaryOperator, Binding, Block, BuiltIn, EntryPoint, Expression,
    Function, FunctionArgument, FunctionResult, GlobalVariable, Handle, Literal, LocalVariable,
    MathFunction, Module, ResourceBinding, Scalar, ScalarKind, ShaderStage, Span, Statement as S,
    StructMember, SwitchCase, SwitchValue, Type, TypeInner, UnaryOperator,
};

/// The byte offsets of a struct's or tuple's fields, in field order, in Moddil's own layout: what
/// the parameters' words are read at. `None` for a type it does not know.
pub type Offsets<'a> = &'a mut dyn FnMut(&Ty) -> Option<Vec<u64>>;

/// The kernel a value of type `k` runs: a function marked `#[kernel]`, named as a value, is a
/// closure that calls it.
pub fn kernel_of(db: &dyn Db, program: Program, k: &Ty) -> Option<u32> {
    let items = moddil_sema::item_tree(db, program);
    let Ty::Closure(c) = k else {
        return None;
    };
    let body = moddil_mir::mir(db, Owner::new(db, program, closure_item(c)?)).as_ref()?;
    body.blocks.iter().find_map(|b| match &b.terminator {
        Terminator::Call {
            callee: Callee::Fn { func, .. },
            ..
        } if items.fns[*func as usize].kernel => Some(*func),
        _ => None,
    })
}

/// The body of closure type `c`, as an item: a closure written in a function or a test (not in
/// another closure).
fn closure_item(c: &ClosureTy) -> Option<Item> {
    match c.owner {
        Item::Fn(f) => Some(Item::Closure(f, false, c.expr)),
        Item::Test(t) => Some(Item::Closure(t, true, c.expr)),
        Item::Closure(..) => None,
    }
}

/// The WGSL of kernel `kernel` (a function of `program`'s item tree marked `#[kernel]`).
///
/// # Errors
///
/// What the kernel does that lowering does not support yet, or why naga refuses the result.
pub fn wgsl(
    db: &dyn Db,
    program: Program,
    kernel: u32,
    offsets: Offsets<'_>,
) -> Result<String, String> {
    let module = module(db, program, kernel, offsets)?;
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    )
    .validate(&module)
    .map_err(|e| format!("the kernel's WGSL does not validate: {:?}", e.as_inner()))?;
    naga::back::wgsl::write_string(&module, &info, naga::back::wgsl::WriterFlags::empty())
        .map_err(|e| format!("writing the kernel's WGSL: {e}"))
}

/// Kernel `kernel` of `program` as naga's IR.
///
/// # Errors
///
/// As [`wgsl`], before validation.
pub fn module(
    db: &dyn Db,
    program: Program,
    kernel: u32,
    offsets: Offsets<'_>,
) -> Result<Module, String> {
    let items = moddil_sema::item_tree(db, program);
    let mut l = Lowering {
        db,
        program,
        items,
        offsets,
        module: Module::default(),
        types: HashMap::new(),
        fns: HashMap::new(),
        globals: None,
    };
    l.globals = Some(l.globals());
    l.entry(kernel)?;
    Ok(l.module)
}

/// The module's buffers, and the pixel of its tile this thread draws (for `Tile::each`).
#[derive(Clone, Copy)]
struct Globals {
    params: Handle<GlobalVariable>,
    pixels: Handle<GlobalVariable>,
    size: Handle<GlobalVariable>,
    px: Handle<GlobalVariable>,
    py: Handle<GlobalVariable>,
}

struct Lowering<'a> {
    db: &'a dyn Db,
    program: Program,
    items: &'a ItemTree,
    offsets: Offsets<'a>,
    module: Module,
    /// Each Moddil type's naga type, or `None` for a type with no value (`()`).
    types: HashMap<Ty, Option<Handle<Type>>>,
    /// Each function instance lowered so far.
    fns: HashMap<(Item, Vec<Ty>), Handle<Function>>,
    globals: Option<Globals>,
}

const U: Span = Span::UNDEFINED;

impl Lowering<'_> {
    fn scalar(&mut self, s: Scalar) -> Handle<Type> {
        self.module.types.insert(
            Type {
                name: None,
                inner: TypeInner::Scalar(s),
            },
            U,
        )
    }

    fn words(&mut self) -> Handle<Type> {
        let u = self.scalar(Scalar::U32);
        self.module.types.insert(
            Type {
                name: None,
                inner: TypeInner::Array {
                    base: u,
                    size: ArraySize::Dynamic,
                    stride: 4,
                },
            },
            U,
        )
    }

    fn globals(&mut self) -> Globals {
        let words = self.words();
        let mut g = |name: &str, binding: u32, access: naga::StorageAccess| {
            self.module.global_variables.append(
                GlobalVariable {
                    name: Some(name.to_string()),
                    space: AddressSpace::Storage { access },
                    binding: Some(ResourceBinding { group: 0, binding }),
                    ty: words,
                    init: None,
                    memory_decorations: naga::MemoryDecorations::empty(),
                },
                U,
            )
        };
        let params = g("params", 0, naga::StorageAccess::LOAD);
        let pixels = g(
            "pixels",
            1,
            naga::StorageAccess::LOAD | naga::StorageAccess::STORE,
        );
        let size = g("size", 2, naga::StorageAccess::LOAD);
        let i32t = self.scalar(Scalar::I32);
        let mut private = |name: &str| {
            self.module.global_variables.append(
                GlobalVariable {
                    name: Some(name.to_string()),
                    space: AddressSpace::Private,
                    binding: None,
                    ty: i32t,
                    init: None,
                    memory_decorations: naga::MemoryDecorations::empty(),
                },
                U,
            )
        };
        let px = private("px");
        let py = private("py");
        Globals {
            params,
            pixels,
            size,
            px,
            py,
        }
    }

    /// `t` with refinements as their bases and borrows as what they borrow: how a kernel holds a
    /// value of it.
    fn plain(&self, t: &Ty) -> Ty {
        match t {
            Ty::Ref(_, inner) => self.plain(inner),
            Ty::Refined(..) => self.plain(&self.items.base(t)),
            t => t.clone(),
        }
    }

    /// Whether `t` is the gpu mod's `Tile`.
    fn is_tile(&self, t: &Ty) -> bool {
        matches!(self.plain(t), Ty::Adt(a, ..) if self.items.adts[a as usize].name == "Tile")
    }

    /// The fields of struct (or tuple) `t`, by type.
    fn fields(&self, t: &Ty) -> Option<Vec<Ty>> {
        match self.plain(t) {
            Ty::Tuple(ts) => Some(ts.to_vec()),
            Ty::Adt(a, _, args) if self.items.adts[a as usize].variants.len() == 1 => Some(
                self.items.adts[a as usize].variants[0]
                    .fields
                    .iter()
                    .map(|f| f.ty.subst(&args))
                    .collect(),
            ),
            _ => None,
        }
    }

    /// Enum `t`'s variants' fields, by type.
    fn variants(&self, t: &Ty) -> Option<Vec<Vec<Ty>>> {
        match self.plain(t) {
            Ty::Adt(a, _, args) if self.items.adts[a as usize].is_enum => Some(
                self.items.adts[a as usize]
                    .variants
                    .iter()
                    .map(|v| v.fields.iter().map(|f| f.ty.subst(&args)).collect())
                    .collect(),
            ),
            _ => None,
        }
    }

    /// The naga type of a value of `t`, and its size and alignment, or `None` for `()`.
    fn ty(&mut self, t: &Ty) -> Result<Option<Handle<Type>>, String> {
        let t = self.plain(t);
        if let Some(h) = self.types.get(&t) {
            return Ok(*h);
        }
        let h = match &t {
            // No value; an error set's values are never made, as error edges are not taken.
            Ty::Unit | Ty::Never | Ty::Errors(_) => None,
            Ty::Int => Some(self.scalar(Scalar::I32)),
            Ty::Bool => Some(self.scalar(Scalar::BOOL)),
            Ty::Char => Some(self.scalar(Scalar::U32)),
            Ty::Num(n) => Some(self.scalar(match n {
                Num::F32 => Scalar::F32,
                Num::I8 | Num::I16 | Num::I32 => Scalar::I32,
                Num::U8 | Num::U16 | Num::U32 => Scalar::U32,
                _ => return Err(format!("a `{t}` in a kernel: no GPU has it")),
            })),
            // The tile: where it is.
            t if self.is_tile(t) => {
                let i = self.scalar(Scalar::I32);
                Some(self.structure("Tile", vec![("x".into(), i), ("y".into(), i)]))
            }
            Ty::Adt(a, ..) if self.items.adts[*a as usize].is_enum => {
                let variants = self.variants(&t).expect("an enum");
                let tag = self.scalar(Scalar::I32);
                let mut members = vec![("tag".to_string(), tag)];
                for (v, fs) in variants.iter().enumerate() {
                    for (k, f) in fs.iter().enumerate() {
                        match self.ty(f)? {
                            Some(h) => members.push((format!("v{v}_{k}"), h)),
                            None => return Err(format!("a field of type `{f}` in a kernel")),
                        }
                    }
                }
                let name = self.items.adts[*a as usize].name.clone();
                Some(self.structure(&name, members))
            }
            Ty::Adt(..) | Ty::Tuple(_) => {
                let fields = self
                    .fields(&t)
                    .ok_or_else(|| format!("a `{t}` in a kernel"))?;
                let mut members = Vec::new();
                for (k, f) in fields.iter().enumerate() {
                    match self.ty(f)? {
                        Some(h) => members.push((format!("f{k}"), h)),
                        None => return Err(format!("a field of type `{f}` in a kernel")),
                    }
                }
                let name = match &t {
                    Ty::Adt(a, ..) => self.items.adts[*a as usize].name.clone(),
                    _ => "Tuple".to_string(),
                };
                // A struct without fields has no value, as `()`.
                (!members.is_empty()).then(|| self.structure(&name, members))
            }
            // A closure (`Tile::each`'s): its captures, the first parameter of its body.
            Ty::Closure(c) => {
                let item = closure_item(c).ok_or("a closure in a closure, in a kernel")?;
                let owner = Owner::new(self.db, self.program, item);
                let body = moddil_mir::mir(self.db, owner)
                    .as_ref()
                    .ok_or("a program with errors")?;
                let env = body.locals.get(1).ok_or("a closure's captures")?.ty.clone();
                self.ty(&env)?
            }
            t => return Err(format!("a `{t}` in a kernel")),
        };
        self.types.insert(t, h);
        Ok(h)
    }

    /// Whether kernel `kernel` draws with `Tile::each`, so each thread of a tile draws a pixel.
    fn draws_each(&self, kernel: u32) -> Result<bool, String> {
        let owner = Owner::new(self.db, self.program, Item::Fn(kernel));
        let body = moddil_mir::mir(self.db, owner)
            .as_ref()
            .ok_or("a program with errors")?;
        Ok(body.blocks.iter().any(|b| match &b.terminator {
            Terminator::Call {
                callee: Callee::Fn { func, .. },
                ..
            } => {
                let f = &self.items.fns[*func as usize];
                f.name == "each"
                    && matches!(f.container, moddil_sema::Container::Impl(k)
                        if self.is_tile(&self.items.impls[k as usize].self_ty))
            }
            _ => false,
        }))
    }

    /// A struct of `members` (each a scalar or a struct of them), laid out as WGSL lays it out.
    fn structure(&mut self, name: &str, members: Vec<(String, Handle<Type>)>) -> Handle<Type> {
        let mut offset: u32 = 0;
        let mut align = 4;
        let mut ms = Vec::new();
        for (n, ty) in members {
            let (size, a) = self.size_align(ty);
            offset = offset.div_ceil(a) * a;
            ms.push(StructMember {
                name: Some(n),
                ty,
                binding: None,
                offset,
            });
            offset += size;
            align = align.max(a);
        }
        let span = offset.div_ceil(align) * align;
        self.module.types.insert(
            Type {
                name: Some(name.to_string()),
                inner: TypeInner::Struct { members: ms, span },
            },
            U,
        )
    }

    fn size_align(&self, ty: Handle<Type>) -> (u32, u32) {
        match &self.module.types[ty].inner {
            TypeInner::Struct { members, span } => {
                let a = members
                    .iter()
                    .map(|m| self.size_align(m.ty).1)
                    .max()
                    .unwrap_or(4);
                (*span, a)
            }
            _ => (4, 4),
        }
    }

    /// Function `item` with type arguments `targs`, lowered (with what it calls) the first time.
    fn function(&mut self, item: Item, targs: &[Ty]) -> Result<Handle<Function>, String> {
        let key = (item, targs.to_vec());
        if let Some(h) = self.fns.get(&key) {
            return Ok(*h);
        }
        let owner = Owner::new(self.db, self.program, item);
        let body = moddil_mir::mir(self.db, owner)
            .as_ref()
            .ok_or("a program with errors")?;
        let f = FnLowering::new(self, body, targs).lower()?;
        let h = self.module.functions.append(f, U);
        self.fns.insert(key, h);
        Ok(h)
    }

    /// The entry point: a workgroup of 8 x 8 invocations per 8 x 8 tile, each of which reads the
    /// parameters, notes its pixel, and calls the kernel with them and its tile (for a kernel
    /// without `Tile::each`, only the first).
    fn entry(&mut self, kernel: u32) -> Result<(), String> {
        let sig = &self.items.fns[kernel as usize];
        if !sig.kernel {
            return Err(format!("`{}` is not a `#[kernel]`", sig.name));
        }
        let param_ty = sig.params[0].ty.clone();
        let tile_ty = sig.params[1].ty.clone();
        let each = self.draws_each(kernel)?;
        let k = self.function(Item::Fn(kernel), &[])?;
        let g = self.globals.expect("the globals");
        let u32v = self.module.types.insert(
            Type {
                name: None,
                inner: TypeInner::Vector {
                    size: naga::VectorSize::Tri,
                    scalar: Scalar::U32,
                },
            },
            U,
        );
        let arg = |name: &str, b| FunctionArgument {
            name: Some(name.to_string()),
            ty: u32v,
            binding: Some(Binding::BuiltIn(b)),
        };
        let mut f = Function {
            name: Some("main".to_string()),
            arguments: vec![
                arg("tile", BuiltIn::WorkGroupId),
                arg("pixel", BuiltIn::LocalInvocationId),
            ],
            ..Function::default()
        };
        let mut e = Exprs::new(&mut f);
        let wg = e.add(Expression::FunctionArgument(0));
        let lid = e.add(Expression::FunctionArgument(1));
        let eight = e.add(Expression::Literal(Literal::U32(8)));
        let int = |e: &mut Exprs<'_>, v| {
            e.emit(Expression::As {
                expr: v,
                kind: ScalarKind::Sint,
                convert: Some(4),
            })
        };
        let wx = e.emit(Expression::AccessIndex { base: wg, index: 0 });
        let wy = e.emit(Expression::AccessIndex { base: wg, index: 1 });
        let x = e.bin(BinaryOperator::Multiply, wx, eight);
        let y = e.bin(BinaryOperator::Multiply, wy, eight);
        let xi = int(&mut e, x);
        let yi = int(&mut e, y);
        let lx = e.emit(Expression::AccessIndex {
            base: lid,
            index: 0,
        });
        let ly = e.emit(Expression::AccessIndex {
            base: lid,
            index: 1,
        });
        let lxi = int(&mut e, lx);
        let lyi = int(&mut e, ly);
        for (g, v) in [(g.px, lxi), (g.py, lyi)] {
            let p = e.add(Expression::GlobalVariable(g));
            e.push(S::Store {
                pointer: p,
                value: v,
            });
        }
        if !each {
            // A kernel that draws its tile itself: the workgroup's first thread.
            let any = e.bin(BinaryOperator::InclusiveOr, lx, ly);
            let zero = e.add(Expression::Literal(Literal::U32(0)));
            let other = e.bin(BinaryOperator::NotEqual, any, zero);
            e.push(S::If {
                condition: other,
                accept: Block::from_vec(vec![S::Return { value: None }]),
                reject: Block::new(),
            });
        }
        let tile = self.ty(&tile_ty)?.expect("a tile type");
        let params = self.decode(&mut f, &param_ty, 0)?;
        let mut e = Exprs::new(&mut f);
        let tile = e.emit(Expression::Compose {
            ty: tile,
            components: vec![xi, yi],
        });
        e.push(S::Call {
            function: k,
            arguments: params.into_iter().chain([tile]).collect(),
            result: None,
        });
        self.module.entry_points.push(EntryPoint {
            name: "main".to_string(),
            stage: ShaderStage::Compute,
            early_depth_test: None,
            workgroup_size: [8, 8, 1],
            workgroup_size_overrides: None,
            function: f,
            mesh_info: None,
            task_payload: None,
            incoming_ray_payload: None,
        });
        Ok(())
    }

    /// The value of type `t` (a borrow, as what it borrows) at byte `at` of the parameters'
    /// words, read in Moddil's own layout; nothing for `()`.
    fn decode(
        &mut self,
        f: &mut Function,
        t: &Ty,
        at: u64,
    ) -> Result<Vec<Handle<Expression>>, String> {
        let g = self.globals.expect("the globals");
        let t = self.plain(t);
        let word = |f: &mut Function, at: u64| {
            let mut e = Exprs::new(f);
            e.word(g.params, (at / 4) as u32)
        };
        Ok(match &t {
            Ty::Unit => Vec::new(),
            Ty::Int | Ty::Num(Num::I32 | Num::I16 | Num::I8) => {
                let w = word(f, at);
                vec![Exprs::new(f).emit(Expression::As {
                    expr: w,
                    kind: ScalarKind::Sint,
                    convert: None,
                })]
            }
            Ty::Num(Num::U32) | Ty::Char => vec![word(f, at)],
            Ty::Num(Num::F32) => {
                let w = word(f, at);
                vec![Exprs::new(f).emit(Expression::As {
                    expr: w,
                    kind: ScalarKind::Float,
                    convert: None,
                })]
            }
            Ty::Bool | Ty::Num(Num::U8) => {
                let w = word(f, at);
                let mut e = Exprs::new(f);
                let shift = e.add(Expression::Literal(Literal::U32(((at % 4) * 8) as u32)));
                let shifted = e.bin(BinaryOperator::ShiftRight, w, shift);
                let mask = e.add(Expression::Literal(Literal::U32(255)));
                let b = e.bin(BinaryOperator::And, shifted, mask);
                if t == Ty::Bool {
                    let zero = e.add(Expression::Literal(Literal::U32(0)));
                    vec![e.bin(BinaryOperator::NotEqual, b, zero)]
                } else {
                    vec![b]
                }
            }
            Ty::Adt(..) | Ty::Tuple(_) if self.fields(&t).is_some() => {
                let fields = self.fields(&t).expect("fields");
                let offs = (self.offsets)(&t).ok_or_else(|| format!("the layout of `{t}`"))?;
                let mut parts = Vec::new();
                for (ft, off) in fields.iter().zip(offs) {
                    parts.extend(self.decode(f, ft, at + off)?);
                }
                let ty = self.ty(&t)?.ok_or("a struct with no value")?;
                vec![Exprs::new(f).emit(Expression::Compose {
                    ty,
                    components: parts,
                })]
            }
            t => return Err(format!("a kernel parameter of type `{t}`")),
        })
    }
}

/// Appending a function's expressions and statements, emitting each expression that needs it.
struct Exprs<'f> {
    f: &'f mut Function,
}

impl<'f> Exprs<'f> {
    fn new(f: &'f mut Function) -> Self {
        Exprs { f }
    }

    /// An expression that needs no emitting (a literal, a variable).
    fn add(&mut self, e: Expression) -> Handle<Expression> {
        self.f.expressions.append(e, U)
    }

    /// An expression, emitted where it is.
    fn emit(&mut self, e: Expression) -> Handle<Expression> {
        let h = self.f.expressions.append(e, U);
        self.f
            .body
            .push(S::Emit(naga::Range::new_from_bounds(h, h)), U);
        h
    }

    fn push(&mut self, s: S) {
        self.f.body.push(s, U);
    }

    fn bin(
        &mut self,
        op: BinaryOperator,
        left: Handle<Expression>,
        right: Handle<Expression>,
    ) -> Handle<Expression> {
        self.emit(Expression::Binary { op, left, right })
    }

    /// Word `i` of buffer `g`.
    fn word(&mut self, g: Handle<GlobalVariable>, i: u32) -> Handle<Expression> {
        let base = self.add(Expression::GlobalVariable(g));
        let p = self.emit(Expression::AccessIndex { base, index: i });
        self.emit(Expression::Load { pointer: p })
    }
}

/// One function's body being lowered: its locals as variables, its blocks as the cases of a
/// switch in a loop, the variable `bb` saying which runs next.
struct FnLowering<'l, 'a> {
    l: &'l mut Lowering<'a>,
    body: &'a Body,
    targs: Vec<Ty>,
    f: Function,
    /// Each MIR local's variable, if it has a value.
    vars: Vec<Option<Handle<LocalVariable>>>,
    /// The variable saying which block runs next.
    bb: Option<Handle<LocalVariable>>,
    /// The statements being written: a block's case.
    out: Vec<S>,
}

impl<'l, 'a> FnLowering<'l, 'a> {
    fn new(l: &'l mut Lowering<'a>, body: &'a Body, targs: &[Ty]) -> Self {
        FnLowering {
            l,
            body,
            targs: targs.to_vec(),
            f: Function::default(),
            vars: Vec::new(),
            bb: None,
            out: Vec::new(),
        }
    }

    fn concrete(&self, t: &Ty) -> Ty {
        Solver::new(self.l.items, &[]).deep(&t.subst(&self.targs))
    }

    fn local_ty(&self, i: usize) -> Ty {
        self.concrete(&self.body.locals[i].ty)
    }

    fn lower(mut self) -> Result<Function, String> {
        self.f.name = Some(sanitize(&self.body.name));
        let ret = self.local_ty(0);
        if let Some(t) = self.l.ty(&ret)? {
            self.f.result = Some(FunctionResult {
                ty: t,
                binding: None,
            });
        }
        for i in 0..self.body.locals.len() {
            let t = self.local_ty(i);
            let h = match self.l.ty(&t)? {
                Some(ty) => Some(self.f.local_variables.append(
                    LocalVariable {
                        name: Some(format!("l{i}")),
                        ty,
                        init: None,
                    },
                    U,
                )),
                None => None,
            };
            self.vars.push(h);
        }
        // The arguments, into their variables.
        let mut args = Vec::new();
        for i in 1..=self.body.arg_count {
            let t = self.local_ty(i);
            if let Some(ty) = self.l.ty(&t)? {
                let k = self.f.arguments.len() as u32;
                self.f.arguments.push(FunctionArgument {
                    name: Some(format!("a{i}")),
                    ty,
                    binding: None,
                });
                args.push((i, k));
            }
        }
        for (i, k) in args {
            let a = self.add(Expression::FunctionArgument(k));
            let p = self.var(i);
            self.out.push(S::Store {
                pointer: p,
                value: a,
            });
        }
        let mut body = std::mem::take(&mut self.out);
        // Each block's statements, and where it goes after them.
        let mut stmts = Vec::new();
        let mut exits = Vec::new();
        for block in &self.body.blocks {
            for s in &block.statements {
                self.statement(s)?;
            }
            exits.push(self.exit(&block.terminator)?);
            stmts.push(std::mem::take(&mut self.out));
        }
        let structured = {
            let mut fresh_return = || {
                let value = self.var_value(0);
                let mut s = std::mem::take(&mut self.out);
                s.push(S::Return { value });
                s
            };
            structure::structure(&stmts, &exits, &mut fresh_return)
        };
        match structured {
            Some(mut s) => body.append(&mut s),
            None => body.extend(self.dispatcher(stmts, &exits)),
        }
        // Never reached (each path returns), but a function giving a value ends with a return:
        // its own load, as the others are out of scope here.
        let ret_value = self.var_value(0);
        body.append(&mut self.out);
        body.push(S::Return { value: ret_value });
        self.f.body = Block::from_vec(body);
        Ok(self.f)
    }

    /// The blocks `stmts` (with their exits) as the cases of a switch in a loop, the variable
    /// `bb` saying which runs next: correct for any control flow.
    fn dispatcher(&mut self, stmts: Vec<Vec<S>>, exits: &[Exit]) -> Vec<S> {
        let i32t = self.l.scalar(Scalar::I32);
        let bb = self.f.local_variables.append(
            LocalVariable {
                name: Some("bb".to_string()),
                ty: i32t,
                init: None,
            },
            U,
        );
        self.bb = Some(bb);
        let mut out = vec![self.goto(0)];
        let mut cases = Vec::new();
        for (n, (mut s, exit)) in stmts.into_iter().zip(exits).enumerate() {
            match *exit {
                Exit::Goto(t) => s.push(self.goto(t)),
                Exit::Branch(condition, t, e) => s.push(S::If {
                    condition,
                    accept: Block::from_vec(vec![self.goto(t)]),
                    reject: Block::from_vec(vec![self.goto(e)]),
                }),
                Exit::Return(value) | Exit::Abort(value) => s.push(S::Return { value }),
            }
            cases.push(SwitchCase {
                value: SwitchValue::I32(n as i32),
                body: Block::from_vec(s),
                fall_through: false,
            });
        }
        let ret_value = self.var_value(0);
        cases.push(SwitchCase {
            value: SwitchValue::Default,
            body: Block::from_vec(vec![S::Return { value: ret_value }]),
            fall_through: false,
        });
        let pbb = self.add(Expression::LocalVariable(bb));
        let sel = self.emit_expr(Expression::Load { pointer: pbb });
        let mut loop_body = std::mem::take(&mut self.out);
        loop_body.push(S::Switch {
            selector: sel,
            cases,
        });
        out.push(S::Loop {
            body: Block::from_vec(loop_body),
            continuing: Block::new(),
            break_if: None,
        });
        out
    }

    fn add(&mut self, e: Expression) -> Handle<Expression> {
        self.f.expressions.append(e, U)
    }

    /// An expression, emitted in the statements being written.
    fn emit_expr(&mut self, e: Expression) -> Handle<Expression> {
        let h = self.f.expressions.append(e, U);
        self.out.push(S::Emit(naga::Range::new_from_bounds(h, h)));
        h
    }

    /// A pointer to local `i`'s variable.
    fn var(&mut self, i: usize) -> Handle<Expression> {
        let v = self.vars[i].expect("a local with a value");
        self.add(Expression::LocalVariable(v))
    }

    /// Local `i`'s value, if it has one; for a `Return`, read where it is.
    fn var_value(&mut self, i: usize) -> Option<Handle<Expression>> {
        let v = self.vars[i]?;
        let p = self.add(Expression::LocalVariable(v));
        // Read at the return itself: emitted in the statements being written.
        Some(self.emit_expr(Expression::Load { pointer: p }))
    }

    /// Goes to block `b` next, in the dispatcher.
    fn goto(&mut self, b: usize) -> S {
        let v = self.add(Expression::Literal(Literal::I32(b as i32)));
        let p = self.add(Expression::LocalVariable(self.bb.expect("bb")));
        S::Store {
            pointer: p,
            value: v,
        }
    }

    /// A pointer to `place`, and the place's type; `None` for a place with no value.
    fn place(&mut self, place: &Place) -> Result<Option<(Handle<Expression>, Ty)>, String> {
        let i = place.local.index();
        if self.vars[i].is_none() {
            return Ok(None);
        }
        let mut p = self.var(i);
        let mut t = self.local_ty(i);
        let mut variant = None;
        for proj in &place.projection {
            match proj {
                Projection::Deref => {
                    t = match t {
                        Ty::Ref(_, inner) => (*inner).clone(),
                        t => t,
                    };
                }
                Projection::Downcast(v) => variant = Some(*v),
                Projection::Field(k) => {
                    let plain = self.l.plain(&t);
                    let (index, ft) = match (variant.take(), self.l.variants(&plain)) {
                        (Some(v), Some(vs)) => {
                            let before: usize = vs[..v as usize].iter().map(Vec::len).sum();
                            (1 + before as u32 + k, vs[v as usize][*k as usize].clone())
                        }
                        _ => {
                            let fs = self
                                .l
                                .fields(&plain)
                                .ok_or_else(|| format!("a field of `{plain}`"))?;
                            (*k, fs[*k as usize].clone())
                        }
                    };
                    p = self.emit_expr(Expression::AccessIndex { base: p, index });
                    t = ft;
                }
                Projection::Index(_) | Projection::Subslice(..) => {
                    return Err("indexing in a kernel".to_string());
                }
            }
        }
        Ok(Some((p, t)))
    }

    /// `op`'s value, if it has one.
    fn operand(&mut self, op: &Operand) -> Result<Option<Handle<Expression>>, String> {
        match op {
            Operand::Copy(p) | Operand::Move(p) => Ok(self
                .place(p)?
                .map(|(ptr, _)| self.emit_expr(Expression::Load { pointer: ptr }))),
            Operand::Const(c) => Ok(match c {
                Const::Unit => None,
                Const::Int(i) => Some(self.add(Expression::Literal(Literal::I32(*i as i32)))),
                Const::Bool(b) => Some(self.add(Expression::Literal(Literal::Bool(*b)))),
                Const::Char(c) => Some(self.add(Expression::Literal(Literal::U32(*c as u32)))),
                Const::Num(n, bits) => Some(self.add(Expression::Literal(match n {
                    Num::F32 => Literal::F32(f32::from_bits(*bits as u32)),
                    Num::I8 | Num::I16 | Num::I32 => Literal::I32(*bits as u32 as i32),
                    Num::U8 | Num::U16 | Num::U32 => Literal::U32(*bits as u32),
                    n => return Err(format!("a `{n:?}` constant in a kernel")),
                }))),
                Const::Str(_) => return Err("text in a kernel".to_string()),
            }),
        }
    }

    /// The type of `op`.
    fn operand_ty(&self, op: &Operand) -> Ty {
        match op {
            Operand::Copy(p) | Operand::Move(p) => {
                let mut t = self.local_ty(p.local.index());
                let mut variant = None;
                for proj in &p.projection {
                    match proj {
                        Projection::Deref => {
                            if let Ty::Ref(_, inner) = t {
                                t = (*inner).clone();
                            }
                        }
                        Projection::Downcast(v) => variant = Some(*v),
                        Projection::Field(k) => {
                            let plain = self.l.plain(&t);
                            t = match (variant.take(), self.l.variants(&plain)) {
                                (Some(v), Some(vs)) => vs[v as usize][*k as usize].clone(),
                                _ => self
                                    .l
                                    .fields(&plain)
                                    .map_or(Ty::Error, |fs| fs[*k as usize].clone()),
                            };
                        }
                        _ => {}
                    }
                }
                t
            }
            Operand::Const(c) => match c {
                Const::Int(_) => Ty::Int,
                Const::Bool(_) => Ty::Bool,
                Const::Num(n, _) => Ty::Num(*n),
                _ => Ty::Unit,
            },
        }
    }

    fn statement(&mut self, s: &Statement) -> Result<(), String> {
        if let Statement::Assign(place, rv) = s {
            let Some((ptr, ty)) = self.place(place)? else {
                return Ok(());
            };
            if let Some(v) = self.rvalue(rv, &ty)? {
                self.out.push(S::Store {
                    pointer: ptr,
                    value: v,
                });
            }
        }
        // `Assert`s (traps are not checked on the GPU), `Read`, `StorageDead`, `Observe`.
        Ok(())
    }

    fn rvalue(&mut self, rv: &Rvalue, ty: &Ty) -> Result<Option<Handle<Expression>>, String> {
        Ok(match rv {
            Rvalue::Use(op) => self.operand(op)?,
            Rvalue::Ref(_, place) => self
                .place(place)?
                .map(|(ptr, _)| self.emit_expr(Expression::Load { pointer: ptr })),
            Rvalue::Unary(op, a) => {
                let t = self.l.plain(&self.operand_ty(a));
                let Some(a) = self.operand(a)? else {
                    return Ok(None);
                };
                let op = match (op, &t) {
                    (UnOp::Neg, _) => UnaryOperator::Negate,
                    (UnOp::Not, Ty::Bool) => UnaryOperator::LogicalNot,
                    (UnOp::Not, _) => UnaryOperator::BitwiseNot,
                };
                Some(self.emit_expr(Expression::Unary { op, expr: a }))
            }
            Rvalue::Binary(op, a, b) => {
                let ta = self.l.plain(&self.operand_ty(a));
                let (Some(a), Some(b)) = (self.operand(a)?, self.operand(b)?) else {
                    return Err("an operator on `()` in a kernel".to_string());
                };
                let bool_ = ta == Ty::Bool;
                let nop = match op {
                    BinOp::Add => BinaryOperator::Add,
                    BinOp::Sub => BinaryOperator::Subtract,
                    BinOp::Mul => BinaryOperator::Multiply,
                    BinOp::Div => BinaryOperator::Divide,
                    BinOp::Rem => BinaryOperator::Modulo,
                    BinOp::BitAnd if bool_ => BinaryOperator::LogicalAnd,
                    BinOp::BitOr if bool_ => BinaryOperator::LogicalOr,
                    BinOp::BitAnd => BinaryOperator::And,
                    BinOp::BitOr => BinaryOperator::InclusiveOr,
                    BinOp::BitXor => BinaryOperator::ExclusiveOr,
                    BinOp::Shl => BinaryOperator::ShiftLeft,
                    BinOp::Shr => BinaryOperator::ShiftRight,
                    BinOp::Eq => BinaryOperator::Equal,
                    BinOp::Ne => BinaryOperator::NotEqual,
                    BinOp::Lt => BinaryOperator::Less,
                    BinOp::Le => BinaryOperator::LessEqual,
                    BinOp::Gt => BinaryOperator::Greater,
                    BinOp::Ge => BinaryOperator::GreaterEqual,
                };
                // A shift's amount is a `u32` in WGSL.
                let b = if matches!(op, BinOp::Shl | BinOp::Shr) {
                    self.emit_expr(Expression::As {
                        expr: b,
                        kind: ScalarKind::Uint,
                        convert: Some(4),
                    })
                } else {
                    b
                };
                Some(self.emit_expr(Expression::Binary {
                    op: nop,
                    left: a,
                    right: b,
                }))
            }
            // Overflow is not checked on the GPU.
            Rvalue::Overflows(..) | Rvalue::NegOverflows(_) => {
                Some(self.add(Expression::Literal(Literal::Bool(false))))
            }
            Rvalue::Aggregate(kind, ops) => {
                let Some(th) = self.l.ty(ty)? else {
                    return Ok(None);
                };
                let mut vals = Vec::new();
                for op in ops {
                    if let Some(v) = self.operand(op)? {
                        vals.push(v);
                    }
                }
                let components = match kind {
                    AggregateKind::Adt { variant, .. } if self.l.variants(ty).is_some() => {
                        let vs = self.l.variants(ty).expect("an enum");
                        let mut c =
                            vec![self.add(Expression::Literal(Literal::I32(*variant as i32)))];
                        let mut vals = vals.into_iter();
                        for (v, fs) in vs.iter().enumerate() {
                            for f in fs {
                                if v as u32 == *variant {
                                    c.push(vals.next().ok_or("a variant's field")?);
                                } else {
                                    let ft = self.l.ty(f)?.ok_or("a field with no value")?;
                                    c.push(self.add(Expression::ZeroValue(ft)));
                                }
                            }
                        }
                        c
                    }
                    // A closure's value is its captures (`Tile::each`'s).
                    AggregateKind::Tuple
                    | AggregateKind::Adt { .. }
                    | AggregateKind::Closure(_) => vals,
                    AggregateKind::Array => {
                        return Err("an array in a kernel".to_string());
                    }
                };
                Some(self.emit_expr(Expression::Compose { ty: th, components }))
            }
            Rvalue::Discriminant(place) => {
                let Some((ptr, _)) = self.place(place)? else {
                    return Err("a variant of `()`".to_string());
                };
                let tag = self.emit_expr(Expression::AccessIndex {
                    base: ptr,
                    index: 0,
                });
                Some(self.emit_expr(Expression::Load { pointer: tag }))
            }
            Rvalue::Widen(op, to) => {
                let to = self.concrete(to);
                let Some(v) = self.operand(op)? else {
                    return Ok(None);
                };
                Some(self.cast(v, &to)?)
            }
            other => return Err(format!("`{other:?}` in a kernel")),
        })
    }

    /// `v` as a value of number type `to`.
    fn cast(&mut self, v: Handle<Expression>, to: &Ty) -> Result<Handle<Expression>, String> {
        let kind = match self.l.plain(to) {
            Ty::Num(Num::F32) => ScalarKind::Float,
            Ty::Int | Ty::Num(Num::I8 | Num::I16 | Num::I32) => ScalarKind::Sint,
            Ty::Num(Num::U8 | Num::U16 | Num::U32) => ScalarKind::Uint,
            t => return Err(format!("a conversion to `{t}` in a kernel")),
        };
        Ok(self.emit_expr(Expression::As {
            expr: v,
            kind,
            convert: Some(4),
        }))
    }

    /// Where a block with terminator `t` goes; a call's statements are written first.
    fn exit(&mut self, t: &Terminator) -> Result<Exit, String> {
        Ok(match t {
            Terminator::Goto(b) | Terminator::Drop { target: b, .. } => Exit::Goto(b.index()),
            Terminator::SwitchBool { cond, then, else_ } => {
                let c = self.operand(cond)?.ok_or("a condition")?;
                Exit::Branch(c, then.index(), else_.index())
            }
            Terminator::Return => Exit::Return(self.var_value(0)),
            // A trap, or code that cannot run: the invocation ends (traps are not checked).
            Terminator::Trap(..)
            | Terminator::Unreachable
            | Terminator::Resume
            | Terminator::Fail(_) => Exit::Abort(self.var_value(0)),
            Terminator::Call {
                callee,
                args,
                dest,
                target,
                ..
            } => {
                self.call(callee, args, dest)?;
                Exit::Goto(target.index())
            }
        })
    }

    /// Calls `callee` with `args`, its result into `dest`.
    fn call(&mut self, callee: &Callee, args: &[Operand], dest: &Place) -> Result<(), String> {
        let items = self.l.items;
        let cargs: Vec<Ty> = callee.args().iter().map(|t| self.concrete(t)).collect();
        // The function the call runs, with its type arguments.
        let (f, fargs) = match callee {
            Callee::Fn { func, .. } => (*func, cargs.clone()),
            Callee::Trait { func, .. } => {
                let t = items.trait_of(*func).ok_or("a trait's method")?;
                let n = items.traits[t as usize].generics.params.len();
                let tr = TraitRef {
                    trait_: t,
                    name: items.traits[t as usize].name.as_str().into(),
                    args: cargs[1..n].to_vec().into(),
                };
                let self_ty = cargs[0].clone();
                let (f, mut parent) = Solver::new(items, &[])
                    .impl_method(*func, &self_ty, &tr)
                    .ok_or_else(|| format!("a trait call on `{self_ty}` in a kernel"))?;
                parent.extend(
                    cargs[items.fns[*func as usize].generics.parent as usize..]
                        .iter()
                        .cloned(),
                );
                (f, parent)
            }
        };
        let sig = &items.fns[f as usize];
        let owner = match sig.container {
            moddil_sema::Container::Impl(k) => match &items.impls[k as usize].self_ty {
                Ty::Adt(a, ..) => items.adts[*a as usize].name.as_str(),
                Ty::Num(_) | Ty::Int => "number",
                _ => "",
            },
            _ => "",
        };
        let name = sig.name.as_str();
        let dest_ptr = self.place(dest)?;
        let mut vals = Vec::new();
        for a in args {
            if let Some(v) = self.operand(a)? {
                vals.push(v);
            }
        }
        let store = |this: &mut Self, v: Handle<Expression>| {
            if let Some((p, _)) = &dest_ptr {
                this.out.push(S::Store {
                    pointer: *p,
                    value: v,
                });
            }
        };
        // The tile's `set`: a pixel of the image.
        if owner == "Tile" && name == "set" {
            return self.set_pixel(&vals);
        }
        // The tile's `each`: this thread's pixel, from the closure.
        if owner == "Tile" && name == "each" {
            return self.each(&args[1], &vals);
        }
        // The number functions: WGSL's built-ins.
        if owner == "number" && (sig.intrinsic || matches!(name, "to")) {
            let math = |fun| Some(fun);
            let fun = match name {
                "sin" => math(MathFunction::Sin),
                "cos" => math(MathFunction::Cos),
                "tan" => math(MathFunction::Tan),
                "asin" => math(MathFunction::Asin),
                "acos" => math(MathFunction::Acos),
                "atan" => math(MathFunction::Atan),
                "atan2" => math(MathFunction::Atan2),
                "sqrt" => math(MathFunction::Sqrt),
                "powf" | "pow" => math(MathFunction::Pow),
                "ln" => math(MathFunction::Log),
                "log2" => math(MathFunction::Log2),
                "exp" => math(MathFunction::Exp),
                "abs" => math(MathFunction::Abs),
                "floor" => math(MathFunction::Floor),
                "ceil" => math(MathFunction::Ceil),
                "max" => math(MathFunction::Max),
                "min" => math(MathFunction::Min),
                "clamp" => math(MathFunction::Clamp),
                _ => None,
            };
            if let Some(fun) = fun {
                let v = self.emit_expr(Expression::Math {
                    fun,
                    arg: vals[0],
                    arg1: vals.get(1).copied(),
                    arg2: vals.get(2).copied(),
                    arg3: None,
                });
                store(self, v);
                return Ok(());
            }
            // A conversion: to the call's last type argument.
            if matches!(name, "to" | "truncate" | "wrap" | "saturate") {
                let to = fargs.last().cloned().ok_or("a conversion's type")?;
                let v = self.cast(vals[0], &to)?;
                store(self, v);
                return Ok(());
            }
        }
        if sig.intrinsic || !sig.has_body {
            return Err(format!("the built-in function `{name}` in a kernel"));
        }
        let h = self.l.function(Item::Fn(f), &fargs)?;
        let result = self.l.module.functions[h]
            .result
            .is_some()
            .then(|| self.add(Expression::CallResult(h)));
        self.out.push(S::Call {
            function: h,
            arguments: vals,
            result,
        });
        if let Some(r) = result {
            store(self, r);
        }
        Ok(())
    }

    /// The tile's `each(f)`, `vals` its tile and `f`'s captures (if any): this thread's pixel of
    /// the tile, `f(px, py)`.
    fn each(&mut self, closure: &Operand, vals: &[Handle<Expression>]) -> Result<(), String> {
        let g = self.l.globals.expect("the globals");
        let ct = self.l.plain(&self.concrete(&self.operand_ty(closure)));
        let Ty::Closure(c) = &ct else {
            return Err("a tile's `each` of other than a closure".to_string());
        };
        let item = closure_item(c).ok_or("a closure in a closure, in a kernel")?;
        let h = self.l.function(item, &self.targs)?;
        let pair_ty = self.l.module.functions[h]
            .arguments
            .last()
            .map(|a| a.ty)
            .ok_or("the closure's arguments")?;
        let tile = *vals.first().ok_or("a tile")?;
        let mut arguments: Vec<Handle<Expression>> = vals.get(1).copied().into_iter().collect();
        let coord = |this: &mut Self, v| {
            let p = this.add(Expression::GlobalVariable(v));
            this.emit_expr(Expression::Load { pointer: p })
        };
        let px = coord(self, g.px);
        let py = coord(self, g.py);
        let pair = self.emit_expr(Expression::Compose {
            ty: pair_ty,
            components: vec![px, py],
        });
        arguments.push(pair);
        let result = self.add(Expression::CallResult(h));
        self.out.push(S::Call {
            function: h,
            arguments,
            result: Some(result),
        });
        self.set_pixel(&[tile, px, py, result])
    }

    /// The tile's `set(px, py, colour)`: the image's pixel, if it is in the image.
    fn set_pixel(&mut self, vals: &[Handle<Expression>]) -> Result<(), String> {
        let g = self.l.globals.expect("the globals");
        let [tile, px, py, colour] = vals else {
            return Err("a tile's `set` with other arguments".to_string());
        };
        let tx = self.emit_expr(Expression::AccessIndex {
            base: *tile,
            index: 0,
        });
        let ty = self.emit_expr(Expression::AccessIndex {
            base: *tile,
            index: 1,
        });
        let x = self.emit_expr(Expression::Binary {
            op: BinaryOperator::Add,
            left: tx,
            right: *px,
        });
        let y = self.emit_expr(Expression::Binary {
            op: BinaryOperator::Add,
            left: ty,
            right: *py,
        });
        let xu = self.emit_expr(Expression::As {
            expr: x,
            kind: ScalarKind::Uint,
            convert: Some(4),
        });
        let yu = self.emit_expr(Expression::As {
            expr: y,
            kind: ScalarKind::Uint,
            convert: Some(4),
        });
        let size = self.add(Expression::GlobalVariable(g.size));
        let wp = self.emit_expr(Expression::AccessIndex {
            base: size,
            index: 0,
        });
        let w = self.emit_expr(Expression::Load { pointer: wp });
        let hp = self.emit_expr(Expression::AccessIndex {
            base: size,
            index: 1,
        });
        let h = self.emit_expr(Expression::Load { pointer: hp });
        let inx = self.emit_expr(Expression::Binary {
            op: BinaryOperator::Less,
            left: xu,
            right: w,
        });
        let iny = self.emit_expr(Expression::Binary {
            op: BinaryOperator::Less,
            left: yu,
            right: h,
        });
        let inside = self.emit_expr(Expression::Binary {
            op: BinaryOperator::LogicalAnd,
            left: inx,
            right: iny,
        });
        let packed = self.emit_expr(Expression::AccessIndex {
            base: *colour,
            index: 0,
        });
        // The pixel's index, `y * width + x`, inside the `if`.
        let outer = std::mem::take(&mut self.out);
        let row = self.emit_expr(Expression::Binary {
            op: BinaryOperator::Multiply,
            left: yu,
            right: w,
        });
        let at = self.emit_expr(Expression::Binary {
            op: BinaryOperator::Add,
            left: row,
            right: xu,
        });
        let pixels = self.add(Expression::GlobalVariable(g.pixels));
        let p = self.emit_expr(Expression::Access {
            base: pixels,
            index: at,
        });
        self.out.push(S::Store {
            pointer: p,
            value: packed,
        });
        let accept = std::mem::replace(&mut self.out, outer);
        self.out.push(S::If {
            condition: inside,
            accept: Block::from_vec(accept),
            reject: Block::new(),
        });
        Ok(())
    }
}

/// `name` as a WGSL identifier.
fn sanitize(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    if out.starts_with(|c: char| c.is_ascii_digit()) || out.is_empty() {
        out.insert(0, 'f');
    }
    out
}

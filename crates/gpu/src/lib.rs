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
//! - **Values:** `int` is 64 bits, as on the CPU: WGSL has no 64-bit integers, so it is
//!   `core::gpu::WideInt`, its two words, and each operation on it a call of that module's
//!   functions for it (`gpu.int.1`); `f32` and `bool` are themselves, `i8` to `i32` an `i32`, `u8`
//!   to `u32` a `u32`; structs and tuples are structs, an enum a struct of its tag and every
//!   variant's fields; an array an array; a borrow is the value it borrows (kernel code only reads
//!   through shared borrows), and the tile (`&mut Tile`) is its position.
//! - **Slices:** every array in a kernel is the kernel's own or part of its parameters, so a
//!   slice is the array it is part of (its capacity: found from where the slice is made, and a
//!   function taking slices is lowered for each capacity it is given), where in it the slice
//!   starts, and how long it is.
//! - **Traps** are checked as on the CPU (`gpu.trap.1`): a failed check writes its site (one more
//!   than its index in the sites [`wgsl_sites`] gives) to the trap word, `@binding(3)` (the
//!   greatest, if several instances trap), and the instance stops: each call is followed by a
//!   return if the callee trapped. The host reads the word back and traps there, on the CPU.
//! - **Bindings:** the parameters (`@binding(0)`, words in Moddil's own layout, which the
//!   generated code reads), the image's pixels (`@binding(1)`, packed RGBA8 row by row), its
//!   width and height (`@binding(2)`), and the trap word (`@binding(3)`).

mod structure;

use std::collections::HashMap;

use structure::Exit;

use moddil_mir::{
    AggregateKind, BinOp, Body, Const, Operand, Place, Projection, Rvalue, Statement, Terminator,
    TrapClass, UnOp,
};
use moddil_sema::{
    Callee, ClosureTy, Code, Db, Diagnostic, Item, ItemTree, Num, Owner, Program, Solver, Src,
    TraitRef, Ty,
};
use moddil_syntax::TextRange;
use naga::{
    AddressSpace, ArraySize, AtomicFunction, BinaryOperator, Binding, Block, BuiltIn, EntryPoint,
    Expression, Function, FunctionArgument, FunctionResult, GlobalVariable, Handle, Literal,
    LocalVariable, MathFunction, Module, ResourceBinding, Scalar, ScalarKind, ShaderStage, Span,
    Statement as S, StructMember, SwitchCase, SwitchValue, Type, TypeInner, UnaryOperator,
};

/// The byte offsets of a struct's or tuple's fields, in field order, in Moddil's own layout: what
/// the parameters' words are read at. `None` for a type it does not know.
pub type Offsets<'a> = &'a mut dyn FnMut(&Ty) -> Option<Vec<u64>>;

/// Where an error is: a statement's source and range, if it is in one.
type At = Option<(Src, TextRange)>;

/// A function instance: the function, its type arguments, and its slice arguments' capacities.
type Instance = (Item, Vec<Ty>, Vec<Option<u32>>);

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

/// A check the kernel makes on the GPU: its kind, and its source.
pub type Site = (TrapClass, Src, TextRange);

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
    wgsl_sites(db, program, kernel, offsets).map(|(text, _)| text)
}

/// As [`wgsl`], with the checks the kernel makes: a trap word of `k + 1` is check `k`'s.
///
/// # Errors
///
/// As [`wgsl`].
pub fn wgsl_sites(
    db: &dyn Db,
    program: Program,
    kernel: u32,
    offsets: Offsets<'_>,
) -> Result<(String, Vec<Site>), String> {
    let (module, sites) = lower(db, program, kernel, offsets).map_err(|(why, _)| why)?;
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    )
    .validate(&module)
    .map_err(|e| format!("the kernel's WGSL does not validate: {:?}", e.as_inner()))?;
    let text =
        naga::back::wgsl::write_string(&module, &info, naga::back::wgsl::WriterFlags::empty())
            .map_err(|e| format!("writing the kernel's WGSL: {e}"))?;
    Ok((text, sites))
}

/// Every kernel of `program`'s files that cannot run on a GPU, with why: what in it lowering
/// does not support, at the statement it is in, or why its WGSL does not validate, at the
/// kernel (E0221, `gpu.kernel.2`).
pub fn check(db: &dyn Db, program: Program) -> Vec<Diagnostic> {
    let items = moddil_sema::item_tree(db, program);
    let mut out = Vec::new();
    for (k, sig) in items.fns.iter().enumerate() {
        let Src::File(file) = sig.src else { continue };
        if !sig.kernel || !sig.has_body {
            continue;
        }
        // The parameters' layout does not matter here: only that they can be read.
        let mut offsets = |t: &Ty| checking_offsets(items, t);
        let (why, at) = match lower(db, program, k as u32, &mut offsets) {
            Ok((module, _)) => match naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::empty(),
            )
            .validate(&module)
            {
                Ok(_) => continue,
                Err(e) => (
                    format!("its WGSL does not validate: {:?}", e.as_inner()),
                    None,
                ),
            },
            Err((why, at)) => (why, at),
        };
        let (range, file) = match at {
            Some((Src::File(f), r)) => (r, f),
            _ => (sig.name_range, file),
        };
        out.push(
            Diagnostic::error(
                Code::E0221,
                range,
                format!("`{}` cannot run on a GPU: {why}", sig.name),
            )
            .in_file(file),
        );
    }
    out
}

/// Byte offsets of `t`'s fields, or an array's elements, one word each: enough for lowering to
/// read them, in [`check`].
fn checking_offsets(items: &ItemTree, t: &Ty) -> Option<Vec<u64>> {
    let n = match t {
        Ty::Array(_, n) => match **n {
            Ty::Len(n) => usize::try_from(n).ok()?,
            _ => return None,
        },
        Ty::Tuple(ts) => ts.len(),
        Ty::Adt(a, ..) => items.adts.get(*a as usize)?.variants.first()?.fields.len(),
        _ => return None,
    };
    Some((0..n as u64).map(|i| i * 8).collect())
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
    lower(db, program, kernel, offsets)
        .map(|(m, _)| m)
        .map_err(|(why, _)| why)
}

/// As [`module`]; an error also says where in the program it is, if it is in a statement.
fn lower(
    db: &dyn Db,
    program: Program,
    kernel: u32,
    offsets: Offsets<'_>,
) -> Result<(Module, Vec<Site>), (String, At)> {
    let items = moddil_sema::item_tree(db, program);
    let mut l = Lowering {
        db,
        program,
        items,
        offsets,
        module: Module::default(),
        types: HashMap::new(),
        fns: HashMap::new(),
        slices: HashMap::new(),
        globals: None,
        at: None,
        sites: Vec::new(),
    };
    l.globals = Some(l.globals());
    match l.entry(kernel) {
        Ok(()) => Ok((l.module, l.sites)),
        Err(why) => Err((why, l.at)),
    }
}

/// The module's buffers, and the pixel of its tile this thread draws (for `Tile::each`).
#[derive(Clone, Copy)]
struct Globals {
    params: Handle<GlobalVariable>,
    pixels: Handle<GlobalVariable>,
    size: Handle<GlobalVariable>,
    px: Handle<GlobalVariable>,
    py: Handle<GlobalVariable>,
    /// The trap word: the site of a check that failed, plus one.
    trap: Handle<GlobalVariable>,
    /// Whether this instance trapped, so it stops.
    trapped: Handle<GlobalVariable>,
}

struct Lowering<'a> {
    db: &'a dyn Db,
    program: Program,
    items: &'a ItemTree,
    offsets: Offsets<'a>,
    module: Module,
    /// Each Moddil type's naga type, or `None` for a type with no value (`()`).
    types: HashMap<Ty, Option<Handle<Type>>>,
    /// Each function instance lowered so far: by its type arguments, and the capacity of each
    /// slice argument.
    fns: HashMap<Instance, Handle<Function>>,
    /// The type of a slice of each element type and capacity.
    slices: HashMap<(Ty, u32), Handle<Type>>,
    globals: Option<Globals>,
    /// The statement being lowered: where an error is.
    at: At,
    /// The checks the kernel makes, by their trap word less one.
    sites: Vec<Site>,
}

const U: Span = Span::UNDEFINED;

impl Lowering<'_> {
    fn scalar_ty_u32_handle(&mut self) -> Handle<Type> {
        self.scalar(Scalar::U32)
    }

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
        let u = self.scalar(Scalar::U32);
        let atomic = self.module.types.insert(
            Type {
                name: None,
                inner: TypeInner::Atomic(Scalar::U32),
            },
            U,
        );
        let _ = u;
        let words = self.module.types.insert(
            Type {
                name: None,
                inner: TypeInner::Array {
                    base: atomic,
                    size: ArraySize::Constant(std::num::NonZeroU32::new(4).expect("4")),
                    stride: 4,
                },
            },
            U,
        );
        let trap = self.module.global_variables.append(
            GlobalVariable {
                name: Some("trap".to_string()),
                space: AddressSpace::Storage {
                    access: naga::StorageAccess::LOAD | naga::StorageAccess::STORE,
                },
                binding: Some(ResourceBinding {
                    group: 0,
                    binding: 3,
                }),
                ty: words,
                init: None,
                memory_decorations: naga::MemoryDecorations::empty(),
            },
            U,
        );
        let bool_ = self.scalar(Scalar::BOOL);
        let trapped = self.module.global_variables.append(
            GlobalVariable {
                name: Some("trapped".to_string()),
                space: AddressSpace::Private,
                binding: None,
                ty: bool_,
                init: None,
                memory_decorations: naga::MemoryDecorations::empty(),
            },
            U,
        );
        Globals {
            params,
            pixels,
            size,
            px,
            py,
            trap,
            trapped,
        }
    }

    /// The prelude's `core::gpu` function `name` (one of `WideInt`'s, `gpu.int.1`).
    fn wide_fn(&self, name: &str) -> Result<u32, String> {
        self.items
            .fns
            .iter()
            .position(|f| f.name == name && f.src == Src::Prelude)
            .map(|f| f as u32)
            .ok_or_else(|| format!("`core::gpu::{name}`, for `int` in a kernel"))
    }

    /// The naga type of `int`: `core::gpu::WideInt`, its two words.
    fn wide_ty(&mut self) -> Result<Handle<Type>, String> {
        let a = self
            .items
            .adts
            .iter()
            .position(|a| a.name == "WideInt" && a.src == Src::Prelude)
            .ok_or("`core::gpu::WideInt`, for `int` in a kernel")?;
        let t = Ty::Adt(a as u32, "WideInt".into(), std::sync::Arc::from([]));
        self.ty(&t)?
            .ok_or_else(|| "a `WideInt` with no value".to_string())
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
            Ty::Int => Some(self.wide_ty()?),
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
                let i = self.wide_ty()?;
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
            Ty::Array(elem, n) => {
                let Ty::Len(n) = **n else {
                    return Err(format!("an array of `{t}` in a kernel"));
                };
                let base = self
                    .ty(elem)?
                    .ok_or_else(|| format!("an array of `{elem}` in a kernel"))?;
                let (size, align) = self.size_align(base);
                let len = u32::try_from(n)
                    .ok()
                    .and_then(std::num::NonZeroU32::new)
                    .ok_or("an empty array in a kernel")?;
                Some(self.module.types.insert(
                    Type {
                        name: None,
                        inner: TypeInner::Array {
                            base,
                            size: ArraySize::Constant(len),
                            stride: size.div_ceil(align) * align,
                        },
                    },
                    U,
                ))
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
            TypeInner::Array {
                base,
                size: ArraySize::Constant(n),
                stride,
            } => (stride * n.get(), self.size_align(*base).1),
            _ => (4, 4),
        }
    }

    /// The element type of `t` if it is a slice (behind borrows).
    fn slice_elem(&self, t: &Ty) -> Option<Ty> {
        match self.plain(t) {
            Ty::Slice(e) => Some((*e).clone()),
            _ => None,
        }
    }

    /// A slice of `elem` whose array has `cap` elements: the array, where the slice starts in
    /// it, and its length.
    fn slice_ty(&mut self, elem: &Ty, cap: u32) -> Result<Handle<Type>, String> {
        let key = (self.plain(elem), cap);
        if let Some(h) = self.slices.get(&key) {
            return Ok(*h);
        }
        let array = self
            .ty(&Ty::array(elem.clone(), Ty::Len(i64::from(cap))))?
            .ok_or_else(|| format!("a slice of `{elem}` in a kernel"))?;
        let i = self.scalar(Scalar::I32);
        let h = self.structure(
            "Slice",
            vec![
                ("data".into(), array),
                ("start".into(), i),
                ("len".into(), i),
            ],
        );
        self.slices.insert(key, h);
        Ok(h)
    }

    /// Function `item` with type arguments `targs` and its slice arguments' capacities `caps`
    /// (by argument), lowered (with what it calls) the first time.
    fn function(
        &mut self,
        item: Item,
        targs: &[Ty],
        caps: &[Option<u32>],
    ) -> Result<Handle<Function>, String> {
        let key = (item, targs.to_vec(), caps.to_vec());
        if let Some(h) = self.fns.get(&key) {
            return Ok(*h);
        }
        let owner = Owner::new(self.db, self.program, item);
        let body = moddil_mir::mir(self.db, owner)
            .as_ref()
            .ok_or("a program with errors")?;
        let f = FnLowering::new(self, body, targs, caps).lower()?;
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
        let k = self.function(Item::Fn(kernel), &[], &[])?;
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
        let wide = self.wide_ty()?;
        self.at = None;
        let params = self.decode(&mut f, &param_ty, 0)?;
        let mut e = Exprs::new(&mut f);
        // The trap word, read once: so every kernel's layout has it, used or not.
        let base = e.add(Expression::GlobalVariable(g.trap));
        let word = e.emit(Expression::AccessIndex { base, index: 0 });
        let seen = e.f.expressions.append(
            Expression::AtomicResult {
                ty: self.scalar_ty_u32_handle(),
                comparison: false,
            },
            U,
        );
        let nothing = e.add(Expression::Literal(Literal::U32(0)));
        e.push(S::Atomic {
            pointer: word,
            fun: AtomicFunction::Add,
            value: nothing,
            result: Some(seen),
        });
        let xw = e.wide_of_i32(wide, xi);
        let yw = e.wide_of_i32(wide, yi);
        let tile = e.emit(Expression::Compose {
            ty: tile,
            components: vec![xw, yw],
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
            Ty::Int => {
                let lo = word(f, at);
                let hi = word(f, at + 4);
                let ty = self.wide_ty()?;
                vec![Exprs::new(f).emit(Expression::Compose {
                    ty,
                    components: vec![lo, hi],
                })]
            }
            Ty::Num(Num::I32 | Num::I16 | Num::I8) => {
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
            // An array: its elements, one stride apart.
            Ty::Array(elem, _) => {
                let offs = (self.offsets)(&t).ok_or_else(|| format!("the layout of `{t}`"))?;
                let mut parts = Vec::new();
                for off in offs {
                    parts.extend(self.decode(f, elem, at + off)?);
                }
                let ty = self.ty(&t)?.ok_or("an array with no value")?;
                vec![Exprs::new(f).emit(Expression::Compose {
                    ty,
                    components: parts,
                })]
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

    /// The `i32` `x` as an `int` (`WideInt` type `wide`): its low word, and its sign.
    fn wide_of_i32(&mut self, wide: Handle<Type>, x: Handle<Expression>) -> Handle<Expression> {
        let lo = self.emit(Expression::As {
            expr: x,
            kind: ScalarKind::Uint,
            convert: None,
        });
        let zero = self.add(Expression::Literal(Literal::I32(0)));
        let neg = self.bin(BinaryOperator::Less, x, zero);
        let all = self.add(Expression::Literal(Literal::U32(u32::MAX)));
        let none = self.add(Expression::Literal(Literal::U32(0)));
        let hi = self.emit(Expression::Select {
            condition: neg,
            accept: all,
            reject: none,
        });
        self.emit(Expression::Compose {
            ty: wide,
            components: vec![lo, hi],
        })
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
    /// For each local that is (or borrows) an array or slice, the array's capacity.
    caps: Vec<Option<u32>>,
    /// The capacities of the slice arguments, by argument.
    arg_caps: Vec<Option<u32>>,
}

impl<'l, 'a> FnLowering<'l, 'a> {
    fn new(l: &'l mut Lowering<'a>, body: &'a Body, targs: &[Ty], caps: &[Option<u32>]) -> Self {
        FnLowering {
            l,
            body,
            targs: targs.to_vec(),
            f: Function::default(),
            vars: Vec::new(),
            bb: None,
            out: Vec::new(),
            caps: Vec::new(),
            arg_caps: caps.to_vec(),
        }
    }

    /// The naga type of local `i`'s value: a slice's from its capacity.
    fn local_naga_ty(&mut self, i: usize) -> Result<Option<Handle<Type>>, String> {
        let t = self.local_ty(i);
        if let Some(elem) = self.l.slice_elem(&t) {
            let cap = self.caps[i].ok_or_else(|| {
                format!("a slice of `{elem}` in a kernel whose array is not the kernel's")
            })?;
            return Ok(Some(self.l.slice_ty(&elem, cap)?));
        }
        self.l.ty(&t)
    }

    /// The type `t` of a place, after projection `proj` (`variant` the downcast before).
    fn step_ty(&self, t: Ty, proj: &Projection, variant: &mut Option<u32>) -> Ty {
        match proj {
            Projection::Deref => match t {
                Ty::Ref(_, inner) => (*inner).clone(),
                t => t,
            },
            Projection::Downcast(v) => {
                *variant = Some(*v);
                t
            }
            Projection::Field(k) => {
                let plain = self.l.plain(&t);
                match (variant.take(), self.l.variants(&plain)) {
                    (Some(v), Some(vs)) => vs[v as usize][*k as usize].clone(),
                    _ => self
                        .l
                        .fields(&plain)
                        .map_or(Ty::Error, |fs| fs[*k as usize].clone()),
                }
            }
            Projection::Index(_) => match self.l.plain(&t) {
                Ty::Array(e, _) | Ty::Slice(e) => (*e).clone(),
                _ => Ty::Error,
            },
            Projection::Subslice(..) => match self.l.plain(&t) {
                Ty::Array(e, _) | Ty::Slice(e) => Ty::Slice(e),
                _ => Ty::Error,
            },
        }
    }

    /// The capacity of the array the place `p` is or is part of, if it is an array or slice,
    /// given the capacities `caps` of the locals found so far.
    fn place_cap(&self, p: &Place, caps: &[Option<u32>]) -> Option<u32> {
        let i = p.local.index();
        let mut t = self.local_ty(i);
        let mut cap = caps[i].or_else(|| array_len(&self.l.plain(&t)));
        let mut variant = None;
        for proj in &p.projection {
            t = self.step_ty(t, proj, &mut variant);
            match proj {
                Projection::Field(_) | Projection::Index(_) => cap = array_len(&self.l.plain(&t)),
                Projection::Deref | Projection::Downcast(_) | Projection::Subslice(..) => {}
            }
        }
        cap
    }

    /// Each local's array capacity, from the arguments' and from where each slice is made.
    fn find_caps(&self) -> Vec<Option<u32>> {
        let n = self.body.locals.len();
        let mut caps = vec![None; n];
        for (k, c) in self.arg_caps.iter().enumerate() {
            if k + 1 < n {
                caps[k + 1] = *c;
            }
        }
        loop {
            let mut changed = false;
            for block in &self.body.blocks {
                for s in &block.statements {
                    let Statement::Assign(dest, rv) = s else {
                        continue;
                    };
                    if !dest.projection.is_empty() || caps[dest.local.index()].is_some() {
                        continue;
                    }
                    let from = match rv {
                        Rvalue::Use(Operand::Copy(p) | Operand::Move(p))
                        | Rvalue::Ref(_, p)
                        | Rvalue::Unsize(Operand::Copy(p) | Operand::Move(p), _) => {
                            self.place_cap(p, &caps)
                        }
                        _ => None,
                    };
                    if from.is_some() {
                        caps[dest.local.index()] = from;
                        changed = true;
                    }
                }
            }
            if !changed {
                return caps;
            }
        }
    }

    fn concrete(&self, t: &Ty) -> Ty {
        Solver::new(self.l.items, &[]).deep(&t.subst(&self.targs))
    }

    fn local_ty(&self, i: usize) -> Ty {
        self.concrete(&self.body.locals[i].ty)
    }

    fn lower(mut self) -> Result<Function, String> {
        // Until its statements: an error is the kernel's, not a statement's.
        self.l.at = None;
        self.f.name = Some(sanitize(&self.body.name));
        self.caps = self.find_caps();
        if let Some(t) = self.local_naga_ty(0)? {
            self.f.result = Some(FunctionResult {
                ty: t,
                binding: None,
            });
        }
        for i in 0..self.body.locals.len() {
            let h = match self.local_naga_ty(i)? {
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
            if let Some(ty) = self.local_naga_ty(i)? {
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
            for (s, span) in block.statements.iter().zip(&block.spans) {
                self.l.at = Some((self.body.src, *span));
                self.statement(s)?;
            }
            self.l.at = Some((self.body.src, block.terminator_span));
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

    fn lit(&mut self, l: Literal) -> Handle<Expression> {
        self.add(Expression::Literal(l))
    }

    /// The `int` `i`, as its two words.
    fn const_wide(&mut self, i: i64) -> Result<Handle<Expression>, String> {
        let ty = self.l.wide_ty()?;
        let lo = self.lit(Literal::U32(i as u32));
        let hi = self.lit(Literal::U32((i >> 32) as u32));
        Ok(self.emit_expr(Expression::Compose {
            ty,
            components: vec![lo, hi],
        }))
    }

    /// The `i32` `x` as an `int`.
    fn wide_of_i32(&mut self, x: Handle<Expression>) -> Result<Handle<Expression>, String> {
        let ty = self.l.wide_ty()?;
        let lo = self.emit_expr(Expression::As {
            expr: x,
            kind: ScalarKind::Uint,
            convert: None,
        });
        let zero = self.lit(Literal::I32(0));
        let neg = self.bin(BinaryOperator::Less, x, zero);
        let all = self.lit(Literal::U32(u32::MAX));
        let none = self.lit(Literal::U32(0));
        let hi = self.emit_expr(Expression::Select {
            condition: neg,
            accept: all,
            reject: none,
        });
        Ok(self.emit_expr(Expression::Compose {
            ty,
            components: vec![lo, hi],
        }))
    }

    /// The `u32` `x` as an `int`.
    fn wide_of_u32(&mut self, x: Handle<Expression>) -> Result<Handle<Expression>, String> {
        let ty = self.l.wide_ty()?;
        let hi = self.lit(Literal::U32(0));
        Ok(self.emit_expr(Expression::Compose {
            ty,
            components: vec![x, hi],
        }))
    }

    /// The `int` `w`'s low (0) or high (1) word.
    fn wide_word(&mut self, w: Handle<Expression>, index: u32) -> Handle<Expression> {
        self.emit_expr(Expression::AccessIndex { base: w, index })
    }

    /// The `int` `w`'s low 32 bits, as an `i32`: an index, a position.
    fn wide_i32(&mut self, w: Handle<Expression>) -> Handle<Expression> {
        let lo = self.wide_word(w, 0);
        self.emit_expr(Expression::As {
            expr: lo,
            kind: ScalarKind::Sint,
            convert: None,
        })
    }

    /// A call of `core::gpu`'s `name` with `args`: an operation on `int`s.
    fn call_wide(
        &mut self,
        name: &str,
        args: Vec<Handle<Expression>>,
    ) -> Result<Handle<Expression>, String> {
        let id = self.l.wide_fn(name)?;
        let h = self.l.function(Item::Fn(id), &[], &[])?;
        let r = self.add(Expression::CallResult(h));
        self.out.push(S::Call {
            function: h,
            arguments: args,
            result: Some(r),
        });
        self.after_call();
        Ok(r)
    }

    /// After a call: this function stops too if the callee trapped.
    fn after_call(&mut self) {
        let g = self.l.globals.expect("the globals");
        let tp = self.add(Expression::GlobalVariable(g.trapped));
        let trapped = self.emit_expr(Expression::Load { pointer: tp });
        let outer = std::mem::take(&mut self.out);
        let value = self.var_value(0);
        self.out.push(S::Return { value });
        let accept = std::mem::replace(&mut self.out, outer);
        self.out.push(S::If {
            condition: trapped,
            accept: Block::from_vec(accept),
            reject: Block::new(),
        });
    }

    /// A trap of kind `class` at `range`: its site in the trap word (the greatest, if several
    /// instances trap), and this instance stops (`gpu.trap.1`).
    fn trap_here(&mut self, class: TrapClass, range: TextRange) {
        let g = self.l.globals.expect("the globals");
        let site = self.l.sites.len() as u32;
        self.l.sites.push((class, self.body.src, range));
        let base = self.add(Expression::GlobalVariable(g.trap));
        let word = self.emit_expr(Expression::AccessIndex { base, index: 0 });
        let value = self.lit(Literal::U32(site + 1));
        self.out.push(S::Atomic {
            pointer: word,
            fun: AtomicFunction::Max,
            value,
            result: None,
        });
        let tp = self.add(Expression::GlobalVariable(g.trapped));
        let yes = self.lit(Literal::Bool(true));
        self.out.push(S::Store {
            pointer: tp,
            value: yes,
        });
    }

    /// Whether `a op b` overflows sized integer type `n` (held in 32 bits), as on the CPU
    /// (`num.ops.1`): the operation's 32-bit result checked against the type's range.
    fn small_overflows(
        &mut self,
        op: BinOp,
        n: Num,
        a: Handle<Expression>,
        b: Handle<Expression>,
    ) -> Handle<Expression> {
        let signed = n.is_signed();
        let bits = n.bits();
        let (lo, hi) = n.int_range();
        let k = |this: &mut Self, v: i128| {
            if signed {
                this.lit(Literal::I32(v as i32))
            } else {
                this.lit(Literal::U32(v as u32))
            }
        };
        // Whether `r` is out of the type's range (never, for 32 bits).
        let out_of_range = |this: &mut Self, r: Handle<Expression>| {
            if bits >= 32 {
                return this.lit(Literal::Bool(false));
            }
            let (min, max) = (k(this, lo), k(this, hi));
            let below = this.bin(BinaryOperator::Less, r, min);
            let above = this.bin(BinaryOperator::Greater, r, max);
            this.bin(BinaryOperator::LogicalOr, below, above)
        };
        match op {
            BinOp::Add | BinOp::Sub | BinOp::Mul => {
                let nop = match op {
                    BinOp::Add => BinaryOperator::Add,
                    BinOp::Sub => BinaryOperator::Subtract,
                    _ => BinaryOperator::Multiply,
                };
                let r = self.bin(nop, a, b);
                if bits < 32 {
                    return out_of_range(self, r);
                }
                match (op, signed) {
                    (BinOp::Add, true) => {
                        let x = self.bin(BinaryOperator::ExclusiveOr, a, r);
                        let y = self.bin(BinaryOperator::ExclusiveOr, b, r);
                        let both = self.bin(BinaryOperator::And, x, y);
                        let zero = k(self, 0);
                        self.bin(BinaryOperator::Less, both, zero)
                    }
                    (BinOp::Sub, true) => {
                        let x = self.bin(BinaryOperator::ExclusiveOr, a, b);
                        let y = self.bin(BinaryOperator::ExclusiveOr, a, r);
                        let both = self.bin(BinaryOperator::And, x, y);
                        let zero = k(self, 0);
                        self.bin(BinaryOperator::Less, both, zero)
                    }
                    (BinOp::Add, false) => self.bin(BinaryOperator::Less, r, a),
                    (BinOp::Sub, false) => self.bin(BinaryOperator::Less, a, b),
                    _ => {
                        // `a * b` overflowed if dividing by `a` does not give `b` back (and
                        // `-1 * MIN`).
                        let zero = k(self, 0);
                        let nonzero = self.bin(BinaryOperator::NotEqual, a, zero);
                        let back = self.bin(BinaryOperator::Divide, r, a);
                        let differs = self.bin(BinaryOperator::NotEqual, back, b);
                        let wrong = if signed {
                            let minus = k(self, -1);
                            let least = k(self, lo);
                            let am = self.bin(BinaryOperator::Equal, a, minus);
                            let bl = self.bin(BinaryOperator::Equal, b, least);
                            let edge = self.bin(BinaryOperator::LogicalAnd, am, bl);
                            self.bin(BinaryOperator::LogicalOr, differs, edge)
                        } else {
                            differs
                        };
                        self.bin(BinaryOperator::LogicalAnd, nonzero, wrong)
                    }
                }
            }
            BinOp::Div if signed => {
                let (minus, least) = (k(self, -1), k(self, lo));
                let al = self.bin(BinaryOperator::Equal, a, least);
                let bm = self.bin(BinaryOperator::Equal, b, minus);
                self.bin(BinaryOperator::LogicalAnd, al, bm)
            }
            BinOp::Shl | BinOp::Shr => {
                // The amount is the type's: negative or the width or more is out of range.
                let amount = self.emit_expr(Expression::As {
                    expr: b,
                    kind: ScalarKind::Uint,
                    convert: None,
                });
                let width = self.lit(Literal::U32(bits));
                let too_far = self.bin(BinaryOperator::GreaterEqual, amount, width);
                if op == BinOp::Shr {
                    return too_far;
                }
                // `<<` also loses no bit: shifting back gives `a`, and the result is in range.
                let r = self.bin(BinaryOperator::ShiftLeft, a, amount);
                let back = self.bin(BinaryOperator::ShiftRight, r, amount);
                let lost = self.bin(BinaryOperator::NotEqual, back, a);
                let range = out_of_range(self, r);
                let bad = self.bin(BinaryOperator::LogicalOr, lost, range);
                self.bin(BinaryOperator::LogicalOr, too_far, bad)
            }
            _ => self.lit(Literal::Bool(false)),
        }
    }

    /// `v`, of number type `from`, as one of type `to` (`num.convert.1`, `num.widen.1`).
    fn convert(
        &mut self,
        v: Handle<Expression>,
        from: &Ty,
        to: &Ty,
    ) -> Result<Handle<Expression>, String> {
        let (from, to) = (self.l.plain(from), self.l.plain(to));
        match (&from, &to) {
            (Ty::Int, Ty::Int) => Ok(v),
            (Ty::Int, Ty::Num(Num::F32)) => {
                let lo = self.wide_word(v, 0);
                let hi = self.wide_word(v, 1);
                let hs = self.emit_expr(Expression::As {
                    expr: hi,
                    kind: ScalarKind::Sint,
                    convert: None,
                });
                let hf = self.emit_expr(Expression::As {
                    expr: hs,
                    kind: ScalarKind::Float,
                    convert: Some(4),
                });
                let lf = self.emit_expr(Expression::As {
                    expr: lo,
                    kind: ScalarKind::Float,
                    convert: Some(4),
                });
                let scale = self.lit(Literal::F32(4_294_967_296.0));
                let high = self.bin(BinaryOperator::Multiply, hf, scale);
                Ok(self.bin(BinaryOperator::Add, high, lf))
            }
            (Ty::Int, Ty::Num(n)) if n.is_signed() => Ok(self.wide_i32(v)),
            (Ty::Int, _) => Ok(self.wide_word(v, 0)),
            (Ty::Num(Num::F32), Ty::Int) => {
                let i = self.emit_expr(Expression::As {
                    expr: v,
                    kind: ScalarKind::Sint,
                    convert: Some(4),
                });
                self.wide_of_i32(i)
            }
            (Ty::Num(n), Ty::Int) if n.is_signed() => self.wide_of_i32(v),
            (_, Ty::Int) => self.wide_of_u32(v),
            _ => self.cast(v, &to),
        }
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
                // An element: of the array, or of the slice's array, past where it starts.
                Projection::Index(l) => {
                    let ip = self.var(l.index());
                    let wide = self.emit_expr(Expression::Load { pointer: ip });
                    let index = self.wide_i32(wide);
                    match self.l.plain(&t) {
                        Ty::Array(e, _) => {
                            p = self.emit_expr(Expression::Access { base: p, index });
                            t = (*e).clone();
                        }
                        Ty::Slice(e) => {
                            let data =
                                self.emit_expr(Expression::AccessIndex { base: p, index: 0 });
                            let sp = self.emit_expr(Expression::AccessIndex { base: p, index: 1 });
                            let start = self.emit_expr(Expression::Load { pointer: sp });
                            let at = self.emit_expr(Expression::Binary {
                                op: BinaryOperator::Add,
                                left: start,
                                right: index,
                            });
                            p = self.emit_expr(Expression::Access {
                                base: data,
                                index: at,
                            });
                            t = (*e).clone();
                        }
                        other => return Err(format!("indexing a `{other}` in a kernel")),
                    }
                }
                Projection::Subslice(..) => {
                    return Err("a part of an array, other than borrowed, in a kernel".to_string());
                }
            }
        }
        Ok(Some((p, t)))
    }

    /// The length of the array or slice at `place`.
    fn len_of(&mut self, place: &Place) -> Result<Handle<Expression>, String> {
        let (p, t) = self.place(place)?.ok_or("the length of `()`")?;
        match self.l.plain(&t) {
            Ty::Array(_, n) => match *n {
                Ty::Len(n) => Ok(self.add(Expression::Literal(Literal::I32(n as i32)))),
                _ => Err(format!("an array of `{t}` in a kernel")),
            },
            Ty::Slice(_) => {
                let lp = self.emit_expr(Expression::AccessIndex { base: p, index: 2 });
                Ok(self.emit_expr(Expression::Load { pointer: lp }))
            }
            t => Err(format!("the length of a `{t}` in a kernel")),
        }
    }

    /// `a op b`, emitted.
    fn bin(
        &mut self,
        op: BinaryOperator,
        left: Handle<Expression>,
        right: Handle<Expression>,
    ) -> Handle<Expression> {
        self.emit_expr(Expression::Binary { op, left, right })
    }

    /// `&base[a..b]`: a slice of the array or slice at `base`.
    fn subslice(
        &mut self,
        base: &Place,
        a: moddil_mir::Local,
        b: moddil_mir::Local,
    ) -> Result<Option<Handle<Expression>>, String> {
        let Some((bp, bt)) = self.place(base)? else {
            return Ok(None);
        };
        let elem = match self.l.plain(&bt) {
            Ty::Array(e, _) | Ty::Slice(e) => (*e).clone(),
            t => return Err(format!("a part of a `{t}` in a kernel")),
        };
        let caps = self.caps.clone();
        let cap = self
            .place_cap(base, &caps)
            .ok_or("a part of an array that is not the kernel's")?;
        let ty = self.l.slice_ty(&elem, cap)?;
        let (ap, bpv) = (self.var(a.index()), self.var(b.index()));
        let aw = self.emit_expr(Expression::Load { pointer: ap });
        let bw = self.emit_expr(Expression::Load { pointer: bpv });
        let (av, bv) = (self.wide_i32(aw), self.wide_i32(bw));
        let len = self.bin(BinaryOperator::Subtract, bv, av);
        let (data, start) = match self.l.plain(&bt) {
            Ty::Array(..) => (self.emit_expr(Expression::Load { pointer: bp }), av),
            _ => {
                let dp = self.emit_expr(Expression::AccessIndex { base: bp, index: 0 });
                let data = self.emit_expr(Expression::Load { pointer: dp });
                let sp = self.emit_expr(Expression::AccessIndex { base: bp, index: 1 });
                let s0 = self.emit_expr(Expression::Load { pointer: sp });
                (data, self.bin(BinaryOperator::Add, s0, av))
            }
        };
        Ok(Some(self.emit_expr(Expression::Compose {
            ty,
            components: vec![data, start, len],
        })))
    }

    /// `op`'s value, if it has one.
    fn operand(&mut self, op: &Operand) -> Result<Option<Handle<Expression>>, String> {
        match op {
            Operand::Copy(p) | Operand::Move(p) => Ok(self
                .place(p)?
                .map(|(ptr, _)| self.emit_expr(Expression::Load { pointer: ptr }))),
            Operand::Const(c) => Ok(match c {
                Const::Unit => None,
                Const::Int(i) => Some(self.const_wide(*i)?),
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
                    t = self.step_ty(t, proj, &mut variant);
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
        // A check (`gpu.trap.1`): if it fails, the trap, and this instance stops.
        if let Statement::Assert {
            cond,
            expected,
            trap,
            range,
            ..
        } = s
        {
            let c = self.operand(cond)?.ok_or("a condition")?;
            let fails = if *expected {
                self.emit_expr(Expression::Unary {
                    op: UnaryOperator::LogicalNot,
                    expr: c,
                })
            } else {
                c
            };
            let outer = std::mem::take(&mut self.out);
            self.trap_here(*trap, *range);
            let value = self.var_value(0);
            self.out.push(S::Return { value });
            let accept = std::mem::replace(&mut self.out, outer);
            self.out.push(S::If {
                condition: fails,
                accept: Block::from_vec(accept),
                reject: Block::new(),
            });
            return Ok(());
        }
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
        // `Read`, `StorageDead`, `Observe`.
        Ok(())
    }

    fn rvalue(&mut self, rv: &Rvalue, ty: &Ty) -> Result<Option<Handle<Expression>>, String> {
        Ok(match rv {
            Rvalue::Use(op) => self.operand(op)?,
            // A part of an array or slice, borrowed: a slice.
            Rvalue::Ref(_, place)
                if matches!(place.projection.last(), Some(Projection::Subslice(..))) =>
            {
                let Some(&Projection::Subslice(a, b)) = place.projection.last() else {
                    unreachable!("a subslice")
                };
                let base = Place {
                    local: place.local,
                    projection: place.projection[..place.projection.len() - 1].to_vec(),
                };
                self.subslice(&base, a, b)?
            }
            // An array borrowed as a slice: all of it.
            Rvalue::Unsize(op, _) if self.l.slice_elem(ty).is_some() => {
                let elem = self.l.slice_elem(ty).expect("a slice");
                let cap = array_len(&self.l.plain(&self.operand_ty(op)))
                    .ok_or("a slice of other than an array, in a kernel")?;
                let sty = self.l.slice_ty(&elem, cap)?;
                let Some(v) = self.operand(op)? else {
                    return Ok(None);
                };
                let zero = self.add(Expression::Literal(Literal::I32(0)));
                let n = self.add(Expression::Literal(Literal::I32(cap as i32)));
                Some(self.emit_expr(Expression::Compose {
                    ty: sty,
                    components: vec![v, zero, n],
                }))
            }
            Rvalue::Repeat(op, n) => {
                let Ty::Len(n) = self.concrete(n) else {
                    return Err("an array of unknown length in a kernel".to_string());
                };
                let Some(th) = self.l.ty(ty)? else {
                    return Ok(None);
                };
                let Some(v) = self.operand(op)? else {
                    return Ok(None);
                };
                Some(self.emit_expr(Expression::Compose {
                    ty: th,
                    components: vec![v; n as usize],
                }))
            }
            Rvalue::Length(n) => {
                let Ty::Len(n) = self.concrete(n) else {
                    return Err("a length not known in a kernel".to_string());
                };
                Some(self.const_wide(n)?)
            }
            Rvalue::Len(place) => {
                let n = self.len_of(place)?;
                Some(self.wide_of_i32(n)?)
            }
            // An `int` index is in bounds if its high word is 0 and its low below the length.
            Rvalue::InBounds(i, place) => {
                let i = self.operand(i)?.ok_or("an index")?;
                let n = self.len_of(place)?;
                let nu = self.emit_expr(Expression::As {
                    expr: n,
                    kind: ScalarKind::Uint,
                    convert: None,
                });
                let (lo, hi) = (self.wide_word(i, 0), self.wide_word(i, 1));
                let zero = self.lit(Literal::U32(0));
                let small = self.bin(BinaryOperator::Equal, hi, zero);
                let below = self.bin(BinaryOperator::Less, lo, nu);
                Some(self.bin(BinaryOperator::LogicalAnd, small, below))
            }
            Rvalue::SpanOk(a, b, place) => {
                let a = self.operand(a)?.ok_or("an index")?;
                let b = self.operand(b)?.ok_or("an index")?;
                let n = self.len_of(place)?;
                let nu = self.emit_expr(Expression::As {
                    expr: n,
                    kind: ScalarKind::Uint,
                    convert: None,
                });
                let zero = self.lit(Literal::U32(0));
                let (alo, ahi) = (self.wide_word(a, 0), self.wide_word(a, 1));
                let (blo, bhi) = (self.wide_word(b, 0), self.wide_word(b, 1));
                let a0 = self.bin(BinaryOperator::Equal, ahi, zero);
                let b0 = self.bin(BinaryOperator::Equal, bhi, zero);
                let c1 = self.bin(BinaryOperator::LessEqual, alo, blo);
                let c2 = self.bin(BinaryOperator::LessEqual, blo, nu);
                let x = self.bin(BinaryOperator::LogicalAnd, a0, b0);
                let y = self.bin(BinaryOperator::LogicalAnd, c1, c2);
                Some(self.bin(BinaryOperator::LogicalAnd, x, y))
            }
            Rvalue::Ref(_, place) => self
                .place(place)?
                .map(|(ptr, _)| self.emit_expr(Expression::Load { pointer: ptr })),
            Rvalue::Unary(op, a) => {
                let t = self.l.plain(&self.operand_ty(a));
                let Some(a) = self.operand(a)? else {
                    return Ok(None);
                };
                if t == Ty::Int {
                    let name = if *op == UnOp::Neg {
                        "wide_neg"
                    } else {
                        "wide_not"
                    };
                    return Ok(Some(self.call_wide(name, vec![a])?));
                }
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
                if ta == Ty::Int {
                    return Ok(Some(self.wide_binary(*op, a, b)?));
                }
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
            // Overflow is checked as on the CPU (`gpu.trap.1`).
            Rvalue::Overflows(op, a, b) => {
                let ta = self.l.plain(&self.operand_ty(a));
                let (Some(a), Some(b)) = (self.operand(a)?, self.operand(b)?) else {
                    return Err("an operator on `()` in a kernel".to_string());
                };
                Some(match ta {
                    Ty::Int => {
                        let (name, args) = match op {
                            BinOp::Add => ("wide_add_overflows", vec![a, b]),
                            BinOp::Sub => ("wide_sub_overflows", vec![a, b]),
                            BinOp::Mul => ("wide_mul_overflows", vec![a, b]),
                            BinOp::Div => ("wide_div_overflows", vec![a, b]),
                            BinOp::Shl => ("wide_shl_overflows", vec![a, b]),
                            BinOp::Shr => ("wide_shift_overflows", vec![b]),
                            _ => return Ok(Some(self.lit(Literal::Bool(false)))),
                        };
                        self.call_wide(name, args)?
                    }
                    Ty::Num(n) if !n.is_float() => self.small_overflows(*op, n, a, b),
                    _ => self.lit(Literal::Bool(false)),
                })
            }
            Rvalue::NegOverflows(a) => {
                let ta = self.l.plain(&self.operand_ty(a));
                let Some(a) = self.operand(a)? else {
                    return Ok(None);
                };
                Some(match ta {
                    Ty::Int => self.call_wide("wide_neg_overflows", vec![a])?,
                    Ty::Num(n) if n.is_signed() => {
                        let least = self.lit(Literal::I32(n.int_range().0 as i32));
                        self.bin(BinaryOperator::Equal, a, least)
                    }
                    _ => self.lit(Literal::Bool(false)),
                })
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
                    | AggregateKind::Closure(_)
                    | AggregateKind::Array => vals,
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
                let tag = self.emit_expr(Expression::Load { pointer: tag });
                Some(self.wide_of_i32(tag)?)
            }
            Rvalue::Widen(op, to) => {
                let to = self.concrete(to);
                let from = self.operand_ty(op);
                let Some(v) = self.operand(op)? else {
                    return Ok(None);
                };
                Some(self.convert(v, &from, &to)?)
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

    /// `a op b` on `int`s: a call of `core::gpu`'s function for it (`gpu.int.1`).
    fn wide_binary(
        &mut self,
        op: BinOp,
        a: Handle<Expression>,
        b: Handle<Expression>,
    ) -> Result<Handle<Expression>, String> {
        let not = |this: &mut Self, e| {
            this.emit_expr(Expression::Unary {
                op: UnaryOperator::LogicalNot,
                expr: e,
            })
        };
        Ok(match op {
            BinOp::Add => self.call_wide("wide_add", vec![a, b])?,
            BinOp::Sub => self.call_wide("wide_sub", vec![a, b])?,
            BinOp::Mul => self.call_wide("wide_mul", vec![a, b])?,
            BinOp::Div => self.call_wide("wide_div", vec![a, b])?,
            BinOp::Rem => self.call_wide("wide_rem", vec![a, b])?,
            BinOp::BitAnd => self.call_wide("wide_and", vec![a, b])?,
            BinOp::BitOr => self.call_wide("wide_or", vec![a, b])?,
            BinOp::BitXor => self.call_wide("wide_xor", vec![a, b])?,
            BinOp::Shl | BinOp::Shr => {
                let n = self.wide_word(b, 0);
                let name = if op == BinOp::Shl {
                    "wide_shl"
                } else {
                    "wide_shr"
                };
                self.call_wide(name, vec![a, n])?
            }
            BinOp::Eq => self.call_wide("wide_eq", vec![a, b])?,
            BinOp::Ne => {
                let e = self.call_wide("wide_eq", vec![a, b])?;
                not(self, e)
            }
            BinOp::Lt => self.call_wide("wide_lt", vec![a, b])?,
            BinOp::Gt => self.call_wide("wide_lt", vec![b, a])?,
            BinOp::Le => {
                let e = self.call_wide("wide_lt", vec![b, a])?;
                not(self, e)
            }
            BinOp::Ge => {
                let e = self.call_wide("wide_lt", vec![a, b])?;
                not(self, e)
            }
        })
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
            // A trap, or code that cannot run: the trap, and the instance stops (`gpu.trap.1`).
            Terminator::Trap(class, _) => {
                let range = self.l.at.map_or_else(TextRange::default, |(_, r)| r);
                self.trap_here(*class, range);
                Exit::Abort(self.var_value(0))
            }
            Terminator::Unreachable | Terminator::Resume | Terminator::Fail(_) => {
                let range = self.l.at.map_or_else(TextRange::default, |(_, r)| r);
                self.trap_here(TrapClass::Unreachable, range);
                Exit::Abort(self.var_value(0))
            }
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
                Ty::Slice(_) => "slice",
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
        // A slice's length.
        if owner == "slice" && name == "len" {
            let n = self.emit_expr(Expression::AccessIndex {
                base: *vals.first().ok_or("a slice")?,
                index: 2,
            });
            let v = self.wide_of_i32(n)?;
            store(self, v);
            return Ok(());
        }
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
                let from = self.operand_ty(&args[0]);
                let v = self.convert(vals[0], &from, &to)?;
                store(self, v);
                return Ok(());
            }
        }
        if sig.intrinsic || !sig.has_body {
            return Err(format!("the built-in function `{name}` in a kernel"));
        }
        // Each slice argument's capacity, which the function is lowered for.
        let caps = self.caps.clone();
        let arg_caps: Vec<Option<u32>> = args
            .iter()
            .map(|a| match a {
                Operand::Copy(p) | Operand::Move(p)
                    if self.l.slice_elem(&self.operand_ty(a)).is_some() =>
                {
                    self.place_cap(p, &caps)
                }
                _ => None,
            })
            .collect();
        let h = self.l.function(Item::Fn(f), &fargs, &arg_caps)?;
        let result = self.l.module.functions[h]
            .result
            .is_some()
            .then(|| self.add(Expression::CallResult(h)));
        self.out.push(S::Call {
            function: h,
            arguments: vals,
            result,
        });
        self.after_call();
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
        let h = self.l.function(item, &self.targs, &[])?;
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
        let (px, py) = (self.wide_of_i32(px)?, self.wide_of_i32(py)?);
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
        self.after_call();
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
        // Positions in the image are well within 32 bits.
        let (tx, ty, px, py) = (
            self.wide_i32(tx),
            self.wide_i32(ty),
            self.wide_i32(*px),
            self.wide_i32(*py),
        );
        let x = self.emit_expr(Expression::Binary {
            op: BinaryOperator::Add,
            left: tx,
            right: px,
        });
        let y = self.emit_expr(Expression::Binary {
            op: BinaryOperator::Add,
            left: ty,
            right: py,
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

/// The length of array type `t`, if it is one of a known length.
fn array_len(t: &Ty) -> Option<u32> {
    match t {
        Ty::Array(_, n) => match **n {
            Ty::Len(n) => u32::try_from(n).ok(),
            _ => None,
        },
        _ => None,
    }
}

//! A [`cranelift_module::Module`] whose functions become WASM: the native lowering builds into
//! it, each function is translated ([`crate::clif`]) as it is defined, and [`WasmModule::finish`]
//! lays out memory and writes the module.
//!
//! **Memory** is the runtime's (imported, shared with it): its static data below `__heap_base`;
//! then the program's scratch ([`Layout`]), its data objects at fixed addresses, and a shadow
//! stack of [`STACK_SIZE`] growing down; the runtime's allocator grows memory above all that.
//!
//! **Functions** are numbered imports first: the host's (`observe` and `log`), then the
//! runtime's the code calls, then the module's own. A function's address is its slot in the
//! table, its number plus one (0 is null). The table is the host's, so the shim module
//! ([`shim`]) can call the functions the runtime is given the addresses of: a task's code and a
//! value's drop glue.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use cranelift_codegen::control::ControlPlane;
use cranelift_codegen::ir;
use cranelift_codegen::isa::{OwnedTargetIsa, TargetIsa};
use cranelift_module::{
    DataDescription, DataId, FuncId, Init, Linkage, ModuleDeclarations, ModuleReloc,
    ModuleRelocTarget, ModuleResult,
};
use wasm_encoder::{
    CodeSection, ConstExpr, DataSection, ElementSection, Elements, EntityType, ExportKind,
    ExportSection, Function, FunctionSection, GlobalSection, GlobalType, ImportSection,
    Instruction as I, MemoryType, Module, RefType, TableType, TypeSection, ValType,
};

use crate::clif::{self, Body, Callee, Env, Op};
use crate::runtime::{Runtime, runtime};

/// The shadow stack's size.
pub const STACK_SIZE: u32 = 16 << 20;
/// How many words of the run's record the host can read (its trap's call stack is at most as
/// deep as the interpreters' limit).
pub const RECORD_WORDS: u32 = 10_016;

/// The runtime's functions the host gives instead: an observation goes to the host, and so does
/// the log, which the host writes.
const HOST: [&str; 2] = ["moddil_rt_observe", "moddil_rt_log"];

/// The runtime's functions for tasks and their isolated scopes: a module that calls them runs
/// only where the host has JSPI.
const TASKS: [&str; 22] = [
    "moddil_rt_scope_open",
    "moddil_rt_spawn",
    "moddil_rt_scope_run",
    "moddil_rt_stop",
    "moddil_rt_join",
    "moddil_rt_detach",
    "moddil_rt_yield_now",
    "moddil_rt_park",
    "moddil_rt_unpark",
    "moddil_rt_stop_requested",
    "moddil_rt_channel",
    "moddil_rt_timer",
    "moddil_rt_clone_sender",
    "moddil_rt_drop_sender",
    "moddil_rt_drop_receiver",
    "moddil_rt_send",
    "moddil_rt_recv",
    "moddil_rt_select",
    "moddil_rt_recv_now",
    "moddil_rt_send_now",
    "moddil_rt_join_now",
    "moddil_rt_trapped",
];

/// Where the program's scratch memory is: what the host passes entries and reads back.
#[derive(Clone, Copy, Debug)]
pub struct Layout {
    /// Each entry's result goes here.
    pub out: u32,
    /// And its error here.
    pub err: u32,
    /// The run's record ([`RECORD_WORDS`] words).
    pub record: u32,
    /// Where the data objects are, from and to: the host puts them back as they were before
    /// each entry, so one entry's `const` items are not another's.
    pub data: (u32, u32),
    /// How many pages of memory the module needs to start with.
    pub pages: u32,
    /// How many slots the table needs.
    pub table: u32,
    /// Where the site tables are: the count, the sites and their strings.
    pub sites: (u32, u32, u32),
    /// Whether the code uses tasks (and so needs JSPI).
    pub tasks: bool,
}

/// A module being built, its functions translated as they are defined.
pub struct WasmModule {
    isa: OwnedTargetIsa,
    decls: ModuleDeclarations,
    bodies: HashMap<FuncId, Body>,
    data: HashMap<DataId, DataDescription>,
    types: Vec<(Vec<ValType>, Vec<ValType>)>,
    /// The first thing that could not be translated.
    problem: Option<String>,
}

/// What a translation asks of the module, answered from its declarations.
struct Answers<'a> {
    decls: &'a ModuleDeclarations,
    types: &'a mut Vec<(Vec<ValType>, Vec<ValType>)>,
    runtime: &'static Runtime,
}

fn type_index(
    types: &mut Vec<(Vec<ValType>, Vec<ValType>)>,
    ps: &[ValType],
    rs: &[ValType],
) -> u32 {
    let key = (ps.to_vec(), rs.to_vec());
    if let Some(k) = types.iter().position(|t| *t == key) {
        return k as u32;
    }
    types.push(key);
    (types.len() - 1) as u32
}

impl Env for Answers<'_> {
    fn callee(&mut self, f: FuncId) -> Result<Callee, String> {
        let d = self.decls.get_function_decl(f);
        if d.linkage != Linkage::Import {
            return Ok(Callee::AsDeclared);
        }
        let name = d.linkage_name(f);
        if HOST.contains(&&*name) {
            return Ok(Callee::AsDeclared);
        }
        let (ps, rs) = self
            .runtime
            .funcs
            .get(&*name)
            .cloned()
            .ok_or_else(|| format!("the runtime's `{name}`"))?;
        Ok(Callee::Adapted(ps, rs))
    }

    fn type_index(&mut self, ps: &[ValType], rs: &[ValType]) -> u32 {
        type_index(self.types, ps, rs)
    }
}

impl WasmModule {
    /// A module with nothing in it yet.
    ///
    /// # Errors
    ///
    /// If Cranelift does not support the host (whose 64-bit target the lowering's IR is made
    /// for).
    pub fn new() -> Result<Self, String> {
        Ok(WasmModule {
            isa: moddil_native::backend::isa()?,
            decls: ModuleDeclarations::default(),
            bodies: HashMap::new(),
            data: HashMap::new(),
            types: Vec::new(),
            problem: None,
        })
    }

    /// Lays out memory and writes the module, with room for a result of `out` bytes and an error
    /// of `err` bytes: its bytes, and where its scratch is.
    ///
    /// # Errors
    ///
    /// The first thing in it that could not be translated.
    pub fn finish(mut self, out: u32, err: u32) -> Result<(Vec<u8>, Layout), String> {
        if let Some(p) = self.problem.take() {
            return Err(p);
        }
        let rt = runtime()?;
        let up = |n: u32, a: u32| n.div_ceil(a) * a;
        // The scratch: the runtime's `i128` results, the record, a result and an error.
        let base = up(rt.heap_base, 16);
        let i128_out = base;
        let record = base + 16;
        let out_at = up(record + 4 * RECORD_WORDS, 16);
        let err_at = up(out_at + out.max(16), 16);
        let mut at = up(err_at + err.max(16), 16);
        let data_start = at;
        // The data objects.
        let mut data_ids: Vec<DataId> = self.data.keys().copied().collect();
        data_ids.sort();
        let mut data_at = HashMap::new();
        for id in &data_ids {
            let d = &self.data[id];
            let align = d.align.unwrap_or(8).clamp(1, 1 << 12) as u32;
            at = up(at, align);
            data_at.insert(*id, at);
            at += d.init.size() as u32;
        }
        let stack_base = up(at, 16);
        let stack_top = stack_base + STACK_SIZE;
        let pages = stack_top.div_ceil(1 << 16);
        // The functions: the imports the code calls, then the module's own.
        let mut used: HashSet<FuncId> = HashSet::new();
        let mut used_rt: HashSet<&'static str> = HashSet::new();
        for b in self.bodies.values() {
            for op in &b.code {
                match op {
                    Op::Call(f) | Op::FuncAddr(f) => {
                        used.insert(*f);
                    }
                    Op::CallRuntime(n) => {
                        used_rt.insert(n);
                    }
                    _ => {}
                }
            }
        }
        let mut index: HashMap<FuncId, u32> = HashMap::new();
        let mut rt_index: HashMap<String, u32> = HashMap::new();
        let mut n = 0u32;
        let mut import_list: Vec<(&str, String, u32)> = Vec::new();
        let funcs: Vec<(FuncId, ir::Signature, Linkage, String)> = self
            .decls
            .get_functions()
            .map(|(id, d)| {
                (
                    id,
                    d.signature.clone(),
                    d.linkage,
                    d.linkage_name(id).into_owned(),
                )
            })
            .collect();
        // The host's, then the runtime's.
        for host in [true, false] {
            for (id, sig, linkage, name) in &funcs {
                if *linkage != Linkage::Import
                    || !used.contains(id)
                    || HOST.contains(&name.as_str()) != host
                {
                    continue;
                }
                let (ps, rs) = if host {
                    clif::signature(sig)?
                } else {
                    rt.funcs
                        .get(name)
                        .cloned()
                        .ok_or_else(|| format!("the runtime's `{name}`"))?
                };
                let ty = type_index(&mut self.types, &ps, &rs);
                let module = if host { "host" } else { "rt" };
                import_list.push((module, name.clone(), ty));
                rt_index.insert(name.clone(), n);
                index.insert(*id, n);
                n += 1;
            }
        }
        let mut extra: Vec<&'static str> = used_rt.into_iter().collect();
        extra.sort_unstable();
        for name in extra {
            if rt_index.contains_key(name) {
                continue;
            }
            let (ps, rs) = rt
                .funcs
                .get(name)
                .cloned()
                .ok_or_else(|| format!("the runtime's `{name}`"))?;
            let ty = type_index(&mut self.types, &ps, &rs);
            import_list.push(("rt", name.to_string(), ty));
            rt_index.insert(name.to_string(), n);
            n += 1;
        }
        let mut defined = Vec::new();
        for (id, _, linkage, name) in &funcs {
            if *linkage == Linkage::Import {
                continue;
            }
            let body = self
                .bodies
                .remove(id)
                .ok_or_else(|| format!("`{name}`, declared but not defined"))?;
            index.insert(*id, n);
            n += 1;
            defined.push((*id, *linkage, name.clone(), body));
        }
        let tasks = funcs.iter().any(|(id, _, linkage, name)| {
            *linkage == Linkage::Import && used.contains(id) && TASKS.contains(&name.as_str())
        });
        // Imports the code names but does not call (their addresses) are not imported.
        for f in &used {
            if !index.contains_key(f) {
                return Err("the address of a runtime function".to_string());
            }
        }
        let mut fs = FunctionSection::new();
        let mut code = CodeSection::new();
        for (_, _, _, body) in &defined {
            let ty = type_index(&mut self.types, &body.params, &body.results);
            fs.function(ty);
            let mut locals: Vec<(u32, ValType)> = Vec::new();
            for &l in &body.locals {
                match locals.last_mut() {
                    Some((k, t)) if *t == l => *k += 1,
                    _ => locals.push((1, l)),
                }
            }
            let mut f = Function::new(locals);
            for op in &body.code {
                let i = match op {
                    Op::I(i) => i.clone(),
                    Op::Call(g) => I::Call(index[g]),
                    Op::CallRuntime(name) => I::Call(rt_index[*name]),
                    Op::FuncAddr(g) => I::I64Const(i64::from(index[g]) + 1),
                    Op::DataAddr(d, off) => I::I64Const(
                        i64::from(*data_at.get(d).ok_or("data declared but not defined")?) + off,
                    ),
                };
                f.instruction(&i);
            }
            f.instruction(&I::End);
            code.function(&f);
        }
        // The data, each object's addresses written in.
        let mut ds = DataSection::new();
        for id in &data_ids {
            let d = &self.data[id];
            let mut bytes = match &d.init {
                Init::Bytes { contents } => contents.to_vec(),
                Init::Zeros { size } => vec![0; *size],
                Init::Uninitialized => return Err("a data object without contents".to_string()),
            };
            let mut write = |off: u32, v: u64| {
                let o = off as usize;
                if let Some(slot) = bytes.get_mut(o..o + 8) {
                    slot.copy_from_slice(&v.to_le_bytes());
                }
            };
            for (off, fr) in &d.function_relocs {
                let ModuleRelocTarget::User { index: f, .. } = d.function_decls[*fr] else {
                    return Err("a data object naming a library function".to_string());
                };
                let f = FuncId::from_u32(f);
                let slot = index
                    .get(&f)
                    .ok_or("a data object naming a runtime function")?;
                write(*off, u64::from(*slot) + 1);
            }
            for (off, gv, addend) in &d.data_relocs {
                let ModuleRelocTarget::User { index: t, .. } = d.data_decls[*gv] else {
                    return Err("a data object naming a library symbol".to_string());
                };
                let a = data_at[&DataId::from_u32(t)];
                write(*off, (i64::from(a) + addend) as u64);
            }
            if bytes.iter().any(|b| *b != 0) {
                ds.active(0, &ConstExpr::i32_const(data_at[id] as i32), bytes);
            }
        }
        // The table's size is known only now: its import says so.
        let mut imports = ImportSection::new();
        imports.import("env", "table", EntityType::Table(table_type(n + 1)));
        imports.import(
            "env",
            "memory",
            EntityType::Memory(MemoryType {
                minimum: u64::from(pages),
                maximum: None,
                memory64: false,
                shared: false,
                page_size_log2: None,
            }),
        );
        for (module, name, ty) in &import_list {
            imports.import(module, name, EntityType::Function(*ty));
        }
        let site = |name: &str| -> u32 {
            match self.decls.get_name(name) {
                Some(cranelift_module::FuncOrDataId::Data(d)) => {
                    data_at.get(&d).copied().unwrap_or(0)
                }
                _ => 0,
            }
        };
        let sites = (
            site("moddil_site_count"),
            site("moddil_sites"),
            site("moddil_strings"),
        );
        let mut m = Module::new();
        let mut ts = TypeSection::new();
        for (ps, rs) in &self.types {
            ts.ty().function(ps.iter().copied(), rs.iter().copied());
        }
        m.section(&ts);
        m.section(&imports);
        m.section(&fs);
        let mut gs = GlobalSection::new();
        for (init, mutable) in [(stack_top, true), (stack_base, false), (i128_out, false)] {
            gs.global(
                GlobalType {
                    val_type: ValType::I32,
                    mutable,
                    shared: false,
                },
                &ConstExpr::i32_const(init as i32),
            );
        }
        m.section(&gs);
        let mut es = ExportSection::new();
        for (id, linkage, name, _) in &defined {
            if *linkage == Linkage::Export {
                es.export(name, ExportKind::Func, index[id]);
            }
        }
        es.export("sp", ExportKind::Global, clif::SP);
        m.section(&es);
        let mut els = ElementSection::new();
        let all: Vec<u32> = (0..n).collect();
        els.active(
            Some(0),
            &ConstExpr::i32_const(1),
            Elements::Functions(Cow::Borrowed(&all)),
        );
        m.section(&els);
        m.section(&code);
        m.section(&ds);
        Ok((
            m.finish(),
            Layout {
                out: out_at,
                err: err_at,
                record,
                data: (data_start, stack_base),
                pages,
                table: n + 1,
                sites,
                tasks,
            },
        ))
    }
}

/// A table of `n` function references.
fn table_type(n: u32) -> TableType {
    TableType {
        element_type: RefType::FUNCREF,
        minimum: u64::from(n),
        maximum: None,
        table64: false,
        shared: false,
    }
}

/// The shim module: it imports the program's table and exports the functions the runtime calls
/// the program's code through, taking the runtime's wasm32 arguments (addresses as `i32`s):
/// `moddil_call_task(code, captures, arg, out, err)`, a task's code, and
/// `moddil_call_drop(glue, value)`, a value's drop glue. Each is a slot in the table.
pub fn shim() -> Vec<u8> {
    let mut m = Module::new();
    let mut ts = TypeSection::new();
    let (a, b) = (ValType::I32, ValType::I64);
    // 0: the task code's type; 1: the glue's; 2: `moddil_call_task`'s; 3: `moddil_call_drop`'s.
    ts.ty().function([b, b, b, b], [a]);
    ts.ty().function([b], [a]);
    ts.ty().function([b, b, b, a, a], [a]);
    ts.ty().function([a, a], [a]);
    m.section(&ts);
    let mut imports = ImportSection::new();
    imports.import("env", "table", EntityType::Table(table_type(1)));
    m.section(&imports);
    let mut fs = FunctionSection::new();
    fs.function(2);
    fs.function(3);
    m.section(&fs);
    let mut es = ExportSection::new();
    es.export("moddil_call_task", ExportKind::Func, 0);
    es.export("moddil_call_drop", ExportKind::Func, 1);
    m.section(&es);
    let mut code = CodeSection::new();
    let mut task = Function::new([]);
    for i in [
        I::LocalGet(1),
        I::LocalGet(2),
        I::LocalGet(3),
        I::I64ExtendI32U,
        I::LocalGet(4),
        I::I64ExtendI32U,
        I::LocalGet(0),
        I::I32WrapI64,
        I::CallIndirect {
            type_index: 0,
            table_index: 0,
        },
        I::End,
    ] {
        task.instruction(&i);
    }
    code.function(&task);
    let mut drop = Function::new([]);
    for i in [
        I::LocalGet(1),
        I::I64ExtendI32U,
        I::LocalGet(0),
        I::CallIndirect {
            type_index: 1,
            table_index: 0,
        },
        I::End,
    ] {
        drop.instruction(&i);
    }
    code.function(&drop);
    m.section(&code);
    m.finish()
}

impl cranelift_module::Module for WasmModule {
    fn isa(&self) -> &dyn TargetIsa {
        &*self.isa
    }

    fn declarations(&self) -> &ModuleDeclarations {
        &self.decls
    }

    fn declare_function(
        &mut self,
        name: &str,
        linkage: Linkage,
        signature: &ir::Signature,
    ) -> ModuleResult<FuncId> {
        let (id, _) = self.decls.declare_function(name, linkage, signature)?;
        Ok(id)
    }

    fn declare_anonymous_function(&mut self, signature: &ir::Signature) -> ModuleResult<FuncId> {
        self.decls.declare_anonymous_function(signature)
    }

    fn declare_data(
        &mut self,
        name: &str,
        linkage: Linkage,
        writable: bool,
        tls: bool,
    ) -> ModuleResult<DataId> {
        let (id, _) = self.decls.declare_data(name, linkage, writable, tls)?;
        Ok(id)
    }

    fn declare_anonymous_data(&mut self, writable: bool, tls: bool) -> ModuleResult<DataId> {
        self.decls.declare_anonymous_data(writable, tls)
    }

    fn define_function_with_control_plane(
        &mut self,
        func: FuncId,
        ctx: &mut cranelift_codegen::Context,
        _: &mut ControlPlane,
    ) -> ModuleResult<()> {
        if self.problem.is_some() {
            return Ok(());
        }
        let mut answers = Answers {
            decls: &self.decls,
            types: &mut self.types,
            runtime: match runtime() {
                Ok(r) => r,
                Err(e) => {
                    self.problem = Some(e);
                    return Ok(());
                }
            },
        };
        match clif::translate(&ctx.func, &mut answers) {
            Ok(body) => {
                self.bodies.insert(func, body);
            }
            Err(e) => self.problem = Some(e),
        }
        Ok(())
    }

    fn define_function_bytes(
        &mut self,
        _: FuncId,
        _: u64,
        _: &[u8],
        _: &[ModuleReloc],
    ) -> ModuleResult<()> {
        self.problem
            .get_or_insert_with(|| "a function as machine code".to_string());
        Ok(())
    }

    fn define_data(&mut self, data_id: DataId, data: &DataDescription) -> ModuleResult<()> {
        self.data.insert(data_id, data.clone());
        Ok(())
    }
}

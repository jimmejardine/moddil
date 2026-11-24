//! Calls between C and the MIR interpreter, through code Cranelift's JIT makes.
//!
//! - **Trampolines**, one per signature: a call of a C function by address, taking its
//!   arguments as words (a struct's word points to its padded copy) and room for its result.
//! - **Callbacks**, one per function and signature: a C function that, when C calls it, hands
//!   its arguments as words to the handler [`with_callbacks`] installed, and gives C the
//!   handler's result.

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use cranelift_codegen::ir::{
    AbiParam, ArgumentPurpose, InstBuilder, MemFlagsData, Signature, types,
};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Linkage, Module};

use crate::abi::{self, CLayout};

/// A C type, as a C function takes or gives it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum CType {
    /// `int8_t`, `uint8_t` or `bool`.
    I8,
    /// `int16_t` or `uint16_t`.
    I16,
    /// `int32_t` or `uint32_t`.
    I32,
    /// `int64_t` or `uint64_t`.
    I64,
    /// `float`.
    F32,
    /// `double`.
    F64,
    /// A pointer, a function pointer included.
    Ptr,
    /// Nothing: `void`, as a result only.
    Void,
    /// A struct by value (`ffi.types.2`).
    Struct(Arc<CLayout>),
}

impl CType {
    /// The type of a scalar's word in Cranelift's terms; `None` for `void` and structs.
    fn clif(&self) -> Option<types::Type> {
        Some(match self {
            // C's callers widen 8- and 16-bit integers to 32 bits; the word holds them widened.
            CType::I8 | CType::I16 | CType::I32 => types::I32,
            CType::I64 | CType::Ptr => types::I64,
            CType::F32 => types::F32,
            CType::F64 => types::F64,
            CType::Void | CType::Struct(_) => return None,
        })
    }
}

/// `fn(f, args, ret)`: calls `f` with the words at `args`, its result into the words at `ret`.
type Trampoline = unsafe extern "C" fn(*const u8, *const u64, *mut u64);

/// The code made so far, by signature (and for callbacks, by id), and the module that holds it
/// (kept for the process's life).
struct Made {
    module: JITModule,
    trampolines: HashMap<(Vec<CType>, CType), usize>,
    callbacks: HashMap<(u64, Vec<CType>, CType), usize>,
}

// SAFETY: the module is only used under the mutex.
unsafe impl Send for Made {}

static MADE: Mutex<Option<Made>> = Mutex::new(None);

fn new_module() -> Result<JITModule, String> {
    let mut flags = settings::builder();
    flags.set("opt_level", "speed").map_err(|e| e.to_string())?;
    let isa = cranelift_native::builder()
        .map_err(|e| e.to_string())?
        .finish(settings::Flags::new(flags))
        .map_err(|e| e.to_string())?;
    Ok(JITModule::new(JITBuilder::with_isa(
        isa,
        cranelift_module::default_libcall_names(),
    )))
}

/// Runs `f` with the code made so far.
fn with_made<R>(f: impl FnOnce(&mut Made) -> Result<R, String>) -> Result<R, String> {
    let mut guard = MADE.lock().map_err(|e| e.to_string())?;
    if guard.is_none() {
        *guard = Some(Made {
            module: new_module()?,
            trampolines: HashMap::new(),
            callbacks: HashMap::new(),
        });
    }
    f(guard.as_mut().expect("just made"))
}

/// The C signature of a function taking `params` and giving `ret`, in convention `cc`. A struct
/// result given through a hidden pointer makes that pointer the first parameter; a struct
/// argument is its parts, a pointer to a copy, or a copy in the stack's argument area
/// ([`abi::arg`]).
pub fn c_signature(
    cc: cranelift_codegen::isa::CallConv,
    params: &[CType],
    ret: &CType,
) -> Signature {
    let target = abi::target();
    let mut s = Signature::new(cc);
    if let CType::Struct(l) = ret {
        match abi::ret(target, l) {
            abi::Ret::Indirect => s
                .params
                .push(AbiParam::special(types::I64, ArgumentPurpose::StructReturn)),
            abi::Ret::Parts(parts) => {
                s.returns
                    .extend(parts.iter().map(|(_, t)| AbiParam::new(*t)));
            }
        }
    } else if let Some(t) = ret.clif() {
        s.returns.push(AbiParam::new(t));
    }
    for p in params {
        match p {
            CType::Struct(l) => match abi::arg(target, l) {
                abi::Arg::Parts(parts) => {
                    s.params
                        .extend(parts.iter().map(|(_, t)| AbiParam::new(*t)));
                }
                abi::Arg::Pointer => s.params.push(AbiParam::new(types::I64)),
                abi::Arg::Stack => s.params.push(AbiParam::special(
                    types::I64,
                    ArgumentPurpose::StructArgument(l.size),
                )),
            },
            p => s.params.push(AbiParam::new(p.clif().unwrap_or(types::I64))),
        }
    }
    s
}

/// Defines function `name` with signature `sig`, its body built by `body`: its code's address.
fn define(
    m: &mut JITModule,
    name: &str,
    sig: Signature,
    body: impl FnOnce(&mut FunctionBuilder<'_>, &[cranelift_codegen::ir::Value]),
) -> Result<usize, String> {
    let id = m
        .declare_function(name, Linkage::Local, &sig)
        .map_err(|e| e.to_string())?;
    let config = m.target_config();
    let mut ctx = m.make_context();
    ctx.func.signature = sig;
    let mut fctx = FunctionBuilderContext::new();
    {
        let mut b = FunctionBuilder::new(&mut ctx.func, &mut fctx);
        let entry = b.create_block();
        b.append_block_params_for_function_params(entry);
        b.switch_to_block(entry);
        b.seal_block(entry);
        let params = b.block_params(entry).to_vec();
        body(&mut b, &params);
        b.finalize(config);
    }
    m.define_function(id, &mut ctx).map_err(|e| e.to_string())?;
    m.finalize_definitions().map_err(|e| e.to_string())?;
    Ok(m.get_finalized_function(id) as usize)
}

/// The trampoline for C functions taking `params` and giving `ret`.
fn trampoline(made: &mut Made, params: &[CType], ret: &CType) -> Result<Trampoline, String> {
    let key = (params.to_vec(), ret.clone());
    if let Some(&p) = made.trampolines.get(&key) {
        // SAFETY: made below with this signature.
        return Ok(unsafe { std::mem::transmute::<usize, Trampoline>(p) });
    }
    if params.contains(&CType::Void) {
        return Err("a `void` parameter".to_string());
    }
    let m = &mut made.module;
    let cc = m.isa().default_call_conv();
    let callee = c_signature(cc, params, ret);
    let mut sig = Signature::new(cc);
    sig.params.extend([
        AbiParam::new(types::I64),
        AbiParam::new(types::I64),
        AbiParam::new(types::I64),
    ]);
    let name = format!("moddil_ffi_trampoline_{}", made.trampolines.len());
    let target = abi::target();
    let code = define(m, &name, sig, |b, ps| {
        let [f, args, out] = ps[..] else {
            unreachable!("three parameters")
        };
        let flags = MemFlagsData::trusted();
        let mut vals = Vec::new();
        let indirect = matches!(ret, CType::Struct(l) if abi::ret(target, l) == abi::Ret::Indirect);
        if indirect {
            vals.push(out);
        }
        for (i, p) in params.iter().enumerate() {
            let word = (i * 8) as i32;
            match p {
                CType::Struct(l) => {
                    let at = b.ins().load(types::I64, flags, args, word);
                    match abi::arg(target, l) {
                        abi::Arg::Parts(parts) => {
                            for (off, t) in parts {
                                vals.push(b.ins().load(t, flags, at, off as i32));
                            }
                        }
                        abi::Arg::Pointer | abi::Arg::Stack => vals.push(at),
                    }
                }
                p => {
                    let ty = p.clif().expect("checked above");
                    vals.push(b.ins().load(ty, flags, args, word));
                }
            }
        }
        let sref = b.import_signature(callee);
        let call = b.ins().call_indirect(sref, f, &vals);
        let results = b.inst_results(call).to_vec();
        match ret {
            CType::Struct(l) => {
                if let abi::Ret::Parts(parts) = abi::ret(target, l) {
                    for ((off, _), r) in parts.iter().zip(results) {
                        b.ins().store(flags, r, out, *off as i32);
                    }
                }
            }
            _ => {
                if let Some(&r) = results.first() {
                    b.ins().store(flags, r, out, 0);
                }
            }
        }
        b.ins().return_(&[]);
    })?;
    made.trampolines.insert(key, code);
    // SAFETY: the code just made has the trampoline's signature.
    Ok(unsafe { std::mem::transmute::<usize, Trampoline>(code) })
}

/// Calls the C function at `f`, taking `params` and giving `ret`, with `args` (one word each: an
/// integer or pointer in its low bits, a float's bits, or for a struct, the address of its copy
/// padded to a multiple of 8 bytes): its result's words (one for a scalar, 0 for `void`; a
/// struct's padded bytes).
///
/// # Errors
///
/// Why no trampoline could be made for the signature.
///
/// # Safety
///
/// `f` is a C function with exactly this signature, and the arguments are what it expects.
pub unsafe fn call(
    f: *const u8,
    params: &[CType],
    ret: &CType,
    args: &[u64],
) -> Result<Vec<u64>, String> {
    if args.len() != params.len() {
        return Err("a C call with the wrong number of arguments".to_string());
    }
    let tramp = with_made(|made| trampoline(made, params, ret))?;
    let words = match ret {
        CType::Struct(l) => (l.padded() / 8).max(1) as usize,
        _ => 1,
    };
    let mut out = vec![0u64; words];
    // SAFETY: as the caller promises; the trampoline reads `args.len()` words and writes at
    // most `words`.
    unsafe { tramp(f, args.as_ptr(), out.as_mut_ptr()) };
    Ok(out)
}

/// What a callback hands its arguments to: the callback's id and its arguments' words, giving
/// the result's word.
type Handler = dyn FnMut(u64, &[u64]) -> u64;

thread_local! {
    /// The handler [`with_callbacks`] installed on this thread, while it runs.
    static HANDLER: Cell<Option<*mut Handler>> = const { Cell::new(None) };
}

/// Runs `f` with `handler` answering the callbacks C makes on this thread meanwhile.
pub fn with_callbacks<R>(handler: &mut dyn FnMut(u64, &[u64]) -> u64, f: impl FnOnce() -> R) -> R {
    let p: *mut (dyn FnMut(u64, &[u64]) -> u64 + '_) = handler;
    // SAFETY: the pointer is used only while `f` runs, while `handler` is borrowed.
    let p = unsafe {
        std::mem::transmute::<
            *mut (dyn FnMut(u64, &[u64]) -> u64 + '_),
            *mut (dyn FnMut(u64, &[u64]) -> u64 + 'static),
        >(p)
    };
    let before = HANDLER.with(|h| h.replace(Some(p)));
    let r = f();
    HANDLER.with(|h| h.set(before));
    r
}

/// Where every callback's code goes: to the handler installed, or 0 with none.
extern "C" fn dispatch(id: u64, n: u64, args: *const u64, out: *mut u64) {
    // SAFETY: the callback's code passes its `n` argument words and room for its result.
    let words = unsafe { std::slice::from_raw_parts(args, n as usize) };
    let r = HANDLER
        .with(Cell::get)
        // SAFETY: installed by `with_callbacks`, which is still running.
        .map_or(0, |h| unsafe { (*h)(id, words) });
    // SAFETY: as above.
    unsafe { *out = r };
}

/// A C function taking `params` and giving `ret`, C's scalar types only, that hands its
/// arguments to the handler installed with `id` when C calls it: its address.
///
/// # Errors
///
/// A struct in the signature, or why the code could not be made.
pub fn callback(id: u64, params: &[CType], ret: &CType) -> Result<*const u8, String> {
    if params
        .iter()
        .chain([ret])
        .any(|t| matches!(t, CType::Struct(_)))
        || params.contains(&CType::Void)
    {
        return Err("a callback takes and gives C's scalar types only".to_string());
    }
    let key = (id, params.to_vec(), ret.clone());
    let code = with_made(|made| {
        if let Some(&p) = made.callbacks.get(&key) {
            return Ok(p);
        }
        let m = &mut made.module;
        let cc = m.isa().default_call_conv();
        let sig = c_signature(cc, params, ret);
        let name = format!("moddil_ffi_callback_{}", made.callbacks.len());
        let code = define(m, &name, sig, |b, ps| {
            let flags = MemFlagsData::trusted();
            let n = ps.len().max(1) as u32;
            let slot = b.create_sized_stack_slot(cranelift_codegen::ir::StackSlotData::new(
                cranelift_codegen::ir::StackSlotKind::ExplicitSlot,
                8 * (n + 1),
                3,
            ));
            let args = b.ins().stack_addr(types::I64, slot, 0);
            let zero = b.ins().iconst(types::I64, 0);
            for k in 0..=n {
                b.ins().store(flags, zero, args, (k * 8) as i32);
            }
            for (k, v) in ps.iter().enumerate() {
                b.ins().store(flags, *v, args, (k * 8) as i32);
            }
            let out = b.ins().stack_addr(types::I64, slot, (n * 8) as i32);
            let mut dsig = Signature::new(cc);
            dsig.params.extend([AbiParam::new(types::I64); 4]);
            let dref = b.import_signature(dsig);
            let target = b
                .ins()
                .iconst(types::I64, dispatch as *const () as usize as i64);
            let idv = b.ins().iconst(types::I64, id as i64);
            let count = b.ins().iconst(types::I64, ps.len() as i64);
            b.ins()
                .call_indirect(dref, target, &[idv, count, args, out]);
            match ret.clif() {
                Some(t) => {
                    let r = b.ins().load(t, flags, out, 0);
                    b.ins().return_(&[r]);
                }
                None => {
                    b.ins().return_(&[]);
                }
            }
        })?;
        made.callbacks.insert(key, code);
        Ok(code)
    })?;
    Ok(code as *const u8)
}

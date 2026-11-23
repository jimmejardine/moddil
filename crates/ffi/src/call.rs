//! Calls of C functions by address, for the MIR interpreter: a trampoline per signature, made
//! by Cranelift's JIT, takes the function, its arguments as words and room for its result.

use std::collections::HashMap;
use std::sync::Mutex;

use cranelift_codegen::ir::{AbiParam, InstBuilder, MemFlagsData, Signature, types};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Linkage, Module};

/// A C type, as a C function takes or gives it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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
    /// A pointer.
    Ptr,
    /// Nothing: `void`, as a result only.
    Void,
}

impl CType {
    fn clif(self) -> Option<types::Type> {
        Some(match self {
            // C's callers widen 8- and 16-bit integers to 32 bits; the word holds them widened.
            CType::I8 | CType::I16 | CType::I32 => types::I32,
            CType::I64 | CType::Ptr => types::I64,
            CType::F32 => types::F32,
            CType::F64 => types::F64,
            CType::Void => return None,
        })
    }
}

/// `fn(f, args, ret)`: calls `f` with the words at `args`, its result into the word at `ret`.
type Trampoline = unsafe extern "C" fn(*const u8, *const u64, *mut u64);

/// The trampolines made so far, by signature, and the module that holds their code (kept for the
/// process's life).
struct Trampolines {
    module: JITModule,
    made: HashMap<(Vec<CType>, CType), usize>,
}

// SAFETY: the module is only used under the mutex.
unsafe impl Send for Trampolines {}

static TRAMPOLINES: Mutex<Option<Trampolines>> = Mutex::new(None);

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

/// The trampoline for C functions taking `params` and giving `ret`.
fn trampoline(t: &mut Trampolines, params: &[CType], ret: CType) -> Result<Trampoline, String> {
    if let Some(&p) = t.made.get(&(params.to_vec(), ret)) {
        // SAFETY: made below with this signature.
        return Ok(unsafe { std::mem::transmute::<usize, Trampoline>(p) });
    }
    let m = &mut t.module;
    let cc = m.isa().default_call_conv();
    let ptr = types::I64;
    // The C function's own signature.
    let mut callee = Signature::new(cc);
    for p in params {
        callee
            .params
            .push(AbiParam::new(p.clif().ok_or("a `void` parameter")?));
    }
    if let Some(r) = ret.clif() {
        callee.returns.push(AbiParam::new(r));
    }
    // The trampoline's: the function, the arguments' words, the result's word.
    let mut sig = Signature::new(cc);
    sig.params
        .extend([AbiParam::new(ptr), AbiParam::new(ptr), AbiParam::new(ptr)]);
    let name = format!("moddil_ffi_trampoline_{}", t.made.len());
    let id = m
        .declare_function(&name, Linkage::Local, &sig)
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
        let [f, args, out] = b.block_params(entry)[..] else {
            unreachable!("three parameters")
        };
        let flags = MemFlagsData::trusted();
        let mut vals = Vec::new();
        for (i, p) in params.iter().enumerate() {
            let ty = p.clif().expect("checked above");
            vals.push(b.ins().load(ty, flags, args, (i * 8) as i32));
        }
        let sref = b.import_signature(callee);
        let call = b.ins().call_indirect(sref, f, &vals);
        if let Some(&r) = b.inst_results(call).first() {
            b.ins().store(flags, r, out, 0);
        }
        b.ins().return_(&[]);
        b.finalize(config);
    }
    m.define_function(id, &mut ctx).map_err(|e| e.to_string())?;
    m.finalize_definitions().map_err(|e| e.to_string())?;
    let code = m.get_finalized_function(id) as usize;
    t.made.insert((params.to_vec(), ret), code);
    // SAFETY: the code just made has the trampoline's signature.
    Ok(unsafe { std::mem::transmute::<usize, Trampoline>(code) })
}

/// Calls the C function at `f`, taking `params` and giving `ret`, with `args` (one word each: an
/// integer or pointer in its low bits, a float's bits): its result's word (0 for `void`).
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
    ret: CType,
    args: &[u64],
) -> Result<u64, String> {
    if args.len() != params.len() {
        return Err("a C call with the wrong number of arguments".to_string());
    }
    let tramp = {
        let mut guard = TRAMPOLINES.lock().map_err(|e| e.to_string())?;
        if guard.is_none() {
            *guard = Some(Trampolines {
                module: new_module()?,
                made: HashMap::new(),
            });
        }
        trampoline(guard.as_mut().expect("just made"), params, ret)?
    };
    let mut out: u64 = 0;
    // SAFETY: as the caller promises; the trampoline reads `args.len()` words.
    unsafe { tramp(f, args.as_ptr(), &raw mut out) };
    Ok(out)
}

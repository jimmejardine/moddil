//! Running compiled code: the one place `unsafe` is allowed, to call the code Cranelift made.

#![allow(unsafe_code)]

use moddil_sema::{Num, Ty};

use crate::lower::{Compiled, Site};
use crate::{Native, Observed};

/// How much stack a run has: deep enough for the frames `moddil-rt` lets a run have.
const STACK: usize = 512 * 1024 * 1024;

impl Compiled {
    /// Runs the entry point, on a thread of its own with a large stack: the values it observed,
    /// and its result as Moddil source, or the trap.
    pub(crate) fn run(self) -> (Vec<Observed>, Result<String, Native>) {
        let ptr = self.module.get_finalized_function(self.entry) as usize;
        let ret = self.ret.clone();
        let (status, out, record) = std::thread::scope(|s| {
            std::thread::Builder::new()
                .stack_size(STACK)
                .spawn_scoped(s, move || {
                    moddil_rt::take();
                    let mut out = [0u64; 2];
                    // SAFETY: `ptr` is the finished entry trampoline, whose signature is
                    // `fn(*mut [u64; 2]) -> i8` in the platform's C calling convention, and the
                    // module that owns its memory lives until this scope ends.
                    let entry: extern "C" fn(*mut [u64; 2]) -> i8 =
                        unsafe { std::mem::transmute::<usize, _>(ptr) };
                    let status = entry(&raw mut out);
                    (status, out, moddil_rt::take())
                })
                .expect("a thread to run on")
                .join()
                .expect("native code does not panic")
        });
        let observed = record
            .observed
            .iter()
            .filter_map(|&(site, lo, hi)| match &self.sites[site as usize] {
                Site::Observe(file, range, ty) => Some((
                    *file,
                    *range,
                    render(ty, u128::from(lo) | (u128::from(hi) << 64)),
                )),
                Site::Trap(..) => None,
            })
            .collect();
        let result = if status == 0 {
            Ok(render(
                &ret,
                u128::from(out[0]) | (u128::from(out[1]) << 64),
            ))
        } else {
            match record.trap.map(|s| &self.sites[s as usize]) {
                Some(Site::Trap(class, file, range)) => Err(Native::Trap(*class, *file, *range)),
                _ => Err(Native::Unsupported("a trap without a site".to_string())),
            }
        };
        // SAFETY: nothing runs the module's code after this.
        unsafe { self.module.free_memory() };
        (observed, result)
    }
}

/// A value of `ty` with these bits, as the interpreters write it.
fn render(ty: &Ty, bits: u128) -> String {
    match ty {
        Ty::Unit => "()".to_string(),
        Ty::Int => (bits as u64 as i64).to_string(),
        Ty::Bool => (bits & 1 == 1).to_string(),
        Ty::Char => {
            char::from_u32(bits as u32).map_or_else(|| "?".to_string(), |c| format!("{c:?}"))
        }
        Ty::Num(n) => {
            let width = n.bits();
            let masked = if width == 128 {
                bits
            } else {
                bits & ((1u128 << width) - 1)
            };
            let masked = if *n == Num::F64 || *n == Num::F32 {
                bits
            } else {
                masked
            };
            moddil_mir::num_text(*n, masked)
        }
        t => format!("<{t}>"),
    }
}

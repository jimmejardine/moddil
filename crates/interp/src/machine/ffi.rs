//! Calls of C functions (`ffi.extern.1`) from the MIR interpreter, through `moddil-ffi`'s
//! trampolines, and the callbacks C makes into it meanwhile (`ffi.native.2`).
//!
//! The interpreter's memory is its own (a borrow is a path into a frame, not an address), so a
//! pointer argument is **marshalled**: what it points to, and the rest of the array it points
//! into, is written in C's layout into real memory for the call, and read back after it. A
//! struct argument is written so too, and a struct result read back. What crosses otherwise is
//! [`crate::cmem`]'s: C's addresses, handles, and callbacks.

#![allow(unsafe_code)]

use moddil_sema::Ty;

use super::{Machine, Stop};
use crate::cmem::{self, ctype, size_align};
use crate::stack::{Frame, Ret};
use crate::value::{Pointer, Value};
use moddil_sema::Item;

/// A pointer argument's memory for the call: where its values came from, and their bytes.
struct Marshalled {
    /// The place: one value, or elements `start..` of the array at `parent`.
    place: Place,
    ty: Ty,
    stride: usize,
    count: usize,
    /// The bytes, as words so that any C alignment up to 8 holds.
    words: Vec<u64>,
}

enum Place {
    One(Pointer),
    Elements(Pointer, u32),
}

/// The C type of value `v`, if C can read it: a number, a `bool`, a `#[repr(C)]` struct (not
/// generic), or an array of those (its element's type).
fn c_type_of(items: &moddil_sema::ItemTree, v: &Value) -> Option<Ty> {
    match v {
        Value::Int(_) => Some(Ty::Int),
        Value::Bool(_) => Some(Ty::Bool),
        Value::Num(n, _) if !matches!(n, moddil_sema::Num::I128 | moddil_sema::Num::U128) => {
            Some(Ty::Num(*n))
        }
        Value::Adt { adt, .. } => {
            let def = &items.adts[*adt as usize];
            let ty = Ty::Adt(*adt, def.name.as_str().into(), Vec::new().into());
            (def.repr_c && items.is_c_type(&ty)).then_some(ty)
        }
        Value::Array(xs) | Value::Buf(_, xs) => xs.first().and_then(|x| c_type_of(items, x)),
        _ => None,
    }
}

/// Why a C call did not give a value.
pub(super) enum CallStop {
    /// It could not be made.
    Failed(String),
    /// A callback it made trapped.
    Stopped(Stop),
}

impl From<String> for CallStop {
    fn from(why: String) -> Self {
        CallStop::Failed(why)
    }
}

impl Machine<'_> {
    /// Calls C function `f` with `values`: its result.
    pub(super) fn c_call(&mut self, f: u32, values: &[Value]) -> Result<Value, CallStop> {
        let items = self.dispatch.items();
        let sig = &items.fns[f as usize];
        let addr = moddil_ffi::lookup(&sig.name)
            .ok_or_else(|| format!("no C library this run has loaded defines `{}`", sig.name))?;
        let params: Vec<Ty> = sig.params.iter().map(|p| p.ty.clone()).collect();
        let ret = sig.ret.clone();
        self.c_call_at(addr, &params, &ret, values)
    }

    /// The value of `Rvalue::FnPtr(f)` (`type.fnptr.1`): a C function's address, or the
    /// Moddil function, which crosses to C as a callback.
    pub(super) fn fn_ptr(&self, f: u32) -> Value {
        let sig = &self.dispatch.items().fns[f as usize];
        if sig.foreign {
            Value::Foreign(moddil_ffi::lookup(&sig.name).map_or(0, |a| a as u64))
        } else {
            Value::FnPtr(f)
        }
    }

    /// Calls the C function at `addr`, taking `params` and giving `ret`, with `values`.
    pub(super) fn c_call_at(
        &mut self,
        addr: *const u8,
        params: &[Ty],
        ret: &Ty,
        values: &[Value],
    ) -> Result<Value, CallStop> {
        let items = self.dispatch.items();
        let mut marshalled = Vec::new();
        let mut structs: Vec<Vec<u64>> = Vec::new();
        let mut words = Vec::new();
        let mut types = Vec::new();
        for (v, ty) in values.iter().zip(params) {
            let ct = ctype(items, ty)?;
            // A `void *` to a value C can read is marshalled as that value; to another, it is a
            // handle.
            let pointee = match (ty, v) {
                (Ty::Ptr(_, inner), Value::Ref(p, _)) if **inner == Ty::Unit => {
                    c_type_of(items, &self.stack.load(p))
                }
                (Ty::Ptr(_, inner), Value::Ref(..)) => Some((**inner).clone()),
                _ => None,
            };
            if let (Some(inner), Value::Ref(p, _)) = (pointee, v) {
                let m = self.marshal(p, &inner)?;
                words.push(m.words.as_ptr() as u64);
                marshalled.push(m);
                types.push(ct);
                continue;
            }
            match (ty, v) {
                (Ty::Adt(..), v) => {
                    let (size, _) = size_align(items, ty)?;
                    let mut bytes = vec![0u8; size.div_ceil(8) * 8 + 8];
                    self.stack.cmem().write(items, &mut bytes, 0, ty, v)?;
                    let w = cmem::to_words(&bytes);
                    words.push(w.as_ptr() as u64);
                    structs.push(w);
                }
                _ => words.push(self.stack.cmem().word(items, ty, v)?),
            }
            types.push(ct);
        }
        let rt = ctype(items, ret)?;
        let this: *mut Self = self;
        let mut stopped: Option<Stop> = None;
        let mut handler = |id: u64, args: &[u64]| {
            // SAFETY: the machine is waiting in this call, and C calls back on this thread.
            unsafe { (*this).callback(id as u32, args, &mut stopped) }
        };
        // SAFETY: the address is the C function the `extern` block (or a `fn` value) names,
        // called with the signature it gives; pointer arguments point into `marshalled` and
        // `structs`, alive across the call.
        let out = moddil_ffi::with_callbacks(&mut handler, || unsafe {
            moddil_ffi::call(addr, &types, &rt, &words)
        })?;
        drop(structs);
        for m in marshalled {
            self.unmarshal(m)?;
        }
        if let Some(stop) = stopped {
            return Err(CallStop::Stopped(stop));
        }
        Ok(match ret {
            Ty::Adt(..) => self
                .stack
                .cmem()
                .read(items, &cmem::to_bytes(&out), 0, ret)?,
            _ => self.stack.cmem().value(items, ret, out[0]),
        })
    }

    /// Runs the callback C made to function `f` with `args`, while a C call runs: its result's
    /// word. A trap is kept in `stopped`, and C gets zeros from then on.
    fn callback(&mut self, f: u32, args: &[u64], stopped: &mut Option<Stop>) -> u64 {
        if stopped.is_some() {
            return 0;
        }
        let items = self.dispatch.items();
        let sig = &items.fns[f as usize];
        let values: Vec<Value> = sig
            .params
            .iter()
            .zip(args)
            .map(|(p, w)| self.stack.cmem().value(items, &p.ty, *w))
            .collect();
        let ret = sig.ret.clone();
        match self.run_callback(f, values) {
            Ok(v) if ret == Ty::Unit => {
                drop(v);
                0
            }
            Ok(v) => self.stack.cmem().word(items, &ret, &v).unwrap_or(0),
            Err(stop) => {
                *stopped = Some(stop);
                0
            }
        }
    }

    /// Runs function `f` with `values` to its end, above the frames already running: its
    /// result, or why it stopped (with the frames it pushed gone).
    fn run_callback(&mut self, f: u32, values: Vec<Value>) -> Result<Value, Stop> {
        let depth = self.stack.depth();
        let code = self.body(Item::Fn(f));
        self.stack
            .push(Frame::new(code, values, Vec::new(), Ret::Callback));
        let r = self.run_to_end(&mut |_, _, _| {});
        if r.is_err() {
            self.stack.unwind_to(depth);
        }
        r
    }

    /// The memory for a pointer `p` to a `ty`: the value it points to, and, if it points into
    /// an array, the elements after it.
    fn marshal(&mut self, p: &Pointer, ty: &Ty) -> Result<Marshalled, String> {
        let items = self.dispatch.items();
        let (size, align) = size_align(items, ty)?;
        let stride = size.div_ceil(align) * align;
        let mut values = Vec::new();
        let place = match p.path.split_last() {
            Some((&last, rest)) => {
                let parent = Pointer {
                    path: rest.to_vec(),
                    span: None,
                    ..p.clone()
                };
                match self.stack.load(&parent) {
                    Value::Array(es) | Value::Buf(_, es) => {
                        values.extend(es.into_iter().skip(last as usize));
                        Place::Elements(parent, last)
                    }
                    _ => {
                        values.push(self.stack.load(p));
                        Place::One(p.clone())
                    }
                }
            }
            None => {
                values.push(self.stack.load(p));
                Place::One(p.clone())
            }
        };
        let count = values.len();
        let mut bytes = vec![0u8; (stride * count).max(8)];
        for (k, v) in values.iter().enumerate() {
            self.stack
                .cmem()
                .write(items, &mut bytes, k * stride, ty, v)?;
        }
        Ok(Marshalled {
            place,
            ty: ty.clone(),
            stride,
            count,
            words: cmem::to_words(&bytes),
        })
    }

    /// Reads a pointer argument's memory back into the places it came from.
    fn unmarshal(&mut self, m: Marshalled) -> Result<(), String> {
        let items = self.dispatch.items();
        let bytes = cmem::to_bytes(&m.words);
        match m.place {
            Place::One(p) => {
                let v = self.stack.cmem().read(items, &bytes, 0, &m.ty)?;
                self.stack.store(&p, v);
            }
            Place::Elements(parent, start) => {
                for k in 0..m.count {
                    let v = self.stack.cmem().read(items, &bytes, k * m.stride, &m.ty)?;
                    let mut path = parent.path.clone();
                    path.push(start + k as u32);
                    let p = Pointer {
                        path,
                        span: None,
                        ..parent.clone()
                    };
                    self.stack.store(&p, v);
                }
            }
        }
        Ok(())
    }
}

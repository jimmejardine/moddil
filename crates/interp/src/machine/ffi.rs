//! Calls of C functions (`ffi.extern.1`) from the MIR interpreter, through `moddil-ffi`'s
//! trampolines.
//!
//! The interpreter's memory is its own (a borrow is a path into a frame, not an address), so a
//! pointer argument is **marshalled**: what it points to, and the rest of the array it points
//! into, is written in C's layout into real memory for the call, and read back after it. A
//! pointer C gives is kept as its address ([`Value::Foreign`]) and passed back as it is; reading
//! through one is native code's alone.

#![allow(unsafe_code)]

use moddil_ffi::CType;
use moddil_sema::{ItemTree, Num, Ty};

use super::Machine;
use crate::value::{Pointer, Value};

/// C's size and alignment of a value of `ty`, as `moddil-ffi` lays it out.
fn size_align(items: &ItemTree, ty: &Ty) -> Result<(usize, usize), String> {
    Ok(match ty {
        Ty::Bool => (1, 1),
        Ty::Int | Ty::Ptr(..) => (8, 8),
        Ty::Num(n) => {
            let s = match n {
                Num::I8 | Num::U8 => 1,
                Num::I16 | Num::U16 => 2,
                Num::I32 | Num::U32 | Num::F32 => 4,
                Num::U64 | Num::F64 => 8,
                Num::I128 | Num::U128 => return Err("a 128-bit integer passed to C".to_string()),
            };
            (s, s)
        }
        Ty::Adt(a, ..) if items.adts[*a as usize].repr_c => {
            let mut off: usize = 0;
            let mut align = 1;
            for f in &items.adts[*a as usize].variants[0].fields {
                let (s, al) = size_align(items, &f.ty)?;
                off = off.div_ceil(al) * al + s;
                align = align.max(al);
            }
            (off.div_ceil(align) * align, align)
        }
        t => return Err(format!("a `{t}` behind a pointer passed to C")),
    })
}

/// `ty` as `moddil-ffi` calls with it.
fn ctype(ty: &Ty) -> Result<CType, String> {
    Ok(match ty {
        Ty::Unit => CType::Void,
        Ty::Bool => CType::I8,
        Ty::Int => CType::I64,
        Ty::Ptr(..) => CType::Ptr,
        Ty::Num(n) => match n {
            Num::I8 | Num::U8 => CType::I8,
            Num::I16 | Num::U16 => CType::I16,
            Num::I32 | Num::U32 => CType::I32,
            Num::U64 => CType::I64,
            Num::F32 => CType::F32,
            Num::F64 => CType::F64,
            Num::I128 | Num::U128 => return Err("a 128-bit integer passed to C".to_string()),
        },
        t => return Err(format!("a `{t}` passed to C")),
    })
}

/// Scalar `v` of type `ty` as a word: an integer widened as its sign says, a float's bits, an
/// address.
fn word(ty: &Ty, v: &Value) -> Result<u64, String> {
    Ok(match (ty, v) {
        (Ty::Int, Value::Int(i)) => *i as u64,
        (Ty::Bool, Value::Bool(b)) => u64::from(*b),
        (Ty::Num(n), Value::Num(_, bits)) => {
            let bits = *bits as u64;
            match n {
                Num::I8 => bits as u8 as i8 as i64 as u64,
                Num::I16 => bits as u16 as i16 as i64 as u64,
                Num::I32 => bits as u32 as i32 as i64 as u64,
                _ => bits,
            }
        }
        (Ty::Ptr(..), Value::Foreign(a)) => *a,
        (Ty::Ptr(..), Value::Ref(..)) => {
            return Err("a pointer to a Moddil value inside what is passed to C".to_string());
        }
        (t, v) => return Err(format!("a `{t}` passed to C as {v:?}")),
    })
}

/// The value of type `ty` that word `w` holds.
fn from_word(ty: &Ty, w: u64) -> Value {
    match ty {
        Ty::Int => Value::Int(w as i64),
        Ty::Bool => Value::Bool(w & 0xff != 0),
        Ty::Ptr(..) => Value::Foreign(w),
        Ty::Num(n) => {
            let mask: u128 = match n {
                Num::I8 | Num::U8 => 0xff,
                Num::I16 | Num::U16 => 0xffff,
                Num::I32 | Num::U32 | Num::F32 => 0xffff_ffff,
                _ => u128::from(u64::MAX),
            };
            Value::Num(*n, u128::from(w) & mask)
        }
        _ => Value::Unit,
    }
}

/// Writes `v`, of type `ty`, at `at` of `buf` in C's layout.
fn write(items: &ItemTree, buf: &mut [u8], at: usize, ty: &Ty, v: &Value) -> Result<(), String> {
    if let (Ty::Adt(a, ..), Value::Adt { fields, .. }) = (ty, v) {
        let mut off: usize = 0;
        for (f, fv) in items.adts[*a as usize].variants[0]
            .fields
            .iter()
            .zip(fields)
        {
            let (s, al) = size_align(items, &f.ty)?;
            off = off.div_ceil(al) * al;
            write(items, buf, at + off, &f.ty, fv)?;
            off += s;
        }
        return Ok(());
    }
    // An out-parameter not assigned yet: zeros.
    if matches!(v, Value::Empty) {
        return Ok(());
    }
    let (s, _) = size_align(items, ty)?;
    let w = word(ty, v)?;
    buf[at..at + s].copy_from_slice(&w.to_le_bytes()[..s]);
    Ok(())
}

/// The value of type `ty` at `at` of `buf`, in C's layout.
fn read(
    items: &ItemTree,
    buf: &[u8],
    at: usize,
    ty: &Ty,
    adt_variant: Option<u32>,
) -> Result<Value, String> {
    if let Ty::Adt(a, ..) = ty {
        let mut off: usize = 0;
        let mut fields = Vec::new();
        for f in &items.adts[*a as usize].variants[0].fields {
            let (s, al) = size_align(items, &f.ty)?;
            off = off.div_ceil(al) * al;
            fields.push(read(items, buf, at + off, &f.ty, None)?);
            off += s;
        }
        return Ok(Value::Adt {
            adt: *a,
            variant: adt_variant.unwrap_or(0),
            fields,
        });
    }
    let (s, _) = size_align(items, ty)?;
    let mut w = [0u8; 8];
    w[..s].copy_from_slice(&buf[at..at + s]);
    Ok(from_word(ty, u64::from_le_bytes(w)))
}

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

impl Machine<'_> {
    /// Calls C function `f` with `values`: its result.
    pub(super) fn c_call(&mut self, f: u32, values: &[Value]) -> Result<Value, String> {
        let items = self.dispatch.items();
        let sig = &items.fns[f as usize];
        let addr = moddil_ffi::lookup(&sig.name)
            .ok_or_else(|| format!("no C library this run has loaded defines `{}`", sig.name))?;
        let params: Vec<Ty> = sig.params.iter().map(|p| p.ty.clone()).collect();
        let ret = sig.ret.clone();
        let mut marshalled = Vec::new();
        let mut words = Vec::new();
        let mut types = Vec::new();
        for (v, ty) in values.iter().zip(&params) {
            types.push(ctype(ty)?);
            match (ty, v) {
                (Ty::Ptr(_, inner), Value::Ref(p, _)) => {
                    let m = self.marshal(p, inner)?;
                    words.push(m.words.as_ptr() as u64);
                    marshalled.push(m);
                }
                _ => words.push(word(ty, v)?),
            }
        }
        // SAFETY: the symbol is the C function the `extern` block declares, called with the
        // signature it gives; pointer arguments point into `marshalled`, alive across the call.
        let w = unsafe { moddil_ffi::call(addr, &types, ctype(&ret)?, &words) }?;
        for m in marshalled {
            self.unmarshal(m)?;
        }
        Ok(from_word(&ret, w))
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
            write(items, &mut bytes, k * stride, ty, v)?;
        }
        let mut words = vec![0u64; bytes.len().div_ceil(8)];
        for (i, chunk) in bytes.chunks(8).enumerate() {
            let mut w = [0u8; 8];
            w[..chunk.len()].copy_from_slice(chunk);
            words[i] = u64::from_le_bytes(w);
        }
        Ok(Marshalled {
            place,
            ty: ty.clone(),
            stride,
            count,
            words,
        })
    }

    /// Reads a pointer argument's memory back into the places it came from.
    fn unmarshal(&mut self, m: Marshalled) -> Result<(), String> {
        let items = self.dispatch.items();
        let bytes: Vec<u8> = m.words.iter().flat_map(|w| w.to_le_bytes()).collect();
        match m.place {
            Place::One(p) => {
                let v = read(items, &bytes, 0, &m.ty, None)?;
                self.stack.store(&p, v);
            }
            Place::Elements(parent, start) => {
                for k in 0..m.count {
                    let v = read(items, &bytes, k * m.stride, &m.ty, None)?;
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

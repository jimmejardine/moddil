//! C's memory as the MIR interpreter sees it (`ffi.native.1`, `ffi.native.2`): values written in
//! C's layout and read back, and what a run gives C that has no address of its own.
//!
//! The interpreter's memory is its own (a borrow is a path into a frame, not an address), so
//! - a Moddil pointer given C as `void *` (`*const ()` or `*mut ()`) crosses as a **handle**:
//!   an address in a range no real pointer uses, which turns back into the pointer when C gives
//!   it back;
//! - a Moddil function given C as a function pointer crosses as a **callback**, a C function
//!   `moddil-ffi` makes that hands its arguments back to the interpreter;
//! - copies made for C that last the run (`ffi::ptr_of`) are **kept** here.
//!
//! A pointer C gives is its address ([`Value::Foreign`]); reading and writing through one reads
//! and writes C's bytes there.

#![allow(unsafe_code)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use moddil_ffi::CType;
use moddil_ffi::abi::{CLayout, Part, Scalar};
use moddil_sema::{ItemTree, Num, Ty};

use crate::value::{Pointer, Value};

/// The frame of a pointer into C's memory: its local is the place's index in [`CMem`].
pub(crate) const CMEM: usize = usize::MAX - 2;

/// Where handles start: above every address user code can have on the targets there are.
const HANDLE_BASE: u64 = 0xFFF0_0000_0000_0000;

/// What a run has given C.
#[derive(Default)]
pub(crate) struct CMem {
    /// The Moddil pointers given C as `void *`, by handle.
    handles: Vec<Value>,
    /// The copies made for C that last the run, as words (so that any alignment up to 8 holds).
    kept: Vec<Box<[u64]>>,
    /// The callbacks given to C: their code's address, and the function it runs.
    callbacks: HashMap<u64, u32>,
    /// The places in C's memory pointers have reached (an address and a type), by index, and
    /// the index of each.
    places: RefCell<Places>,
}

/// C places by index, and each one's index.
type Places = (Vec<(u64, Ty)>, HashMap<(u64, Ty), u32>);

/// C's size and alignment of a value of `ty`, as sema lays it out (`ffi.repr.1`).
pub(crate) fn size_align(items: &ItemTree, ty: &Ty) -> Result<(usize, usize), String> {
    let base = items.base(ty);
    if matches!(base, Ty::Num(Num::I128 | Num::U128)) {
        return Err("a 128-bit integer given to C".to_string());
    }
    match &base {
        // A struct's fields are C's too.
        Ty::Adt(a, ..) if items.adts[*a as usize].repr_c => {
            for f in &items.adts[*a as usize].variants[0].fields {
                size_align(items, &f.ty)?;
            }
        }
        Ty::Unit => {}
        t if items.is_c_scalar(t) => {}
        _ => return Err(format!("a `{ty}` in C's memory")),
    }
    let (size, align) = items.size_of(&base);
    Ok((size as usize, align as usize))
}

/// Field `i` of `#[repr(C)]` struct `adt`: its offset and its type.
pub(crate) fn field(items: &ItemTree, adt: u32, i: u32) -> Result<(usize, Ty), String> {
    let def = &items.adts[adt as usize];
    let ty = Ty::Adt(adt, def.name.as_str().into(), Vec::new().into());
    // Its fields are C's types, so sema's layout of it is C's.
    size_align(items, &ty)?;
    let f = def.variants[0]
        .fields
        .get(i as usize)
        .ok_or("a field the struct does not have")?;
    Ok((items.field_offset(&ty, i) as usize, f.ty.clone()))
}

/// The layout of `#[repr(C)]` struct type `ty`, as `moddil-ffi` passes it by value.
fn layout(items: &ItemTree, ty: &Ty) -> Result<CLayout, String> {
    let (size, align) = size_align(items, ty)?;
    CLayout::flatten(size as u32, align as u32, ty, |t| {
        Ok(match t {
            Ty::Adt(a, ..) => Part::Fields(
                (0..items.adts[*a as usize].variants[0].fields.len() as u32)
                    .map(|i| field(items, *a, i).map(|(off, t)| (off as u32, t)))
                    .collect::<Result<_, _>>()?,
            ),
            Ty::Num(Num::F32) => Part::Scalar(Scalar::F32),
            Ty::Num(Num::F64) => Part::Scalar(Scalar::F64),
            t => Part::Scalar(Scalar::Int(size_align(items, t)?.0 as u8)),
        })
    })
}

/// `ty` as `moddil-ffi` calls with it.
pub(crate) fn ctype(items: &ItemTree, ty: &Ty) -> Result<CType, String> {
    Ok(match ty {
        Ty::Unit => CType::Void,
        Ty::Bool => CType::I8,
        Ty::Int => CType::I64,
        Ty::Ptr(..) | Ty::FnPtr(..) => CType::Ptr,
        Ty::Refined(..) => return ctype(items, &items.base(ty)),
        Ty::Num(n) => match n {
            Num::I8 | Num::U8 => CType::I8,
            Num::I16 | Num::U16 => CType::I16,
            Num::I32 | Num::U32 => CType::I32,
            Num::U64 => CType::I64,
            Num::F32 => CType::F32,
            Num::F64 => CType::F64,
            Num::I128 | Num::U128 => return Err("a 128-bit integer given to C".to_string()),
        },
        Ty::Adt(a, ..) if items.adts[*a as usize].repr_c => {
            CType::Struct(Arc::new(layout(items, ty)?))
        }
        t => return Err(format!("a `{t}` given to C")),
    })
}

/// Bytes, as words, so that any C alignment up to 8 holds.
pub(crate) fn to_words(bytes: &[u8]) -> Vec<u64> {
    let mut words = vec![0u64; bytes.len().div_ceil(8).max(1)];
    for (i, chunk) in bytes.chunks(8).enumerate() {
        let mut w = [0u8; 8];
        w[..chunk.len()].copy_from_slice(chunk);
        words[i] = u64::from_le_bytes(w);
    }
    words
}

/// Words' bytes.
pub(crate) fn to_bytes(words: &[u64]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}

impl CMem {
    /// Scalar `v` of type `ty` as a word: an integer widened as its sign says, a float's bits,
    /// an address (a handle for a Moddil pointer given as `void *`, a callback's code for a
    /// Moddil function).
    pub(crate) fn word(&mut self, items: &ItemTree, ty: &Ty, v: &Value) -> Result<u64, String> {
        Ok(match (ty, v) {
            (Ty::Refined(..), v) => return self.word(items, &items.base(ty), v),
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
            (Ty::Ptr(..) | Ty::FnPtr(..), Value::Foreign(a)) => *a,
            (Ty::Ptr(_, inner), v @ (Value::Ref(..) | Value::Dyn(..))) if **inner == Ty::Unit => {
                let at = self.handles.iter().position(|h| h == v).unwrap_or_else(|| {
                    self.handles.push(v.clone());
                    self.handles.len() - 1
                });
                HANDLE_BASE + 16 * at as u64
            }
            (Ty::Ptr(..), Value::Ref(..)) => {
                return Err(
                    "a pointer to a Moddil value given to C inside another value, not as \
                     `void *`"
                        .to_string(),
                );
            }
            (Ty::FnPtr(ps, r), Value::FnPtr(f)) => {
                let params = ps
                    .iter()
                    .map(|t| ctype(items, t))
                    .collect::<Result<Vec<_>, _>>()?;
                let code = moddil_ffi::callback(u64::from(*f), &params, &ctype(items, r)?)?;
                self.callbacks.insert(code as u64, *f);
                code as u64
            }
            (t, v) => return Err(format!("a `{t}` given to C as {v:?}")),
        })
    }

    /// The value of scalar type `ty` that word `w` holds.
    pub(crate) fn value(&self, items: &ItemTree, ty: &Ty, w: u64) -> Value {
        match ty {
            Ty::Refined(..) => self.value(items, &items.base(ty), w),
            Ty::Int => Value::Int(w as i64),
            Ty::Bool => Value::Bool(w & 0xff != 0),
            Ty::Ptr(..) => {
                let at = w.wrapping_sub(HANDLE_BASE) / 16;
                match self.handles.get(at as usize) {
                    Some(h) if w >= HANDLE_BASE => h.clone(),
                    _ => Value::Foreign(w),
                }
            }
            Ty::FnPtr(..) => match self.callbacks.get(&w) {
                Some(f) => Value::FnPtr(*f),
                None => Value::Foreign(w),
            },
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
    pub(crate) fn write(
        &mut self,
        items: &ItemTree,
        buf: &mut [u8],
        at: usize,
        ty: &Ty,
        v: &Value,
    ) -> Result<(), String> {
        if let (Ty::Adt(a, ..), Value::Adt { fields, .. }) = (ty, v) {
            for (i, fv) in fields.iter().enumerate() {
                let (off, t) = field(items, *a, i as u32)?;
                self.write(items, buf, at + off, &t, fv)?;
            }
            return Ok(());
        }
        // An out-parameter not assigned yet: zeros.
        if matches!(v, Value::Empty) || *ty == Ty::Unit {
            return Ok(());
        }
        let (s, _) = size_align(items, ty)?;
        let w = self.word(items, ty, v)?;
        buf[at..at + s].copy_from_slice(&w.to_le_bytes()[..s]);
        Ok(())
    }

    /// The value of type `ty` at `at` of `buf`, in C's layout.
    pub(crate) fn read(
        &self,
        items: &ItemTree,
        buf: &[u8],
        at: usize,
        ty: &Ty,
    ) -> Result<Value, String> {
        if let Ty::Adt(a, ..) = ty {
            let n = items.adts[*a as usize].variants[0].fields.len();
            let mut fields = Vec::with_capacity(n);
            for i in 0..n {
                let (off, t) = field(items, *a, i as u32)?;
                fields.push(self.read(items, buf, at + off, &t)?);
            }
            return Ok(Value::Adt {
                adt: *a,
                variant: 0,
                fields,
            });
        }
        let (s, _) = size_align(items, ty)?;
        let mut w = [0u8; 8];
        w[..s].copy_from_slice(&buf[at..at + s]);
        Ok(self.value(items, ty, u64::from_le_bytes(w)))
    }

    /// The value of type `ty` at address `addr` of C's memory.
    ///
    /// # Safety
    ///
    /// `addr` holds a `ty`, as C lays it out.
    pub(crate) unsafe fn load(
        &self,
        items: &ItemTree,
        addr: u64,
        ty: &Ty,
    ) -> Result<Value, String> {
        let (s, _) = size_align(items, ty)?;
        // SAFETY: as the caller promises.
        let bytes = unsafe { std::slice::from_raw_parts(addr as *const u8, s) };
        self.read(items, bytes, 0, ty)
    }

    /// Writes `v`, of type `ty`, at address `addr` of C's memory.
    ///
    /// # Safety
    ///
    /// `addr` has room for a `ty`, which nothing else is reading or writing.
    pub(crate) unsafe fn store(
        &mut self,
        items: &ItemTree,
        addr: u64,
        ty: &Ty,
        v: &Value,
    ) -> Result<(), String> {
        let (s, _) = size_align(items, ty)?;
        // SAFETY: as the caller promises; read first, so that bytes `v` leaves alone stay.
        let mut bytes = unsafe { std::slice::from_raw_parts(addr as *const u8, s) }.to_vec();
        self.write(items, &mut bytes, 0, ty, v)?;
        // SAFETY: as above.
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), addr as *mut u8, s) };
        Ok(())
    }

    /// A pointer to the `ty` at address `addr` of C's memory.
    pub(crate) fn place(&self, addr: u64, ty: Ty) -> Pointer {
        let mut places = self.places.borrow_mut();
        let (all, index) = &mut *places;
        let at = *index.entry((addr, ty.clone())).or_insert_with(|| {
            all.push((addr, ty));
            (all.len() - 1) as u32
        });
        Pointer {
            task: 0,
            frame: CMEM,
            local: at,
            path: Vec::new(),
            span: None,
        }
    }

    /// The address and type of the place pointer `p` (into C's memory) points to.
    pub(crate) fn at(&self, p: &Pointer) -> (u64, Ty) {
        self.places.borrow().0[p.local as usize].clone()
    }

    /// Field `i` of the `#[repr(C)]` struct at C place `p`.
    pub(crate) fn field_of(&self, items: &ItemTree, p: &Pointer, i: u32) -> Pointer {
        let (addr, ty) = self.at(p);
        let Ty::Adt(a, ..) = ty else {
            unreachable!("a field of a `{ty}` in C's memory")
        };
        let (off, t) = field(items, a, i).unwrap_or_else(|e| unreachable!("{e}"));
        self.place(addr + off as u64, t)
    }

    /// Element `i` of the array at C place `p`.
    pub(crate) fn element_of(&self, items: &ItemTree, p: &Pointer, i: i64) -> Pointer {
        let (addr, ty) = self.at(p);
        let Ty::Array(elem, _) = ty else {
            unreachable!("an element of a `{ty}` in C's memory")
        };
        let (s, al) = size_align(items, &elem).unwrap_or_else(|e| unreachable!("{e}"));
        let stride = (s.div_ceil(al) * al) as i64;
        self.place(addr.wrapping_add((i * stride) as u64), (*elem).clone())
    }

    /// The value at C place `p`.
    pub(crate) fn load_at(&self, items: &ItemTree, p: &Pointer) -> Value {
        let (addr, ty) = self.at(p);
        // SAFETY: a C place is where an `unsafe` block dereferenced a pointer C gave.
        unsafe { self.load(items, addr, &ty) }.unwrap_or_else(|e| unreachable!("{e}"))
    }

    /// Writes `v` at C place `p`.
    pub(crate) fn store_at(&mut self, items: &ItemTree, p: &Pointer, v: &Value) {
        let (addr, ty) = self.at(p);
        // SAFETY: as for `load_at`.
        unsafe { self.store(items, addr, &ty, v) }.unwrap_or_else(|e| unreachable!("{e}"));
    }

    /// Keeps `bytes` for the rest of the run: their address.
    pub(crate) fn keep(&mut self, bytes: &[u8]) -> u64 {
        let words = to_words(bytes).into_boxed_slice();
        let at = words.as_ptr() as u64;
        self.kept.push(words);
        at
    }
}

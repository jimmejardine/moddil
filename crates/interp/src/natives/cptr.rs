//! The `core` mod's raw pointer functions (`core.ptr.2`), and those of its `ffi` module that
//! read C's memory (`ffi.core.2`), as the MIR interpreter runs them.
//!
//! A pointer is a Moddil place ([`Value::Ref`]), a handle back to one, or C's address
//! ([`Value::Foreign`]). Borrows of C's memory are copies, kept for the run; so are the
//! addresses `ptr_of` and `text_ptr` give, which C reads.

#![allow(unsafe_code)]

use moddil_mir::TrapClass;
use moddil_sema::{Num, Ty};

use super::Natives;
use crate::cmem::size_align;
use crate::stack::Stack;
use crate::value::Value;

impl Natives<'_> {
    /// Runs `core::ptr` or `core::ffi` function `name` of module `module`, if it is one this
    /// file runs.
    pub(super) fn c_native(
        &self,
        module: u32,
        name: &str,
        args: &[Value],
        type_args: &[Ty],
        stack: &mut Stack<'_>,
    ) -> Option<Result<Value, TrapClass>> {
        let path = &self.items.modules[module as usize].path;
        let ptr = path == &["ptr"];
        let ffi = path == &["ffi"];
        let elem = || type_args.first().cloned().unwrap_or(Ty::Unit);
        let stride = |t: &Ty| {
            size_align(self.items, t)
                .map(|(s, a)| (s.div_ceil(a) * a) as u64)
                .map_err(|_| TrapClass::Unimplemented)
        };
        Some(match name {
            "null" | "null_mut" if ptr => Ok(Value::Foreign(0)),
            "is_null" if ptr => Ok(Value::Bool(matches!(args[0], Value::Foreign(0)))),
            "addr_eq" if ptr => Ok(Value::Bool(match (&args[0], &args[1]) {
                (Value::Ref(a, _), Value::Ref(b, _)) => a == b,
                (a, b) => a == b,
            })),
            "cast" | "cast_mut_to" if ptr => Ok(args[0].clone()),
            "add" | "add_mut" if ptr => {
                let n = args[1].as_int();
                match &args[0] {
                    Value::Foreign(a) => stride(&elem())
                        .map(|s| Value::Foreign(a.wrapping_add((n as u64).wrapping_mul(s)))),
                    // Along the array the place is in.
                    Value::Ref(p, m) => match p.path.split_last() {
                        Some((&last, rest)) => {
                            let at = i64::from(last) + n;
                            let mut q = p.clone();
                            q.path = rest.to_vec();
                            q.path.push(at as u32);
                            if at < 0 {
                                Err(TrapClass::IndexOutOfBounds)
                            } else {
                                Ok(Value::Ref(q, *m))
                            }
                        }
                        None if n == 0 => Ok(args[0].clone()),
                        None => Err(TrapClass::IndexOutOfBounds),
                    },
                    _ => Err(TrapClass::Unimplemented),
                }
            }
            "c_len" if ffi => match &args[0] {
                // SAFETY: as the function's caller promises, a NUL-terminated string.
                Value::Foreign(a) => Ok(Value::Int(unsafe { strlen(*a as *const u8) } as i64)),
                Value::Ref(p, _) => {
                    // A Moddil array of bytes, from the place on.
                    let (start, parent) = match p.path.split_last() {
                        Some((&last, rest)) => {
                            let mut q = p.clone();
                            q.path = rest.to_vec();
                            (last as usize, q)
                        }
                        None => (0, p.clone()),
                    };
                    match stack.load(&parent) {
                        Value::Array(xs) | Value::Buf(_, xs) => Ok(Value::Int(
                            xs.iter()
                                .skip(start)
                                .take_while(|x| !matches!(x, Value::Num(_, 0)))
                                .count() as i64,
                        )),
                        _ => Err(TrapClass::Unimplemented),
                    }
                }
                _ => Err(TrapClass::Unimplemented),
            },
            "slice" | "slice_mut" | "text" if ffi => {
                let n = args[1].as_int();
                let t = if name == "text" {
                    Ty::Num(Num::U8)
                } else {
                    elem()
                };
                match &args[0] {
                    // A copy of C's values, kept; C's memory is not changed through it.
                    Value::Foreign(a) if name != "slice_mut" => {
                        let s = match stride(&t) {
                            Ok(s) => s,
                            Err(e) => return Some(Err(e)),
                        };
                        let mut xs = Vec::with_capacity(n.max(0) as usize);
                        for k in 0..n.max(0) as u64 {
                            // SAFETY: as the function's caller promises, `n` values at `a`.
                            match unsafe { stack.cmem().load(self.items, a + k * s, &t) } {
                                Ok(v) => xs.push(v),
                                Err(_) => return Some(Err(TrapClass::Unimplemented)),
                            }
                        }
                        if name == "text" {
                            let bytes: Vec<u8> = xs
                                .iter()
                                .map(|v| match v {
                                    Value::Num(_, b) => *b as u8,
                                    _ => 0,
                                })
                                .collect();
                            match String::from_utf8(bytes) {
                                Ok(s) => Ok(Value::Str(s.as_str().into())),
                                Err(_) => Err(TrapClass::Unimplemented),
                            }
                        } else {
                            match stack.alloc(Value::Array(xs)) {
                                Value::Ref(p, _) => Ok(Value::Ref(p, false)),
                                other => unreachable!("a heap pointer: {other:?}"),
                            }
                        }
                    }
                    // Part of a Moddil array.
                    Value::Ref(p, _) if name != "text" => match p.path.split_last() {
                        Some((&last, rest)) => {
                            let mut q = p.clone();
                            q.path = rest.to_vec();
                            q.span = Some((last, n.max(0) as u32));
                            Ok(Value::Ref(q, name == "slice_mut"))
                        }
                        None => Err(TrapClass::Unimplemented),
                    },
                    _ => Err(TrapClass::Unimplemented),
                }
            }
            "ptr_of" | "text_ptr" if ffi => {
                // A copy in C's memory, kept for the run, which C reads.
                let (t, values) = match stack.deref(&args[0]) {
                    Value::Str(s) => (
                        Ty::Num(Num::U8),
                        s.as_bytes()
                            .iter()
                            .map(|b| Value::Num(Num::U8, u128::from(*b)))
                            .collect::<Vec<_>>(),
                    ),
                    Value::Array(xs) | Value::Buf(_, xs) => (elem(), xs),
                    _ => return Some(Err(TrapClass::Unimplemented)),
                };
                let s = match stride(&t) {
                    Ok(s) => s as usize,
                    Err(e) => return Some(Err(e)),
                };
                let mut bytes = vec![0u8; (s * values.len()).max(8)];
                for (k, v) in values.iter().enumerate() {
                    if stack
                        .cmem()
                        .write(self.items, &mut bytes, k * s, &t, v)
                        .is_err()
                    {
                        return Some(Err(TrapClass::Unimplemented));
                    }
                }
                Ok(Value::Foreign(stack.cmem().keep(&bytes)))
            }
            _ => return None,
        })
    }
}

/// The bytes before the NUL at `p`.
///
/// # Safety
///
/// `p` is a NUL-terminated string.
unsafe fn strlen(p: *const u8) -> usize {
    let mut n = 0;
    // SAFETY: as the caller promises, every byte up to the NUL is there.
    while unsafe { *p.add(n) } != 0 {
        n += 1;
    }
    n
}

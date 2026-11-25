//! Const items' values, computed when compiling and written as static data
//! (`comptime.static.1`): the value the interpreter froze, laid out as the code reads it, with
//! each pointer in it an address of the data its pointee is written to. Nothing frees it.

use std::collections::HashMap;

use cranelift_module::{DataDescription, DataId, Module};
use moddil_interp::{Frozen, FrozenValue};
use moddil_sema::{Item, Ty};

use super::Compiler;
use crate::layout::{Shape, Tag};

/// The data being written for one const: its pointees' data, by index, made once each.
struct Writer<'v> {
    value: &'v FrozenValue,
    made: HashMap<usize, DataId>,
}

/// One data object's bytes and the addresses written into them (where, and of what data).
#[derive(Default)]
struct Bytes {
    bytes: Vec<u8>,
    relocs: Vec<(u32, DataId)>,
}

impl Bytes {
    fn put(&mut self, at: u32, bits: u128, n: usize) {
        let at = at as usize;
        self.bytes[at..at + n].copy_from_slice(&bits.to_le_bytes()[..n]);
    }
}

impl<M: Module> Compiler<'_, M> {
    /// The data holding const item `f`'s value, of type `ty`, frozen when compiling; `None` if
    /// it is not frozen (a generic const), and the error if its value is not one static data
    /// can hold.
    pub(super) fn frozen_static(&mut self, f: u32, ty: &Ty) -> Result<Option<DataId>, String> {
        if !self.items.is_frozen_const(f) {
            return Ok(None);
        }
        if let Some(d) = self.statics.get(&f) {
            return Ok(Some(*d));
        }
        let name = self.items.fns[f as usize].name.clone();
        let id = self.freeze(Item::Fn(f), &name, ty)?;
        self.statics.insert(f, id);
        Ok(Some(id))
    }

    /// The data holding the value of the `const { }` block whose item is `item`, of type `ty`,
    /// computed when compiling (`comptime.block.2`).
    pub(super) fn frozen_block(&mut self, item: Item, ty: &Ty) -> Result<DataId, String> {
        if let Some(d) = self.blocks.get(&item) {
            return Ok(*d);
        }
        let id = self.freeze(item, "a `const { }` block", ty)?;
        self.blocks.insert(item, id);
        Ok(id)
    }

    /// `item`'s value (named `name` in errors), evaluated when compiling and written as data.
    fn freeze(&mut self, item: Item, name: &str, ty: &Ty) -> Result<DataId, String> {
        let value = match moddil_interp::frozen_item(self.db, self.program, item) {
            Ok(Ok(v)) => v,
            Ok(Err(what)) => return Err(format!("`{name}` holds {what}")),
            Err(_) => return Err(format!("evaluating `{name}` when compiling fails")),
        };
        let mut w = Writer {
            value: &value,
            made: HashMap::new(),
        };
        let id = self.declare_frozen()?;
        self.define_frozen(&mut w, id, &value.root, ty)?;
        Ok(id)
    }

    fn declare_frozen(&mut self) -> Result<DataId, String> {
        // Writable: an `Rc` in it counts its owners there.
        self.module
            .declare_anonymous_data(true, false)
            .map_err(|e| e.to_string())
    }

    /// Writes `v`, of type `ty`, as data `id`.
    fn define_frozen(
        &mut self,
        w: &mut Writer<'_>,
        id: DataId,
        v: &Frozen,
        ty: &Ty,
    ) -> Result<(), String> {
        let l = self.layouts.of(ty)?;
        let mut out = Bytes {
            bytes: vec![0; (l.size as usize).max(1)],
            relocs: Vec::new(),
        };
        self.write_frozen(w, &mut out, 0, v, ty)?;
        self.define_bytes(id, out, l.align)
    }

    fn define_bytes(&mut self, id: DataId, out: Bytes, align: u32) -> Result<(), String> {
        let mut desc = DataDescription::new();
        desc.define(out.bytes.into_boxed_slice());
        desc.set_align(u64::from(align.max(8)));
        for (at, target) in out.relocs {
            let gv = self.module.declare_data_in_data(target, &mut desc);
            desc.write_data_addr(at, gv, 0);
        }
        self.module
            .define_data(id, &desc)
            .map_err(|e| e.to_string())
    }

    /// The data of pointee `i`, of type `ty`, written once.
    fn pointee(&mut self, w: &mut Writer<'_>, i: usize, ty: &Ty) -> Result<DataId, String> {
        if let Some(d) = w.made.get(&i) {
            return Ok(*d);
        }
        let id = self.declare_frozen()?;
        w.made.insert(i, id);
        let v = w.value.pointees[i].clone();
        self.define_frozen(w, id, &v, ty)?;
        Ok(id)
    }

    /// Writes `v`, of type `ty`, into `out` at `at`.
    fn write_frozen(
        &mut self,
        w: &mut Writer<'_>,
        out: &mut Bytes,
        at: u32,
        v: &Frozen,
        ty: &Ty,
    ) -> Result<(), String> {
        let l = self.layouts.of(ty)?;
        let unsupported = || format!("a `{ty}` in a const's value");
        match (&l.shape, v) {
            (Shape::Empty, _) => {}
            (Shape::Scalar(_), Frozen::Ptr(i)) => {
                let inner = match ty {
                    Ty::Ref(_, inner) | Ty::Ptr(_, inner) => (**inner).clone(),
                    _ => return Err(unsupported()),
                };
                let d = self.pointee(w, *i, &inner)?;
                out.relocs.push((at, d));
            }
            (Shape::Scalar(t), v) => {
                let bits = match v {
                    Frozen::Int(i) => *i as u64 as u128,
                    Frozen::Num(_, bits) => *bits,
                    Frozen::Bool(b) => u128::from(*b),
                    Frozen::Char(c) => u128::from(u32::from(*c)),
                    _ => return Err(unsupported()),
                };
                out.put(at, bits, t.bytes() as usize);
            }
            (Shape::Wide, Frozen::Str(s)) => {
                let d = self.text_data(s)?;
                out.relocs.push((at, d));
                out.put(at + 8, s.len() as u128, 8);
            }
            (Shape::Buf(elem), Frozen::Buf(xs)) => {
                let el = self.layouts.of(elem)?;
                let stride = el.size.div_ceil(el.align.max(1)) * el.align.max(1);
                if !xs.is_empty() && stride > 0 {
                    let id = self.declare_frozen()?;
                    let mut data = Bytes {
                        bytes: vec![0; (stride as usize) * xs.len()],
                        relocs: Vec::new(),
                    };
                    for (k, x) in xs.iter().enumerate() {
                        self.write_frozen(w, &mut data, stride * k as u32, x, elem)?;
                    }
                    self.define_bytes(id, data, el.align)?;
                    out.relocs.push((at, id));
                }
                out.put(at + 8, xs.len() as u128, 8);
                out.put(at + 16, xs.len() as u128, 8);
            }
            (Shape::Fields(fields), Frozen::Tuple(xs) | Frozen::Adt { fields: xs, .. }) => {
                for ((off, t), x) in fields.clone().iter().zip(xs) {
                    self.write_frozen(w, out, at + off, x, t)?;
                }
            }
            (Shape::Array { elem, stride, .. }, Frozen::Array(xs)) => {
                let (elem, stride) = (elem.clone(), *stride);
                for (k, x) in xs.iter().enumerate() {
                    self.write_frozen(w, out, at + stride * k as u32, x, &elem)?;
                }
            }
            (
                Shape::Enum { tag, variants },
                Frozen::Adt {
                    variant, fields, ..
                },
            ) => {
                let (tag, layout) = (tag.clone(), variants[*variant as usize].clone());
                for ((off, t), x) in layout.iter().zip(fields) {
                    self.write_frozen(w, out, at + off, x, t)?;
                }
                match tag {
                    Tag::None => {}
                    Tag::Direct { offset, ty } => {
                        out.put(at + offset, u128::from(*variant), ty.bytes() as usize);
                    }
                    Tag::Niche { dataful, niche } => {
                        if *variant != dataful {
                            let rank = if *variant < dataful {
                                *variant
                            } else {
                                *variant - 1
                            };
                            out.put(
                                at + niche.offset,
                                niche.first + u128::from(rank),
                                niche.ty.bytes() as usize,
                            );
                        }
                    }
                }
            }
            _ => return Err(unsupported()),
        }
        Ok(())
    }
}

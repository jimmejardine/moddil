//! Layouts: how values of each type sit in memory (`specs/numerics.md`: Data layout).
//!
//! - **Scalars** have their own size and alignment; a borrow of a sized type is a pointer.
//! - **Tuples and structs** put their fields in order of alignment, largest first (ties in the
//!   order declared), so there is as little padding as can be; the layout keeps each field's
//!   offset by its declared index, which is what code and printing use.
//! - **Arrays** repeat their element at its stride.
//! - **Enums** use a niche when they can: one variant has a payload, the others none, and the
//!   payload has a field with invalid values to spare (a `bool`'s 2 to 255, a `char` past
//!   U+10FFFF, a borrow's 0, an enum tag's unused values), so the other variants are stored as
//!   those values and no tag is needed (`Option<&T>` is a pointer). Otherwise a tag, the smallest
//!   integer that numbers the variants, comes first, then the largest payload.
//! - **`Buf<T>`** is its memory's address, length and capacity; a closure is its captures.

use std::collections::HashMap;
use std::sync::Arc;

use cranelift_codegen::ir::{self, types};
use moddil_sema::{ItemTree, Ty};

use crate::lower::clif_type;

/// Where a value's invalid values are: a scalar at `offset` of type `ty`, whose values from
/// `first` (for `count` values) are never a valid value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Niche {
    pub(crate) offset: u32,
    pub(crate) ty: ir::Type,
    pub(crate) first: u128,
    pub(crate) count: u128,
}

/// How an enum tells its variants apart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Tag {
    /// One variant, or none: nothing to tell apart.
    None,
    /// An integer of type `ty` at `offset`, the variant's index.
    Direct { offset: u32, ty: ir::Type },
    /// Variant `dataful` holds a payload; each other one is stored as a value of the payload's
    /// niche: the `k`th other variant (in order) as `niche.first + k`.
    Niche { dataful: u32, niche: Niche },
}

/// The shape of a layout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Shape {
    /// A scalar (a pointer included) of this Cranelift type.
    Scalar(ir::Type),
    /// A borrow of an unsized value: its address, then its length (`str`, `[T]`) or its
    /// vtable (`dyn Trait`), two words; a `&[dyn Trait]` has its length, then its vtable.
    Wide,
    /// A `Buf<T>`: its memory's address, its length and its capacity, three words; a
    /// `Buf<dyn Trait>` (a `Box<dyn Trait>`'s) has its vtable last, a fourth.
    Buf(Ty),
    /// An error set: the member's id (its struct's or enum's index), a word, then its value.
    Errors(Vec<Ty>),
    /// No bytes: `()`, `!`, a struct without fields.
    Empty,
    /// A tuple's or a struct's fields: each one's offset and type, by declared index.
    Fields(Vec<(u32, Ty)>),
    /// An array: its element, the stride and how many.
    Array { elem: Ty, stride: u32, len: u32 },
    /// An enum: how its variants are told apart, and each variant's fields (offset and type, by
    /// declared index).
    Enum {
        tag: Tag,
        variants: Vec<Vec<(u32, Ty)>>,
    },
}

/// A type's layout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Layout {
    pub(crate) size: u32,
    pub(crate) align: u32,
    pub(crate) shape: Shape,
    /// Its invalid values, if it has any to spare.
    pub(crate) niche: Option<Niche>,
}

fn align_up(n: u32, a: u32) -> u32 {
    n.div_ceil(a) * a
}

/// Every type's layout, computed once.
pub(crate) struct Layouts<'a> {
    items: &'a ItemTree,
    cache: HashMap<Ty, Arc<Layout>>,
}

impl<'a> Layouts<'a> {
    pub(crate) fn new(items: &'a ItemTree) -> Self {
        Layouts {
            items,
            cache: HashMap::new(),
        }
    }

    /// `ty`'s layout, or what about it is not compiled natively yet.
    pub(crate) fn of(&mut self, ty: &Ty) -> Result<Arc<Layout>, String> {
        if let Some(l) = self.cache.get(ty) {
            return Ok(l.clone());
        }
        let l = Arc::new(self.compute(ty)?);
        self.cache.insert(ty.clone(), l.clone());
        Ok(l)
    }

    fn compute(&mut self, ty: &Ty) -> Result<Layout, String> {
        match ty {
            Ty::Unit | Ty::Never => Ok(empty()),
            Ty::Ref(_, inner) => {
                if matches!(**inner, Ty::Str | Ty::Slice(_) | Ty::Dyn(..)) {
                    let dyn_slice = matches!(&**inner, Ty::Slice(e) if matches!(**e, Ty::Dyn(..)));
                    return Ok(Layout {
                        size: if dyn_slice { 24 } else { 16 },
                        align: 8,
                        shape: Shape::Wide,
                        // The address is never null.
                        niche: Some(Niche {
                            offset: 0,
                            ty: types::I64,
                            first: 0,
                            count: 1,
                        }),
                    });
                }
                Ok(scalar(types::I64, Some((0, 1))))
            }
            Ty::Bool => Ok(scalar(types::I8, Some((2, 254)))),
            Ty::Char => Ok(scalar(
                types::I32,
                Some((0x11_0000, 0xFFFF_FFFF - 0x11_0000 + 1)),
            )),
            Ty::Tuple(ts) => self.fields(ts),
            // An error set: its member's id, a word, then the member (`A | B`).
            Ty::Errors(ms) => {
                let mut size = 0;
                for m in ms.iter() {
                    let l = self.of(m)?;
                    if l.align > 8 {
                        return Err(format!("an error of type `{m}`, aligned to {}", l.align));
                    }
                    size = size.max(l.size);
                }
                Ok(Layout {
                    size: 8 + align_up(size, 8),
                    align: 8,
                    shape: Shape::Errors(ms.to_vec()),
                    niche: None,
                })
            }
            // A closure is its captures.
            Ty::Closure(c) => self.fields(&c.captures),
            Ty::Buf(elem) => Ok(Layout {
                size: if matches!(**elem, Ty::Dyn(..)) {
                    32
                } else {
                    24
                },
                align: 8,
                shape: Shape::Buf((**elem).clone()),
                // The address is never null.
                niche: Some(Niche {
                    offset: 0,
                    ty: types::I64,
                    first: 0,
                    count: 1,
                }),
            }),
            Ty::Array(elem, n) => {
                let Ty::Len(n) = **n else {
                    return Err(format!("an array of length `{n}`"));
                };
                let e = self.of(elem)?;
                let stride = align_up(e.size, e.align);
                let len = u32::try_from(n).map_err(|_| "a huge array".to_string())?;
                Ok(Layout {
                    size: stride * len,
                    align: e.align,
                    niche: if len > 0 { e.niche } else { None },
                    shape: Shape::Array {
                        elem: (**elem).clone(),
                        stride,
                        len,
                    },
                })
            }
            Ty::Adt(a, ..) => {
                let def = &self.items.adts[*a as usize];
                if def.is_enum {
                    self.enumeration(ty, def.variants.len() as u32)
                } else {
                    let ts = self.items.field_types(ty, 0);
                    self.fields(&ts)
                }
            }
            t => match clif_type(t)? {
                Some(c) => Ok(scalar(c, None)),
                None => Ok(empty()),
            },
        }
    }

    /// The fields `ts` of a tuple or struct, reordered by alignment.
    fn fields(&mut self, ts: &[Ty]) -> Result<Layout, String> {
        let layouts = ts
            .iter()
            .map(|t| self.of(t))
            .collect::<Result<Vec<_>, _>>()?;
        let mut order: Vec<usize> = (0..ts.len()).collect();
        order.sort_by_key(|&i| std::cmp::Reverse(layouts[i].align));
        let mut offsets = vec![0u32; ts.len()];
        let mut at = 0;
        let mut align = 1;
        for &i in &order {
            let l = &layouts[i];
            at = align_up(at, l.align);
            offsets[i] = at;
            at += l.size;
            align = align.max(l.align);
        }
        // The roomiest niche among the fields.
        let niche = layouts
            .iter()
            .zip(&offsets)
            .filter_map(|(l, &off)| {
                l.niche.map(|n| Niche {
                    offset: n.offset + off,
                    ..n
                })
            })
            .max_by_key(|n| n.count);
        Ok(Layout {
            size: align_up(at, align),
            align,
            niche,
            shape: Shape::Fields(offsets.into_iter().zip(ts.iter().cloned()).collect()),
        })
    }

    fn enumeration(&mut self, ty: &Ty, n: u32) -> Result<Layout, String> {
        let payloads: Vec<Layout> = (0..n)
            .map(|v| {
                let ts = self.items.field_types(ty, v);
                self.fields(&ts)
            })
            .collect::<Result<_, _>>()?;
        let fields_of = |l: &Layout, shift: u32| match &l.shape {
            Shape::Fields(fs) => fs.iter().map(|(o, t)| (o + shift, t.clone())).collect(),
            _ => Vec::new(),
        };
        if n <= 1 {
            let (size, align, niche) = payloads
                .first()
                .map_or((0, 1, None), |p| (p.size, p.align, p.niche));
            return Ok(Layout {
                size,
                align,
                niche,
                shape: Shape::Enum {
                    tag: Tag::None,
                    variants: payloads.iter().map(|p| fields_of(p, 0)).collect(),
                },
            });
        }
        // A niche: one variant with a payload, the others without, and room for them.
        let dataful: Vec<u32> = (0..n).filter(|&v| payloads[v as usize].size > 0).collect();
        if let [d] = dataful.as_slice()
            && let Some(niche) = payloads[*d as usize].niche
            && niche.count >= u128::from(n - 1)
        {
            let p = &payloads[*d as usize];
            let rest = Niche {
                first: niche.first + u128::from(n - 1),
                count: niche.count - u128::from(n - 1),
                ..niche
            };
            return Ok(Layout {
                size: p.size,
                align: p.align,
                niche: (rest.count > 0).then_some(rest),
                shape: Shape::Enum {
                    tag: Tag::Niche { dataful: *d, niche },
                    variants: payloads.iter().map(|p| fields_of(p, 0)).collect(),
                },
            });
        }
        // A tag, then the payload.
        let (tag_ty, tag_size, capacity) = if n <= 256 {
            (types::I8, 1, 256u128)
        } else if n <= 65_536 {
            (types::I16, 2, 65_536)
        } else {
            (types::I32, 4, 1 << 32)
        };
        let payload_align = payloads.iter().map(|p| p.align).max().unwrap_or(1);
        let payload_size = payloads.iter().map(|p| p.size).max().unwrap_or(0);
        let start = align_up(tag_size, payload_align);
        let align = payload_align.max(tag_size);
        Ok(Layout {
            size: align_up(start + payload_size, align),
            align,
            niche: Some(Niche {
                offset: 0,
                ty: tag_ty,
                first: u128::from(n),
                count: capacity - u128::from(n),
            }),
            shape: Shape::Enum {
                tag: Tag::Direct {
                    offset: 0,
                    ty: tag_ty,
                },
                variants: payloads.iter().map(|p| fields_of(p, start)).collect(),
            },
        })
    }
}

fn empty() -> Layout {
    Layout {
        size: 0,
        align: 1,
        shape: Shape::Empty,
        niche: None,
    }
}

fn scalar(t: ir::Type, niche: Option<(u128, u128)>) -> Layout {
    let size = t.bytes();
    Layout {
        size,
        align: size,
        shape: Shape::Scalar(t),
        niche: niche.map(|(first, count)| Niche {
            offset: 0,
            ty: t,
            first,
            count,
        }),
    }
}

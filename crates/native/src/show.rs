//! Showing values as the interpreters write them (`Point { x: 1, y: 2 }`, `Some(3)`, `&[1, 2]`),
//! from their bytes and their types: each type described from its layout as a
//! [`moddil_show::Types`] table, which renders the values (and which a built program holds, to
//! show `main`'s result with). What holds the bytes is a [`Memory`]: native code's own, while
//! the values are alive, or a snapshot of a WASM module's.

use std::collections::HashMap;

use moddil_sema::{ItemTree, Shape as VariantShape, Ty};
pub use moddil_show::Memory;
use moddil_show::{Fields, Node, Pointee, Types, Variant};

use crate::layout::{Layouts, Shape, Tag};

/// What showing values needs: the items, the layouts, the types `dyn` borrows point to (by the
/// number their vtables start with), and the types described so far.
pub struct Show<'a> {
    /// The program's items.
    pub items: &'a ItemTree,
    /// The layouts the values have.
    pub layouts: Layouts<'a>,
    /// The types `dyn` borrows point to.
    pub dyn_types: &'a [Ty],
    types: Types,
    index: HashMap<Ty, u32>,
}

fn stride(l: &crate::layout::Layout) -> u32 {
    l.size.div_ceil(l.align) * l.align
}

/// A layout's tag, as `moddil-show` reads it.
fn tag(t: &Tag) -> moddil_show::Tag {
    match t {
        Tag::None => moddil_show::Tag::None,
        Tag::Direct { offset, ty } => moddil_show::Tag::Direct {
            offset: *offset,
            bytes: ty.bytes(),
        },
        Tag::Niche { dataful, niche } => moddil_show::Tag::Niche {
            dataful: *dataful,
            offset: niche.offset,
            bytes: niche.ty.bytes(),
            first: niche.first,
        },
    }
}

impl<'a> Show<'a> {
    /// Showing the values of `items`'s types, `dyn_types` those `dyn` borrows point to.
    pub fn new(items: &'a ItemTree, dyn_types: &'a [Ty]) -> Show<'a> {
        Show {
            items,
            layouts: Layouts::new(items),
            dyn_types,
            types: Types::default(),
            index: HashMap::new(),
        }
    }

    /// The value of type `ty` at `p`, as the interpreters write it.
    pub fn render(&mut self, mem: &dyn Memory, ty: &Ty, p: u64) -> String {
        let t = self.describe(ty);
        self.types.render(mem, t, p)
    }

    /// The table describing `ty`, and its index in it: what a built program holds to show a
    /// value of `ty` with.
    pub fn table(mut self, ty: &Ty) -> (Types, u32) {
        let t = self.describe(ty);
        (self.types, t)
    }

    /// `ty`'s index in the table, described with what it holds if it is not there yet.
    fn describe(&mut self, ty: &Ty) -> u32 {
        if let Some(&t) = self.index.get(ty) {
            return t;
        }
        // Its index first, so a type that holds itself through a pointer finds it.
        let t = self.types.nodes.len() as u32;
        self.types.nodes.push(Node::Opaque(ty.to_string()));
        self.index.insert(ty.clone(), t);
        let node = self.node(ty);
        self.types.nodes[t as usize] = node;
        t
    }

    /// The types `dyn` borrows point to, described, by their vtables' numbers.
    fn describe_dyns(&mut self) {
        if self.types.dyns.is_empty() && !self.dyn_types.is_empty() {
            // Filled first, so a `dyn` borrow inside one of them stops here.
            self.types.dyns = vec![u32::MAX; self.dyn_types.len()];
            for (k, t) in self.dyn_types.iter().enumerate() {
                self.types.dyns[k] = self.describe(t);
            }
        }
    }

    fn node(&mut self, ty: &Ty) -> Node {
        let items = self.items;
        let Ok(l) = self.layouts.of(ty) else {
            return Node::Opaque(ty.to_string());
        };
        match ty {
            Ty::Unit | Ty::Never => Node::Unit,
            // A refined value is shown as its base's (`refine.base.1`).
            Ty::Refined(..) => {
                let base = items.base(ty);
                let b = self.describe(&base);
                self.types.nodes[b as usize].clone()
            }
            Ty::Int => Node::Int,
            Ty::Bool => Node::Bool,
            Ty::Char => Node::Char,
            Ty::Num(n) => Node::Num {
                code: moddil_mir::num_code(Some(*n)),
                bytes: n.bits() / 8,
            },
            Ty::Ref(m, inner) => {
                let to = match &**inner {
                    // Text is shown as itself, in quotes, as the interpreters show it.
                    Ty::Str => Pointee::Str,
                    Ty::Slice(elem) => Pointee::Slice {
                        elem: self.describe(elem),
                        stride: self.layouts.of(elem).map_or(0, |l| stride(&l)),
                    },
                    Ty::Dyn(..) => {
                        self.describe_dyns();
                        Pointee::Dyn
                    }
                    inner => Pointee::Sized(self.describe(inner)),
                };
                Node::Ref { mutable: *m, to }
            }
            Ty::Tuple(_) => match &l.shape {
                Shape::Fields(fs) => {
                    Node::Tuple(fs.iter().map(|(off, t)| (*off, self.describe(t))).collect())
                }
                _ => Node::Opaque(ty.to_string()),
            },
            Ty::Array(..) => match &l.shape {
                Shape::Array { elem, stride, len } => Node::Array {
                    elem: self.describe(elem),
                    stride: *stride,
                    len: *len,
                },
                _ => Node::Opaque(ty.to_string()),
            },
            // An error set: the member its id says.
            Ty::Errors(ms) => Node::Errors {
                value: crate::lower::ERROR_VALUE,
                members: ms
                    .iter()
                    .filter_map(|t| Some((t.adt()?, self.describe(t))))
                    .collect(),
            },
            Ty::Buf(elem) => Node::Buf {
                elem: self.describe(elem),
                stride: self.layouts.of(elem).map_or(0, |l| stride(&l)),
            },
            // A `String` is shown as its text, in quotes; a `Vec` as the literal that makes one.
            Ty::Adt(a, ..)
                if items.adts[*a as usize].src == moddil_sema::Src::Prelude
                    && matches!(items.adts[*a as usize].name.as_str(), "String" | "Vec") =>
            {
                let Shape::Fields(fs) = &l.shape else {
                    return Node::Opaque(ty.to_string());
                };
                let (offset, inner) = fs[0].clone();
                if items.adts[*a as usize].name == "Vec" {
                    return Node::Vec {
                        offset,
                        buf: self.describe(&inner),
                    };
                }
                // Its `StringRepr`: inline bytes and their count, or a `Buf<u8>`.
                match self.layouts.of(&inner).map(|l| l.shape.clone()) {
                    Ok(Shape::Enum { tag: t, variants }) => Node::String {
                        offset,
                        tag: tag(&t),
                        variants: variants
                            .iter()
                            .map(|fs| fs.iter().map(|(off, _)| *off).collect())
                            .collect(),
                    },
                    _ => Node::Opaque("String".to_string()),
                }
            }
            Ty::Adt(a, ..) => {
                let (t, fields) = match &l.shape {
                    Shape::Fields(fs) => (moddil_show::Tag::None, vec![fs.clone()]),
                    Shape::Enum { tag: t, variants } => (tag(t), variants.clone()),
                    Shape::Empty => (moddil_show::Tag::None, vec![Vec::new()]),
                    _ => return Node::Opaque(ty.to_string()),
                };
                let mut variants = Vec::new();
                for (v, fs) in fields.iter().enumerate() {
                    let def = items.variant(*a, v as u32);
                    variants.push(Variant {
                        path: items.adts[*a as usize].path(v as u32),
                        fields: match def.shape {
                            VariantShape::Unit => Fields::Unit,
                            VariantShape::Tuple => Fields::Tuple,
                            VariantShape::Named => Fields::Named,
                        },
                        names: def.fields.iter().map(|f| f.name.clone()).collect(),
                        parts: fs.iter().map(|(off, t)| (*off, self.describe(t))).collect(),
                    });
                }
                Node::Adt { tag: t, variants }
            }
            t => Node::Opaque(t.to_string()),
        }
    }

    /// The exit code for the result of type `ty` at `p`, if it is the prelude's `ExitCode`: its
    /// `sysexits.h` number, or `Other(n)`'s `n`, as the interpreter's `moddil run` gives it.
    pub fn exit_code(&mut self, mem: &dyn Memory, ty: &Ty, p: u64) -> Option<u8> {
        let Ty::Adt(a, ..) = ty else { return None };
        let def = &self.items.adts[*a as usize];
        if def.src != moddil_sema::Src::Prelude || def.name != "ExitCode" {
            return None;
        }
        let l = self.layouts.of(ty).ok()?;
        let Shape::Enum { tag: t, variants } = &l.shape else {
            return None;
        };
        const CODES: [u8; 16] = [
            0, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76, 77, 78,
        ];
        let v = moddil_show::variant_at(mem, &tag(t), variants.len() as u32, p);
        if let Some(c) = CODES.get(v as usize) {
            return Some(*c);
        }
        let at = variants[v as usize].first()?.0;
        let n = mem.bits(p + u64::from(at), 8) as u64 as i64;
        Some(n.clamp(0, 255) as u8)
    }

    /// The error at `p`, tagged: its value, as the member of `errors` its id says.
    pub fn error(&mut self, mem: &dyn Memory, errors: &[Ty], p: u64) -> String {
        let id = mem.bits(p, 8) as u32;
        match errors.iter().find(|t| t.adt() == Some(id)).cloned() {
            Some(t) => self.render(mem, &t, p + u64::from(crate::lower::ERROR_VALUE)),
            None => "<an error>".to_string(),
        }
    }
}

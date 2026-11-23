//! Showing values as the interpreters write them (`Point { x: 1, y: 2 }`, `Some(3)`, `&[1, 2]`),
//! from their bytes and their layouts. What holds the bytes is a [`Memory`]: native code's own,
//! while the values are alive, or a snapshot of a WASM module's.

use moddil_sema::{ItemTree, Num, Shape as VariantShape, Ty};

use crate::layout::{Layouts, Shape, Tag};

/// Where values are read from, by address.
pub trait Memory {
    /// The bits of the `n` (at most 16) bytes at `addr`, little-endian.
    fn bits(&self, addr: u64, n: u32) -> u128;
    /// The `n` bytes at `addr`.
    fn bytes(&self, addr: u64, n: usize) -> Vec<u8>;
}

/// What showing values needs: the items, the layouts, and the types `dyn` borrows point to (by
/// the number their vtables start with).
pub struct Show<'a> {
    /// The program's items.
    pub items: &'a ItemTree,
    /// The layouts the values have.
    pub layouts: Layouts<'a>,
    /// The types `dyn` borrows point to.
    pub dyn_types: &'a [Ty],
}

fn num_bytes(n: Num) -> u32 {
    n.bits() / 8
}

fn stride(l: &crate::layout::Layout) -> u32 {
    l.size.div_ceil(l.align) * l.align
}

impl Show<'_> {
    /// The value of type `ty` at `p`, as the interpreters write it.
    pub fn render(&mut self, mem: &dyn Memory, ty: &Ty, p: u64) -> String {
        let items = self.items;
        let Ok(l) = self.layouts.of(ty) else {
            return format!("<{ty}>");
        };
        let at = |off: u32| p + u64::from(off);
        match ty {
            Ty::Unit | Ty::Never => "()".to_string(),
            Ty::Int => (mem.bits(p, 8) as u64 as i64).to_string(),
            Ty::Bool => (mem.bits(p, 1) & 1 == 1).to_string(),
            Ty::Char => char::from_u32(mem.bits(p, 4) as u32)
                .map_or_else(|| "?".to_string(), |c| format!("{c:?}")),
            Ty::Num(n) => moddil_mir::num_text(*n, mem.bits(p, num_bytes(*n))),
            Ty::Ref(m, inner) => {
                let shown = match &**inner {
                    // Text is shown as itself, in quotes, as the interpreters show it.
                    Ty::Str => {
                        let q = mem.bits(p, 8) as u64;
                        let n = mem.bits(p + 8, 8) as usize;
                        let bytes = mem.bytes(q, n);
                        return format!("{:?}", String::from_utf8_lossy(&bytes));
                    }
                    Ty::Slice(elem) => {
                        let q = mem.bits(p, 8) as u64;
                        let n = mem.bits(p + 8, 8) as u32;
                        let st = self.layouts.of(elem).map_or(0, |l| stride(&l));
                        let xs: Vec<String> = (0..n)
                            .map(|k| self.render(mem, elem, q + u64::from(k * st)))
                            .collect();
                        format!("[{}]", xs.join(", "))
                    }
                    Ty::Dyn(..) => {
                        // The vtable starts with the number of the type it is for.
                        let q = mem.bits(p, 8) as u64;
                        let vt = mem.bits(p + 8, 8) as u64;
                        let k = mem.bits(vt, 8) as usize;
                        match self.dyn_types.get(k).cloned() {
                            Some(t) => self.render(mem, &t, q),
                            None => "…".to_string(),
                        }
                    }
                    inner => {
                        let q = mem.bits(p, 8) as u64;
                        self.render(mem, inner, q)
                    }
                };
                if *m {
                    format!("&mut {shown}")
                } else {
                    format!("&{shown}")
                }
            }
            Ty::Tuple(_) => {
                let Shape::Fields(fs) = &l.shape else {
                    return format!("<{ty}>");
                };
                let xs: Vec<String> = fs
                    .iter()
                    .map(|(off, t)| self.render(mem, t, at(*off)))
                    .collect();
                if xs.len() == 1 {
                    format!("({},)", xs[0])
                } else {
                    format!("({})", xs.join(", "))
                }
            }
            Ty::Array(..) => {
                let Shape::Array { elem, stride, len } = &l.shape else {
                    return format!("<{ty}>");
                };
                let xs: Vec<String> = (0..*len)
                    .map(|k| self.render(mem, elem, at(k * stride)))
                    .collect();
                format!("[{}]", xs.join(", "))
            }
            // An error set: the member its id says.
            Ty::Errors(ms) => self.error(mem, ms, p),
            Ty::Buf(elem) => {
                let q = mem.bits(p, 8) as u64;
                let n = mem.bits(p + 8, 8) as u32;
                let st = self.layouts.of(elem).map_or(0, |l| stride(&l));
                let xs: Vec<String> = (0..n)
                    .map(|k| self.render(mem, elem, q + u64::from(k * st)))
                    .collect();
                format!("[{}]", xs.join(", "))
            }
            // A `String` is shown as its text, in quotes; a `Vec` as the literal that makes one.
            Ty::Adt(a, ..)
                if items.adts[*a as usize].src == moddil_sema::Src::Prelude
                    && matches!(items.adts[*a as usize].name.as_str(), "String" | "Vec") =>
            {
                let Shape::Fields(fs) = &l.shape else {
                    return format!("<{ty}>");
                };
                let (off, inner) = fs[0].clone();
                if items.adts[*a as usize].name == "Vec" {
                    return self.render(mem, &inner, at(off));
                }
                self.string_text(mem, &inner, at(off))
            }
            Ty::Adt(a, ..) => {
                let (variant, fields) = match &l.shape {
                    Shape::Fields(fs) => (0, fs.clone()),
                    Shape::Enum { tag, variants } => {
                        let v = variant_at(mem, tag, variants.len() as u32, p);
                        (v, variants[v as usize].clone())
                    }
                    Shape::Empty => (0, Vec::new()),
                    _ => return format!("<{ty}>"),
                };
                let def = items.variant(*a, variant);
                let path = items.adts[*a as usize].path(variant);
                let xs: Vec<String> = fields
                    .iter()
                    .map(|(off, t)| self.render(mem, t, at(*off)))
                    .collect();
                match def.shape {
                    VariantShape::Unit => path,
                    VariantShape::Tuple => format!("{path}({})", xs.join(", ")),
                    VariantShape::Named => {
                        let named: Vec<String> = def
                            .fields
                            .iter()
                            .zip(&xs)
                            .map(|(f, x)| format!("{}: {x}", f.name))
                            .collect();
                        format!("{path} {{ {} }}", named.join(", "))
                    }
                }
            }
            t => format!("<{t}>"),
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
        let Shape::Enum { tag, variants } = &l.shape else {
            return None;
        };
        const CODES: [u8; 16] = [
            0, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76, 77, 78,
        ];
        let v = variant_at(mem, tag, variants.len() as u32, p);
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

    /// A `String`'s text, in quotes, from its `StringRepr` (of type `repr`) at `p`: inline bytes
    /// and their count, or a `Buf<u8>`.
    fn string_text(&mut self, mem: &dyn Memory, repr: &Ty, p: u64) -> String {
        let Ok(l) = self.layouts.of(repr) else {
            return "<String>".to_string();
        };
        let Shape::Enum { tag, variants } = &l.shape else {
            return "<String>".to_string();
        };
        let v = variant_at(mem, tag, variants.len() as u32, p);
        let fs = &variants[v as usize];
        let (bytes, n) = if fs.len() == 2 {
            // `Inline([u8; 23], u8)`.
            let n = mem.bits(p + u64::from(fs[1].0), 1) as usize;
            (p + u64::from(fs[0].0), n)
        } else {
            // `Heap(Buf<u8>)`.
            let b = p + u64::from(fs[0].0);
            (mem.bits(b, 8) as u64, mem.bits(b + 8, 8) as usize)
        };
        let text = mem.bytes(bytes, n);
        format!("{:?}", String::from_utf8_lossy(&text))
    }
}

/// Which of `n` variants the enum at `p` holds, by its tag.
fn variant_at(mem: &dyn Memory, tag: &Tag, n: u32, p: u64) -> u32 {
    match tag {
        Tag::None => 0,
        Tag::Direct { offset, ty } => mem.bits(p + u64::from(*offset), ty.bytes()) as u32,
        Tag::Niche { dataful, niche } => {
            let raw = mem.bits(p + u64::from(niche.offset), niche.ty.bytes());
            let rel = raw.wrapping_sub(niche.first);
            if rel < u128::from(n - 1) {
                let rel = rel as u32;
                if rel >= *dataful { rel + 1 } else { rel }
            } else {
                *dataful
            }
        }
    }
}

//! Showing values as the interpreters write them (`Point { x: 1, y: 2 }`, `Some(3)`, `&[1, 2]`),
//! from their bytes and a description of their type: [`Types`], a table of [`Node`]s, each a
//! type's layout as showing needs it, its parts by their index in the table (so a type can hold
//! itself through a pointer).
//!
//! The description is made by native code from its layouts (`moddil-native`'s `show`), for a
//! run's values, and written into a built program, whose `moddil-rt-main` shows `main`'s result
//! with it ([`Types::encode`], [`Types::decode`]). What holds the bytes is a [`Memory`].

/// Where values are read from, by address.
pub trait Memory {
    /// The bits of the `n` (at most 16) bytes at `addr`, little-endian.
    fn bits(&self, addr: u64, n: u32) -> u128;
    /// The `n` bytes at `addr`.
    fn bytes(&self, addr: u64, n: usize) -> Vec<u8>;
}

/// How an enum's variants are told apart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tag {
    /// One variant, or none.
    None,
    /// An integer at `offset`, the variant's index.
    Direct {
        /// Where it is.
        offset: u32,
        /// Its size.
        bytes: u32,
    },
    /// Variant `dataful` holds a payload; the `k`th other one is stored as `first + k` in a
    /// scalar of the payload.
    Niche {
        /// The variant with a payload.
        dataful: u32,
        /// Where the scalar is.
        offset: u32,
        /// Its size.
        bytes: u32,
        /// The value the first other variant is stored as.
        first: u128,
    },
}

/// What a borrow points to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pointee {
    /// Text: its address and length.
    Str,
    /// A slice: its address and length, and each element `stride` apart.
    Slice {
        /// The elements' type.
        elem: u32,
        /// Bytes from one element to the next.
        stride: u32,
    },
    /// A `dyn` value: its address and vtable, whose first word is the number of its type in
    /// [`Types::dyns`].
    Dyn,
    /// A value of type `0`, at the address.
    Sized(u32),
}

/// How a variant writes its fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fields {
    /// None: `A`.
    Unit,
    /// By position: `A(x, y)`.
    Tuple,
    /// By name: `A { x, y }`.
    Named,
}

/// A struct's or enum's variant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Variant {
    /// How it is written: `Shape::Circle`, `Point`.
    pub path: String,
    /// How its fields are written.
    pub fields: Fields,
    /// Its fields' names, for a named variant.
    pub names: Vec<String>,
    /// Its fields' offsets and types.
    pub parts: Vec<(u32, u32)>,
}

/// A type, as showing needs it. Parts are the types' indices in [`Types::nodes`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Node {
    /// `()` or `!`.
    Unit,
    /// `int`.
    Int,
    /// `bool`.
    Bool,
    /// `char`.
    Char,
    /// A sized integer or a float.
    Num {
        /// Its number code, as `moddil-rt`'s `num::text` takes it.
        code: u32,
        /// Its size.
        bytes: u32,
    },
    /// A borrow.
    Ref {
        /// Whether it is `&mut`.
        mutable: bool,
        /// What it points to.
        to: Pointee,
    },
    /// A tuple's fields: offset and type.
    Tuple(Vec<(u32, u32)>),
    /// An array.
    Array {
        /// Its elements' type.
        elem: u32,
        /// Bytes from one element to the next.
        stride: u32,
        /// How many.
        len: u32,
    },
    /// A `Buf<T>`: its address and length.
    Buf {
        /// Its elements' type.
        elem: u32,
        /// Bytes from one element to the next.
        stride: u32,
    },
    /// A `Vec`: shown as its `Buf`.
    Vec {
        /// Where its `Buf` is.
        offset: u32,
        /// The `Buf`'s type.
        buf: u32,
    },
    /// A `String`: its text, from its repr at `offset`, an enum told apart by `tag` whose
    /// variants' field offsets are `variants` (two fields: inline bytes and their count; one:
    /// a `Buf<u8>`).
    String {
        /// Where its repr is.
        offset: u32,
        /// How the repr's variants are told apart.
        tag: Tag,
        /// Each variant's fields' offsets.
        variants: Vec<Vec<u32>>,
    },
    /// A struct or an enum.
    Adt {
        /// How its variants are told apart.
        tag: Tag,
        /// Its variants (a struct's one).
        variants: Vec<Variant>,
    },
    /// An error set: the member's id, a word, then its value.
    Errors {
        /// Where the value is.
        value: u32,
        /// Each member's id and type.
        members: Vec<(u32, u32)>,
    },
    /// Something not shown: its name.
    Opaque(String),
}

/// A table of type descriptions.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Types {
    /// The types, by index.
    pub nodes: Vec<Node>,
    /// The types `dyn` borrows point to, by the number their vtables start with.
    pub dyns: Vec<u32>,
}

impl Types {
    /// The value of type `ty` (an index into the table) at `p`, as the interpreters write it.
    pub fn render(&self, mem: &dyn Memory, ty: u32, p: u64) -> String {
        let Some(node) = self.nodes.get(ty as usize) else {
            return "<?>".to_string();
        };
        let at = |off: u32| p + u64::from(off);
        match node {
            Node::Unit => "()".to_string(),
            Node::Int => (mem.bits(p, 8) as u64 as i64).to_string(),
            Node::Bool => (mem.bits(p, 1) & 1 == 1).to_string(),
            Node::Char => char::from_u32(mem.bits(p, 4) as u32)
                .map_or_else(|| "?".to_string(), |c| format!("{c:?}")),
            Node::Num { code, bytes } => moddil_rt::num::text(*code, mem.bits(p, *bytes)),
            Node::Ref { mutable, to } => {
                let shown = match to {
                    // Text is shown as itself, in quotes, as the interpreters show it.
                    Pointee::Str => {
                        let q = mem.bits(p, 8) as u64;
                        let n = mem.bits(p + 8, 8) as usize;
                        return format!("{:?}", String::from_utf8_lossy(&mem.bytes(q, n)));
                    }
                    Pointee::Slice { elem, stride } => {
                        let q = mem.bits(p, 8) as u64;
                        let n = mem.bits(p + 8, 8) as u32;
                        self.list(mem, *elem, q, *stride, n)
                    }
                    Pointee::Dyn => {
                        let q = mem.bits(p, 8) as u64;
                        let vt = mem.bits(p + 8, 8) as u64;
                        let k = mem.bits(vt, 8) as usize;
                        match self.dyns.get(k) {
                            Some(&t) => self.render(mem, t, q),
                            None => "…".to_string(),
                        }
                    }
                    Pointee::Sized(t) => self.render(mem, *t, mem.bits(p, 8) as u64),
                };
                if *mutable {
                    format!("&mut {shown}")
                } else {
                    format!("&{shown}")
                }
            }
            Node::Tuple(fs) => {
                let xs: Vec<String> = fs
                    .iter()
                    .map(|(off, t)| self.render(mem, *t, at(*off)))
                    .collect();
                if xs.len() == 1 {
                    format!("({},)", xs[0])
                } else {
                    format!("({})", xs.join(", "))
                }
            }
            Node::Array { elem, stride, len } => self.list(mem, *elem, p, *stride, *len),
            Node::Buf { elem, stride } => {
                let q = mem.bits(p, 8) as u64;
                let n = mem.bits(p + 8, 8) as u32;
                self.list(mem, *elem, q, *stride, n)
            }
            Node::Vec { offset, buf } => self.render(mem, *buf, at(*offset)),
            Node::String {
                offset,
                tag,
                variants,
            } => {
                let p = at(*offset);
                let v = variant_at(mem, tag, variants.len() as u32, p);
                let fs = &variants[v as usize];
                let (bytes, n) = if fs.len() == 2 {
                    // `Inline([u8; 23], u8)`.
                    let n = mem.bits(p + u64::from(fs[1]), 1) as usize;
                    (p + u64::from(fs[0]), n)
                } else {
                    // `Heap(Buf<u8>)`.
                    let b = p + u64::from(fs[0]);
                    (mem.bits(b, 8) as u64, mem.bits(b + 8, 8) as usize)
                };
                format!("{:?}", String::from_utf8_lossy(&mem.bytes(bytes, n)))
            }
            Node::Adt { tag, variants } => {
                let v = variant_at(mem, tag, variants.len() as u32, p);
                let Some(def) = variants.get(v as usize) else {
                    return "<?>".to_string();
                };
                let xs: Vec<String> = def
                    .parts
                    .iter()
                    .map(|(off, t)| self.render(mem, *t, at(*off)))
                    .collect();
                match def.fields {
                    Fields::Unit => def.path.clone(),
                    Fields::Tuple => format!("{}({})", def.path, xs.join(", ")),
                    Fields::Named => {
                        let named: Vec<String> = def
                            .names
                            .iter()
                            .zip(&xs)
                            .map(|(f, x)| format!("{f}: {x}"))
                            .collect();
                        format!("{} {{ {} }}", def.path, named.join(", "))
                    }
                }
            }
            Node::Errors { value, members } => {
                let id = mem.bits(p, 8) as u32;
                match members.iter().find(|(m, _)| *m == id) {
                    Some(&(_, t)) => self.render(mem, t, at(*value)),
                    None => "<an error>".to_string(),
                }
            }
            Node::Opaque(name) => format!("<{name}>"),
        }
    }

    /// `n` elements of type `elem` from `q`, `stride` apart, as a list.
    fn list(&self, mem: &dyn Memory, elem: u32, q: u64, stride: u32, n: u32) -> String {
        let xs: Vec<String> = (0..n)
            .map(|k| self.render(mem, elem, q + u64::from(k) * u64::from(stride)))
            .collect();
        format!("[{}]", xs.join(", "))
    }

    /// The table as bytes, for a built program to hold.
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer(Vec::new());
        w.u32(self.nodes.len() as u32);
        for n in &self.nodes {
            w.node(n);
        }
        w.list(&self.dyns);
        w.0
    }

    /// The table [`Types::encode`] wrote, or `None` if `bytes` is not one.
    pub fn decode(bytes: &[u8]) -> Option<Types> {
        let mut r = Reader(bytes);
        let n = r.u32()?;
        let mut nodes = Vec::new();
        for _ in 0..n {
            nodes.push(r.node()?);
        }
        let dyns = r.list()?;
        Some(Types { nodes, dyns })
    }
}

/// Which of `n` variants the enum at `p` holds, by its tag.
pub fn variant_at(mem: &dyn Memory, tag: &Tag, n: u32, p: u64) -> u32 {
    match tag {
        Tag::None => 0,
        Tag::Direct { offset, bytes } => mem.bits(p + u64::from(*offset), *bytes) as u32,
        Tag::Niche {
            dataful,
            offset,
            bytes,
            first,
        } => {
            let raw = mem.bits(p + u64::from(*offset), *bytes);
            let rel = raw.wrapping_sub(*first);
            if rel < u128::from(n.saturating_sub(1)) {
                let rel = rel as u32;
                if rel >= *dataful { rel + 1 } else { rel }
            } else {
                *dataful
            }
        }
    }
}

struct Writer(Vec<u8>);

impl Writer {
    fn u32(&mut self, x: u32) {
        self.0.extend_from_slice(&x.to_le_bytes());
    }

    fn u128(&mut self, x: u128) {
        self.0.extend_from_slice(&x.to_le_bytes());
    }

    fn text(&mut self, s: &str) {
        self.u32(s.len() as u32);
        self.0.extend_from_slice(s.as_bytes());
    }

    fn list(&mut self, xs: &[u32]) {
        self.u32(xs.len() as u32);
        for &x in xs {
            self.u32(x);
        }
    }

    fn pairs(&mut self, xs: &[(u32, u32)]) {
        self.u32(xs.len() as u32);
        for &(a, b) in xs {
            self.u32(a);
            self.u32(b);
        }
    }

    fn tag(&mut self, t: &Tag) {
        match t {
            Tag::None => self.u32(0),
            Tag::Direct { offset, bytes } => {
                self.u32(1);
                self.u32(*offset);
                self.u32(*bytes);
            }
            Tag::Niche {
                dataful,
                offset,
                bytes,
                first,
            } => {
                self.u32(2);
                self.u32(*dataful);
                self.u32(*offset);
                self.u32(*bytes);
                self.u128(*first);
            }
        }
    }

    fn node(&mut self, n: &Node) {
        match n {
            Node::Unit => self.u32(0),
            Node::Int => self.u32(1),
            Node::Bool => self.u32(2),
            Node::Char => self.u32(3),
            Node::Num { code, bytes } => {
                self.u32(4);
                self.u32(*code);
                self.u32(*bytes);
            }
            Node::Ref { mutable, to } => {
                self.u32(5);
                self.u32(u32::from(*mutable));
                match to {
                    Pointee::Str => self.u32(0),
                    Pointee::Slice { elem, stride } => {
                        self.u32(1);
                        self.u32(*elem);
                        self.u32(*stride);
                    }
                    Pointee::Dyn => self.u32(2),
                    Pointee::Sized(t) => {
                        self.u32(3);
                        self.u32(*t);
                    }
                }
            }
            Node::Tuple(fs) => {
                self.u32(6);
                self.pairs(fs);
            }
            Node::Array { elem, stride, len } => {
                self.u32(7);
                self.u32(*elem);
                self.u32(*stride);
                self.u32(*len);
            }
            Node::Buf { elem, stride } => {
                self.u32(8);
                self.u32(*elem);
                self.u32(*stride);
            }
            Node::Vec { offset, buf } => {
                self.u32(9);
                self.u32(*offset);
                self.u32(*buf);
            }
            Node::String {
                offset,
                tag,
                variants,
            } => {
                self.u32(10);
                self.u32(*offset);
                self.tag(tag);
                self.u32(variants.len() as u32);
                for v in variants {
                    self.list(v);
                }
            }
            Node::Adt { tag, variants } => {
                self.u32(11);
                self.tag(tag);
                self.u32(variants.len() as u32);
                for v in variants {
                    self.text(&v.path);
                    self.u32(match v.fields {
                        Fields::Unit => 0,
                        Fields::Tuple => 1,
                        Fields::Named => 2,
                    });
                    self.u32(v.names.len() as u32);
                    for n in &v.names {
                        self.text(n);
                    }
                    self.pairs(&v.parts);
                }
            }
            Node::Errors { value, members } => {
                self.u32(12);
                self.u32(*value);
                self.pairs(members);
            }
            Node::Opaque(name) => {
                self.u32(13);
                self.text(name);
            }
        }
    }
}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> Option<[u8; N]> {
        let (head, rest) = self.0.split_at_checked(N)?;
        self.0 = rest;
        head.try_into().ok()
    }

    fn u32(&mut self) -> Option<u32> {
        self.take::<4>().map(u32::from_le_bytes)
    }

    fn u128(&mut self) -> Option<u128> {
        self.take::<16>().map(u128::from_le_bytes)
    }

    fn text(&mut self) -> Option<String> {
        let n = self.u32()? as usize;
        let (head, rest) = self.0.split_at_checked(n)?;
        self.0 = rest;
        String::from_utf8(head.to_vec()).ok()
    }

    fn list(&mut self) -> Option<Vec<u32>> {
        let n = self.u32()?;
        (0..n).map(|_| self.u32()).collect()
    }

    fn pairs(&mut self) -> Option<Vec<(u32, u32)>> {
        let n = self.u32()?;
        (0..n).map(|_| Some((self.u32()?, self.u32()?))).collect()
    }

    fn tag(&mut self) -> Option<Tag> {
        Some(match self.u32()? {
            0 => Tag::None,
            1 => Tag::Direct {
                offset: self.u32()?,
                bytes: self.u32()?,
            },
            2 => Tag::Niche {
                dataful: self.u32()?,
                offset: self.u32()?,
                bytes: self.u32()?,
                first: self.u128()?,
            },
            _ => return None,
        })
    }

    fn node(&mut self) -> Option<Node> {
        Some(match self.u32()? {
            0 => Node::Unit,
            1 => Node::Int,
            2 => Node::Bool,
            3 => Node::Char,
            4 => Node::Num {
                code: self.u32()?,
                bytes: self.u32()?,
            },
            5 => {
                let mutable = self.u32()? == 1;
                let to = match self.u32()? {
                    0 => Pointee::Str,
                    1 => Pointee::Slice {
                        elem: self.u32()?,
                        stride: self.u32()?,
                    },
                    2 => Pointee::Dyn,
                    3 => Pointee::Sized(self.u32()?),
                    _ => return None,
                };
                Node::Ref { mutable, to }
            }
            6 => Node::Tuple(self.pairs()?),
            7 => Node::Array {
                elem: self.u32()?,
                stride: self.u32()?,
                len: self.u32()?,
            },
            8 => Node::Buf {
                elem: self.u32()?,
                stride: self.u32()?,
            },
            9 => Node::Vec {
                offset: self.u32()?,
                buf: self.u32()?,
            },
            10 => {
                let offset = self.u32()?;
                let tag = self.tag()?;
                let n = self.u32()?;
                let variants = (0..n).map(|_| self.list()).collect::<Option<_>>()?;
                Node::String {
                    offset,
                    tag,
                    variants,
                }
            }
            11 => {
                let tag = self.tag()?;
                let n = self.u32()?;
                let mut variants = Vec::new();
                for _ in 0..n {
                    let path = self.text()?;
                    let fields = match self.u32()? {
                        0 => Fields::Unit,
                        1 => Fields::Tuple,
                        _ => Fields::Named,
                    };
                    let k = self.u32()?;
                    let names = (0..k).map(|_| self.text()).collect::<Option<_>>()?;
                    let parts = self.pairs()?;
                    variants.push(Variant {
                        path,
                        fields,
                        names,
                        parts,
                    });
                }
                Node::Adt { tag, variants }
            }
            12 => Node::Errors {
                value: self.u32()?,
                members: self.pairs()?,
            },
            13 => Node::Opaque(self.text()?),
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Memory of plain bytes, from address 0.
    struct Bytes(Vec<u8>);

    impl Memory for Bytes {
        fn bits(&self, addr: u64, n: u32) -> u128 {
            let mut b = [0u8; 16];
            let a = addr as usize;
            b[..n as usize].copy_from_slice(&self.0[a..a + n as usize]);
            u128::from_le_bytes(b)
        }

        fn bytes(&self, addr: u64, n: usize) -> Vec<u8> {
            self.0[addr as usize..addr as usize + n].to_vec()
        }
    }

    #[test]
    fn a_struct_of_an_int_and_a_bool_round_trips_and_renders() {
        let types = Types {
            nodes: vec![
                Node::Adt {
                    tag: Tag::None,
                    variants: vec![Variant {
                        path: "P".to_string(),
                        fields: Fields::Named,
                        names: vec!["x".to_string(), "ok".to_string()],
                        parts: vec![(0, 1), (8, 2)],
                    }],
                },
                Node::Int,
                Node::Bool,
            ],
            dyns: vec![],
        };
        let back = Types::decode(&types.encode()).expect("its own encoding");
        assert_eq!(back, types);
        let mut mem = (-3i64).to_le_bytes().to_vec();
        mem.push(1);
        assert_eq!(back.render(&Bytes(mem), 0, 0), "P { x: -3, ok: true }");
    }
}

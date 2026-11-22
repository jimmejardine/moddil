//! Running compiled code, and reading the values it observes: the one place `unsafe` is allowed,
//! to call the code Cranelift made and to read its values' bytes while they are alive.

#![allow(unsafe_code)]

use moddil_mir::TrapClass;
use moddil_sema::{ItemTree, Num, Shape as VariantShape, Ty};

use crate::layout::{Layouts, Shape, Tag};
use crate::lower::{Compiled, Site};
use crate::{Native, Observed};

/// How much stack a run has: deep enough for the frames `moddil-rt` lets a run have.
const STACK: usize = 512 * 1024 * 1024;

/// What the observer needs while a run goes on: the sites, the layouts, and what it has seen.
struct Seen<'a> {
    sites: &'a [Site],
    show: Show<'a>,
    observed: Vec<Observed>,
}

/// What showing a value needs: the items, the layouts, and the types `dyn` borrows point to.
struct Show<'a> {
    items: &'a ItemTree,
    layouts: Layouts<'a>,
    dyn_types: &'a [Ty],
}

/// `moddil-rt`'s observer: renders the value at `addr` for `site`.
fn observe(cx: *mut (), site: u32, addr: *const u8) {
    // SAFETY: `cx` is the `Seen` the run installed, on the running thread, alive until the run
    // ends and the observer is removed.
    let seen = unsafe { &mut *cx.cast::<Seen<'_>>() };
    if let Some(Site::Observe(file, range, ty)) = seen.sites.get(site as usize) {
        let ty = ty.clone();
        // SAFETY: the generated code passes the address of a live value of type `ty`.
        let text = unsafe { seen.show.render(&ty, addr) };
        seen.observed.push((*file, *range, text));
    }
}

impl Compiled<'_> {
    /// Runs the entry point, on a thread of its own with a large stack: the values it observed,
    /// and its result as Moddil source, or the trap.
    pub(crate) fn run(self) -> (Vec<Observed>, Result<String, Native>) {
        let ptr = self.module.get_finalized_function(self.entry) as usize;
        let drop_ptr = self.module.get_finalized_function(self.drop_result) as usize;
        let (sites, items, ret) = (&self.sites, self.items, &self.ret);
        let dyn_types = &self.dyn_types;
        let errors = &self.errors;
        let (status, text, observed, record, leaked) = std::thread::scope(|s| {
            std::thread::Builder::new()
                .stack_size(STACK)
                .spawn_scoped(s, move || {
                    moddil_rt::take();
                    crate::host::clear_texts();
                    let live = moddil_rt::live();
                    let mut seen = Seen {
                        sites,
                        show: Show {
                            items,
                            layouts: Layouts::new(items),
                            dyn_types,
                        },
                        observed: Vec::new(),
                    };
                    moddil_rt::set_observer(Some((observe, (&raw mut seen).cast())));
                    // Room for any result, aligned for any field.
                    let size = seen.show.layouts.of(ret).map_or(16, |l| l.size.max(16)) as usize;
                    let mut out = vec![0u128; size.div_ceil(16)];
                    // And for any error, after its id.
                    let room = errors
                        .iter()
                        .map(|t| seen.show.layouts.of(t).map_or(0, |l| l.size))
                        .max()
                        .unwrap_or(0) as usize;
                    let mut err = vec![0u128; (8 + room).div_ceil(16).max(1)];
                    // SAFETY: `ptr` is the finished entry trampoline, whose signature is
                    // `fn(*mut u8, *mut u8) -> i8` in the platform's C calling convention; the
                    // module that owns its memory lives until this scope ends; `out` has room
                    // for the result and `err` for an error.
                    let entry: extern "C" fn(*mut u8, *mut u8) -> i8 =
                        unsafe { std::mem::transmute::<usize, _>(ptr) };
                    let status = entry(out.as_mut_ptr().cast(), err.as_mut_ptr().cast());
                    moddil_rt::set_observer(None);
                    let text = match status {
                        // SAFETY: the run returned, so `out` holds a value of type `ret`.
                        0 => Some(unsafe { seen.show.render(ret, out.as_ptr().cast()) }),
                        // SAFETY: the run failed, so `err` holds an error of those types.
                        2 => Some(unsafe { seen.show.error(errors, err.as_ptr().cast()) }),
                        _ => None,
                    };
                    // Once shown, the result is dropped; then a run that returned has freed
                    // everything it allocated.
                    let mut leaked = 0;
                    let mut status = status;
                    if status == 0 {
                        // SAFETY: `drop_ptr` is the finished `drop_result`, of the same
                        // signature as the entry, and `out` holds the result it drops.
                        let drop: extern "C" fn(*mut u8) -> i8 =
                            unsafe { std::mem::transmute::<usize, _>(drop_ptr) };
                        status = drop(out.as_mut_ptr().cast());
                        if status == 0 {
                            leaked = moddil_rt::live() - live;
                        }
                    }
                    (status, text, seen.observed, moddil_rt::take(), leaked)
                })
                .expect("a thread to run on")
                .join()
                .expect("native code does not panic")
        });
        let result = match text {
            Some(_) if status == 0 && leaked != 0 => Err(Native::Leak(leaked)),
            Some(t) if status == 0 => Ok(t),
            // The error left the run: where the last `fail` was.
            Some(t) if status == 2 => match record.failed.map(|s| &self.sites[s as usize]) {
                Some(Site::Fail(file, range)) => Err(Native::Failed(*file, *range, t)),
                _ => Err(Native::Unsupported("an error without a site".to_string())),
            },
            _ => match record.trap {
                Some(t) => Ok(self.reported(&t)).and_then(Err),
                None => Err(Native::Unsupported("a trap without a site".to_string())),
            },
        };
        // SAFETY: nothing runs the module's code after this.
        unsafe { self.module.free_memory() };
        (observed, result)
    }
}

impl Compiled<'_> {
    /// A trap as the interpreters report it: where it happened, or, in tracked code, at the
    /// innermost call untracked code made.
    fn reported(&self, t: &moddil_rt::Trapped) -> Native {
        let (class, file, range) = match &self.sites[t.site as usize] {
            Site::Trap(class, file, range) => (*class, *file, *range),
            Site::Call(file, range, _) => (TrapClass::StackOverflow, *file, *range),
            Site::Observe(..) | Site::Fail(..) => {
                return Native::Unsupported("a trap at an observation".to_string());
            }
        };
        if !t.tracked {
            return Native::Trap(class, file, range);
        }
        let at = t
            .stack
            .iter()
            .rev()
            .find_map(|s| match &self.sites[*s as usize] {
                Site::Call(file, range, false) => Some((*file, *range)),
                _ => None,
            });
        let (file, range) = at.unwrap_or((file, range));
        Native::Trap(class, file, range)
    }
}

/// The bits of the `n` bytes at `p`, little-endian.
///
/// # Safety
///
/// `p` points to `n` (at most 16) readable bytes.
unsafe fn bits_at(p: *const u8, n: u32) -> u128 {
    let mut bytes = [0u8; 16];
    // SAFETY: as the caller says.
    unsafe { std::ptr::copy_nonoverlapping(p, bytes.as_mut_ptr(), n as usize) };
    u128::from_le_bytes(bytes)
}

impl Show<'_> {
    /// The value of type `ty` at `p`, as the interpreters write it.
    ///
    /// # Safety
    ///
    /// `p` points to a live value of type `ty`, whose borrows point to live values.
    unsafe fn render(&mut self, ty: &Ty, p: *const u8) -> String {
        let items = self.items;
        let Ok(l) = self.layouts.of(ty) else {
            return format!("<{ty}>");
        };
        // SAFETY (for every read below): `p` is a live value of `ty`, laid out as `l`.
        let field = |off: u32| unsafe { p.add(off as usize) };
        match ty {
            Ty::Unit | Ty::Never => "()".to_string(),
            Ty::Int => (unsafe { bits_at(p, 8) } as u64 as i64).to_string(),
            Ty::Bool => (unsafe { bits_at(p, 1) } & 1 == 1).to_string(),
            Ty::Char => char::from_u32(unsafe { bits_at(p, 4) } as u32)
                .map_or_else(|| "?".to_string(), |c| format!("{c:?}")),
            Ty::Num(n) => moddil_mir::num_text(*n, unsafe { bits_at(p, num_bytes(*n)) }),
            Ty::Ref(m, inner) => {
                let q = unsafe { bits_at(p, 8) } as usize as *const u8;
                let shown = match &**inner {
                    // Text is shown as itself, in quotes, as the interpreters show it.
                    Ty::Str => {
                        let n = unsafe { bits_at(p.add(8), 8) } as usize;
                        // SAFETY: a `&str` is the address and length of live UTF-8.
                        let bytes = unsafe { std::slice::from_raw_parts(q, n) };
                        return format!("{:?}", String::from_utf8_lossy(bytes));
                    }
                    Ty::Slice(elem) => {
                        let n = unsafe { bits_at(p.add(8), 8) } as u32;
                        let stride = self
                            .layouts
                            .of(elem)
                            .map_or(0, |l| l.size.div_ceil(l.align) * l.align);
                        let xs: Vec<String> = (0..n)
                            .map(|k| unsafe { self.render(elem, q.add((k * stride) as usize)) })
                            .collect();
                        format!("[{}]", xs.join(", "))
                    }
                    Ty::Dyn(..) => {
                        // The vtable starts with the number of the type it is for.
                        let vt = unsafe { bits_at(p.add(8), 8) } as usize as *const u8;
                        let k = unsafe { bits_at(vt, 8) } as usize;
                        match self.dyn_types.get(k).cloned() {
                            Some(t) => unsafe { self.render(&t, q) },
                            None => "…".to_string(),
                        }
                    }
                    inner => unsafe { self.render(inner, q) },
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
                    .map(|(off, t)| unsafe { self.render(t, field(*off)) })
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
                    .map(|k| unsafe { self.render(elem, field(k * stride)) })
                    .collect();
                format!("[{}]", xs.join(", "))
            }
            // An error set: the member its id says.
            Ty::Errors(ms) => unsafe { self.error(ms, p) },
            Ty::Buf(elem) => {
                let q = unsafe { bits_at(p, 8) } as usize as *const u8;
                let n = unsafe { bits_at(p.add(8), 8) } as u32;
                let stride = self
                    .layouts
                    .of(elem)
                    .map_or(0, |l| l.size.div_ceil(l.align) * l.align);
                let xs: Vec<String> = (0..n)
                    .map(|k| unsafe { self.render(elem, q.add((k * stride) as usize)) })
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
                    return unsafe { self.render(&inner, field(off)) };
                }
                unsafe { self.string_text(&inner, field(off)) }
            }
            Ty::Adt(a, ..) => {
                let (variant, fields) = match &l.shape {
                    Shape::Fields(fs) => (0, fs.clone()),
                    Shape::Enum { tag, variants } => {
                        let v = unsafe { variant_at(tag, variants.len() as u32, p) };
                        (v, variants[v as usize].clone())
                    }
                    Shape::Empty => (0, Vec::new()),
                    _ => return format!("<{ty}>"),
                };
                let def = items.variant(*a, variant);
                let path = items.adts[*a as usize].path(variant);
                let xs: Vec<String> = fields
                    .iter()
                    .map(|(off, t)| unsafe { self.render(t, field(*off)) })
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
}

impl Show<'_> {
    /// The error at `p`, tagged: its value, as the member of `errors` its id says.
    ///
    /// # Safety
    ///
    /// `p` points to a live error of one of `errors`, after its id.
    unsafe fn error(&mut self, errors: &[Ty], p: *const u8) -> String {
        let id = unsafe { bits_at(p, 8) } as u32;
        match errors.iter().find(|t| t.adt() == Some(id)).cloned() {
            Some(t) => unsafe { self.render(&t, p.add(crate::lower::ERROR_VALUE as usize)) },
            None => "<an error>".to_string(),
        }
    }

    /// A `String`'s text, in quotes, from its `StringRepr` (of type `repr`) at `p`: inline bytes
    /// and their count, or a `Buf<u8>`.
    ///
    /// # Safety
    ///
    /// `p` points to a live `StringRepr`.
    unsafe fn string_text(&mut self, repr: &Ty, p: *const u8) -> String {
        let Ok(l) = self.layouts.of(repr) else {
            return "<String>".to_string();
        };
        let Shape::Enum { tag, variants } = &l.shape else {
            return "<String>".to_string();
        };
        let v = unsafe { variant_at(tag, variants.len() as u32, p) };
        let fs = &variants[v as usize];
        let (bytes, n) = if fs.len() == 2 {
            // `Inline([u8; 23], u8)`.
            let n = unsafe { bits_at(p.add(fs[1].0 as usize), 1) } as usize;
            (unsafe { p.add(fs[0].0 as usize) }, n)
        } else {
            // `Heap(Buf<u8>)`.
            let b = unsafe { p.add(fs[0].0 as usize) };
            let q = unsafe { bits_at(b, 8) } as usize as *const u8;
            (q, unsafe { bits_at(b.add(8), 8) } as usize)
        };
        // SAFETY: a `String` holds `n` bytes of UTF-8 there.
        let text = unsafe { std::slice::from_raw_parts(bytes, n) };
        format!("{:?}", String::from_utf8_lossy(text))
    }
}

/// Which of `n` variants the enum at `p` holds, by its tag.
///
/// # Safety
///
/// `p` points to a live enum laid out with `tag`.
unsafe fn variant_at(tag: &Tag, n: u32, p: *const u8) -> u32 {
    match tag {
        Tag::None => 0,
        Tag::Direct { offset, ty } => {
            // SAFETY: the tag is inside the value.
            (unsafe { bits_at(p.add(*offset as usize), ty.bytes()) }) as u32
        }
        Tag::Niche { dataful, niche } => {
            // SAFETY: the niche field is inside the value.
            let raw = unsafe { bits_at(p.add(niche.offset as usize), niche.ty.bytes()) };
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

fn num_bytes(n: Num) -> u32 {
    n.bits() / 8
}

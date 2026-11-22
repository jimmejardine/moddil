//! Running compiled code, and reading the values it observes: the one place `unsafe` is allowed,
//! to call the code Cranelift made and to read its values' bytes while they are alive.

#![allow(unsafe_code)]

use moddil_sema::{ItemTree, Num, Shape as VariantShape, Ty};

use crate::layout::{Layouts, Shape, Tag};
use crate::lower::{Compiled, Site};
use crate::{Native, Observed};

/// How much stack a run has: deep enough for the frames `moddil-rt` lets a run have.
const STACK: usize = 512 * 1024 * 1024;

/// What the observer needs while a run goes on: the sites, the layouts, and what it has seen.
struct Seen<'a> {
    sites: &'a [Site],
    layouts: Layouts<'a>,
    items: &'a ItemTree,
    observed: Vec<Observed>,
}

/// `moddil-rt`'s observer: renders the value at `addr` for `site`.
fn observe(cx: *mut (), site: u32, addr: *const u8) {
    // SAFETY: `cx` is the `Seen` the run installed, on the running thread, alive until the run
    // ends and the observer is removed.
    let seen = unsafe { &mut *cx.cast::<Seen<'_>>() };
    if let Some(Site::Observe(file, range, ty)) = seen.sites.get(site as usize) {
        let ty = ty.clone();
        // SAFETY: the generated code passes the address of a live value of type `ty`.
        let text = unsafe { render(seen.items, &mut seen.layouts, &ty, addr) };
        seen.observed.push((*file, *range, text));
    }
}

impl Compiled<'_> {
    /// Runs the entry point, on a thread of its own with a large stack: the values it observed,
    /// and its result as Moddil source, or the trap.
    pub(crate) fn run(self) -> (Vec<Observed>, Result<String, Native>) {
        let ptr = self.module.get_finalized_function(self.entry) as usize;
        let (sites, items, ret) = (&self.sites, self.items, &self.ret);
        let (status, text, observed, record) = std::thread::scope(|s| {
            std::thread::Builder::new()
                .stack_size(STACK)
                .spawn_scoped(s, move || {
                    moddil_rt::take();
                    let mut seen = Seen {
                        sites,
                        layouts: Layouts::new(items),
                        items,
                        observed: Vec::new(),
                    };
                    moddil_rt::set_observer(Some((observe, (&raw mut seen).cast())));
                    // Room for any result, aligned for any field.
                    let size = seen.layouts.of(ret).map_or(16, |l| l.size.max(16)) as usize;
                    let mut out = vec![0u128; size.div_ceil(16)];
                    // SAFETY: `ptr` is the finished entry trampoline, whose signature is
                    // `fn(*mut u8) -> i8` in the platform's C calling convention; the module
                    // that owns its memory lives until this scope ends; `out` has room for the
                    // result.
                    let entry: extern "C" fn(*mut u8) -> i8 =
                        unsafe { std::mem::transmute::<usize, _>(ptr) };
                    let status = entry(out.as_mut_ptr().cast());
                    moddil_rt::set_observer(None);
                    let text = (status == 0).then(|| {
                        // SAFETY: the run returned, so `out` holds a value of type `ret`.
                        unsafe { render(items, &mut seen.layouts, ret, out.as_ptr().cast()) }
                    });
                    (status, text, seen.observed, moddil_rt::take())
                })
                .expect("a thread to run on")
                .join()
                .expect("native code does not panic")
        });
        let result = match text {
            Some(t) if status == 0 => Ok(t),
            _ => match record.trap.map(|s| &self.sites[s as usize]) {
                Some(Site::Trap(class, file, range)) => Err(Native::Trap(*class, *file, *range)),
                _ => Err(Native::Unsupported("a trap without a site".to_string())),
            },
        };
        // SAFETY: nothing runs the module's code after this.
        unsafe { self.module.free_memory() };
        (observed, result)
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

/// The value of type `ty` at `p`, as the interpreters write it.
///
/// # Safety
///
/// `p` points to a live value of type `ty`, whose borrows point to live values.
unsafe fn render(items: &ItemTree, layouts: &mut Layouts<'_>, ty: &Ty, p: *const u8) -> String {
    let Ok(l) = layouts.of(ty) else {
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
            let shown = unsafe { render(items, layouts, inner, q) };
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
                .map(|(off, t)| unsafe { render(items, layouts, t, field(*off)) })
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
                .map(|k| unsafe { render(items, layouts, elem, field(k * stride)) })
                .collect();
            format!("[{}]", xs.join(", "))
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
                .map(|(off, t)| unsafe { render(items, layouts, t, field(*off)) })
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

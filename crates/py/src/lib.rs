//! Python bindings for a Moddil mod (`moddil build --wheel`, `reference/2027/interop.md`): from
//! its `#[export(python)]` functions and structs, three generated texts.
//!
//! - **Moddil shims**, a mod of their own over the user's: an `#[export(status)]` C function for
//!   each Python function, method, property and value conversion, taking C pieces (scalars,
//!   pointers and lengths, handles: a `Box`'s raw pointer) and giving a status (0 ok, 1 a `fail`
//!   error, 2 a trap) with its results through out-parameters.
//! - **C glue** over them, against CPython's stable ABI (3.11): the module, its functions, its
//!   classes, its exceptions, and the conversions between Python values and C pieces (any
//!   C-contiguous buffer of the right format crosses without a copy).
//! - **`.pyi` stubs**, the module's types for editors and type checkers.

use std::fmt::Write as _;

use moddil_sema::{Container, ItemTree, Num, SelfKind, Src, Ty, Vis};

/// What [`generate`] gives.
pub struct Bindings {
    /// The shims' Moddil source: the root module of a mod depending on the user's.
    pub shims: String,
    /// The C glue.
    pub glue: String,
    /// The stubs.
    pub pyi: String,
}

/// A type as it crosses to Python.
#[derive(Clone, Debug, PartialEq)]
enum PyTy {
    Unit,
    Bool,
    /// An integer: its Moddil type, and its C type.
    Int(&'static str, &'static str, bool),
    /// A float: its Moddil type, and its C type.
    Float(&'static str, &'static str),
    /// `&str`.
    Str,
    /// `String`.
    String,
    /// `&[T]` or `&mut [T]` of a number.
    Slice(Elem, bool),
    /// `Vec<T>` of a number.
    Vec(Elem),
    /// `Option<T>`, as a result.
    Option(Box<PyTy>),
    /// An exported struct: by value, or by `&` (`false`) or `&mut` (`true`).
    Struct(u32, Option<bool>),
}

/// A number as an element of a slice or a `Vec`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Elem {
    /// Its Moddil type.
    moddil: &'static str,
    /// Its C type.
    c: &'static str,
    /// Its kind in a buffer's format: `i` signed, `u` unsigned, `f` float.
    kind: char,
    /// Its size.
    size: u8,
}

fn number(n: Num) -> Option<(&'static str, &'static str, char, u8)> {
    Some(match n {
        Num::I8 => ("i8", "int8_t", 'i', 1),
        Num::U8 => ("u8", "uint8_t", 'u', 1),
        Num::I16 => ("i16", "int16_t", 'i', 2),
        Num::U16 => ("u16", "uint16_t", 'u', 2),
        Num::I32 => ("i32", "int32_t", 'i', 4),
        Num::U32 => ("u32", "uint32_t", 'u', 4),
        Num::U64 => ("u64", "uint64_t", 'u', 8),
        Num::F32 => ("f32", "float", 'f', 4),
        Num::F64 => ("f64", "double", 'f', 8),
        Num::I128 | Num::U128 => return None,
    })
}

fn elem(t: &Ty) -> Option<Elem> {
    match t {
        Ty::Int => Some(Elem {
            moddil: "int",
            c: "int64_t",
            kind: 'i',
            size: 8,
        }),
        Ty::Num(n) => number(*n).map(|(moddil, c, kind, size)| Elem {
            moddil,
            c,
            kind,
            size,
        }),
        _ => None,
    }
}

/// The prelude's struct `name`, with its type arguments, if `t` is one.
fn prelude_adt<'t>(items: &ItemTree, t: &'t Ty, name: &str) -> Option<&'t [Ty]> {
    match t {
        Ty::Adt(a, _, args)
            if items.adts[*a as usize].src == Src::Prelude
                && items.adts[*a as usize].name == name =>
        {
            Some(args)
        }
        _ => None,
    }
}

fn classify(items: &ItemTree, t: &Ty, result: bool) -> Option<PyTy> {
    Some(match t {
        Ty::Unit => PyTy::Unit,
        Ty::Bool => PyTy::Bool,
        Ty::Int => PyTy::Int("int", "int64_t", true),
        Ty::Num(n) => match number(*n)? {
            (m, c, 'f', _) => PyTy::Float(m, c),
            (m, c, k, _) => PyTy::Int(m, c, k == 'i'),
        },
        Ty::Ref(false, inner) if **inner == Ty::Str => PyTy::Str,
        Ty::Ref(m, inner) => match &**inner {
            Ty::Slice(e) => PyTy::Slice(elem(e)?, *m),
            Ty::Adt(a, ..) if items.adts[*a as usize].py_export => PyTy::Struct(*a, Some(*m)),
            _ => return None,
        },
        t if prelude_adt(items, t, "String").is_some() => PyTy::String,
        t if prelude_adt(items, t, "Vec").is_some() => {
            PyTy::Vec(elem(prelude_adt(items, t, "Vec")?.first()?)?)
        }
        t if result && prelude_adt(items, t, "Option").is_some() => {
            let inner = classify(items, prelude_adt(items, t, "Option")?.first()?, true)?;
            if matches!(inner, PyTy::Option(_) | PyTy::Unit) {
                return None;
            }
            PyTy::Option(Box::new(inner))
        }
        Ty::Adt(a, ..) if items.adts[*a as usize].py_export => PyTy::Struct(*a, None),
        _ => return None,
    })
}

/// A callable: a free function, a method, or an associated function.
struct Callable {
    f: u32,
    /// The shim's name.
    shim: String,
    /// How Moddil calls it: `stats::mean`, `stats::Counter::new`, or a method's name.
    path: String,
    /// The receiver, for a method: by `&` (false) or `&mut` (true).
    receiver: Option<bool>,
    params: Vec<(String, PyTy)>,
    ret: PyTy,
    /// The error type's name, if it can fail.
    error: Option<String>,
}

/// The Python bindings of the functions and structs of the user's mod `name` marked
/// `#[export(python)]`; or what cannot cross, each a message.
///
/// # Errors
///
/// Each function or struct whose signature Python cannot carry, and why.
pub fn generate(items: &ItemTree, name: &str, root_mod: u32) -> Result<Bindings, Vec<String>> {
    let mut problems = Vec::new();
    let in_root = |module: u32| {
        items.mod_of(module) == root_mod && items.modules[module as usize].path.is_empty()
    };
    let structs: Vec<u32> = (0..items.adts.len() as u32)
        .filter(|&a| items.adts[a as usize].py_export && in_root(items.adts[a as usize].module))
        .collect();
    let mut errors: Vec<(String, Ty)> = Vec::new();
    let mut callable = |f: u32,
                        receiver: Option<bool>,
                        path: String,
                        shim: String,
                        problems: &mut Vec<String>|
     -> Option<Callable> {
        let sig = &items.fns[f as usize];
        let mut params = Vec::new();
        let skip = usize::from(receiver.is_some());
        for p in sig.params.iter().skip(skip) {
            let Some(t) = classify(items, &p.ty, false) else {
                problems.push(format!(
                    "`{}`'s parameter `{}` is a `{}`, which cannot cross to Python",
                    sig.name,
                    p.name.as_deref().unwrap_or("_"),
                    p.ty
                ));
                return None;
            };
            if matches!(t, PyTy::Unit) {
                problems.push(format!("`{}` takes `()`", sig.name));
                return None;
            }
            params.push((p.name.clone().unwrap_or_else(|| "_".to_string()), t));
        }
        let Some(ret) = classify(items, &sig.ret, true) else {
            problems.push(format!(
                "`{}` gives a `{}`, which cannot cross to Python",
                sig.name, sig.ret
            ));
            return None;
        };
        if !sig.effects.is_empty() || sig.control != 0 {
            problems.push(format!(
                "`{}` needs handlers or suspends, which Python bindings do not carry yet",
                sig.name
            ));
            return None;
        }
        let error = match &sig.fails {
            Ty::Never => None,
            Ty::Adt(a, ..) => {
                let n = items.adts[*a as usize].name.clone();
                if !errors.iter().any(|(e, _)| *e == n) {
                    errors.push((n.clone(), sig.fails.clone()));
                }
                Some(n)
            }
            t => {
                problems.push(format!(
                    "`{}` fails with `{t}`: one error type crosses to Python, not a set",
                    sig.name
                ));
                return None;
            }
        };
        Some(Callable {
            f,
            shim,
            path,
            receiver,
            params,
            ret,
            error,
        })
    };
    let mut fns = Vec::new();
    for (f, sig) in items.fns.iter().enumerate() {
        if sig.py_export
            && matches!(sig.container, Container::Free)
            && in_root(sig.module)
            && let Some(c) = callable(
                f as u32,
                None,
                format!("{name}::{}", sig.name),
                format!("__py_{}", sig.name),
                &mut problems,
            )
        {
            fns.push(c);
        }
    }
    // Each exported struct's inherent `pub fn`s.
    let mut methods: Vec<(u32, Vec<Callable>)> = Vec::new();
    for &a in &structs {
        let sname = items.adts[a as usize].name.clone();
        let mut ms = Vec::new();
        for imp in &items.impls {
            if imp.trait_ref.is_some() || !matches!(&imp.self_ty, Ty::Adt(b, ..) if *b == a) {
                continue;
            }
            for &m in &imp.methods {
                let sig = &items.fns[m as usize];
                if sig.vis != Vis::Pub {
                    continue;
                }
                let receiver = match sig.self_kind {
                    None => None,
                    Some(SelfKind::Ref) => Some(false),
                    Some(SelfKind::RefMut) => Some(true),
                    Some(SelfKind::Value) => {
                        problems.push(format!("`{sname}::{}` takes `self` by value, which Python objects do not give up", sig.name));
                        continue;
                    }
                };
                let path = if receiver.is_some() {
                    sig.name.clone()
                } else {
                    format!("{name}::{sname}::{}", sig.name)
                };
                if let Some(c) = callable(
                    m,
                    receiver,
                    path,
                    format!("__py_{sname}_{}", sig.name),
                    &mut problems,
                ) {
                    ms.push(c);
                }
            }
        }
        methods.push((a, ms));
    }
    if !problems.is_empty() {
        return Err(problems);
    }
    let mut g = Gen {
        items,
        name,
        shims: String::new(),
        glue: String::new(),
        pyi: String::new(),
        structs: structs.clone(),
    };
    g.write(&fns, &methods, &errors);
    Ok(Bindings {
        shims: g.shims,
        glue: g.glue,
        pyi: g.pyi,
    })
}

struct Gen<'a> {
    items: &'a ItemTree,
    name: &'a str,
    shims: String,
    glue: String,
    pyi: String,
    structs: Vec<u32>,
}

/// A safety reason for the shims' `unsafe` blocks: the glue keeps every pointer valid.
const WHY: &str = "the generated C glue passes live Python buffers and handles this mod made";

impl Gen<'_> {
    fn sname(&self, a: u32) -> String {
        self.items.adts[a as usize].name.clone()
    }

    /// The Moddil type of a handle's value.
    fn handle_ty(&self, t: &PyTy) -> String {
        match t {
            PyTy::String => "String".to_string(),
            PyTy::Vec(e) => format!("Vec<{}>", e.moddil),
            PyTy::Struct(a, _) => format!("{}::{}", self.name, self.sname(*a)),
            _ => unreachable!("a handle's type"),
        }
    }

    /// Whether a result of this type crosses as a handle.
    fn is_handle(t: &PyTy) -> bool {
        matches!(t, PyTy::String | PyTy::Vec(_) | PyTy::Struct(..))
    }

    /// The shim's parameters for a value of `t` named `n`.
    fn shim_params(&self, n: &str, t: &PyTy) -> Vec<String> {
        match t {
            PyTy::Bool => vec![format!("{n}: bool")],
            PyTy::Int(m, ..) | PyTy::Float(m, _) => vec![format!("{n}: {m}")],
            PyTy::Str | PyTy::String => vec![format!("{n}_p: *const u8"), format!("{n}_n: int")],
            PyTy::Slice(e, m) => vec![
                format!("{n}_p: *{} {}", if *m { "mut" } else { "const" }, e.moddil),
                format!("{n}_n: int"),
            ],
            PyTy::Struct(a, _) => vec![format!("{n}: *mut {}::{}", self.name, self.sname(*a))],
            PyTy::Unit | PyTy::Vec(_) | PyTy::Option(_) => unreachable!("not a parameter"),
        }
    }

    /// The statement making the Moddil value `n` from the shim's parameters.
    fn shim_convert(&self, n: &str, t: &PyTy) -> Option<String> {
        Some(match t {
            PyTy::Str => format!("let {n} = unsafe(\"{WHY}\") {{ ffi::text({n}_p, {n}_n) }};"),
            PyTy::String => {
                format!("let {n} = unsafe(\"{WHY}\") {{ ffi::text({n}_p, {n}_n) }}.to_string();")
            }
            PyTy::Slice(_, false) => {
                format!("let {n} = unsafe(\"{WHY}\") {{ ffi::slice({n}_p, {n}_n) }};")
            }
            PyTy::Slice(_, true) => {
                format!("let {n} = unsafe(\"{WHY}\") {{ ffi::slice_mut({n}_p, {n}_n) }};")
            }
            PyTy::Struct(_, Some(false)) => format!("let {n} = unsafe(\"{WHY}\") {{ &*{n} }};"),
            PyTy::Struct(_, Some(true)) => format!("let {n} = unsafe(\"{WHY}\") {{ &mut *{n} }};"),
            PyTy::Struct(_, None) => format!("let {n} = unsafe(\"{WHY}\") {{ (*{n}).clone() }};"),
            _ => return None,
        })
    }

    /// The out-parameters for a result of `t`.
    fn shim_outs(&self, t: &PyTy) -> Vec<String> {
        match t {
            PyTy::Unit => Vec::new(),
            PyTy::Bool => vec!["out: *mut bool".to_string()],
            PyTy::Int(m, ..) | PyTy::Float(m, _) => vec![format!("out: *mut {m}")],
            t if Self::is_handle(t) => vec![format!("out: *mut *mut {}", self.handle_ty(t))],
            PyTy::Option(inner) => {
                let mut v = vec!["has: *mut bool".to_string()];
                v.extend(self.shim_outs(inner));
                v
            }
            _ => unreachable!("not a result"),
        }
    }

    /// The statements storing result `r` of `t`.
    fn shim_store(&self, r: &str, t: &PyTy) -> String {
        match t {
            PyTy::Unit => String::new(),
            t if Self::is_handle(t) => {
                format!("unsafe(\"{WHY}\") {{ *out = ffi::into_raw(Box::new({r})); }}")
            }
            PyTy::Option(inner) => format!(
                "match {r} {{\n        Some(v) => {{\n            unsafe(\"{WHY}\") {{ *has = true; }}\n            {}\n        }}\n        None => unsafe(\"{WHY}\") {{ *has = false; }},\n    }}",
                self.shim_store("v", inner)
            ),
            _ => format!("unsafe(\"{WHY}\") {{ *out = {r}; }}"),
        }
    }

    fn shim(&mut self, c: &Callable) {
        let mut params = Vec::new();
        if c.receiver.is_some() {
            let a = match self.items.fns[c.f as usize].container {
                Container::Impl(k) => match &self.items.impls[k as usize].self_ty {
                    Ty::Adt(a, ..) => *a,
                    _ => unreachable!("an exported struct's impl"),
                },
                _ => unreachable!("a method"),
            };
            params.push(format!("this: *mut {}::{}", self.name, self.sname(a)));
        }
        for (i, (_, t)) in c.params.iter().enumerate() {
            params.extend(self.shim_params(&format!("a{i}"), t));
        }
        params.extend(self.shim_outs(&c.ret));
        if let Some(e) = &c.error {
            params.push(format!("err: *mut *mut {}::{e}", self.name));
        }
        let mut body = String::new();
        if let Some(m) = c.receiver {
            let r = if m { "&mut *this" } else { "&*this" };
            let _ = writeln!(body, "    let this = unsafe(\"{WHY}\") {{ {r} }};");
        }
        for (i, (_, t)) in c.params.iter().enumerate() {
            if let Some(s) = self.shim_convert(&format!("a{i}"), t) {
                let _ = writeln!(body, "    {s}");
            }
        }
        let args: Vec<String> = (0..c.params.len()).map(|i| format!("a{i}")).collect();
        let call = match c.receiver {
            Some(_) => format!("this.{}({})", c.path, args.join(", ")),
            None => format!("{}({})", c.path, args.join(", ")),
        };
        match &c.error {
            Some(_) => {
                let _ = writeln!(
                    body,
                    "    let r = try {{ {call}? }} catch {{\n        e => {{\n            unsafe(\"{WHY}\") {{ *err = ffi::into_raw(Box::new(e)); }}\n            return 1;\n        }}\n    }};"
                );
            }
            None => {
                let _ = writeln!(body, "    let r = {call};");
            }
        }
        let store = self.shim_store("r", &c.ret);
        if !store.is_empty() {
            let _ = writeln!(body, "    {store}");
        }
        let _ = writeln!(
            self.shims,
            "#[export(status)]\npub fn {}({}) -> i32 {{\n{body}    0\n}}\n",
            c.shim,
            params.join(", ")
        );
    }

    fn write(
        &mut self,
        fns: &[Callable],
        methods: &[(u32, Vec<Callable>)],
        errors: &[(String, Ty)],
    ) {
        let name = self.name;
        let _ = writeln!(
            self.shims,
            "// Generated by `moddil build --wheel`: the C functions the Python module `{name}` calls.\n\nuse core::ffi;\n"
        );
        // The handles' accessors.
        let mut handles: Vec<PyTy> = vec![PyTy::String];
        let note = |t: &PyTy, hs: &mut Vec<PyTy>| {
            let t = match t {
                PyTy::Option(i) => (**i).clone(),
                t => t.clone(),
            };
            if matches!(t, PyTy::Vec(_)) && !hs.contains(&t) {
                hs.push(t);
            }
        };
        for c in fns.iter().chain(methods.iter().flat_map(|(_, m)| m)) {
            note(&c.ret, &mut handles);
        }
        for h in &handles {
            let (tag, ty) = match h {
                PyTy::String => ("string".to_string(), "String".to_string()),
                PyTy::Vec(e) => (format!("vec_{}", e.moddil), format!("Vec<{}>", e.moddil)),
                _ => unreachable!("a handle"),
            };
            let (view, ptr) = match h {
                PyTy::String => ("let t = v.as_str();", "ffi::text_ptr(t)"),
                _ => ("let t = v.as_slice();", "ffi::ptr_of(t)"),
            };
            let elem = match h {
                PyTy::Vec(e) => e.moddil,
                _ => "u8",
            };
            let _ = writeln!(
                self.shims,
                "#[export(status)]\npub fn __py_{tag}_view(h: *mut {ty}, p: *mut *const {elem}, n: *mut int) -> i32 {{\n    let v = unsafe(\"{WHY}\") {{ &*h }};\n    {view}\n    unsafe(\"{WHY}\") {{\n        *p = {ptr};\n        *n = t.len();\n    }}\n    0\n}}\n\n#[export(status)]\npub fn __py_{tag}_free(h: *mut {ty}) -> i32 {{\n    let b = unsafe(\"{WHY}\") {{ ffi::from_raw(h) }};\n    0\n}}\n"
            );
        }
        for (e, _) in errors {
            let _ = writeln!(
                self.shims,
                "#[export(status)]\npub fn __py_err_{e}(h: *mut {name}::{e}, out: *mut *mut String) -> i32 {{\n    let b = unsafe(\"{WHY}\") {{ ffi::from_raw(h) }};\n    let e = b.into_inner();\n    let t: String = fmt\"{{e:?}}\";\n    unsafe(\"{WHY}\") {{ *out = ffi::into_raw(Box::new(t)); }}\n    0\n}}\n"
            );
        }
        for &a in &self.structs.clone() {
            let s = self.sname(a);
            let _ = writeln!(
                self.shims,
                "#[export(status)]\npub fn __py_free_{s}(h: *mut {name}::{s}) -> i32 {{\n    let b = unsafe(\"{WHY}\") {{ ffi::from_raw(h) }};\n    0\n}}\n"
            );
            for f in self.fields(a) {
                let (fname, t) = f;
                let outs = self.shim_outs(&t).join(", ");
                let store = self.shim_store(&format!("v.{fname}"), &t);
                let store = if Self::is_handle(&t) {
                    store.replace(
                        &format!("Box::new(v.{fname})"),
                        &format!("Box::new(v.{fname}.clone())"),
                    )
                } else {
                    store
                };
                let _ = writeln!(
                    self.shims,
                    "#[export(status)]\npub fn __py_{s}_get_{fname}(h: *mut {name}::{s}, {outs}) -> i32 {{\n    let v = unsafe(\"{WHY}\") {{ &*h }};\n    {store}\n    0\n}}\n"
                );
                if let PyTy::Int(m, ..) | PyTy::Float(m, _) = &t {
                    let _ = writeln!(
                        self.shims,
                        "#[export(status)]\npub fn __py_{s}_set_{fname}(h: *mut {name}::{s}, x: {m}) -> i32 {{\n    let v = unsafe(\"{WHY}\") {{ &mut *h }};\n    v.{fname} = x;\n    0\n}}\n"
                    );
                } else if t == PyTy::Bool {
                    let _ = writeln!(
                        self.shims,
                        "#[export(status)]\npub fn __py_{s}_set_{fname}(h: *mut {name}::{s}, x: bool) -> i32 {{\n    let v = unsafe(\"{WHY}\") {{ &mut *h }};\n    v.{fname} = x;\n    0\n}}\n"
                    );
                }
            }
        }
        for c in fns.iter().chain(methods.iter().flat_map(|(_, m)| m)) {
            self.shim(c);
        }
        glue::write(self, fns, methods, errors, &handles);
        pyi::write(self, fns, methods);
    }

    /// Struct `a`'s `pub` fields Python sees, as properties: name and type.
    fn fields(&self, a: u32) -> Vec<(String, PyTy)> {
        let adt = &self.items.adts[a as usize];
        adt.variants[0]
            .fields
            .iter()
            .filter(|f| f.vis == Vis::Pub)
            .filter_map(|f| match classify(self.items, &f.ty, true) {
                Some(
                    t
                    @ (PyTy::Bool | PyTy::Int(..) | PyTy::Float(..) | PyTy::String | PyTy::Vec(_)),
                ) => Some((f.name.clone(), t)),
                _ => None,
            })
            .collect()
    }
}

mod glue;
mod pyi;

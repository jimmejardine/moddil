//! The C glue: a CPython extension module over the shims, against the stable ABI (3.11).

use std::fmt::Write as _;

use crate::{Callable, Elem, Gen, PyTy, Recv, c_prefix};

const PRELUDE: &str = r#"#define Py_LIMITED_API 0x030B0000
#define PY_SSIZE_T_CLEAN
#include <Python.h>
#include <stdbool.h>
#include <stdint.h>

/* The module's exceptions, and the lock that keeps one call in Moddil at a time (its runtime runs
 * one thread's code at once); the GIL is released while it holds. */
static PyObject *Error;
static PyObject *Trap;
static PyThread_type_lock lock;

#define CALL(st, expr)                                                                      \
    do {                                                                                    \
        Py_BEGIN_ALLOW_THREADS PyThread_acquire_lock(lock, WAIT_LOCK);                       \
        st = (expr);                                                                        \
        PyThread_release_lock(lock);                                                        \
        Py_END_ALLOW_THREADS                                                                \
    } while (0)
#define LOCKED(st, expr)                                                                    \
    do {                                                                                    \
        PyThread_acquire_lock(lock, WAIT_LOCK);                                             \
        st = (expr);                                                                        \
        PyThread_release_lock(lock);                                                        \
    } while (0)

/* An instance of an exported struct: its Moddil value, behind a handle. */
typedef struct {
    PyObject_HEAD void *h;
} Obj;

static int to_i64(PyObject *o, int64_t *v, int64_t lo, int64_t hi, const char *what) {
    long long x = PyLong_AsLongLong(o);
    if (x == -1 && PyErr_Occurred()) return -1;
    if (x < lo || x > hi) {
        PyErr_Format(PyExc_OverflowError, "%s is out of range", what);
        return -1;
    }
    *v = (int64_t)x;
    return 0;
}

static int to_u64(PyObject *o, uint64_t *v, uint64_t hi, const char *what) {
    unsigned long long x = PyLong_AsUnsignedLongLong(o);
    if (x == (unsigned long long)-1 && PyErr_Occurred()) return -1;
    if (x > hi) {
        PyErr_Format(PyExc_OverflowError, "%s is out of range", what);
        return -1;
    }
    *v = (uint64_t)x;
    return 0;
}

static int to_f64(PyObject *o, double *v) {
    double x = PyFloat_AsDouble(o);
    if (x == -1.0 && PyErr_Occurred()) return -1;
    *v = x;
    return 0;
}

static int to_bool(PyObject *o, bool *v) {
    int x = PyObject_IsTrue(o);
    if (x < 0) return -1;
    *v = x != 0;
    return 0;
}

static int to_str(PyObject *o, const char **p, Py_ssize_t *n, const char *what) {
    if (!PyUnicode_Check(o)) {
        PyErr_Format(PyExc_TypeError, "%s: a str was expected", what);
        return -1;
    }
    *p = PyUnicode_AsUTF8AndSize(o, n);
    return *p ? 0 : -1;
}

/* A C-contiguous buffer of numbers of `kind` ('i', 'u' or 'f') and `size` bytes each, without a
 * copy: `bytes`, `bytearray`, `array.array`, `memoryview`, numpy arrays. */
static int get_buf(PyObject *o, Py_buffer *b, char kind, Py_ssize_t size, int writable,
                   const char *what) {
    int flags = PyBUF_FORMAT | PyBUF_C_CONTIGUOUS | (writable ? PyBUF_WRITABLE : 0);
    if (PyObject_GetBuffer(o, b, flags) < 0) return -1;
    const char *f = b->format ? b->format : "B";
    while (*f == '@' || *f == '=' || *f == '<') f++;
    char k = '?';
    switch (*f) {
    case 'b': case 'h': case 'i': case 'l': case 'q': case 'n': k = 'i'; break;
    case 'B': case 'H': case 'I': case 'L': case 'Q': case 'N': case 'c': k = 'u'; break;
    case 'f': case 'd': k = 'f'; break;
    }
    if (k != kind || b->itemsize != size || f[1] != 0) {
        PyErr_Format(PyExc_TypeError, "%s: a buffer of %d-byte %s was expected, not one of format '%s'",
                     what, (int)size, kind == 'f' ? "floats" : kind == 'i' ? "signed integers" : "unsigned integers",
                     b->format ? b->format : "B");
        PyBuffer_Release(b);
        return -1;
    }
    return 0;
}
"#;

/// `name.Array`, made only where a `Vec` of numbers other than bytes crosses.
const ARRAY: &str = r#"
/* A `Vec` of numbers Moddil gave: a read-only sequence and buffer over its elements, without a
 * copy, which frees the `Vec` when it goes. */
typedef struct {
    PyObject_HEAD void *h;
    int32_t (*free)(void *);
    PyObject *(*item)(const void *, Py_ssize_t);
    const void *p;
    Py_ssize_t n, size;
    const char *format;
} Arr;
static PyTypeObject *type_Array;

static PyObject *array_of(void *h, int32_t (*free)(void *), PyObject *(*item)(const void *, Py_ssize_t),
                          const void *p, int64_t n, Py_ssize_t size, const char *format) {
    allocfunc alloc = (allocfunc)PyType_GetSlot(type_Array, Py_tp_alloc);
    Arr *a = (Arr *)alloc(type_Array, 0);
    int32_t st;
    if (a == NULL) { LOCKED(st, free(h)); (void)st; return NULL; }
    a->h = h;
    a->free = free;
    a->item = item;
    a->p = p;
    a->n = (Py_ssize_t)n;
    a->size = size;
    a->format = format;
    return (PyObject *)a;
}

static void array_dealloc(PyObject *self) {
    Arr *a = (Arr *)self;
    int32_t st;
    if (a->h) { LOCKED(st, a->free(a->h)); (void)st; }
    PyTypeObject *tp = Py_TYPE(self);
    freefunc f = (freefunc)PyType_GetSlot(tp, Py_tp_free);
    f(self);
    Py_DECREF(tp);
}

static Py_ssize_t array_len(PyObject *self) { return ((Arr *)self)->n; }

static PyObject *array_item(PyObject *self, Py_ssize_t i) {
    Arr *a = (Arr *)self;
    if (a->h == NULL || i < 0 || i >= a->n) {
        PyErr_SetString(PyExc_IndexError, "Array index out of range");
        return NULL;
    }
    return a->item(a->p, i);
}

static int array_getbuffer(PyObject *self, Py_buffer *v, int flags) {
    Arr *a = (Arr *)self;
    if (flags & PyBUF_WRITABLE) {
        PyErr_SetString(PyExc_BufferError, "an Array is read-only");
        return -1;
    }
    v->buf = a->p ? (void *)a->p : (void *)&a->n;
    v->obj = self;
    Py_INCREF(self);
    v->len = a->n * a->size;
    v->itemsize = a->size;
    v->readonly = 1;
    v->ndim = 1;
    v->format = (flags & PyBUF_FORMAT) ? (char *)(a->format ? a->format : "B") : NULL;
    v->shape = (flags & PyBUF_ND) ? &a->n : NULL;
    v->strides = (flags & PyBUF_STRIDES) == PyBUF_STRIDES ? &a->size : NULL;
    v->suboffsets = NULL;
    v->internal = NULL;
    return 0;
}

static PyObject *array_tolist(PyObject *self, PyObject *unused) {
    (void)unused;
    return PySequence_List(self);
}

static PyObject *array_repr(PyObject *self) {
    PyObject *l = PySequence_List(self);
    if (l == NULL) return NULL;
    PyObject *r = PyUnicode_FromFormat("Array(%R)", l);
    Py_DECREF(l);
    return r;
}

static PyMethodDef methods_Array[] = {
    {"tolist", (PyCFunction)array_tolist, METH_NOARGS, NULL},
    {NULL, NULL, 0, NULL},
};

static PyType_Slot slots_Array[] = {
    {Py_tp_dealloc, (void *)array_dealloc},
    {Py_tp_repr, (void *)array_repr},
    {Py_tp_methods, methods_Array},
    {Py_sq_length, (void *)array_len},
    {Py_sq_item, (void *)array_item},
    {Py_bf_getbuffer, (void *)array_getbuffer},
    {0, NULL},
};
"#;

/// The message for an instance whose value is gone, or was never made.
fn no_value(s: &str) -> String {
    format!("the {s} has no value: it was never made, or a method taking it by value took it")
}

/// The C type of a shim's parameter or out-parameter.
fn c_param(t: &PyTy) -> Vec<String> {
    match t {
        PyTy::Bool => vec!["bool".to_string()],
        PyTy::Int(_, c, _) | PyTy::Float(_, c) => vec![(*c).to_string()],
        PyTy::Str | PyTy::String => vec!["const uint8_t *".to_string(), "int64_t".to_string()],
        PyTy::Slice(e, m) => vec![
            format!("{}{} *", if *m { "" } else { "const " }, e.c),
            "int64_t".to_string(),
        ],
        PyTy::Struct(..) => vec!["void *".to_string()],
        _ => unreachable!("not a parameter"),
    }
}

fn c_outs(t: &PyTy) -> Vec<String> {
    match t {
        PyTy::Unit => Vec::new(),
        PyTy::Bool => vec!["bool *".to_string()],
        PyTy::Int(_, c, _) | PyTy::Float(_, c) => vec![format!("{c} *")],
        PyTy::String | PyTy::Vec(_) | PyTy::Struct(..) => vec!["void **".to_string()],
        PyTy::Option(inner) => {
            let mut v = vec!["bool *".to_string()];
            v.extend(c_outs(inner));
            v
        }
        _ => unreachable!("not a result"),
    }
}

/// An integer's range, as C bounds, and whether it is unsigned.
fn range(m: &str) -> (&'static str, &'static str, bool) {
    match m {
        "i8" => ("INT8_MIN", "INT8_MAX", false),
        "i16" => ("INT16_MIN", "INT16_MAX", false),
        "i32" => ("INT32_MIN", "INT32_MAX", false),
        "u8" => ("0", "UINT8_MAX", true),
        "u16" => ("0", "UINT16_MAX", true),
        "u32" => ("0", "UINT32_MAX", true),
        "u64" => ("0", "UINT64_MAX", true),
        "usize" => ("0", "SIZE_MAX", true),
        "isize" => ("PTRDIFF_MIN", "PTRDIFF_MAX", false),
        _ => ("INT64_MIN", "INT64_MAX", false),
    }
}

fn vec_tag(e: &Elem) -> String {
    format!("vec_{}", e.moddil)
}

/// An element's format in Python's `struct` notation, as a buffer gives it.
fn format(e: &Elem) -> &'static str {
    match (e.kind, e.size) {
        ('f', 4) => "f",
        ('f', _) => "d",
        ('i', 1) => "b",
        ('i', 2) => "h",
        ('i', 4) => "i",
        ('i', _) => "q",
        (_, 1) => "B",
        (_, 2) => "H",
        (_, 4) => "I",
        _ => "Q",
    }
}

/// The C statements converting Python object `o` into the C pieces named `n`, or failing to
/// `fail` (a statement ending the function with an error).
fn convert_in(
    g: &Gen<'_>,
    o: &str,
    n: &str,
    t: &PyTy,
    what: &str,
    fail: &str,
) -> (String, Vec<String>) {
    let mut s = String::new();
    let args = match t {
        PyTy::Bool => {
            let _ = writeln!(s, "    bool {n};\n    if (to_bool({o}, &{n}) < 0) {fail}");
            vec![n.to_string()]
        }
        PyTy::Int(m, c, _) => {
            let (lo, hi, unsigned) = range(m);
            if unsigned {
                let _ = writeln!(
                    s,
                    "    uint64_t {n}_w;\n    if (to_u64({o}, &{n}_w, {hi}, \"{what}\") < 0) {fail}\n    {c} {n} = ({c}){n}_w;"
                );
            } else {
                let _ = writeln!(
                    s,
                    "    int64_t {n}_w;\n    if (to_i64({o}, &{n}_w, {lo}, {hi}, \"{what}\") < 0) {fail}\n    {c} {n} = ({c}){n}_w;"
                );
            }
            vec![n.to_string()]
        }
        PyTy::Float(_, c) => {
            let _ = writeln!(
                s,
                "    double {n}_w;\n    if (to_f64({o}, &{n}_w) < 0) {fail}\n    {c} {n} = ({c}){n}_w;"
            );
            vec![n.to_string()]
        }
        PyTy::Str | PyTy::String => {
            let _ = writeln!(
                s,
                "    const char *{n}_p;\n    Py_ssize_t {n}_n;\n    if (to_str({o}, &{n}_p, &{n}_n, \"{what}\") < 0) {fail}"
            );
            vec![format!("(const uint8_t *){n}_p"), format!("(int64_t){n}_n")]
        }
        PyTy::Slice(e, m) => {
            let _ = writeln!(
                s,
                "    if (get_buf({o}, &{n}_b, '{}', {}, {}, \"{what}\") < 0) {fail}\n    {n}_held = 1;",
                e.kind,
                e.size,
                i32::from(*m)
            );
            vec![
                format!("({}{} *){n}_b.buf", if *m { "" } else { "const " }, e.c),
                format!("(int64_t)({n}_b.len / {})", e.size),
            ]
        }
        PyTy::Struct(a, _) => {
            let (sn, cn) = (g.sname(*a), g.cname(*a));
            let _ = writeln!(
                s,
                "    if (!PyObject_IsInstance({o}, (PyObject *)type_{cn})) {{\n        PyErr_SetString(PyExc_TypeError, \"{what}: a {sn} was expected\");\n        {fail}\n    }}\n    void *{n} = ((Obj *){o})->h;\n    if ({n} == NULL) {{\n        PyErr_SetString(PyExc_ValueError, \"{what}: {}\");\n        {fail}\n    }}",
                no_value(&sn)
            );
            vec![n.to_string()]
        }
        _ => unreachable!("not a parameter"),
    };
    (s, args)
}

/// The C expression making the Python result from the out-parameters.
fn convert_out(g: &Gen<'_>, t: &PyTy, out: &str) -> String {
    match t {
        PyTy::Unit => "(Py_INCREF(Py_None), Py_None)".to_string(),
        PyTy::Bool => format!("PyBool_FromLong({out})"),
        PyTy::Int(m, ..) => {
            if range(m).2 {
                format!("PyLong_FromUnsignedLongLong((unsigned long long){out})")
            } else {
                format!("PyLong_FromLongLong((long long){out})")
            }
        }
        PyTy::Float(..) => format!("PyFloat_FromDouble((double){out})"),
        PyTy::String => format!("string_result({out})"),
        PyTy::Vec(e) => format!("{}_result({out})", vec_tag(e)),
        PyTy::Struct(a, _) => format!("wrap_{}({out})", g.cname(*a)),
        PyTy::Option(inner) => format!(
            "({out}_has ? {} : (Py_INCREF(Py_None), Py_None))",
            convert_out(g, inner, out)
        ),
        _ => unreachable!("not a result"),
    }
}

/// The declarations of a callable's out-parameters, and the C arguments passing them.
fn out_decls(t: &PyTy) -> (String, Vec<String>) {
    match t {
        PyTy::Unit => (String::new(), Vec::new()),
        PyTy::Bool => ("    bool out = 0;\n".to_string(), vec!["&out".to_string()]),
        PyTy::Int(_, c, _) | PyTy::Float(_, c) => {
            (format!("    {c} out = 0;\n"), vec!["&out".to_string()])
        }
        PyTy::String | PyTy::Vec(_) | PyTy::Struct(..) => (
            "    void *out = NULL;\n".to_string(),
            vec!["&out".to_string()],
        ),
        PyTy::Option(inner) => {
            let (d, mut a) = out_decls(inner);
            a.insert(0, "&out_has".to_string());
            (format!("    bool out_has = 0;\n{d}"), a)
        }
        _ => unreachable!("not a result"),
    }
}

/// The body of a C function calling callable `c`, its Python arguments in `args` and `kwds`:
/// `self_h` is the receiver's handle (a method), `fail_value` what the function gives on an
/// error.
fn call_body(g: &Gen<'_>, c: &Callable, self_h: Option<&str>, fail_value: &str) -> String {
    let n = c.params.len();
    let mut s = String::new();
    let fail = "goto fail;".to_string();
    for i in 0..n {
        let _ = writeln!(s, "    PyObject *o{i} = NULL;");
    }
    for (i, (_, t)) in c.params.iter().enumerate() {
        if matches!(t, PyTy::Slice(..)) {
            let _ = writeln!(s, "    Py_buffer a{i}_b;\n    int a{i}_held = 0;");
        }
    }
    // By keyword too, but `_` only by position (and so each parameter before it).
    let last_positional = c.params.iter().rposition(|(p, _)| p == "_");
    let mut kw: Vec<String> = c
        .params
        .iter()
        .enumerate()
        .map(|(i, (p, _))| {
            if last_positional.is_some_and(|l| i <= l) {
                "\"\"".to_string()
            } else {
                format!("\"{p}\"")
            }
        })
        .collect();
    kw.push("NULL".to_string());
    let unpack: Vec<String> = (0..n).map(|i| format!("&o{i}")).collect();
    let pyname = &g.items.fns[c.f as usize].name;
    let _ = writeln!(
        s,
        "    static char *kw[] = {{{}}};\n    if (!PyArg_ParseTupleAndKeywords(args, kwds, \"{}:{pyname}\", kw{}{})) return {fail_value};",
        kw.join(", "),
        "O".repeat(n),
        if n > 0 { ", " } else { "" },
        unpack.join(", ")
    );
    let mut cargs: Vec<String> = Vec::new();
    if let Some(h) = self_h {
        cargs.push(h.to_string());
    }
    for (i, (pn, t)) in c.params.iter().enumerate() {
        let (code, args) = convert_in(g, &format!("o{i}"), &format!("a{i}"), t, pn, &fail);
        s.push_str(&code);
        cargs.extend(args);
    }
    let (decls, outs) = out_decls(&c.ret);
    s.push_str(&decls);
    cargs.extend(outs);
    if c.error.is_some() {
        s.push_str("    void *err = NULL;\n");
        cargs.push("&err".to_string());
    }
    let _ = writeln!(
        s,
        "    int32_t st;\n    CALL(st, {}({}));",
        c.shim,
        cargs.join(", ")
    );
    if c.receiver == Some(Recv::Value) {
        // The call took the value: the instance has none now.
        s.push_str("    ((Obj *)self)->h = NULL;\n");
    }
    for (i, (_, t)) in c.params.iter().enumerate() {
        if matches!(t, PyTy::Slice(..)) {
            let _ = writeln!(
                s,
                "    if (a{i}_held) {{ PyBuffer_Release(&a{i}_b); a{i}_held = 0; }}"
            );
        }
    }
    if let Some(e) = c.error {
        let _ = writeln!(
            s,
            "    if (st == 1) {{ raise_{}(err); return {fail_value}; }}",
            g.cname(e)
        );
    }
    let _ = writeln!(
        s,
        "    if (st == 2) {{ PyErr_SetString(Trap, \"the Moddil code trapped\"); return {fail_value}; }}"
    );
    s
}

/// The cleanup after a failed conversion: the buffers taken so far.
fn fail_label(c: &Callable, fail_value: &str) -> String {
    let mut s = String::from("fail:\n");
    for (i, (_, t)) in c.params.iter().enumerate() {
        if matches!(t, PyTy::Slice(..)) {
            let _ = writeln!(s, "    if (a{i}_held) PyBuffer_Release(&a{i}_b);");
        }
    }
    let _ = writeln!(s, "    return {fail_value};");
    s
}

/// A method table's entry for a C function taking arguments by position and keyword.
fn def(pyname: &str, cfn: &str, flags: &str) -> String {
    format!(
        "    {{\"{pyname}\", (PyCFunction)(void (*)(void)){cfn}, METH_VARARGS | METH_KEYWORDS{flags}, NULL}},"
    )
}

/// The C variable holding the Python module of `path` inside the mod.
fn mod_var(path: &[String]) -> String {
    if path.is_empty() {
        "m".to_string()
    } else {
        format!("mod_{}", c_prefix(path))
    }
}

pub(crate) fn write(
    g: &mut Gen<'_>,
    fns: &[Callable],
    methods: &[(u32, Vec<Callable>)],
    errors: &[u32],
    handles: &[PyTy],
) {
    let name = g.name;
    let mut c = String::new();
    let _ = writeln!(
        c,
        "/* Generated by `moddil build --wheel`: the Python module `{name}`. */\n{PRELUDE}"
    );
    let arrays = handles
        .iter()
        .any(|h| matches!(h, PyTy::Vec(e) if e.moddil != "u8"));
    if arrays {
        c.push_str(ARRAY);
    }
    // The shims' prototypes.
    let _ = writeln!(c, "/* The shims, in the Moddil library. */");
    let all: Vec<&Callable> = fns
        .iter()
        .chain(methods.iter().flat_map(|(_, m)| m))
        .collect();
    for f in &all {
        let mut ps = Vec::new();
        if f.receiver.is_some() {
            ps.push("void *".to_string());
        }
        for (_, t) in &f.params {
            ps.extend(c_param(t));
        }
        ps.extend(c_outs(&f.ret));
        if f.error.is_some() {
            ps.push("void **".to_string());
        }
        let _ = writeln!(
            c,
            "extern int32_t {}({});",
            f.shim,
            if ps.is_empty() {
                "void".to_string()
            } else {
                ps.join(", ")
            }
        );
    }
    for h in handles {
        let (tag, elem) = match h {
            PyTy::String => ("string".to_string(), "uint8_t"),
            PyTy::Vec(e) => (vec_tag(e), e.c),
            _ => unreachable!("a handle"),
        };
        let _ = writeln!(
            c,
            "extern int32_t __py_{tag}_view(void *, const {elem} **, int64_t *);\nextern int32_t __py_{tag}_free(void *);"
        );
    }
    let field_protos = |c: &mut String, prefix: &str, a: u32, setters: bool| {
        for (f, t) in g.fields(a) {
            let outs = c_outs(&t).join(", ");
            let _ = writeln!(c, "extern int32_t {prefix}_get_{f}(void *, {outs});");
            if !setters {
                continue;
            }
            if let PyTy::Int(_, ct, _) | PyTy::Float(_, ct) = &t {
                let _ = writeln!(c, "extern int32_t {prefix}_set_{f}(void *, {ct});");
            } else if t == PyTy::Bool {
                let _ = writeln!(c, "extern int32_t {prefix}_set_{f}(void *, bool);");
            }
        }
    };
    for &e in errors {
        let ce = g.cname(e);
        let _ = writeln!(c, "extern int32_t __py_err_{ce}(void *, void **);");
        field_protos(&mut c, &format!("__py_errf_{ce}"), e, false);
        if g.items.adts[e as usize].is_enum {
            let _ = writeln!(c, "extern int32_t __py_errv_{ce}(void *, int64_t *);");
            for k in 0..g.items.adts[e as usize].variants.len() {
                for (f, t) in g.variant_fields(e, k) {
                    let outs = c_outs(&t).join(", ");
                    let _ = writeln!(c, "extern int32_t __py_errf_{ce}_{k}_{f}(void *, {outs});");
                }
            }
        }
    }
    let structs = g.structs.clone();
    for &a in &structs {
        let s = g.cname(a);
        let _ = writeln!(c, "extern int32_t __py_free_{s}(void *);");
        field_protos(&mut c, &format!("__py_{s}"), a, true);
        let _ = writeln!(c, "static PyTypeObject *type_{s};");
    }
    // Results through handles.
    for h in handles {
        match h {
            PyTy::String => {
                let _ = writeln!(
                    c,
                    "\nstatic PyObject *string_result(void *h) {{\n    const uint8_t *p;\n    int64_t n;\n    int32_t st;\n    LOCKED(st, __py_string_view(h, &p, &n));\n    PyObject *r = PyUnicode_FromStringAndSize((const char *)p, (Py_ssize_t)n);\n    LOCKED(st, __py_string_free(h));\n    (void)st;\n    return r;\n}}"
                );
            }
            PyTy::Vec(e) if e.moddil == "u8" => {
                let tag = vec_tag(e);
                let _ = writeln!(
                    c,
                    "\nstatic PyObject *{tag}_result(void *h) {{\n    const uint8_t *p;\n    int64_t n;\n    int32_t st;\n    LOCKED(st, __py_{tag}_view(h, &p, &n));\n    PyObject *r = PyBytes_FromStringAndSize((const char *)p, (Py_ssize_t)n);\n    LOCKED(st, __py_{tag}_free(h));\n    (void)st;\n    return r;\n}}"
                );
            }
            PyTy::Vec(e) => {
                let tag = vec_tag(e);
                let item = match e.kind {
                    'f' => "PyFloat_FromDouble((double)",
                    'u' => "PyLong_FromUnsignedLongLong((unsigned long long)",
                    _ => "PyLong_FromLongLong((long long)",
                };
                let _ = writeln!(
                    c,
                    "\nstatic PyObject *item_{tag}(const void *p, Py_ssize_t i) {{\n    return {item}((const {ct} *)p)[i]);\n}}\n\nstatic PyObject *{tag}_result(void *h) {{\n    const {ct} *p;\n    int64_t n;\n    int32_t st;\n    LOCKED(st, __py_{tag}_view(h, &p, &n));\n    (void)st;\n    return array_of(h, __py_{tag}_free, item_{tag}, p, n, {}, \"{}\");\n}}",
                    e.size,
                    format(e),
                    ct = e.c
                );
            }
            _ => {}
        }
    }
    // The exceptions: an instance of the error's class, with its fields as attributes; an
    // enum's, its variant's name as `variant` and that variant's fields (a tuple variant's as
    // `_0`, `_1`, …).
    for &e in errors {
        let ce = g.cname(e);
        let mut fields = String::new();
        let mut sets = String::new();
        if g.items.adts[e as usize].is_enum {
            let variants = g.items.adts[e as usize].variants.clone();
            let most = 1
                + (0..variants.len())
                    .map(|k| g.variant_fields(e, k).len())
                    .max()
                    .unwrap_or(0);
            let _ = writeln!(
                fields,
                "    const char *an[{most}];\n    PyObject *av[{most}];\n    int na = 0;\n    int64_t k = -1;\n    LOCKED(st, __py_errv_{ce}(err, &k));\n    switch (k) {{"
            );
            for (k, v) in variants.iter().enumerate() {
                let _ = writeln!(
                    fields,
                    "    case {k}:\n        an[na] = \"variant\";\n        av[na++] = PyUnicode_FromString(\"{}\");",
                    v.name
                );
                for (f, t) in g.variant_fields(e, k) {
                    let (decls, outs) = out_decls(&t);
                    let res = convert_out(g, &t, "out");
                    let attr = if f.chars().all(|c| c.is_ascii_digit()) {
                        format!("_{f}")
                    } else {
                        f.clone()
                    };
                    let _ = writeln!(
                        fields,
                        "        {{\n    {decls}        LOCKED(st, __py_errf_{ce}_{k}_{f}(err, {}));\n            an[na] = \"{attr}\";\n            av[na] = {res};\n            if (av[na] == NULL) PyErr_Clear();\n            na++;\n        }}",
                        outs.join(", ")
                    );
                }
                let _ = writeln!(fields, "        break;");
            }
            let _ = writeln!(fields, "    }}");
            let _ = writeln!(
                sets,
                "        for (int i = 0; i < na; i++) {{\n            if (av[i] == NULL) continue;\n            if (PyObject_SetAttrString(e, an[i], av[i]) < 0) PyErr_Clear();\n            Py_DECREF(av[i]);\n        }}"
            );
        }
        for (f, t) in g.fields(e) {
            let (decls, outs) = out_decls(&t);
            let res = convert_out(g, &t, "out");
            let _ = writeln!(
                fields,
                "    PyObject *f_{f};\n    {{\n    {decls}    LOCKED(st, __py_errf_{ce}_get_{f}(err, {}));\n        f_{f} = {res};\n        if (f_{f} == NULL) PyErr_Clear();\n    }}",
                outs.join(", ")
            );
            let _ = writeln!(
                sets,
                "        if (f_{f} != NULL && PyObject_SetAttrString(e, \"{f}\", f_{f}) < 0) PyErr_Clear();"
            );
            let _ = writeln!(sets, "        Py_XDECREF(f_{f});");
        }
        let _ = writeln!(
            c,
            "\nstatic PyObject *exc_{ce};\nstatic void raise_{ce}(void *err) {{\n    int32_t st;\n{fields}    void *text = NULL;\n    LOCKED(st, __py_err_{ce}(err, &text));\n    (void)st;\n    PyObject *t = string_result(text);\n    PyObject *e = t ? PyObject_CallFunctionObjArgs(exc_{ce}, t, NULL) : NULL;\n    Py_XDECREF(t);\n    if (e != NULL) {{\n{sets}        PyErr_SetObject(exc_{ce}, e);\n        Py_DECREF(e);\n    }}\n}}"
        );
    }
    // The classes' instances, made from handles.
    for &a in &structs {
        let s = g.cname(a);
        let _ = writeln!(
            c,
            "\nstatic PyObject *wrap_{s}(void *h) {{\n    allocfunc alloc = (allocfunc)PyType_GetSlot(type_{s}, Py_tp_alloc);\n    PyObject *o = alloc(type_{s}, 0);\n    int32_t st;\n    if (o == NULL) {{ LOCKED(st, __py_free_{s}(h)); (void)st; return NULL; }}\n    ((Obj *)o)->h = h;\n    return o;\n}}\n\nstatic void dealloc_{s}(PyObject *self) {{\n    int32_t st;\n    if (((Obj *)self)->h) {{ LOCKED(st, __py_free_{s}(((Obj *)self)->h)); (void)st; }}\n    PyTypeObject *tp = Py_TYPE(self);\n    freefunc f = (freefunc)PyType_GetSlot(tp, Py_tp_free);\n    f(self);\n    Py_DECREF(tp);\n}}"
        );
    }
    // The functions, by module.
    let fn_c = |f: &Callable| format!("py_{}", f.shim.trim_start_matches("__py_"));
    for f in fns {
        let body = call_body(g, f, None, "NULL");
        let res = convert_out(g, &f.ret, "out");
        let _ = writeln!(
            c,
            "\nstatic PyObject *{}(PyObject *self, PyObject *args, PyObject *kwds) {{\n    (void)self;\n{body}    return {res};\n{}}}",
            fn_c(f),
            fail_label(f, "NULL")
        );
    }
    // The classes.
    for (a, ms) in methods {
        let (s, cs) = (g.sname(*a), g.cname(*a));
        let mut defs = Vec::new();
        let mut init = None;
        for m in ms {
            let mname = g.items.fns[m.f as usize].name.clone();
            let own = matches!(m.ret, PyTy::Struct(b, None) if b == *a);
            if m.receiver.is_none() && mname == "new" && own {
                let body = call_body(g, m, None, "-1");
                let _ = writeln!(
                    c,
                    "\nstatic int init_{cs}(PyObject *self, PyObject *args, PyObject *kwds) {{\n{body}    int32_t st2;\n    if (((Obj *)self)->h) {{ LOCKED(st2, __py_free_{cs}(((Obj *)self)->h)); (void)st2; }}\n    ((Obj *)self)->h = out;\n    return 0;\n{}}}",
                    fail_label(m, "-1")
                );
                init = Some(());
                continue;
            }
            let res = convert_out(g, &m.ret, "out");
            match m.receiver {
                Some(_) => {
                    let body = call_body(g, m, Some("h"), "NULL");
                    let _ = writeln!(
                        c,
                        "\nstatic PyObject *{cs}_{mname}(PyObject *self, PyObject *args, PyObject *kwds) {{\n    void *h = ((Obj *)self)->h;\n    if (h == NULL) {{ PyErr_SetString(PyExc_ValueError, \"{}\"); return NULL; }}\n{body}    return {res};\n{}}}",
                        no_value(&s),
                        fail_label(m, "NULL")
                    );
                    defs.push(def(&mname, &format!("{cs}_{mname}"), ""));
                }
                None => {
                    let body = call_body(g, m, None, "NULL");
                    let _ = writeln!(
                        c,
                        "\nstatic PyObject *{cs}_{mname}(PyObject *self, PyObject *args, PyObject *kwds) {{\n    (void)self;\n{body}    return {res};\n{}}}",
                        fail_label(m, "NULL")
                    );
                    defs.push(def(&mname, &format!("{cs}_{mname}"), " | METH_STATIC"));
                }
            }
        }
        if init.is_none() {
            let _ = writeln!(
                c,
                "\nstatic int init_{cs}(PyObject *self, PyObject *args, PyObject *kwds) {{\n    (void)self; (void)args; (void)kwds;\n    PyErr_SetString(PyExc_TypeError, \"a {s} is made by the module's functions, not directly\");\n    return -1;\n}}"
            );
        }
        // Properties.
        let mut gets = Vec::new();
        for (f, t) in g.fields(*a) {
            let (decls, outs) = out_decls(&t);
            let res = convert_out(g, &t, "out");
            let _ = writeln!(
                c,
                "\nstatic PyObject *{cs}_get_{f}(PyObject *self, void *closure) {{\n    (void)closure;\n    void *h = ((Obj *)self)->h;\n    if (h == NULL) {{ PyErr_SetString(PyExc_ValueError, \"{}\"); return NULL; }}\n{decls}    int32_t st;\n    LOCKED(st, __py_{cs}_get_{f}(h, {}));\n    (void)st;\n    return {res};\n}}",
                no_value(&s),
                outs.join(", ")
            );
            let setter = if matches!(t, PyTy::Int(..) | PyTy::Float(..) | PyTy::Bool) {
                let (code, args) = convert_in(g, "value", "x", &t, &f, "return -1;");
                let _ = writeln!(
                    c,
                    "\nstatic int {cs}_set_{f}(PyObject *self, PyObject *value, void *closure) {{\n    (void)closure;\n    void *h = ((Obj *)self)->h;\n    if (value == NULL || h == NULL) {{ PyErr_SetString(PyExc_TypeError, \"{f} cannot be deleted\"); return -1; }}\n{code}    int32_t st;\n    LOCKED(st, __py_{cs}_set_{f}(h, {}));\n    (void)st;\n    return 0;\n}}",
                    args.join(", ")
                );
                format!("(setter){cs}_set_{f}")
            } else {
                "NULL".to_string()
            };
            gets.push(format!(
                "    {{\"{f}\", (getter){cs}_get_{f}, {setter}, NULL, NULL}},"
            ));
        }
        let dotted: String = g.module_of(*a).iter().map(|p| format!("{p}.")).collect();
        let _ = writeln!(
            c,
            "\nstatic PyMethodDef methods_{cs}[] = {{\n{}\n    {{NULL, NULL, 0, NULL}},\n}};\n\nstatic PyGetSetDef getset_{cs}[] = {{\n{}\n    {{NULL, NULL, NULL, NULL, NULL}},\n}};\n\nstatic PyType_Slot slots_{cs}[] = {{\n    {{Py_tp_dealloc, (void *)dealloc_{cs}}},\n    {{Py_tp_init, (void *)init_{cs}}},\n    {{Py_tp_new, (void *)PyType_GenericNew}},\n    {{Py_tp_methods, methods_{cs}}},\n    {{Py_tp_getset, getset_{cs}}},\n    {{0, NULL}},\n}};\n\nstatic PyType_Spec spec_{cs} = {{\"{name}.{dotted}{s}\", sizeof(Obj), 0, Py_TPFLAGS_DEFAULT, slots_{cs}}};",
            defs.join("\n"),
            gets.join("\n")
        );
    }
    if arrays {
        let _ = writeln!(
            c,
            "\nstatic PyType_Spec spec_Array = {{\"{name}.Array\", sizeof(Arr), 0, Py_TPFLAGS_DEFAULT, slots_Array}};"
        );
    }
    // The modules: the root, and one for each module inside the mod that exports something.
    let fn_path = |f: &Callable| {
        g.items.modules[g.items.fns[f.f as usize].module as usize]
            .path
            .clone()
    };
    let mut paths: std::collections::BTreeSet<Vec<String>> = std::collections::BTreeSet::new();
    for p in fns
        .iter()
        .map(fn_path)
        .chain(structs.iter().map(|&a| g.module_of(a).to_vec()))
        .chain(errors.iter().map(|&e| g.module_of(e).to_vec()))
    {
        for k in 0..=p.len() {
            paths.insert(p[..k].to_vec());
        }
    }
    paths.insert(Vec::new());
    for p in &paths {
        let defs: Vec<String> = fns
            .iter()
            .filter(|f| fn_path(f) == *p)
            .map(|f| def(&g.items.fns[f.f as usize].name, &fn_c(f), ""))
            .collect();
        let table = if p.is_empty() {
            "methods".to_string()
        } else {
            format!("methods_{}", c_prefix(p))
        };
        let _ = writeln!(
            c,
            "\nstatic PyMethodDef {table}[] = {{\n{}\n    {{NULL, NULL, 0, NULL}},\n}};",
            defs.join("\n")
        );
    }
    let _ = writeln!(
        c,
        "\nstatic struct PyModuleDef module = {{PyModuleDef_HEAD_INIT, \"{name}\", NULL, -1, methods, NULL, NULL, NULL, NULL}};\n\nstatic int add(PyObject *m, const char *n, PyObject *o) {{\n    if (o == NULL) return -1;\n    Py_INCREF(o);\n    if (PyModule_AddObject(m, n, o) < 0) {{ Py_DECREF(o); return -1; }}\n    return 0;\n}}\n\nPyMODINIT_FUNC PyInit_{name}(void) {{\n    PyObject *m = PyModule_Create(&module);\n    if (m == NULL) return NULL;\n    lock = PyThread_allocate_lock();\n    Error = PyErr_NewException(\"{name}.Error\", NULL, NULL);\n    Trap = PyErr_NewException(\"{name}.Trap\", NULL, NULL);\n    if (lock == NULL || add(m, \"Error\", Error) < 0 || add(m, \"Trap\", Trap) < 0) return NULL;"
    );
    for p in paths.iter().filter(|p| !p.is_empty()) {
        let (var, parent) = (mod_var(p), mod_var(&p[..p.len() - 1]));
        let dotted = format!("{name}.{}", p.join("."));
        let _ = writeln!(
            c,
            "    PyObject *{var} = PyModule_New(\"{dotted}\");\n    if ({var} == NULL || PyModule_AddFunctions({var}, methods_{}) < 0) return NULL;\n    if (add({parent}, \"{}\", {var}) < 0) return NULL;\n    if (PyDict_SetItemString(PyImport_GetModuleDict(), \"{dotted}\", {var}) < 0) return NULL;",
            c_prefix(p),
            p[p.len() - 1]
        );
    }
    if arrays {
        let _ = writeln!(
            c,
            "    type_Array = (PyTypeObject *)PyType_FromSpec(&spec_Array);\n    if (add(m, \"Array\", (PyObject *)type_Array) < 0) return NULL;"
        );
    }
    // Each exception in the module of its error type's module.
    for &e in errors {
        let (s, ce) = (g.sname(e), g.cname(e));
        let dotted: String = g.module_of(e).iter().map(|p| format!("{p}.")).collect();
        let _ = writeln!(
            c,
            "    exc_{ce} = PyErr_NewException(\"{name}.{dotted}{s}\", Error, NULL);\n    if (add({}, \"{s}\", exc_{ce}) < 0) return NULL;",
            mod_var(g.module_of(e))
        );
    }
    for &a in &structs {
        let (s, cs) = (g.sname(a), g.cname(a));
        let _ = writeln!(
            c,
            "    type_{cs} = (PyTypeObject *)PyType_FromSpec(&spec_{cs});\n    if (add({}, \"{s}\", (PyObject *)type_{cs}) < 0) return NULL;",
            mod_var(g.module_of(a))
        );
    }
    let _ = writeln!(c, "    return m;\n}}");
    g.glue = c;
}

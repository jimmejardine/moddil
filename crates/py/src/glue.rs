//! The C glue: a CPython extension module over the shims, against the stable ABI (3.11).

use std::fmt::Write as _;

use moddil_sema::Ty;

use crate::{Callable, Elem, Gen, PyTy};

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

/// The C type of a shim's parameter or out-parameter.
fn c_param(g: &Gen<'_>, t: &PyTy) -> Vec<String> {
    let _ = g;
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
        _ => ("INT64_MIN", "INT64_MAX", false),
    }
}

fn vec_tag(e: &Elem) -> String {
    format!("vec_{}", e.moddil)
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
            let sn = g.sname(*a);
            let _ = writeln!(
                s,
                "    if (!PyObject_IsInstance({o}, (PyObject *)type_{sn})) {{\n        PyErr_SetString(PyExc_TypeError, \"{what}: a {sn} was expected\");\n        {fail}\n    }}\n    void *{n} = ((Obj *){o})->h;\n    if ({n} == NULL) {{\n        PyErr_SetString(PyExc_ValueError, \"{what}: the {sn} was never made\");\n        {fail}\n    }}"
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
        PyTy::Struct(a, _) => format!("wrap_{}({out})", g.sname(*a)),
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

/// The body of a C function calling callable `c`: `self_h` is the receiver's handle (a method),
/// `fail` what ends the function with an error, `done` how it gives `result`.
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
    let unpack: Vec<String> = (0..n).map(|i| format!("&o{i}")).collect();
    let pyname = &g.items.fns[c.f as usize].name;
    let _ = writeln!(
        s,
        "    if (!PyArg_UnpackTuple(args, \"{pyname}\", {n}, {n}{}{})) return {fail_value};",
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
    for (i, (_, t)) in c.params.iter().enumerate() {
        if matches!(t, PyTy::Slice(..)) {
            let _ = writeln!(
                s,
                "    if (a{i}_held) {{ PyBuffer_Release(&a{i}_b); a{i}_held = 0; }}"
            );
        }
    }
    if let Some(e) = &c.error {
        let _ = writeln!(
            s,
            "    if (st == 1) {{ raise_{e}(err); return {fail_value}; }}"
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

pub(crate) fn write(
    g: &mut Gen<'_>,
    fns: &[Callable],
    methods: &[(u32, Vec<Callable>)],
    errors: &[(String, Ty)],
    handles: &[PyTy],
) {
    let name = g.name;
    let mut c = String::new();
    let _ = writeln!(
        c,
        "/* Generated by `moddil build --wheel`: the Python module `{name}`. */\n{PRELUDE}"
    );
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
            ps.extend(c_param(g, t));
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
    for (e, _) in errors {
        let _ = writeln!(c, "extern int32_t __py_err_{e}(void *, void **);");
    }
    let structs = g.structs.clone();
    for &a in &structs {
        let s = g.sname(a);
        let _ = writeln!(c, "extern int32_t __py_free_{s}(void *);");
        for (f, t) in g.fields(a) {
            let outs = c_outs(&t).join(", ");
            let _ = writeln!(c, "extern int32_t __py_{s}_get_{f}(void *, {outs});");
            if let PyTy::Int(_, ct, _) | PyTy::Float(_, ct) = &t {
                let _ = writeln!(c, "extern int32_t __py_{s}_set_{f}(void *, {ct});");
            } else if t == PyTy::Bool {
                let _ = writeln!(c, "extern int32_t __py_{s}_set_{f}(void *, bool);");
            }
        }
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
            PyTy::Vec(e) => {
                let tag = vec_tag(e);
                let body = if e.moddil == "u8" {
                    "    PyObject *r = PyBytes_FromStringAndSize((const char *)p, (Py_ssize_t)n);\n"
                        .to_string()
                } else {
                    let item = match e.kind {
                        'f' => "PyFloat_FromDouble((double)p[i])",
                        'u' => "PyLong_FromUnsignedLongLong((unsigned long long)p[i])",
                        _ => "PyLong_FromLongLong((long long)p[i])",
                    };
                    format!(
                        "    PyObject *r = PyList_New((Py_ssize_t)n);\n    for (int64_t i = 0; r != NULL && i < n; i++) PyList_SetItem(r, (Py_ssize_t)i, {item});\n"
                    )
                };
                let _ = writeln!(
                    c,
                    "\nstatic PyObject *{tag}_result(void *h) {{\n    const {} *p;\n    int64_t n;\n    int32_t st;\n    LOCKED(st, __py_{tag}_view(h, &p, &n));\n{body}    LOCKED(st, __py_{tag}_free(h));\n    (void)st;\n    return r;\n}}",
                    e.c
                );
            }
            _ => {}
        }
    }
    // The exceptions.
    for (e, _) in errors {
        let _ = writeln!(
            c,
            "\nstatic PyObject *exc_{e};\nstatic void raise_{e}(void *err) {{\n    void *text = NULL;\n    int32_t st;\n    LOCKED(st, __py_err_{e}(err, &text));\n    (void)st;\n    PyObject *t = string_result(text);\n    PyErr_SetObject(exc_{e}, t);\n    Py_XDECREF(t);\n}}"
        );
    }
    // The classes' instances, made from handles.
    for &a in &structs {
        let s = g.sname(a);
        let _ = writeln!(
            c,
            "\nstatic PyObject *wrap_{s}(void *h) {{\n    allocfunc alloc = (allocfunc)PyType_GetSlot(type_{s}, Py_tp_alloc);\n    PyObject *o = alloc(type_{s}, 0);\n    int32_t st;\n    if (o == NULL) {{ LOCKED(st, __py_free_{s}(h)); (void)st; return NULL; }}\n    ((Obj *)o)->h = h;\n    return o;\n}}\n\nstatic void dealloc_{s}(PyObject *self) {{\n    int32_t st;\n    if (((Obj *)self)->h) {{ LOCKED(st, __py_free_{s}(((Obj *)self)->h)); (void)st; }}\n    PyTypeObject *tp = Py_TYPE(self);\n    freefunc f = (freefunc)PyType_GetSlot(tp, Py_tp_free);\n    f(self);\n    Py_DECREF(tp);\n}}"
        );
    }
    // The functions.
    for f in fns {
        let fname = &g.items.fns[f.f as usize].name;
        let body = call_body(g, f, None, "NULL");
        let res = convert_out(g, &f.ret, "out");
        let _ = writeln!(
            c,
            "\nstatic PyObject *py_{fname}(PyObject *self, PyObject *args) {{\n    (void)self;\n{body}    return {res};\n{}}}",
            fail_label(f, "NULL")
        );
    }
    // The classes.
    for (a, ms) in methods {
        let s = g.sname(*a);
        let mut defs = Vec::new();
        let mut init = None;
        for m in ms {
            let mname = g.items.fns[m.f as usize].name.clone();
            let own = matches!(m.ret, PyTy::Struct(b, None) if b == *a);
            if m.receiver.is_none() && mname == "new" && own {
                let body = call_body(g, m, None, "-1");
                let _ = writeln!(
                    c,
                    "\nstatic int init_{s}(PyObject *self, PyObject *args, PyObject *kwds) {{\n    if (kwds != NULL && PyDict_Size(kwds) > 0) {{\n        PyErr_SetString(PyExc_TypeError, \"{s}() takes its arguments by position\");\n        return -1;\n    }}\n{body}    int32_t st2;\n    if (((Obj *)self)->h) {{ LOCKED(st2, __py_free_{s}(((Obj *)self)->h)); (void)st2; }}\n    ((Obj *)self)->h = out;\n    return 0;\n{}}}",
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
                        "\nstatic PyObject *{s}_{mname}(PyObject *self, PyObject *args) {{\n    void *h = ((Obj *)self)->h;\n    if (h == NULL) {{ PyErr_SetString(PyExc_ValueError, \"the {s} was never made\"); return NULL; }}\n{body}    return {res};\n{}}}",
                        fail_label(m, "NULL")
                    );
                    defs.push(format!(
                        "    {{\"{mname}\", (PyCFunction){s}_{mname}, METH_VARARGS, NULL}},"
                    ));
                }
                None => {
                    let body = call_body(g, m, None, "NULL");
                    let _ = writeln!(
                        c,
                        "\nstatic PyObject *{s}_{mname}(PyObject *self, PyObject *args) {{\n    (void)self;\n{body}    return {res};\n{}}}",
                        fail_label(m, "NULL")
                    );
                    defs.push(format!(
                        "    {{\"{mname}\", (PyCFunction){s}_{mname}, METH_VARARGS | METH_STATIC, NULL}},"
                    ));
                }
            }
        }
        if init.is_none() {
            let _ = writeln!(
                c,
                "\nstatic int init_{s}(PyObject *self, PyObject *args, PyObject *kwds) {{\n    (void)self; (void)args; (void)kwds;\n    PyErr_SetString(PyExc_TypeError, \"a {s} is made by the module's functions, not directly\");\n    return -1;\n}}"
            );
        }
        // Properties.
        let mut gets = Vec::new();
        for (f, t) in g.fields(*a) {
            let (decls, outs) = out_decls(&t);
            let res = convert_out(g, &t, "out");
            let _ = writeln!(
                c,
                "\nstatic PyObject *{s}_get_{f}(PyObject *self, void *closure) {{\n    (void)closure;\n    void *h = ((Obj *)self)->h;\n    if (h == NULL) {{ PyErr_SetString(PyExc_ValueError, \"the {s} was never made\"); return NULL; }}\n{decls}    int32_t st;\n    LOCKED(st, __py_{s}_get_{f}(h, {}));\n    (void)st;\n    return {res};\n}}",
                outs.join(", ")
            );
            let setter = if matches!(t, PyTy::Int(..) | PyTy::Float(..) | PyTy::Bool) {
                let (code, args) = convert_in(g, "value", "x", &t, &f, "return -1;");
                let _ = writeln!(
                    c,
                    "\nstatic int {s}_set_{f}(PyObject *self, PyObject *value, void *closure) {{\n    (void)closure;\n    void *h = ((Obj *)self)->h;\n    if (value == NULL || h == NULL) {{ PyErr_SetString(PyExc_TypeError, \"{f} cannot be deleted\"); return -1; }}\n{code}    int32_t st;\n    LOCKED(st, __py_{s}_set_{f}(h, {}));\n    (void)st;\n    return 0;\n}}",
                    args.join(", ")
                );
                format!("(setter){s}_set_{f}")
            } else {
                "NULL".to_string()
            };
            gets.push(format!(
                "    {{\"{f}\", (getter){s}_get_{f}, {setter}, NULL, NULL}},"
            ));
        }
        let _ = writeln!(
            c,
            "\nstatic PyMethodDef methods_{s}[] = {{\n{}\n    {{NULL, NULL, 0, NULL}},\n}};\n\nstatic PyGetSetDef getset_{s}[] = {{\n{}\n    {{NULL, NULL, NULL, NULL, NULL}},\n}};\n\nstatic PyType_Slot slots_{s}[] = {{\n    {{Py_tp_dealloc, (void *)dealloc_{s}}},\n    {{Py_tp_init, (void *)init_{s}}},\n    {{Py_tp_new, (void *)PyType_GenericNew}},\n    {{Py_tp_methods, methods_{s}}},\n    {{Py_tp_getset, getset_{s}}},\n    {{0, NULL}},\n}};\n\nstatic PyType_Spec spec_{s} = {{\"{name}.{s}\", sizeof(Obj), 0, Py_TPFLAGS_DEFAULT, slots_{s}}};",
            defs.join("\n"),
            gets.join("\n")
        );
    }
    // The module.
    let mut defs = Vec::new();
    for f in fns {
        let fname = &g.items.fns[f.f as usize].name;
        defs.push(format!(
            "    {{\"{fname}\", (PyCFunction)py_{fname}, METH_VARARGS, NULL}},"
        ));
    }
    let _ = writeln!(
        c,
        "\nstatic PyMethodDef methods[] = {{\n{}\n    {{NULL, NULL, 0, NULL}},\n}};\n\nstatic struct PyModuleDef module = {{PyModuleDef_HEAD_INIT, \"{name}\", NULL, -1, methods, NULL, NULL, NULL, NULL}};\n\nstatic int add(PyObject *m, const char *n, PyObject *o) {{\n    if (o == NULL) return -1;\n    Py_INCREF(o);\n    if (PyModule_AddObject(m, n, o) < 0) {{ Py_DECREF(o); return -1; }}\n    return 0;\n}}\n\nPyMODINIT_FUNC PyInit_{name}(void) {{\n    PyObject *m = PyModule_Create(&module);\n    if (m == NULL) return NULL;\n    lock = PyThread_allocate_lock();\n    Error = PyErr_NewException(\"{name}.Error\", NULL, NULL);\n    Trap = PyErr_NewException(\"{name}.Trap\", NULL, NULL);\n    if (lock == NULL || add(m, \"Error\", Error) < 0 || add(m, \"Trap\", Trap) < 0) return NULL;",
        defs.join("\n")
    );
    for (e, _) in errors {
        let _ = writeln!(
            c,
            "    exc_{e} = PyErr_NewException(\"{name}.{e}\", Error, NULL);\n    if (add(m, \"{e}\", exc_{e}) < 0) return NULL;"
        );
    }
    for &a in &structs {
        let s = g.sname(a);
        let _ = writeln!(
            c,
            "    type_{s} = (PyTypeObject *)PyType_FromSpec(&spec_{s});\n    if (add(m, \"{s}\", (PyObject *)type_{s}) < 0) return NULL;"
        );
    }
    let _ = writeln!(c, "    return m;\n}}");
    g.glue = c;
}

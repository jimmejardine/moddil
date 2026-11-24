# C interop

Calling C from Moddil and Moddil from C, with the C ABI ([`specs/interop.md`](../../specs/interop.md)).
Added in M3 slice M3a; Python in M3b; header import (`#[c_import]`) comes later.

**ffi.extern.1** `extern "C" { fn name(params) -> T; … }` declares C functions: each is a free
function without a body, named by its C symbol, called with the platform's C calling convention.
Only C's ABI is declared (another string is E0222), and a C function has no body (E0222). Only a
mod whose manifest lists the `unsafe` capability declares them (E0219), and a call of one is
allowed only inside an `unsafe` block (E0218): nothing checks what C does.

**ffi.types.1** A C function's, and an `#[export]` function's, parameters and result have **C's
types** (E0222 otherwise): the sized numbers but the 128-bit ones (`i8` as `int8_t`, … `f64` as
`double`), `int` (`int64_t`), `bool` (C's `bool`), and raw pointers (`*const T` as `const T *`,
`*mut T` as `T *`), to anything; a result may also be `()` (`void`). A struct crosses behind a
pointer. Neither kind of function needs handlers, can fail, or suspends or spawns (E0222).

**ffi.repr.1** `#[repr(C)]` on a struct lays it out as C lays out a struct of the same fields:
in the order written, each at the next offset its alignment allows, the size rounded up to the
largest alignment. Other structs' layout is the compiler's to choose.

**ffi.export.1** `#[export]` on a `pub` free function with named parameters makes it callable
from C under its own name, with C's types (**ffi.types.1**). `moddil build --lib` builds the root
mod's exported functions, with the runtime, into a shared library, and writes its header,
`name.h`: the `#[repr(C)]` structs the signatures name, the other structs as incomplete types,
and the prototypes. The runtime starts with the first call. A trap in an exported function ends
the process with exit code 70, as a built program's does: C has no way to be told. In Moddil an
exported function is called as any other.

**ffi.export.2** `#[export(status)]` is `#[export]` for generated bindings: the function gives
an `i32` status, and a trap in it makes it give 2 instead of ending the process.

**ffi.core.1** The `core` mod's `ffi` module ([source](core/ffi.moddil)), named only in a mod
with the `unsafe` capability (E0219), makes borrows of what C gives and hands boxes to C:
`slice(p, n)` and `slice_mut(p, n)` (the `n` values at `p`), `text(p, n)` (`n` bytes of UTF-8),
`ptr_of(s)` and `text_ptr(s)` (where a slice's or a text's values are), and `into_raw(b)` and
`from_raw(p)` (a box's value kept alive behind a raw pointer, and owned again). Nothing checks
that a pointer and a length describe live memory. Native code runs them; the interpreters trap.

**ffi.python.1** `#[export(python)]` on a `pub fn` of a mod's root module makes it a function of
the mod's Python module, which `moddil build --wheel` builds (with CPython's stable ABI, 3.11 and
later) with its stubs (`name.pyi`) and a wheel. Its parameters and result cross as Python values:
numbers as `int` and `float` (out of range is `OverflowError`), `bool`, `&str` and `String` as
`str`, slices of numbers as any C-contiguous buffer of that type without a copy (`bytes`,
`bytearray`, `array.array`, `memoryview`, numpy arrays; `&mut` ones changed in place), `Vec<u8>`
as `bytes` and other `Vec`s of numbers as `list`s, `Option` (a result) as the value or `None`,
and `()` as `None`. A `fail` error is raised as an exception of its type's name, a subclass of
`name.Error`, carrying its `Debug` text; a trap is raised as `name.Trap`. A function that needs
handlers or suspends is not yet carried. Calls release the GIL, and run one at a time.

**ffi.python.2** `#[export(python)]` on a `pub struct` makes it a class of the module: an
instance owns a value of it. Its inherent `pub fn`s are the class's: `new` (giving the struct) its
constructor, the others taking `&self` or `&mut self` its methods, and those without `self` its
static methods; its `pub` fields of the types above are properties, which can be set if they are
numbers or `bool`. A method taking `self` by value is not carried (an error at `moddil build
--wheel`). Structs cross to and from functions by `&`, `&mut`, or by value (cloned).

**ffi.c.1** A mod's manifest may have a `[c]` table, the C code compiled with it by the system's
C compiler: `sources` (files, or `dir/*.c` for every `.c` file in `dir`), `include` (header
directories), `defines` (`NAME` or `NAME=value`) and `libs` (system libraries, by name). Its paths
are the manifest's directory's. A run of the mod's code (native, or on the MIR interpreter) has
the code loaded; `moddil build` links it in.

**ffi.native.1** C functions run in native code and on the MIR interpreter. On the interpreter a
pointer argument's value (and the rest of the array it points into) is copied into C's layout for
the call and back after it; a pointer C gives is kept as its address, and only native code reads
through it. WASM does not run C yet: building or testing a program that calls C as WASM is an
error, and the reference interpreter does not run it.

## Diagnostics

| Code | Meaning |
|---|---|
| E0218 | A raw pointer dereferenced, or a C function called, outside an `unsafe` block |
| E0219 | A C function declared, or `core::ffi` named, in a mod without the `unsafe` capability |
| E0222 | A C or `#[export]` function with a type C does not have, or with effects, errors or suspension; an `extern` block that is not `extern "C"`; a C function with a body; an `#[export]` that is not a free function with named parameters |

# C interop

Calling C from Moddil and Moddil from C, with the C ABI ([`specs/interop.md`](../../specs/interop.md)).
Added in M3 slice M3a; header import (`#[c_import]`) comes later.

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
| E0219 | A C function declared in a mod without the `unsafe` capability |
| E0222 | A C or `#[export]` function with a type C does not have, or with effects, errors or suspension; an `extern` block that is not `extern "C"`; a C function with a body; an `#[export]` that is not a free function with named parameters |

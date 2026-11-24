# C interop

Calling C from Moddil and Moddil from C, with the C ABI ([`specs/interop.md`](../../specs/interop.md)).
Added in M3 slice M3a; Python in M3b; structs by value, function pointers and C's memory on the
MIR interpreter in M3c; header import (`#[c_import]`) comes later.

**ffi.extern.1** `extern "C" { fn name(params) -> T; … }` declares C functions: each is a free
function without a body, named by its C symbol, called with the platform's C calling convention.
Only C's ABI is declared (another string is E0222), and a C function has no body (E0222). Only a
mod whose manifest lists the `unsafe` capability declares them (E0219), and a call of one is
allowed only inside an `unsafe` block (E0218): nothing checks what C does.

**ffi.types.1** A C function's, and an `#[export]` function's, parameters and result have **C's
types** (E0222 otherwise): the sized numbers but the 128-bit ones (`i8` as `int8_t`, … `f64` as
`double`), `int` (`int64_t`), `bool` (C's `bool`), and raw pointers (`*const T` as `const T *`,
`*mut T` as `T *`), to anything; a result may also be `()` (`void`). A struct crosses behind a
pointer, or by value (**ffi.types.2**). Neither kind of function needs handlers, can fail, or
suspends or spawns (E0222).

**ffi.types.2** A `#[repr(C)]` struct that is not generic and whose fields are C's types (numbers,
`bool`, raw pointers, `fn` types, and such structs) is one of C's types too: C functions and
`#[export]` functions take and give it by value, passed as the platform's C compiler passes it (in
registers, split into parts, or through a hidden pointer, by its size and its fields' types), and
the header declares it so. `*const ()` and `*mut ()` are C's `const void *` and `void *`.

**type.fnptr.1** `fn(A, B) -> R` is the type of a C function pointer: C's scalar types only (sized
numbers, `int`, `bool`, raw pointers and `fn` types; E0222 otherwise), and no result for `void`.
Its values are C functions' addresses, and functions named where one is wanted: a free function,
not generic, with the pointer's parameters and result, which needs no handlers, cannot fail and
does not suspend or spawn (E0222 otherwise). C calls a Moddil function so given through a C-ABI
wrapper, as an `#[export]`'s; a trap in it ends the process with exit code 70 (on the MIR
interpreter, see **ffi.native.2**). A `fn` value is kept in variables and `#[repr(C)]` fields,
given to C, and called as a function, `f(x)`, but only in an `unsafe` block (E0218), with as many
arguments as its type takes (E0202): nothing checks what it runs. Natively a Moddil function so
called runs through its C-ABI wrapper too, so a trap in it ends the process; on the MIR
interpreter it is a call as any other.

**ffi.repr.1** `#[repr(C)]` on a struct lays it out as C lays out a struct of the same fields:
in the order written, each at the next offset its alignment allows, the size rounded up to the
largest alignment. Other structs' layout is the compiler's to choose.

**ffi.export.1** `#[export]` on a `pub` free function with named parameters makes it callable
from C under its own name, with C's types (**ffi.types.1**). `moddil build --lib` builds the root
mod's exported functions, with the runtime, into a shared library, and writes its header,
`name.h`: the `#[repr(C)]` structs the signatures name, the other structs as incomplete types,
and the prototypes. With `--static` it builds a static library instead (`name.lib` on Windows,
`libname.a` elsewhere) holding the functions, the runtime and the mods' C code, and writes
`name.link`, the system libraries a program linking it needs too. The runtime starts with the
first call. A trap in an exported function ends
the process with exit code 70, as a built program's does: C has no way to be told. In Moddil an
exported function is called as any other.

**ffi.export.2** `#[export(status)]` is `#[export]` for generated bindings: the function gives
an `i32` status, and a trap in it makes it give 2 instead of ending the process.

**ffi.core.1** The `core` mod's `ffi` module ([source](core/ffi.moddil)), named only in a mod
with the `unsafe` capability (E0219), makes borrows of what C gives and hands boxes to C:
`slice(p, n)` and `slice_mut(p, n)` (the `n` values at `p`), `text(p, n)` (`n` bytes of UTF-8),
`ptr_of(s)` and `text_ptr(s)` (where a slice's or a text's values are), and `into_raw(b)` and
`from_raw(p)` (a box's value kept alive behind a raw pointer, and owned again). Nothing checks
that a pointer and a length describe live memory. Native code and the MIR interpreter run them
all, the interpreter as **ffi.native.2** says; the reference interpreter runs `into_raw` and
`from_raw` (keeping the value on a heap of its own) and traps on the others.

**ffi.core.2** `core::ffi` also has C's strings: `c_len(p)` (the bytes of the NUL-terminated
string at `p` before its NUL), `c_text(p)` (that string as text, which traps if it is not UTF-8)
and `c_string(s)` (the bytes of `s` and a NUL, which traps if `s` holds a NUL).

**ffi.python.1** `#[export(python)]` on a `pub fn` of a mod's root module makes it a function of
the mod's Python module, which `moddil build --wheel` builds (with CPython's stable ABI, 3.11 and
later) with its stubs (`name.pyi`) and a wheel; on a `pub fn` of a module inside the mod, a
function of the Python submodule of that module's path (`name.geo`, also importable as `import
name.geo`). Its arguments are given by position or by keyword (a parameter named `_` by position
only, and so each before it). Its parameters and result cross as Python values: numbers as `int`
and `float` (out of range is `OverflowError`), `bool`, `&str` and `String` as `str`, slices of
numbers as any C-contiguous buffer of that type without a copy (`bytes`, `bytearray`,
`array.array`, `memoryview`, numpy arrays; `&mut` ones changed in place), `Vec<u8>` as `bytes`
and other `Vec`s of numbers as `name.Array`, `Option` (a result) as the value or `None`, and `()`
as `None`. A `name.Array` owns the `Vec`: a read-only sequence (`len`, indexing, iteration,
`tolist()`) and buffer of the element's format, so `memoryview` and numpy read it without a
copy. A `fail` error is raised as an exception of its type's name, a subclass of `name.Error`,
carrying its `Debug` text and, for a struct, its `pub` fields of the types above as attributes; a
trap is raised as `name.Trap`. A function that needs handlers or suspends is not yet carried.
Calls release the GIL, and run one at a time.

**ffi.python.2** `#[export(python)]` on a `pub struct` makes it a class of the module (or of the
submodule of its module's path): an instance owns a value of it. Its inherent `pub fn`s are the
class's: `new` (giving the struct) its constructor, the others taking `self` its methods, and
those without `self` its static methods; its `pub` fields of the types above are properties,
which can be set if they are numbers or `bool`. A method taking `self` by value takes the
instance's value, as a move does: the instance has none after the call, and using it is
`ValueError`. Structs cross to and from functions by `&`, `&mut`, or by value (cloned).

**ffi.c.1** A mod's manifest may have a `[c]` table, the C code compiled with it by the system's
C compiler: `sources` (files, or `dir/*.c` for every `.c` file in `dir`), `include` (header
directories), `defines` (`NAME` or `NAME=value`) and `libs` (system libraries, by name). Its paths
are the manifest's directory's. A run of the mod's code (native, or on the MIR interpreter) has
the code loaded; `moddil build` links it in.

**ffi.native.1** C functions run in native code and on the MIR interpreter. On the interpreter a
pointer argument's value (and the rest of the array it points into) is copied into C's layout for
the call and back after it; a pointer C gives is kept as its address (**ffi.native.2**). WASM does
not run C yet: building or testing a program that calls C as WASM is an error, and the reference
interpreter does not run it.

**ffi.native.2** On the MIR interpreter, a pointer C gives is read and written through as in
native code, in C's layout: what it points to, its fields, and the values after it. A Moddil
pointer given C as `void *` is copied as **ffi.native.1** copies other pointers' values if C can
read its value (a number, a `bool`, a `#[repr(C)]` struct, or an array of those), and otherwise
crosses as a handle, which C does not read through and which is the pointer again when C gives it
back. A Moddil function given as a `fn` value crosses as a C function the interpreter makes, which
runs it when C calls it during the C call; a trap in it is raised when the C call returns, C
getting zeros from it meanwhile. There `core::ffi`'s `slice`, `text` and `c_text` of C's memory
borrow a copy of it, and `ptr_of` and `text_ptr` give the address of a copy kept until the run
ends; `slice_mut` of C's memory traps.

## Diagnostics

| Code | Meaning |
|---|---|
| E0218 | A raw pointer dereferenced, or a C function called, outside an `unsafe` block |
| E0219 | A C function declared, or `core::ffi` named, in a mod without the `unsafe` capability |
| E0222 | A C or `#[export]` function with a type C does not have, or with effects, errors or suspension; an `extern` block that is not `extern "C"`; a C function with a body; an `#[export]` that is not a free function with named parameters |

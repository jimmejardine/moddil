# cabi

C calling Moddil: a Moddil library whose `#[export]` functions are built into an ordinary shared
library with a generated C header, and a C program, [`c/main.c`](c/main.c), that calls them. It is
the example for **exporting to C** ([`reference/2027/interop.md`](../../reference/2027/interop.md)).
For the other direction, Moddil calling a C library, see [`../zlib`](../zlib/README.md).

## Build it and call it from C

```
moddil build --lib examples/cabi -o out/cabi
```

This writes `cabi.h` and the library into `out/cabi/`:

| Platform | Library |
|---|---|
| Windows | `cabi.dll`, with its import library `cabi.lib` |
| macOS | `libcabi.dylib` |
| Linux | `libcabi.so` |

The Moddil runtime is linked into it, and starts on the first call. Then compile the C program
against it.

On Linux or macOS:

```
cc examples/cabi/c/main.c -Iout/cabi -Lout/cabi -lcabi -Wl,-rpath,out/cabi -o out/cabi/main
out/cabi/main
```

On Windows, from a Developer Command Prompt:

```
cl examples\cabi\c\main.c /Iout\cabi /Fe:out\cabi\main.exe /Fo:out\cabi\ /link out\cabi\cabi.lib
cd out\cabi && main.exe
```

On Windows the DLL is found beside the program. The program prints
`C called Moddil: every result is right` and exits with 0, or exits with the number of the first
result that was wrong.

The Moddil side tests on its own, too, everywhere, because it calls no C:

```
moddil test examples/cabi            # natively; --interp and --wasm as well
```

## What crosses

| Moddil | C, in `cabi.h` | Shows |
|---|---|---|
| `moddil_gcd(a: int, b: int) -> int` | `int64_t moddil_gcd(int64_t a, int64_t b);` | `int` is `int64_t` |
| `moddil_is_even(n: u8) -> bool` | `bool moddil_is_even(uint8_t n);` | sized integers and `bool` |
| `moddil_distance(a: *const Point, b: *const Point) -> f64` | `double moddil_distance(const Point *a, const Point *b);` | a struct read through `const` pointers |
| `moddil_scale(p: *mut Point, k: f64)` | `void moddil_scale(Point *p, double k);` | a struct changed in place; `()` is `void` |

- **`Point` is `#[repr(C)]`** (`ffi.repr.1`), so the header declares it as a full C struct with
  the same fields, and C can make one on its own stack. A struct without `#[repr(C)]` would be an
  incomplete type in the header, which C can only hold a pointer to.
- **Structs cross behind pointers** (`ffi.types.1`). Reading or writing through one is an `unsafe`
  block, whose text says what C promises: "C passes a point it owns, and nothing else uses it
  meanwhile".
- **An exported function is still an ordinary Moddil function.** The `test` block in
  [`src/lib.moddil`](src/lib.moddil) calls each export directly, passing borrows where C passes
  pointers.
- **The rules for exports:** each export is a `pub` free function with named parameters and C's
  types. It can't need effect handlers, fail with an error, suspend or spawn (E0222). A trap in one
  ends the process with exit code 70, since C has no way to be told (`ffi.export.1`).

## Where it is checked

- `crates/moddil/tests/ffi.rs` builds the library, checks the header's prototypes, compiles
  `c/main.c` with the system's C compiler, and runs it.
- `crates/moddil/tests/projects.rs` runs the Moddil tests natively, on the MIR interpreter, and
  as WASM.

## Status

Built in M3 slice M3a ([`specs/roadmap.md`](../../specs/roadmap.md#milestones)). Next, in M3b,
`#[export(python)]` and `moddil build --wheel` export to Python (`ffi.python.1`).

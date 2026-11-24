# cabi

C calling Moddil: a Moddil library whose `#[export]` functions are built into an ordinary shared
library with a generated C header, and a C program, [`c/main.c`](c/main.c), that calls them. It is
the example for **exporting to C** ([`reference/2027/interop.md`](../../reference/2027/interop.md)),
and for structs by value in both directions: the library calls C too,
[`c/shapes.c`](c/shapes.c), and hands it Moddil functions to call back. For a whole C library
called from Moddil, see [`../zlib`](../zlib/README.md).

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

The Moddil side tests on its own too. It calls `c/shapes.c`, compiled with it, so natively and
on the MIR interpreter, not as WASM:

```
moddil test examples/cabi            # natively; --interp as well
```

## What crosses

| Moddil | C, in `cabi.h` | Shows |
|---|---|---|
| `moddil_gcd(a: int, b: int) -> int` | `int64_t moddil_gcd(int64_t a, int64_t b);` | `int` is `int64_t` |
| `moddil_is_even(n: u8) -> bool` | `bool moddil_is_even(uint8_t n);` | sized integers and `bool` |
| `moddil_distance(a: *const Point, b: *const Point) -> f64` | `double moddil_distance(const Point *a, const Point *b);` | a struct read through `const` pointers |
| `moddil_scale(p: *mut Point, k: f64)` | `void moddil_scale(Point *p, double k);` | a struct changed in place; `()` is `void` |
| `moddil_midpoint(a: Point, b: Point) -> Point` | `Point moddil_midpoint(Point a, Point b);` | structs by value: two doubles |
| `moddil_swap(p: Pair) -> Pair`, `moddil_bump(m: Mixed) -> Mixed`, `moddil_rotate(t: Triple) -> Triple`, `moddil_halves(h: Halves) -> Halves` | likewise | the other shapes the C conventions treat differently |

- **`Point` is `#[repr(C)]`** (`ffi.repr.1`), so the header declares it as a full C struct with
  the same fields, and C can make one on its own stack. A struct without `#[repr(C)]` would be an
  incomplete type in the header, which C can only hold a pointer to.
- **Structs cross behind pointers** (`ffi.types.1`), or by value (`ffi.types.2`). Reading or
  writing through a pointer is an `unsafe` block, whose text says what C promises: "C passes a
  point it owns, and nothing else uses it meanwhile".
- **By value, each shape crosses as the platform's C compiler passes it.** Two doubles go in
  vector registers on Linux, in float registers on AArch64, and behind a pointer on Windows;
  three words go in memory or behind a pointer; and so on. The five structs cover the cases.
- **An exported function is still an ordinary Moddil function.** The `test` block in
  [`src/lib.moddil`](src/lib.moddil) calls each export directly, passing borrows where C passes
  pointers.
- **The rules for exports:** each export is a `pub` free function with named parameters and C's
  types. It can't need effect handlers, fail with an error, suspend or spawn (E0222). A trap in one
  ends the process with exit code 70, since C has no way to be told (`ffi.export.1`).

## Moddil calling C back

`c/shapes.c` takes and gives the same five structs by value, so the library's tests check the
calling side as `c/main.c` checks the called side. It also calls Moddil back (`type.fnptr.1`):
- `shape_fold(xs, n, add, init)` folds an array with `add`, a Moddil function given as a
  `fn(int, int) -> int`;
- `shape_op(which)` gives back one of two C functions as a `fn(int, int) -> int`, which
  Moddil then calls itself (in an `unsafe` block);
- `shape_each(n, count, data)` calls `count` with each number and `data`, a `void *` to the
  Moddil `Tally` it adds them up in, as C APIs with callbacks do.

## Where it is checked

- `crates/moddil/tests/ffi.rs` builds the library, checks the header's prototypes, compiles
  `c/main.c` with the system's C compiler, and runs it.
- `crates/moddil/tests/projects.rs` runs the Moddil tests natively and on the MIR interpreter,
  and checks that WASM refuses them.

## Status

Built in M3 slice M3a ([`specs/roadmap.md`](../../specs/roadmap.md#milestones)); structs by
value and the calls back in M3c. `#[export(python)]` and `moddil build --wheel` export to Python
(M3b, [`../python`](../python/src/lib.moddil)).

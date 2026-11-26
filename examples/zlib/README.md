# zlib

Moddil calling a real C library: [zlib](https://zlib.net) 1.3.1, vendored in [`c/`](c) and
compiled with the mod, wrapped in a typed, safe Moddil API. It is the example for **calling C**
([`reference/2027/interop.md`](../../reference/2027/interop.md)). For the other direction, C calling
Moddil, see [`../cabi`](../cabi/README.md).

```
moddil test examples/zlib            # natively
moddil test --interp examples/zlib   # on the MIR interpreter, which calls the same C
moddil run examples/zlib             # src/main.moddil
```

`moddil run` prints:

```
[info] 127 bytes compress to 75, and come back true
[info] crc32 4172008142
```

The first build compiles zlib with the system's C compiler (on Windows, MSVC's `cl` lists each
file as it goes).

## How it fits together

| File | Part |
|---|---|
| [`moddil.toml`](moddil.toml) | The `[c]` table: compile `c/*.c` with the mod, with `c/` on the include path (`ffi.c.1`), and the `unsafe` capability that declaring C functions needs (`ffi.extern.1`) |
| [`c/moddil_shim.c`](c/moddil_shim.c) | A thin C layer over zlib's API with fixed-width types |
| [`src/lib.moddil`](src/lib.moddil) | The `extern "C"` declarations of the shim, and over them the Moddil API, with its tests |
| [`src/main.moddil`](src/main.moddil) | A small program: compress a text, get it back, print the sizes and the CRC-32 |
| `c/*.c`, `c/*.h` | zlib itself, unchanged, under its own licence ([`c/LICENSE`](c/LICENSE)) |

The steps below go from C up to the safe API.

1. **The shim is the C ABI's boundary.** zlib's `uLong` is C's `unsigned long`: 32 bits on Windows,
   64 elsewhere. Its `z_stream` holds `uLong`s, so its layout changes with the platform too. The
   shim takes `uint64_t` lengths and keeps each `z_stream` behind an opaque pointer, so a single
   set of Moddil declarations is right on every platform. That's the usual practice for a portable
   C ABI, and the reason not to bind most of zlib's own functions directly. The few whose types
   are the same everywhere, `zlibVersion`, `zError` and `get_crc_table`, are bound directly.
2. **The `extern "C"` block declares the shim's functions** with C's types (`ffi.types.1`): `u64`,
   `i32`, and raw pointers. `ZStream` is an empty struct that is only ever behind a pointer C gave.
   `Stats` is `#[repr(C)]` (`ffi.repr.1`), laid out like the shim's `moddil_zlib_stats`, so C
   gives one back by value (`ffi.types.2`: 24 bytes, so through a hidden pointer or in memory).
3. **Every call is in an `unsafe` block, and the block's text says why the call is sound**: for
   example, "the buffers are as long as the lengths say, and live across the call". Nothing checks
   what C does, so those reasons are the contract.
4. **The public API has no `unsafe` in it:**
   - `compress(data, level)` and `uncompress(data, size)` fail with a `ZlibError` carrying zlib's
     code, for example `-3` (`Z_DATA_ERROR`) for corrupt input;
   - `crc32` and `adler32` give the checksums;
   - `bound(n)` gives the most bytes that compressing `n` bytes can make;
   - `Deflater` compresses a piece at a time (`write`, then `finish`). It owns its C state and frees
     it in `Drop`, so a deflater can't leak or be freed twice;
   - `version()` and `ZlibError::message()` read zlib's C strings (`ffi.core.2`);
   - `crc32_by_table` works the CRC-32 out in Moddil from zlib's table, a C array walked through
     its pointer (`core.ptr.2`);
   - `uncompress_through_callbacks` hands zlib's `inflateBack` two Moddil functions (`type.fnptr.1`):
     `pull`, which gives it the input, and `push`, which appends each piece of output to a `Vec`.
     Each gets its state back through the `void *` it was given with.

Some details worth noticing:
- **An empty input still passes a valid pointer.** C reads no byte of it, but `&data[0]` would
  trap, so the code points at a spare byte instead.
- **Lengths cross as `u64`, and are converted with `?!`**, which records why they can't fail (for
  example, a length is never negative).

## Where it runs

- **Native code and the MIR interpreter both run it** (`ffi.native.1`). On the interpreter, each
  buffer a pointer argument points to is copied into C's layout for the call and copied back
  afterwards. Pointers C gives are kept as addresses, and read through in C's layout
  (`ffi.native.2`). `pull` and `push` run as C functions the interpreter makes, which call back
  into it during the C call; the `Vec` behind `push`'s `void *` crosses as a handle, since C
  can't read it.
- **WASM doesn't run C yet.** `moddil test --wasm examples/zlib` is refused ("runs only in native
  code"), and the reference interpreter doesn't run it either.
- **CI:** `crates/moddil/tests/projects.rs` runs the tests natively and on the interpreter, and
  checks that WASM refuses them.

## Status

Built in M3 slice M3a ([`specs/roadmap.md`](../../specs/roadmap.md#how-work-is-built)); the strings, the
CRC table, `Stats` by value and the callbacks in slice M3c. Later, header
import (`#[c_import]`) will read `zlib.h` itself. The shim would still be the portable choice for
the `uLong` types.

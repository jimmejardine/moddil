# Interoperability with C and Rust

> **Status:** decided direction, details open. Expands [`rationale.md`](rationale.md) §2.13.
> Snippets are **illustration only**; no syntax is committed.

## 1. Summary

| Rule | One line |
|---|---|
| **Two foreign worlds only** | The language supports the **C ABI** and **Rust**. C++ is reached through C shims, never through a C++ importer in the compiler. |
| **C, both directions, zero overhead** | Direct header import, layout-compatible types, C export with generated headers. |
| **The toolchain compiles C** | C dependencies are built hermetically and cross-compiled by the bundled clang/LLVM, as `zig cc` does. |
| **Overlays type the C surface** | Shared annotation overlays add ownership, nullability, error conventions, effects (including `blocking`), thread affinity and capabilities. |
| **Rust through a generated bridge** | Tooling reads a crate's API and generates a C-ABI shim plus Moddil bindings, mapping ownership, `Option`, `Result`, panics and futures. |
| **WASM components as the sandbox** | Untrusted or browser-bound C and Rust code can be linked as WASM components over WIT. |
| **Foreign code is visible** | Raw bindings are `unsafe`; using them needs the `ffi` capability, which shows in signatures, audits and semantic diffs. |
| **Callbacks are entry points** | Code called from C must handle all errors (R3); nothing unwinds across the boundary. |

## 2. Scope: why C and Rust only

C++ interop is the most expensive interop feature a language can take on: templates, overloading,
exceptions and compiler-specific ABIs. Swift and Carbon have each spent years on it. Moddil does not.

| Need | Typical form | Covered by |
|---|---|---|
| Exchange SDKs, FIX engines (HFT) | Often C++ only | A thin **C shim** (§10) |
| Game physics engines | Mixed: some C (e.g. Box2D v3), some C++ | C APIs where they exist; shims otherwise; or compiled to WASM and linked as a component (§9) |
| CUDA libraries, LLVM, SQLite, OpenSSL, DPDK, libpq | C APIs | Direct (§3) |
| Mobile platform APIs (web store) | iOS: Objective-C/Swift; Android: Java/Kotlin | Still C underneath: JNI is a C ABI, the Objective-C runtime has a C API, and Swift can export C functions (`@_cdecl`). Platform mods wrap them. |
| The Rust ecosystem, existing Rust code (Hashiverse) | Rust crates | The generated bridge (§8) or components (§9) |

## 3. The C ABI, both directions

- **Zero-overhead calls** in both directions: a call to C is a direct call with the platform C calling
  convention.
- **Layout-compatible types**: structs, unions, enums and fixed arrays can be declared with C layout
  (`@repr(C)`); variadic C functions can be called.
- **Direct header import** through libclang (as Zig's `@cImport`): types, functions, constants and
  simple macros become raw bindings automatically.
- **The toolchain compiles C.** C sources of a dependency are compiled by the bundled clang, using the
  LLVM Moddil already ships ([`bootstrap.md`](bootstrap.md)):
  - hermetic and reproducible: declared inputs only, the same compiler on every machine (rationale
    §2.16);
  - cross-compiled to every Moddil target, including WASM (rationale §2.15, R1);
  - run in the build sandbox like any other build step.

```
import c "sqlite3.h" with overlay "sqlite3.overlay"   // raw bindings + overlay → typed API
```

## 4. Annotation overlays

C headers say nothing about ownership, nullability, errors, blocking or threads. **Overlays** add it,
and turn raw bindings into Moddil types:

| C fact | Overlay says | Moddil result |
|---|---|---|
| Who frees a returned pointer | owned / borrowed / linear | Owned: a Moddil type whose drop calls the C free function. Borrowed: a view. Linear: when freeing can fail (e.g. a close that reports errors). |
| Can a pointer be null | nullable | `Option<…>` (rationale §2.4) |
| How errors are reported | errno, negative return, out-parameter | `fail(E)` with a typed error |
| Can the call block | `blocking` | The runtime moves the call off its workers to a blocking pool, so one slow C call can't stall the scheduler. Such calls are the one exception to promptly cancellable I/O ([`concurrency.md`](concurrency.md) §9.5). |
| Thread requirements | thread-affine, not thread-safe | Pinned tasks; values that can't be sent are wrapped accordingly |
| What it touches | capabilities (`fs`, `net`, …) | Checked like Moddil code ([`mods.md`](mods.md) §2) |

- Overlays live in a **shared repository**, versioned against library versions, like TypeScript's
  DefinitelyTyped, so each C library is annotated once for everyone.
- An overlay is reviewable code: a wrong overlay is a bug in a specific, auditable place.

## 5. The safety boundary

- **Raw bindings are `unsafe`.** Safe wrappers are ordinary mods built on them.
- Any mod that uses foreign code declares the **`ffi` capability**. It appears in signatures, package
  audits and semantic diffs (rationale §2.16, §2.19): "`foo` 1.5 now uses `ffi`" is flagged on
  upgrade.
- Faults in foreign code are traps (class 3; [`error-handling.md`](error-handling.md) D8).
- Foreign code is **outside record and replay** unless it is wrapped by effects; the recorder flags it
  ([`determinism.md`](determinism.md) §5, [`observability.md`](observability.md) §8).

## 6. Callbacks into Moddil

- A Moddil function passed to C as a callback is an **entry point**
  ([`error-handling.md`](error-handling.md) §4.4): its failure set must be empty, so no Moddil error can
  escape into C.
- Nothing unwinds across the boundary in either direction.
- Borrowed data passed to a callback follows the ordinary borrow and view rules
  ([`memory-and-resources.md`](memory-and-resources.md) §3–§4); a callback cannot keep it.

## 7. Export

- Moddil functions and `@repr(C)` types can be **exported with the C ABI**, with a **generated header**.
- Moddil mods can also be exposed to Rust as a **generated crate** (§8, reverse direction).
- This is the adoption path: Moddil can be introduced inside existing C, C++ or Rust projects one
  module at a time (rationale §7).

## 8. Rust: the generated bridge

Rust has **no stable ABI**: layouts, generics, trait objects and `Result` cannot cross a boundary
directly. Rust's experimental interoperable ABI (crABI) is tracked but not depended on. Moddil uses
the approach of Mozilla's UniFFI, ICU4X's Diplomat and `cxx`: **generate both sides of a C-ABI
bridge from one description.**

- **Input:** the crate's public API, read from rustdoc's JSON output, plus light annotations where
  intent is ambiguous.
- **Output:** a small Rust **shim crate** of `extern "C"` functions, and **Moddil bindings** that look
  like a native mod.

| Rust | Moddil |
|---|---|
| Owned values, `Box<T>` | Owned values, dropped through the shim |
| `&T`, `&mut T` | `let`, `mut` borrows |
| `Option<T>` | `Option<T>` |
| `Result<T, E>` | `fail(E)` |
| `String`, `Vec<T>`, slices | Owned Moddil equivalents (moved or copied) or borrowed views |
| `Arc<T>` | Opaque handle: Moddil's biased RC has a different layout |
| Trait objects | Opaque handles with generated methods |
| Generics | Explicit instantiations listed in the bridge description |
| Panics | Caught in the shim; become traps |
| `async fn`, `Future` | A bridge mod drives the future from a Moddil task, and the waker reschedules the task, so Rust async code suspends as an ordinary Moddil effect |

```
import rust "hashiverse-lib" version "1.4"     // generates shim + bindings; the mod needs `ffi`
let post = hashiverse.Post.decode(bytes)?      // Result<Post, DecodeError> → fail(DecodeError)
```

## 9. WASM components: the sandboxed path

- C and Rust code compiled to WASM can be linked as **components**, with interfaces in **WIT**, which
  Moddil maps to and from automatically (rationale R1). Rust has strong tooling for this
  (`wit-bindgen`).
- It costs copying at the boundary (the canonical ABI), so it isn't zero-copy.
- In return, the foreign code is **isolated**: it can reach only the capabilities it is given, as
  RLBox does for C libraries in Firefox.
- It is the natural path for **untrusted crates**, for plugins, and for the **browser**.

## 10. C++ through C shims

- C++ libraries are wrapped by a thin `extern "C"` layer, written in C++, that exposes a C API.
  Moddil then imports that like any C library.
- C++ exceptions must not cross the shim: catch them and return error codes, which the overlay maps
  to `fail(E)`.
- A **shim generator** for a subset of C++ (plain classes, non-template functions, explicit
  instantiations) may exist as an optional tool (standard or community). It is tooling, not part of
  the language or compiler.

## 11. Fit check against the reference examples

| Example | Uses |
|---|---|
| [Multiplayer game](examples/01-multiplayer-game.md) | A physics engine through its C API, a shim, or as a WASM component in the browser |
| [HFT](examples/02-hft-platform.md) | DPDK / ef_vi-style kernel-bypass through C with zero overhead; C++ exchange SDKs and FIX engines through shims; `blocking` overlays for SDK calls that block |
| [Web store](examples/03-web-store.md) | PostgreSQL through libpq or a native mod; mobile platform APIs through JNI and the Objective-C/Swift C surface |
| [Hashiverse](examples/04-hashiverse.md) | The generated Rust bridge to `hashiverse-lib` during migration, replaced module by module by native Moddil |

## 12. Open questions

- How stable is rustdoc's JSON format, and how much generator maintenance follows Rust releases?
- How far can generics cross the Rust bridge beyond explicit instantiations?
- If crABI stabilises, can the bridge use it instead of the C ABI?
- Overlay distribution and trust: who may publish overlays, and how are they audited?
- Python interop (raised in the HFT example): out of scope, or through Python's C API as a mod?
- Size of the bundled clang in the toolchain download.

## Background

The problem and the state of the art that led to this design. (Moved from the rationale.)

**Problem.** Nobody adopts a language that can't use existing code. Decades of C libraries, huge C++
codebases, and a growing Rust ecosystem must be reachable with low friction and low overhead.

**State of the art.**
- **C ABI**: the de facto lingua franca. Every language speaks it.
- **Zig**: `@cImport` reads C headers directly; `zig cc` is a drop-in C/C++ cross-compiler. This is the
  best-in-class C story.
- **Swift**: direct C++ interop (importing classes, templates in some cases) without wrappers.
- **Carbon**: designed explicitly as a C++ successor with bidirectional interop.
- **Rust**: `bindgen`, `cbindgen`, `cxx` (safe C++ bridge), `crubit` (Google's C++/Rust interop).
- **D**: `extern(C++)` with substantial C++ ABI support.
- **Mojo**: Python interop as a primary adoption strategy.

**What still hurts.**
- C headers carry no ownership, lifetime, nullability or thread-safety information; bindings must
  guess or annotate by hand.
- C++ interop is hard: templates, overloading, exceptions, ABI differences between compilers.
- **Rust has no stable ABI**; interop goes through `extern "C"`, losing generics, traits, and ownership.
- Each FFI boundary is a hole in safety guarantees.

## Prior art

Zig `@cImport` and `zig cc`; Rust `bindgen`, `cbindgen`, `cxx` and `autocxx`; Mozilla UniFFI; ICU4X
Diplomat; Google crubit; the Rust crABI proposal; the WASM Component Model and `wit-bindgen`; RLBox;
Swift's C interop and `@_cdecl`; JNI; TypeScript DefinitelyTyped. Swift's and Carbon's C++ interop
are the paths deliberately not taken.

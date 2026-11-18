# Requirements and Non-Goals

> **Status:** R1–R3 are decided. The non-goals are a draft. Expands [`rationale.md`](rationale.md) §5.

## 1. Firm requirements (decided)

These are no longer open for debate. Every other design decision must be compatible with them.

### R1. WebAssembly is a first-class compilation target.

WASM is not a port done later. It is a tier-one target from the first release, tested in CI like
native targets. Reasons:
- **Multi-tier programs (rationale §2.10)** need it: the browser tier compiles to WASM, and edge/serverless
  platforms increasingly run WASM components.
- **It fits the capability model (rationale §2.16, §2.1).** A WASM module has no ambient authority; it can only
  call the imports it is given. WASI and the Component Model are object-capability systems.
- **It is the sandbox for untrusted code**: build-time logic and compile-time evaluation (rationale §2.14, §2.6),
  package build steps (rationale §2.16), plugins, and even imported C libraries compiled to WASM and isolated
  (as RLBox does in Firefox, rationale §2.13).
- **Portability and determinism**: one artefact that runs anywhere, with deterministic execution
  available for simulation testing (rationale §2.11).

What "first-class" means in practice:
- **Targets**: core WASM (`wasm32`, later `wasm64`/memory64), **WASI preview 2+ and the Component
  Model**, and the **browser** (with JS interop generated from Moddil types).
- **Interfaces**: Moddil module interfaces map to and from **WIT** automatically, so Moddil
  components compose with components written in Rust, C, Go, JS, etc.
- **Effects map to imports**: a capability a Moddil module requires becomes a WASM import; one it does
  not require cannot be reached.
- **Small binaries and fast start-up** are explicit goals (they matter for browsers and edge).
- **The standard library** works on WASM, with platform-specific parts behind capabilities.

Design constraints this places on the rest of Moddil:
- **Memory model**: Moddil has no global tracing GC, so linear memory is the natural fit. The WASM GC
  proposal is an optional optimisation for interop with JS objects, not a dependency.
- **Concurrency without function colouring (rationale §2.8)** needs suspendable stacks. *Revised after checking
  engine support (Nov 2026; see [`bootstrap.md`](bootstrap.md) §6 and §8):* stack switching is Phase 3
  but ships in **no** browser or Node (Chrome has it behind a flag only); Wasmtime has it at Tier 3 on
  x86_64 Linux. JSPI, however, is standardised and ships in all major browsers and Node ≥ 26.
  **Decision:** the concurrency semantics are target-independent, and the backend picks a lowering:
  1. **WASM stack switching**: the preferred lowering, built first against **Wasmtime**.
  2. **JSPI**: the browser and Node baseline until stack switching ships there. Each Moddil task is
     its own suspended `promising` stack, scheduled from JS.
  3. **CPS/Asyncify-style transform**: only if WASI runtimes without stack switching must be
     supported (open question).

  Keep the dependency in one place in the backend (the task/continuation runtime) so proposal
  changes stay local.
- **Threads**: shared memory + atomics are available but restricted in browsers (cross-origin
  isolation). Structured concurrency must degrade gracefully to a single thread; on a single thread
  every biased-RC operation takes the cheap owner path ([`concurrency.md`](concurrency.md) §6).
- **Exceptions, SIMD, tail calls**: use the standard proposals (exception handling, 128-bit SIMD,
  tail calls) where available; error handling (rationale §2.3) must not depend on unwinding.
- **GPU (rationale §2.9)** in the browser goes through WebGPU/WGSL, so kernel lowering needs a WGSL path.
- **FFI (rationale §2.13)** on WASM means C code compiled to WASM and linked as components, not native calls.

### R2. Small core; functionality lives in mods.

The core language is deliberately very small. Almost everything else, including collections,
networking, crypto, the async runtime, GPU support and UI, is provided by **mods** (Moddil's package
unit, equivalent to a Rust crate). Standard mods live under the `moddil/` namespace (like C++'s
STL/Boost); anyone can write and publish their own (for example, a tokio-style runtime).

- *Terminology*: a **mod** is a package; a **module** is a namespace/file unit inside a mod.
- *Boundary rule*: a feature belongs in the core only if the compiler must check it (ownership,
  effects, capabilities, placement, contracts), if other mods need it to be defined at all, or if it
  cannot be expressed efficiently in a mod.
- *Runtimes are mods*: scheduling, I/O and time are core **effects**; runtimes are mods that provide
  **handlers**. Every library therefore works with every runtime, which avoids Rust's tokio/async-std
  split.

Details: [`mods.md`](mods.md).

### R3. No crash from an unhandled error.

Every error the language knows about (the `fail` effect / `Result`) must be handled. An unhandled
error at any entry point (`main`, task bodies, exported/FFI functions, tier endpoints, handlers) or a
silently dropped result is a **compile-time error**. There are **no fire-and-forget tasks**: every
task is awaited by its enclosing scope. Out of scope for R3: traps such as out of memory, stack
overflow, integer division by zero and out-of-bounds indexing, which are crashes as in Rust.

Details: [`error-handling.md`](error-handling.md).

## 2. Non-goals (draft)

- Maximum syntactic novelty. Familiar syntax wins unless there is a strong reason.
- Replacing every language. Scripting, notebooks and quick glue may remain better served elsewhere
  (open question: is a REPL/script mode desirable?).
- C++ interop in the language. C++ is reached through C shims ([`interop.md`](interop.md) §10).
- A custom VCS that replaces Git (tooling on top of Git first).
- A cloud platform, orchestrator or service mesh. Moddil emits artefacts and manifests for existing
  ones.
- Transparent remote calls.
- Runtime reflection / dynamic metaprogramming.

## 3. Open questions

- R1: should WASI runtimes without stack switching be supported, which would need the
  CPS/Asyncify-style lowering? (Also in [`bootstrap.md`](bootstrap.md) §9.)
- Non-goals: is a REPL or script mode desirable?
- Should determinism become R4 ([`determinism.md`](determinism.md) §12)?

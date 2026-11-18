# Compiler and bootstrap

> **Status:** proposal. Answers [`roadmap.md`](roadmap.md) Q6: how the compiler is built, structured and
> bootstrapped, and which backends it uses. Milestones are in [`roadmap.md`: Milestones](roadmap.md#milestones). WASM support facts verified November 2026 ([WASM support: verified status (November 2026)](#wasm-support-verified-status-november-2026) below).

## Answers in brief

**What do we build the first compiler in?** **Rust.** A Rust "stage 0" compiler implements a core
subset of Moddil. The compiler is then rewritten in Moddil and compiled by stage 0, and from then on
it compiles itself. After self-hosting, bootstrapping uses a checked-in WASM build of the compiler
(Zig's approach), with the Rust stage 0 kept alive to cross-check it.

**Our own backends, or existing ones?** **Existing ones, except WASM.** Moddil owns its front end and
its mid-level IR (where all the Moddil-specific checking and lowering happens). Cranelift does fast
debug builds, LLVM does optimised release builds and broad target coverage, `naga` and LLVM handle
GPUs, and Moddil writes its **own WASM backend** because WASM is central (R1) and needs proposals that
LLVM lags on.

**Which WASM runtime first?** **Wasmtime**, the reference runtime for the early milestones. It is the
only mainstream runtime that implements stack switching today. Browser support (via JSPI) follows
later.

---

## Stage 0 language: Rust

**Why Rust:**
- **Ecosystem fit.** Most of the infrastructure Moddil needs already exists as Rust crates:
  - `cranelift`: fast code generation for debug builds.
  - `wasmtime`: the WASM runtime ([Risks and open questions](#risks-and-open-questions)), embeddable in the compiler.
  - `wasm-encoder`, `wasmparser`, `wit-parser`, `wit-component`: WASM and Component Model (R1).
  - `naga`: shader translation between WGSL, SPIR-V, MSL and HLSL ([Concurrency across GPUs and accelerators](accelerators.md)).
  - `gimli`: DWARF debug info.
  - `salsa`: query-based incremental compilation, as used by rust-analyzer ([Build times](build-times.md), compiler-as-a-service [LLMs as first-class programmers](agent-interface.md)).
  - `inkwell` / `llvm-sys`: LLVM.
  - `tower-lsp`, `ariadne`/`codespan-reporting`, `logos`: language server, diagnostics, lexing.
- **Familiarity.** The project owner works in Rust daily (Hashiverse).
- **Translation path.** Rust's ownership model is close to Moddil's, so rewriting the compiler in
  Moddil later is largely mechanical, which suits LLM-assisted porting.

**Alternatives considered:**
- **OCaml**: a classic compiler language (Rust's first compiler was in OCaml), but a weaker fit with
  the WASM, Cranelift and GPU ecosystems above.
- **C++**: native access to LLVM and MLIR, but slow builds and no safety.
- **Zig**: attractive, but a younger ecosystem with fewer of the pieces above.

**Accepted cost:** stage 0 itself builds at Rust speed. Mitigate with a workspace split into many small
crates, plus Cranelift for the compiler's own debug builds.

## Staged bootstrap

```
 stage 0  (Rust)     compiles Moddil-core ───────────────┐
                                                         ▼
 stage 1  (Moddil)   compiler source, compiled by stage 0
 stage 2             stage 1 compiling itself
 stage 3             stage 2 compiling itself    →  stage 2 == stage 3 (bit-identical)
```

- **Stage 0** implements **Moddil-core**: the subset needed to write a compiler. That is types, sum
  types, generics, ownership, modules and mods, basic effects and handlers. It is not the full language.
- Stage 0 includes an **interpreter for MIR** (the mid-level IR, [Backends](#backends)). Compile-time evaluation ([Metaprogramming](comptime.md))
  needs one anyway, and it lets the language run tests before any backend exists.
- **Dogfood early.** Write the first standard mods (`moddil/collections`, `text`, `serde`, `test`)
  in Moddil while the compiler is still in Rust.
- **Fixed-point check**: stage 2 and stage 3 must be bit-identical. This runs in CI, and also checks
  reproducible builds ([Package management and supply-chain security](packages-and-supply-chain.md)).
- **When to self-host:** not until the core semantics are stable, and **after the Hashiverse client
  milestone** has proved the language on real code. Self-hosting too early means rewriting the
  compiler every time the language changes.

## Bootstrapping after self-hosting

**Zig's trick, applied to Moddil.** Zig commits a WASM build of its compiler (`zig1.wasm`). Building
Zig from source then needs only a C compiler and a small WASM-to-C translator. Moddil already has a
first-class WASM target (R1), so it can do the same: `bootstrap/moddil1.wasm` plus a minimal runner
(Wasmtime, or a `wasm2c` path).

**Trusting trust.** A checked-in binary raises Ken Thompson's "Reflections on Trusting Trust"
problem, which is exactly the supply-chain concern of [Package management and supply-chain security](packages-and-supply-chain.md). Mitigations:
- The WASM blob must be **reproducible**: anyone can rebuild it from source and get the same bytes.
- Keep the **Rust stage 0 buildable** so the compiler can be built by two independent paths and the
  results compared (**diverse double-compiling**, David A. Wheeler).
- Optional third path: a **C backend** (as Nim has and as Zig's CBE provides). It also helps port to
  unusual platforms.

## Compiler architecture

- **Query-based and demand-driven** (Salsa, as in rust-analyzer), with dependencies tracked down to
  individual **declarations** (Zig's incremental compilation goes to the same granularity). Only
  queries whose inputs actually changed are recomputed.
- **Lazy compilation:** only code reachable from a root (a binary, a test) is analysed and compiled,
  as Zig does. Unused parts of large mods cost nothing.
- **Parallel at every stage:** mods in DAG order; functions within a mod during code generation.
- **Content-addressed, layered cache** ([Cached builds](build-times.md#cached-builds)).
- **No link step in dev builds:** functions are reached through an indirection table and compiled by
  Cranelift in process, which also enables **live update** ([Live update (debug builds)](build-times.md#live-update-debug-builds)). Release builds link normally with
  `lld` or `mold`, with split debug info.
- **Check-only mode:** type, effect and ownership checking without code generation, powering the
  language server and agents ([LLMs as first-class programmers](agent-interface.md)).
- **Test impact analysis:** the query graph knows which tests depend on a change, so only those run.

## Backends

```
 source ─► parse ─► canonical AST (agent-interface.md, semantic-vcs.md)
                       │
                       ▼
        names · types · effects · ownership · placement · capabilities
                       │
                       ▼
                    ┌──────┐
                    │ MIR  │  Moddil mid-level IR: handlers, task scopes and
                    └──┬───┘  suspension points lowered here, per target
     ┌─────────┬──────┼──────────┬───────────────┬──────────────┐
     ▼         ▼      ▼          ▼               ▼              ▼
 interpreter Cranelift LLVM   own WASM       GPU: naga /     C (optional)
 (comptime,  (debug)  (release, emitter      LLVM NVPTX /
  tests)              all targets) (+ wasm-opt) AMDGPU; MLIR later
```

| Backend | Role | Own or reuse | Notes |
|---|---|---|---|
| **MIR + front end** | All Moddil-specific semantics | **Own** | This is where the language's value lies |
| MIR interpreter | Compile-time evaluation, fast tests | **Own** | Needed anyway for [Metaprogramming](comptime.md) |
| **Cranelift** | Debug/dev builds | Reuse | Fast compile; x86-64, AArch64, RISC-V 64, s390x |
| **LLVM** | Release builds; iOS/Android/embedded; mature optimisation | Reuse | Pin one version; use via the C API / `inkwell` |
| **WASM** | Browser, Node, WASI, components, bootstrap | **Own** emitter (`wasm-encoder`) + Binaryen `wasm-opt` | WASM is simple; needs stack switching, JSPI glue, exnref, GC and Component Model earlier than LLVM provides them; binary size matters |
| **GPU** | Kernels | Reuse | `naga` for WGSL/SPIR-V/MSL/HLSL; LLVM NVPTX/AMDGPU for CUDA/ROCm-class targets; MLIR later if tensor-level optimisation is needed |
| **C** | Third bootstrap path, odd platforms | Own (optional) | Decide later |
| Native backends of our own | Faster compiles, full control | **Not now** | Go and Zig did this at great cost; revisit only if Cranelift + LLVM block the build-time goals |

**Linkers:** `lld` or `mold` for native code; `wasm-ld` or a Moddil linker for WASM modules and
components. **Cross-compilation ([Cross-compilation and deployment](targets.md)):** LLVM plus bundled libc/sysroots, as Zig does
([`targets.md`](targets.md)).

## WASM concurrency lowering

Moddil's concurrency semantics (structured tasks, the `suspend` effect, no function colouring, [Concurrency across CPUs](concurrency.md))
are target-independent. The backend chooses how to implement suspension:

| # | Lowering | Where | Status |
|---|---|---|---|
| 1 | Native stacks | Native targets | Always available |
| 2 | **WASM stack switching** (`cont.new` / `resume` / `suspend`) | Wasmtime (x86_64 Linux), Wasmer; browsers when shipped | **Preferred** WASM lowering; first implemented against Wasmtime |
| 3 | **WASM + JSPI** | All major browsers, Node ≥ 26 | **Browser baseline today.** Each Moddil task is a separate `WebAssembly.promising` entry with its own suspended WASM stack; switching goes through a JS-side scheduler promise. Slower per switch, but works everywhere now. |
| 4 | CPS / Asyncify-style transform | WASI runtimes without stack switching | **Open question.** Either require a capable runtime, or build this later. |

The dependency on the stack-switching proposal stays isolated in the task/continuation runtime, so
changes to the proposal stay local.

Output formats: **core modules plus generated JS glue** for browsers and Node (which do not run
components natively); **components** (WIT interfaces, WASI 0.2/0.3) for Wasmtime and other component
hosts.

## Wasmtime as the short-term reference runtime

**Why:**
- It is the only mainstream runtime that implements **stack switching** today, so Moddil builds its
  *intended* concurrency lowering first rather than a workaround.
- It is the most complete runtime for the **Component Model and WASI 0.3 async** (on by default), with
  GC, exnref and memory64 at Tier 1. This makes it the natural home for WIT interop and capability
  mapping (R1).
- It is Rust, so it embeds in stage 0. The compiler can use it to run compile-time evaluation, tests,
  `moddil/runtime-sim`, and later the WASM bootstrap blob ([Bootstrapping after self-hosting](#bootstrapping-after-self-hosting)).
- It shares Cranelift with the native debug backend.

**Caveats:**
- **Stack switching in Wasmtime is Tier 3 and x86_64 Linux only.** Development on Windows needs WSL2
  or Linux CI for that path. Tier 3 means incomplete fuzzing and no C API yet, so expect bugs.
- Pin a recent Wasmtime version (the docs reference ~47; a local Windows install was 37.0.1).
- Wasmtime does not cover the browser. Examples 1 and 3, and the Hashiverse web client, wait for
  lowering 3 (JSPI) at M3/M4.

## WASM support: verified status (November 2026)

Sources: [WebAssembly proposals](https://github.com/WebAssembly/proposals),
[webassembly.org features data](https://github.com/WebAssembly/website/blob/main/features.json),
[Wasmtime proposal stability](https://docs.wasmtime.dev/stability-wasm-proposals.html), and local
probes on Node v24.16.0 (V8 13.6).

| Feature | CG phase | Chrome | Firefox | Safari | Node | Wasmtime |
|---|---|---|---|---|---|---|
| **Stack switching** | 3 | flag only | ✗ | ✗ | ✗ | Tier 3, x86_64 Linux only |
| **JSPI** | 5 (standard) | 137 | 153 | 27 | 26.0 (24.x: flag only) | n/a (JS API) |
| Exceptions (exnref) | standard | 137 | 131 | 18.4 | 24.15 | 47 |
| GC, typed function refs | standard | 119 | 120 | 18.2 | 22 | 47 |
| Tail calls | standard | 112 | 121 | 18.2 | 20 | 22 |
| Memory64 | standard | 133 | 134 | Technology Preview only | 24 | 30 |
| Threads | 4 | 74 | 79 | 14.1 | 16.4 | Tier 2 |
| SIMD / relaxed SIMD | standard | 91 / 114 | 89 / 145 | 16.4 / flag | 16.4 / 21 | yes |
| Component Model | **1** | ✗ (transpile with `jco`) | ✗ | ✗ | ✗ | Tier 1, on by default (incl. async / WASI 0.3) |
| Shared-everything threads | 1 | ✗ | ✗ | ✗ | ✗ | ✗ |

**Local probes (Node v24.16.0, V8 13.6):**
- `WebAssembly.Suspending` / `WebAssembly.promising` are undefined by default and present with
  `--experimental-wasm-jspi`.
- A hand-assembled module using a `cont` type and `cont.new` is rejected by default ("core stack
  switching not enabled"). With `--experimental-wasm-wasmfx`, V8 accepts the `cont` type but rejects
  `cont.new` ("Invalid opcode 0xe0"), so the prototype in this V8 version is incomplete.

**Consequences:**
- Do not assume stack switching in browsers or Node yet; JSPI is the browser baseline ([WASM concurrency lowering](#wasm-concurrency-lowering)).
- Memory64 is not available in Safari: `wasm32` is the browser default.
- Browser threads need cross-origin isolation (COOP/COEP headers).
- The Component Model is only Phase 1 and absent from browsers, so emit core modules + JS glue for the
  web.

## Risks and open questions

- Stack switching may change shape before standardisation, or arrive late in Safari.
- Should the MIR be designed so that MLIR can consume it later (GPU/tensor work)?
- C backend: worth it for bootstrapping and portability?
- Should WASI runtimes without stack switching be supported (lowering 4)?
- LLVM version pinning and upgrade cadence.
- Is Moddil-core (the stage 0 subset) small enough to build quickly, yet large enough to write a
  pleasant compiler in?

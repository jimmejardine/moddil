# Bootstrapping and Backends

> **Status:** proposal. Answers rationale §8 Q6. WASM support facts verified November 2026 (§8 below).

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

## 1. Stage 0 language: Rust

**Why Rust:**
- **Ecosystem fit.** Most of the infrastructure Moddil needs already exists as Rust crates:
  - `cranelift`: fast code generation for debug builds.
  - `wasmtime`: the WASM runtime (§9), embeddable in the compiler.
  - `wasm-encoder`, `wasmparser`, `wit-parser`, `wit-component`: WASM and Component Model (R1).
  - `naga`: shader translation between WGSL, SPIR-V, MSL and HLSL (§2.2).
  - `gimli`: DWARF debug info.
  - `salsa`: query-based incremental compilation, as used by rust-analyzer (§2.4, compiler-as-a-service §2.6).
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

## 2. Staged bootstrap

```
 stage 0  (Rust)     compiles Moddil-core ───────────────┐
                                                         ▼
 stage 1  (Moddil)   compiler source, compiled by stage 0
 stage 2             stage 1 compiling itself
 stage 3             stage 2 compiling itself    →  stage 2 == stage 3 (bit-identical)
```

- **Stage 0** implements **Moddil-core**: the subset needed to write a compiler. That is types, sum
  types, generics, ownership, modules and mods, basic effects and handlers. It is not the full language.
- Stage 0 includes an **interpreter for MIR** (the mid-level IR, §4). Compile-time evaluation (§3.4)
  needs one anyway, and it lets the language run tests before any backend exists.
- **Dogfood early.** Write the first standard mods (`moddil/collections`, `text`, `serde`, `test`)
  in Moddil while the compiler is still in Rust.
- **Fixed-point check**: stage 2 and stage 3 must be bit-identical. This runs in CI, and also checks
  reproducible builds (§2.5).
- **When to self-host:** not until the core semantics are stable, and **after the Hashiverse client
  milestone** has proved the language on real code. Self-hosting too early means rewriting the
  compiler every time the language changes.

## 3. Bootstrapping after self-hosting

**Zig's trick, applied to Moddil.** Zig commits a WASM build of its compiler (`zig1.wasm`). Building
Zig from source then needs only a C compiler and a small WASM-to-C translator. Moddil already has a
first-class WASM target (R1), so it can do the same: `bootstrap/moddil1.wasm` plus a minimal runner
(Wasmtime, or a `wasm2c` path).

**Trusting trust.** A checked-in binary raises Ken Thompson's "Reflections on Trusting Trust"
problem, which is exactly the supply-chain concern of §2.5. Mitigations:
- The WASM blob must be **reproducible**: anyone can rebuild it from source and get the same bytes.
- Keep the **Rust stage 0 buildable** so the compiler can be built by two independent paths and the
  results compared (**diverse double-compiling**, David A. Wheeler).
- Optional third path: a **C backend** (as Nim has and as Zig's CBE provides). It also helps port to
  unusual platforms.

## 4. Backends

```
 source ─► parse ─► canonical AST (§2.6, §2.7)
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
| MIR interpreter | Compile-time evaluation, fast tests | **Own** | Needed anyway for §3.4 |
| **Cranelift** | Debug/dev builds | Reuse | Fast compile; x86-64, AArch64, RISC-V 64, s390x |
| **LLVM** | Release builds; iOS/Android/embedded; mature optimisation | Reuse | Pin one version; use via the C API / `inkwell` |
| **WASM** | Browser, Node, WASI, components, bootstrap | **Own** emitter (`wasm-encoder`) + Binaryen `wasm-opt` | WASM is simple; needs stack switching, JSPI glue, exnref, GC and Component Model earlier than LLVM provides them; binary size matters |
| **GPU** | Kernels | Reuse | `naga` for WGSL/SPIR-V/MSL/HLSL; LLVM NVPTX/AMDGPU for CUDA/ROCm-class targets; MLIR later if tensor-level optimisation is needed |
| **C** | Third bootstrap path, odd platforms | Own (optional) | Decide later |
| Native backends of our own | Faster compiles, full control | **Not now** | Go and Zig did this at great cost; revisit only if Cranelift + LLVM block the build-time goals |

**Linkers:** `lld` or `mold` for native code; `wasm-ld` or a Moddil linker for WASM modules and
components. **Cross-compilation (§3.8):** LLVM plus bundled libc/sysroots, as Zig does.

## 5. Milestones

| Milestone | Contents |
|---|---|
| **M0** | Stage 0: parser, type, effect and ownership checker, MIR, interpreter. Small programs run. |
| **M1** | Cranelift native backend. Own WASM backend producing **WASI components for Wasmtime**, with stack switching (Linux x86_64). |
| **M2** | First standard mods written in Moddil; `moddil/runtime` and `moddil/runtime-sim`; LSP prototype. |
| **M3** | **Hashiverse CLI/native client** (example 4, phase A). LLVM release backend. **JSPI lowering for browsers and Node.** |
| **M4** | Hashiverse **browser** client. Self-hosted stage 1; fixed-point CI; checked-in WASM bootstrap; double-compiling check. |

## 6. WASM concurrency lowering

Moddil's concurrency semantics (structured tasks, the `suspend` effect, no function colouring, §2.1)
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

## 7. Wasmtime as the short-term reference runtime

**Why:**
- It is the only mainstream runtime that implements **stack switching** today, so Moddil builds its
  *intended* concurrency lowering first rather than a workaround.
- It is the most complete runtime for the **Component Model and WASI 0.3 async** (on by default), with
  GC, exnref and memory64 at Tier 1. This makes it the natural home for WIT interop and capability
  mapping (R1).
- It is Rust, so it embeds in stage 0. The compiler can use it to run compile-time evaluation, tests,
  `moddil/runtime-sim`, and later the WASM bootstrap blob (§3).
- It shares Cranelift with the native debug backend.

**Caveats:**
- **Stack switching in Wasmtime is Tier 3 and x86_64 Linux only.** Development on Windows needs WSL2
  or Linux CI for that path. Tier 3 means incomplete fuzzing and no C API yet, so expect bugs.
- Pin a recent Wasmtime version (the docs reference ~47; a local Windows install was 37.0.1).
- Wasmtime does not cover the browser. Examples 1 and 3, and the Hashiverse web client, wait for
  lowering 3 (JSPI) at M3/M4.

## 8. WASM support: verified status (November 2026)

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
- Do not assume stack switching in browsers or Node yet; JSPI is the browser baseline (§6).
- Memory64 is not available in Safari: `wasm32` is the browser default.
- Browser threads need cross-origin isolation (COOP/COEP headers).
- The Component Model is only Phase 1 and absent from browsers, so emit core modules + JS glue for the
  web.

## 9. Risks and open questions

- Stack switching may change shape before standardisation, or arrive late in Safari.
- Should the MIR be designed so that MLIR can consume it later (GPU/tensor work)?
- C backend: worth it for bootstrapping and portability?
- Should WASI runtimes without stack switching be supported (lowering 4)?
- LLVM version pinning and upgrade cadence.
- Is Moddil-core (the stage 0 subset) small enough to build quickly, yet large enough to write a
  pleasant compiler in?

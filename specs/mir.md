# MIR: the Mid-Level IR

Expands [`compiler.md`: Backends](compiler.md#backends).

## Summary

| Topic | Decision |
|---|---|
| **Pipeline** | Canonical AST → **typed HIR** → **MIR** (built → analysed → runtime) → per-target lowering → backends (decided) |
| **Form** | **Places and a control-flow graph**, as Rust's MIR: locals, places, statements and terminators in basic blocks; not SSA (decided) |
| **Ownership** | Moves, drops, linear consumption and second-class borrows are **checked on MIR**, and drops are elaborated there (decided) |
| **Suspension** | `suspend` is an **explicit terminator in every body**, so every suspension point is visible to checks, lints and lowerings (decided) |
| **Effects** | Handlers are hidden parameters (evidence passing); a pass turns statically known handlers into direct calls; `fail` is a second exit edge on calls (proposed) |
| **Checks** | Overflow, bounds, refinements, `?!` and contracts are explicit `Assert` statements with a trap class and a source span (proposed) |
| **Reference counting** | Explicit `dup`/`drop` after a Perceus-style pass, with reuse tokens (proposed); reuse analysis runs on MIR (decided) |
| **Generics** | Bodies stay polymorphic with witness tables; monomorphisation is a pass (proposed) |
| **Identity** | A canonical, deterministic text form and a hash per body; never a stable public format (decided) |

## Problem

The MIR has more consumers than any other part of the compiler, and each needs something different:

| Consumer | Needs |
|---|---|
| **MIR interpreter**: comptime ([`comptime.md`](comptime.md)), tests before any backend, the playground | Simple, directly executable bodies; target-independent |
| **Miri-style checker** ([`correctness.md`: A Miri-style checker](correctness.md#a-miri-style-checker)) | A precise memory model: places, provenance, validity |
| **Ownership and resource checking** | A control-flow graph: moves, drop flags, "consumed on every path" |
| **Backends**: Cranelift, LLVM, the own WASM emitter, GPU ([`compiler.md`: Backends](compiler.md#backends)) | Explicit drops, checks, layouts and calls; suspension made concrete per target |
| **Edition migration** ([`evolution.md`](evolution.md)) | A canonical form, so "the migrated program has the same MIR" is a meaningful check |
| **Build cache and live update** ([`build-times.md`](build-times.md)) | Per-item bodies with stable hashes; inlining of trivial functions even in debug builds |

The canonical AST is the source of truth for formatting, diffs and cache keys, but it has no
control-flow graph, no drops, no explicit checks and no resolved handlers, so none of these can
work on it directly.

## State of the art

| IR | Shape | Lesson for Moddil |
|---|---|---|
| **Rust MIR** | Places and a CFG; built, analysis and runtime phases; borrow checking on MIR since non-lexical lifetimes; Miri interprets it | The closest model: checking, interpretation and drop elaboration on one IR |
| **Swift SIL** | SSA with ownership annotations; raw and canonical SIL; ARC optimisation and witness tables | Shows witness tables and generic code without monomorphisation at scale; SSA-based ownership is powerful but a larger design |
| **Koka, Lean 4 IR** | Explicit `dup`/`drop`, reuse tokens (Perceus) | Reference-count operations made explicit in the IR, then optimised |
| **Zig ZIR / AIR** | Untyped IR from the AST, typed IR after semantic analysis; comptime runs during analysis | A small, fast pipeline; comptime on an intermediate form |
| **MLIR** | Extensible dialects | A possible later bridge for GPU and tensor work, not the core IR |
| **Cranelift CLIF, LLVM IR** | SSA, low level | The targets: both build their own SSA from what they are given |

## Decisions

### Pipeline

```
canonical AST ──► typed HIR ──► MIR (built) ──► MIR (analysed) ──► MIR (runtime) ──► lowered MIR ──► backend
                   names,        from HIR        ownership and      checks made        per target:
                   types,                        drops checked,     explicit, handlers suspension,
                   effects,                      drops elaborated   resolved, RC ops,  layouts, ABI
                   error sets                                       generics resolved
                                                      ▲                   ▲
                                                      └── interpreter ────┘ (comptime, tests, checker)
```

- **A typed HIR (decided)** sits between the canonical AST and MIR: desugared and name-resolved,
  carrying inferred types, effects and error sets. Salsa queries, the language server and
  `moddil explain` read it ([`agent-interface.md`](agent-interface.md)). The canonical AST stays the
  source of truth for formatting, semantic diffs and cache keys.
- **MIR is built per item** (function, closure, constant, comptime block) from the HIR, on demand,
  through the same query system.
- **Phases** (proposed, after Rust's): *built*, as lowered from HIR; *analysed*, after ownership
  checking and drop elaboration; *runtime*, after checks, handlers, reference counting and generics
  are made explicit. The interpreter runs analysed or runtime MIR; it never needs target lowering
  ([`compiler.md`: Two interpreters](compiler.md#two-interpreters)).
- **Lowered MIR** is per target: suspension, layouts and calling conventions are made concrete for
  native, JSPI, WASM stack switching or GPU.

### Core shape

- **Places and a CFG (decided), not SSA.** A body has typed **locals** (parameters, temporaries,
  user variables), **places** (`_3.field[_5]`), **statements** (assign, `Assert`, `dup`/`drop`,
  storage markers) and one **terminator** per basic block (goto, switch, call, return, `Suspend`,
  trap). Cranelift and LLVM build SSA themselves; optimisations that need SSA stay in the backends.
- **Types in MIR** are Moddil types with layouts available on demand. Units of measure and
  refinements are **erased to their base types** ([`units.md`](units.md), [`correctness.md`](correctness.md)); what they
  checked becomes explicit `Assert`s or disappears.
- Every statement and terminator carries a **source span**, for diagnostics, traces and debug info.

```
// illustration only: `fn make_code() -> Code with rand: Rand` from the tour
fn make_code(_ev_rand: &dyn Rand) -> Code {
    bb0: _1 = call collect_pick(_ev_rand, ALPHABET, 6) -> bb1
    bb1: _2 = call Code::try_new(move _1) -> [ok: bb2, err: bb3]
    bb2: _0 = move _2; return
    bb3: trap CannotFail("built from ALPHABET")            // ?! (class 3), span tour.md:105
}
```

### Effects

Proposed lowering of [`effects-and-capabilities.md`: Compilation](effects-and-capabilities.md#compilation):

- **Handlers are hidden parameters** (evidence passing): a function `with Net, Time` receives one
  evidence argument per effect trait, as a `&dyn Trait`. Context data is a hidden `&T` parameter.
- **Devirtualisation** is a runtime-phase pass: where the handler is known statically (production
  `main`), calls through evidence become direct calls and the parameter disappears.
- **`fail` is a second exit edge**: a call to a function that may fail has `[ok: bbA, err: bbB]`
  targets, compiled to the ordinary return channel. Error sets are tagged layouts; the error return
  trace is a debug-build side table ([`error-handling.md`: Summary of the model](error-handling.md#summary-of-the-model)).
- **`escape`** is a further exit edge through effect-polymorphic calls
  ([`closures.md`: Control flow out of a lambda](closures.md#control-flow-out-of-a-lambda)).
- **Ambient effects** (`log`, `span`, `metric`, `Contracts`) are calls to the ambient handler
  slot of the current task; they never return values into the program.

### Concurrency

- **`Suspend` is an explicit terminator (decided).** Every call that may suspend ends its block,
  with the resume block as its target. The cancellation check sits on that edge, the lint for locks
  held across suspension reads it, and any CPS lowering
  ([`compiler.md`: WASM concurrency lowering](compiler.md#wasm-concurrency-lowering), lowering 4)
  starts from it. On native targets it lowers to an ordinary call into the runtime.
- **Task scopes are structured regions** (proposed): `ScopeEnter`, `Spawn` and `ScopeExit`
  terminators, so ownership checking knows which borrows children hold and that they end at
  `ScopeExit` ([`concurrency.md`: Tasks and borrowing](concurrency.md#tasks-and-borrowing)). `select`
  is a terminator with one target per arm.
- **Generators** use the same one-shot suspension, with a yield value
  ([`stdlib.md`: Generators](stdlib.md#generators)).

### Ownership and resources

- **Checked on MIR (decided)**, as Rust's borrow checker has been since non-lexical lifetimes:
  - moves and copies are explicit operands; use after move is an error on the CFG;
  - **second-class borrows** need no lifetime inference: the check is that a borrow never flows into
    a place that outlives its scope, never crosses a `ScopeExit` it started outside, and never
    reaches a stored closure ([`memory.md`](memory.md));
  - **linear values must be consumed on every path**, including `fail` and `escape` edges
    ([`resources.md`: Resources: affine and linear](resources.md#resources-affine-and-linear));
  - drop bodies are checked to have an empty effect set ([`resources.md`: Destructors](resources.md#destructors)).
- **Drop elaboration** happens in the same pass: drop flags where a value is moved on only some
  paths; `defer` and `errdefer` become cleanup blocks on the normal and error/cancellation edges
  ([`resources.md`: Cleanup that can fail](resources.md#cleanup-that-can-fail)).
- **Unwind edges exist only inside isolated scopes** (decided November 2026). A trap elsewhere ends
  the process, so ordinary code has no landing pads; inside `scope(…, isolate: true)` a trap
  unwinds, running drops, poisoning held sync cells and calling `abandon` on linear values
  ([`lifecycle.md`: Traps](lifecycle.md#traps-the-process-by-default-isolated-scopes-on-request)).
  Unwinding is **table-based on every target**, with exnref on WASM, and a CI benchmark guards
  the cost of isolated scopes.

### Reference counting

- After ownership checking, a Perceus-style pass (proposed) inserts explicit **`dup` and `drop`**
  for `Rc` values, removes redundant pairs, and threads **reuse tokens** so a functional update
  becomes an in-place write when the old value is dead ([`memory.md`: Biased reference counting](memory.md#biased-reference-counting)).
- **Reuse analysis runs on MIR** (decided November 2026), not in the backends: it is done once for
  every runner, so its effect is the same everywhere and the MIR interpreter can test it.
- Biased counting (owner task versus shared count) stays behind those two operations, in the
  runtime.

### Explicit checks

- **`Assert` statements** (proposed) make every class-3 check visible: overflow, division, bounds,
  refinement validation, `?!` and contracts. Each carries its **trap class**, a message and the
  span.
- **Contracts** are asserts tagged `requires`, `requires always`, `ensures` or `invariant`; the
  `Contracts` policy strips or keeps them per scope, and `requires always` is never stripped
  ([`correctness.md`: Checking policy](correctness.md#checking-policy-the-contracts-handler)).
- Optimisation may remove an assert only when it is proven redundant (bounds-check elimination);
  `moddil verify` reads the same asserts as proof obligations.

### Memory model

- The interpreter gives places **provenance** and checks validity and the scoped-borrow aliasing
  rules for `unsafe` code ([`unsafe.md`: Aliasing: the scoped-borrow model](unsafe.md#aliasing-the-scoped-borrow-model)).
  This is what the Miri-style checker enforces, and what backends may assume.
- **Floating point is strict IEEE**; fast-math flags and float atomics appear only as operations
  tagged `nondet`, so replayable code provably contains none
  ([`determinism.md`: The `nondet` effect](determinism.md#the-nondet-effect)).

### Generics

- **Bodies stay polymorphic** (proposed): generic code calls through witness tables (size,
  alignment, copy, drop, trait methods), which debug builds run as is
  ([`build-times.md`: Language rules that decide build speed](build-times.md#language-rules-that-decide-build-speed)).
- **Monomorphisation is a pass** producing specialised bodies, run for release builds and for
  generics instantiated from optimised dependencies; trivial functions are inlined on MIR in debug
  builds too.
- Comptime runs on MIR before monomorphisation, so the interpreter handles polymorphic bodies.

### GPU kernels

- `#[kernel]` bodies are a **restricted MIR subset** (proposed): no `dyn`, no `Suspend`, no
  allocation, no unwind edges, statically known handlers only
  ([`accelerators.md`: Race freedom inside kernels](accelerators.md#race-freedom-inside-kernels)).
  The subset is checked on MIR and lowered to `naga` or LLVM NVPTX/AMDGPU.

### Identity and stability

- **A canonical text form** (decided November 2026): deterministic numbering and printing, for
  snapshot tests, `moddil explain --mir` and bug reports. It is **never** a stable public format:
  tools use the compiler's queries and the language server instead.
- **A hash per body** keys the build cache and live update
  ([`build-times.md`: Live update (debug builds)](build-times.md#live-update-debug-builds)): in debug builds
  functions are called through a patchable indirection.
- **Edition-neutral**: editions are a surface concern, so two editions that mean the same thing
  produce the same MIR, which is how edition migrations are checked ([`evolution.md`](evolution.md)).

## Fit check against the reference examples

| Example | What the MIR must do |
|---|---|
| [Tour](tour.md) | Evidence parameters for `Links` and `Rand`; `?!` as a trap edge; isolated scopes with unwind edges per connection |
| [HFT](examples/hft-platform.md) | Devirtualise every handler on the hot path; no `dup`/`drop` or allocation left where `#[no_alloc]` is checked |
| [Firmware](examples/sensor-firmware.md) | Static tasks: the `Suspend` terminators let the compiler size every task's stack at compile time |
| [Game](examples/multiplayer-game.md) | Kernels in the GPU subset; strict floats for bit-identical simulation on WASM and native |

## Open questions

- **The lowerings marked proposed** are confirmed as each is built.

## Prior art

Rust MIR, non-lexical lifetimes and Miri; Swift SIL and value witness tables; Koka and Lean 4
(Perceus, *Reference Counting with Frame-Limited Reuse*); Zig ZIR and AIR; MLIR; Cranelift CLIF;
Koka's evidence passing (Xie and Leijen, *Generalized Evidence Passing for Effect Handlers*).

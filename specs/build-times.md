# Build Times

> **Status:** decided direction, details open. Indexed in [`README.md`](README.md).

## Summary

Build speed is decided mostly by **language rules**, not by compiler cleverness. Go is fast because of
what it forbids; Rust and C++ are slow largely because of what they allow.

| Rule | One line |
|---|---|
| **Language rules for speed** | No cycles between mods, explicit interfaces at mod boundaries, generics checked once, pure comptime, local inference, canonical-AST cache keys ([Language rules that decide build speed](#language-rules-that-decide-build-speed)). |
| **Debug generics** | Witness tables for your own generics, with the same semantics as release; generics from optimised dependencies are specialised and cached even in debug builds. |
| **Live update** | Debug programs are updated while running; additive layout changes migrate automatically; production hot upgrades are a non-goal ([Live update (debug builds)](#live-update-debug-builds)). |
| **Four cache layers** | A warm daemon, a local content-addressed store, a team cache and trusted shared artefacts; a hit can never be stale ([Cached builds](#cached-builds)). |
| **Prebuilt dependencies, verified** | Only from sources you configure, signed, with a sample of hits rebuilt and compared ([Prebuilt dependencies](#prebuilt-dependencies)). |
| **Optimised dependencies in dev** | The default dev profile optimises dependencies and builds your workspace fast ([Profiles](#profiles)). |
| **Budgets** | < 100 ms keystroke to diagnostics; < 1 s from a body edit to a runnable test; regressions are bugs ([Budgets](#budgets)). |

The compiler itself is query-based and lazy, and skips linking in dev builds
([`compiler.md`: Compiler architecture](compiler.md#compiler-architecture)).

## Where build time goes today

| Cost | Where it bites |
|---|---|
| Front end re-checking | C++ re-parses headers and re-checks templates in every translation unit |
| Monomorphisation feeding LLVM | Rust: every generic instantiation becomes separate machine code; usually the dominant cost |
| Linking | Often the slowest single step for large binaries |
| Build scripts and proc macros | Run arbitrary code, are opaque to caching, and pull in heavy dependencies (`syn`) |
| Dependency count | Hundreds of packages, each compiled from scratch on a cold cache |
| Coarse invalidation | Editing one function re-checks or regenerates far more than that function |

**Evidence from Hashiverse** (Rust, about 573 packages in `Cargo.lock`):
- `Cargo.toml` sets `[profile.dev.package."*"] opt-level = 3`: every dependency is optimised even in
  dev builds, because unoptimised hash and crypto code is too slow to use.
- Tests run under a custom `profiling` profile (release optimisation plus debug info), not the default
  dev profile, because proof-of-work in debug builds is too slow.

Both are workarounds that every serious Rust project rediscovers. Moddil should make them unnecessary.

## Language rules that decide build speed

**Decided.** Each is very hard to add later.

1. **No cycles between mods** (as in Go). Mods form a DAG and compile in parallel. Cycles between
   modules inside a mod are allowed ([`modules.md`: Cycles](modules.md#cycles)).
2. **Explicit interfaces at mod boundaries.** Public signatures, effects and error sets are written
   out; nothing is inferred across mods (consistent with [`error-handling.md`: Error types](error-handling.md#error-types)).
   Each mod produces a small **interface artefact** (like OCaml's `.mli` or Go's export data).
   Dependents are checked against that artefact only, so **editing a function body never re-checks
   dependents**.
3. **No inlining across mods in debug builds.** Cross-mod optimisation (LTO, ThinLTO) happens only in
   release builds.
4. **Generics are checked once, at their definition** (decided, [`comptime.md`: Generics are bounded](comptime.md#generics-are-bounded)).
   No per-instantiation re-checking as with C++ templates.
5. **Generics compile differently in debug and release, with identical semantics** (decided,
   [`types.md`: Generics](types.md#generics)).
   - Debug: one copy of the code, receiving a hidden **witness table** of type information and
     operations (size, alignment, copy, drop, trait methods).
   - Release: a specialised copy per type (monomorphisation).
   - The semantics are identical because everything layout-dependent lives in the witness table.
     Swift's value witness tables show this works at scale.
   - **Generics from optimised dependencies are specialised even in debug builds.** When workspace
     code instantiates a generic from an optimised dependency (collections, iterators, hashing,
     crypto), that instantiation is specialised and optimised, and cached under the key (generic,
     type arguments, dependency version), so it is compiled once. The workspace's own generics keep
     witness tables, so editing them stays fast.
   - Trivial functions are inlined at the MIR level even in debug builds.
   - **Target:** debug code runs at most about 3× slower than release on the benchmark suite
     ([Budgets](#budgets)).
6. **Pure, sandboxed, cached comptime** (decided, [`comptime.md`](comptime.md)). No build scripts, no
   proc macros with I/O.
7. **Local type inference only.** Function signatures are explicit; there is no whole-program
   inference.
8. **Trait implementations are checked locally** (an orphan-style coherence rule,
   [`types.md`: Coherence: one impl per trait and type](types.md#coherence-one-impl-per-trait-and-type)), with no global
   search across all mods.
9. **Cache keys are hashes of the canonical AST**, not of text, so formatting and comment changes
   invalidate nothing ([LLMs as first-class programmers](agent-interface.md), [Diffs, code review and version control](semantic-vcs.md)).

## Live update (debug builds)

Change code while the program keeps running. Release builds may compile slowly; the debug loop must
not.

**Mechanism**
- In debug builds every function is called through an **indirection table**. Release builds have no
  table.
- A changed function is recompiled by Cranelift into new executable memory, and its table entry is
  swapped.
- Updates are applied in **batches at safe points**, the preemption safe points of
  [`runtime.md`: Tasks, threads and executors](runtime.md#tasks-threads-and-executors): all tasks pause briefly, the batch is swapped atomically,
  and tasks resume.
- **Frames already running finish on the old code** (Erlang keeps a "current" and an "old" version
  in the same way). Old code is freed once no task's stack refers to it; stackful tasks make that
  checkable.
- Cost: one indirect call per call and no inlining across functions, in debug builds only.
- There are no mutable globals (state is reached through capabilities), so no hidden global state
  is left stale.

**What can change while running**

| Change | Live? | How |
|---|---|---|
| Function body, same signature | Yes | Swap the table entry |
| Signature (parameters, effects, error set) | Yes | Recompile the callers incrementally and swap them together in one batch |
| New functions, types, modules | Yes | Added to the table |
| Layout of a type with live values | Partly | **Automatic:** added fields with defaults, removed fields, reordered fields, new enum variants. **Renames and type changes** need an explicit `migrate(old) -> new` hook (like Erlang's `code_change`); otherwise the affected task scope restarts and replays. |
| Effect handlers, scope structure | Restart | The affected scope restarts |

**Restart plus replay.** A restarted scope can rebuild its state by **replaying its recorded effects
through the new code** ([`determinism.md`: Record and replay](determinism.md#record-and-replay)). If the replay diverges, the tool
reports exactly where. Even layout changes then feel live in most development loops.

**Targets**
- Native: new code in executable memory; table entries swapped.
- WASM (browser and Wasmtime): calls go through a function table (`call_indirect`). New code is a new
  module instantiated against the same memory and table, then its entries are swapped in.

**Scope:** debug builds only. **Production hot upgrades are a non-goal**
([`requirements.md`: Non-goals (draft)](requirements.md#non-goals-draft)): production uses rolling
deploys with compiler-checked compatibility between versions
([Distributed and multi-tier programs](distribution.md)).

## Cached builds

**Four layers, nearest first**

| Layer | What it holds | Prior art |
|---|---|---|
| 1. Compiler daemon | The warm query graph, shared by builds, the language server, tests and live update | rust-analyzer, Bazel server, Gradle daemon |
| 2. Local content-addressed store | Results on disk; survives restarts; shared across branches and worktrees, so switching branches does not rebuild; garbage-collected by size and age | ccache, sccache, Nix store |
| 3. Team / CI remote cache | Results uploaded by CI, downloaded by developers | Bazel remote cache, Turborepo, Nx |
| 4. Trusted shared artefacts | Signed, prebuilt dependency artefacts per mod version, target and profile, only from sources the user configures (there is no central registry, [`packages-and-supply-chain.md`: The source is the release](packages-and-supply-chain.md#the-source-is-the-release)); off by default ([Prebuilt dependencies](#prebuilt-dependencies)) | Nix binary cache |

**Keys and granularity**
- Key = hash of (the item's canonical AST, the interface artefacts it depends on, toolchain version,
  target, profile, flags).
- Granularity: per declaration for checking; per function for debug code generation; per mod for
  release objects, with ThinLTO summaries cached separately.
- **A hit cannot be stale.** Builds are hermetic by language design: comptime is pure, there are no
  build scripts and no environment access. Bazel achieves this only when every rule is written
  carefully; in Moddil it is not optional.

**Trust**
- A shared cache is a supply-chain risk (cache poisoning).
- Remote and shared-artefact entries are **signed**; only trusted CI writes, developers read.
- Builds are reproducible, so independent rebuilders and random sampling can verify hits
  ([`determinism.md`: Reproducible builds](determinism.md#reproducible-builds), [Package management and supply-chain security](packages-and-supply-chain.md)).

### Prebuilt dependencies

**Decided: prebuilt artefacts are opt-in and verified.**

- They come **only from sources the user configures**: the team's CI, or a foundation-run cache.
  With nothing configured, only the local store and the team cache are used.
- Every artefact is **signed by its builder**.
- A **configurable fraction of hits is rebuilt locally** and compared byte for byte; reproducible
  builds make that possible. A mismatch is a security alert, reported like an advisory by
  `moddil audit`.
- A project can require mods holding `unsafe` or `ffi` to be **built from source**.

**Release builds** may be slow. Their optimised artefacts are cached per mod, and profile-guided
optimisation profiles are just more inputs to the key. Dependencies normally arrive optimised from the
cache and are never rebuilt locally ([Profiles](#profiles)).

**Visibility:** `moddil build --timings` shows hit and miss rates per layer and, for each miss, which
input changed.

## Profiles

**Decided default dev profile:**
- **dependencies optimised** and fetched from the cache;
- **your workspace** built with fast, unoptimised code generation and full debug info.

Hashiverse needed a manual override to get this; in Moddil it is the default. Per-mod overrides remain
available (for example, optimising one hot mod of your own in dev builds).

Other profiles: `release` (full optimisation, LTO), `profiling` (release plus debug info), `fuzz`.
PGO from recorded runs and the debug-build speed target are in [`performance.md`: Optimisation control](performance.md#optimisation-control).

## Budgets

**Decided**, tracked in CI as benchmarks. A regression is a bug.

| Operation | Target |
|---|---|
| Keystroke to diagnostics (language server) | < 100 ms |
| Edit one function body → runnable test | < 1 s, independent of project size |
| Interface change | Proportional to direct dependents, not project size |
| Clean debug build, warm dependency cache | Comparable to Go (exact figure to be set) |
| Release build | Slower is acceptable; always cacheable |
| Debug-build run time | At most about 3× slower than release on the benchmark suite |

Supporting tools:
- a public performance dashboard, as perf.rust-lang.org does for Rust;
- `moddil build --timings`, showing time per mod and per query, so authors can see which generics or
  comptime code are expensive.

## Agents

Agents rebuild dozens of times per task ([LLMs as first-class programmers](agent-interface.md)), so the following matter even more for them
than for humans:
- check-only mode for fast feedback on types, effects and ownership;
- test impact analysis;
- the shared cache;
- structured diagnostics streamed as soon as they are available.

## Fit with the reference examples

| Example | What it needs |
|---|---|
| [Game](examples/01-multiplayer-game.md) | Live update of game rules and rendering in the browser, keeping game state |
| [HFT](examples/02-hft-platform.md) | A fast dev loop; long release builds are acceptable if cached |
| [Web store](examples/03-web-store.md) | Large-team iteration; sub-second rebuilds; patching an API handler without dropping sessions; a shared team cache |
| [Hashiverse](examples/04-hashiverse.md) | Fast hash and crypto code in dev builds without manual overrides; tests in the default profile; live-patching one node of a running simulation |

## Open questions

1. The exact clean-build figure behind "comparable to Go".
2. The default sampling rate for rebuilding prebuilt-artefact hits.
3. Do automatic layout migrations reach values nested in containers (`Vec<Old>`, `Map<K, Old>`)?

## Background

The problem and the state of the art that led to this design.

**Problem.** Slow builds damage the edit-compile-test loop for humans, and are worse for LLM agents
that iterate dozens of times per task.

**State of the art.**
- **Go**: designed for fast compilation (no circular imports, simple type system, export data per
  package). It is the benchmark.
- **Zig**: incremental compilation and its own fast backends; it aims for sub-second rebuilds.
- **Jai / D / Delphi**: famously fast compilers.
- **Rust**: Cranelift backend for debug builds, incremental compilation, parallel front end (in progress).
- **Bazel/Buck2**: content-addressed, hermetic, remote-cached builds at monorepo scale.

**What still hurts.**
- **C++**: textual `#include`, template instantiation in every translation unit; modules are arriving slowly.
- **Rust**: monomorphisation and LLVM optimisation are the dominant costs, and proc macros are opaque
  to caching.
- Macro and compile-time evaluation systems that can run arbitrary code break incremental caching.
- Linking is often the slowest single step (mold and lld help).

## Prior art

- Go: package DAG, export data, fast compiler.
- Zig: incremental compilation, lazy analysis, in-place binary patching.
- Rust: Cranelift backend, incremental compilation, parallel front end; rust-analyzer and Salsa.
- Swift: value witness tables, specialisation.
- OCaml: separate interface files.
- Jai, D, Delphi: fast compilers.
- Bazel, Buck2, sccache, ccache, Turborepo, Nx, Nix binary cache; Gradle daemon.
- Live update: Erlang/OTP hot code loading; Smalltalk and Lisp images; .NET Hot Reload; JVM HotSwap;
  Dart/Flutter stateful hot reload; Live++; Rust `subsecond` (Dioxus).
- `lld`, `mold`.
- perf.rust-lang.org.

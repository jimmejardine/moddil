# Build Times

> **Status:** proposal. Expands [`rationale.md`](rationale.md) §2.14. Nothing here is decided;
> recommendations are marked **Recommended**. Numbers in §6 are initial targets.

## 1. Summary

Build speed is decided mostly by **language rules**, not by compiler cleverness. Go is fast because of
what it forbids; Rust and C++ are slow largely because of what they allow. Moddil should:
- lock in the language rules that make fast builds possible (§3), because they cannot be retrofitted;
- build a query-based, lazy, cached compiler that skips linking in dev builds (§4);
- optimise dependencies by default and keep your own code fast to compile (§5);
- publish build-time budgets and treat regressions as bugs (§6).

## 2. Where build time goes today

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

## 3. Language rules that decide build speed

**Recommended** to lock early; each is very hard to add later.

1. **No cycles between mods** (as in Go). Mods form a DAG and compile in parallel. *Open:* are cycles
   allowed between modules inside a single mod?
2. **Explicit interfaces at mod boundaries.** Public signatures, effects and error sets are written
   out; nothing is inferred across mods (consistent with [`error-handling.md`](error-handling.md) §5).
   Each mod produces a small **interface artefact** (like OCaml's `.mli` or Go's export data).
   Dependents are checked against that artefact only, so **editing a function body never re-checks
   dependents**.
3. **No inlining across mods in debug builds.** Cross-mod optimisation (LTO, ThinLTO) happens only in
   release builds.
4. **Generics are checked once, at their definition** (decided, [`comptime.md`](comptime.md) §5).
   No per-instantiation re-checking as with C++ templates.
5. **Generics compile differently in debug and release, with identical semantics.**
   - Debug: one copy of the code, receiving a hidden **witness table** of type information and
     operations (size, alignment, copy, drop, trait methods).
   - Release: a specialised copy per type (monomorphisation).
   - This answers rationale §2.14's open question: the semantics are identical as long as everything
     layout-dependent lives in the witness table. Swift's value witness tables show this works at
     scale.
   - Risk: generic-heavy code (iterators, collections) can be much slower in debug builds.
     Mitigations: dependencies are optimised by default (§5), and trivial functions are inlined at the
     MIR level even in debug builds.
6. **Pure, sandboxed, cached comptime** (decided, [`comptime.md`](comptime.md)). No build scripts, no
   proc macros with I/O.
7. **Local type inference only.** Function signatures are explicit; there is no whole-program
   inference.
8. **Trait implementations are checked locally** (an orphan-style coherence rule), with no global
   search across all mods.
9. **Cache keys are hashes of the canonical AST**, not of text, so formatting and comment changes
   invalidate nothing (rationale §2.18, §2.19).

## 4. Compiler architecture

- **Query-based and demand-driven** (Salsa, as in rust-analyzer), with dependencies tracked down to
  individual **declarations** (Zig's incremental compilation goes to the same granularity). Only
  queries whose inputs actually changed are recomputed.
- **Lazy compilation:** only code reachable from a root (a binary, a test) is analysed and compiled,
  as Zig does. Unused parts of large mods cost nothing.
- **Parallel at every stage:** mods in DAG order; functions within a mod during code generation.
- **Content-addressed cache** keyed by (normalised input, toolchain, target, profile):
  - shared between a developer's machine, the team, CI and a remote cache (as with Bazel or sccache);
  - safe to share because builds are reproducible ([`determinism.md`](determinism.md) §6);
  - the registry may serve **prebuilt, signed artefacts** per mod version, target and profile,
    checked by independent rebuilders (like Nix's binary cache; rationale §2.16).
- **No link step in dev builds:**
  - functions are reached through an indirection table, or code runs under an in-process Cranelift
    JIT;
  - this also gives **hot reload** of individual function bodies;
  - alternatively, Zig-style in-place patching of the output binary.
  - Release builds link normally with `lld` or `mold`, with split debug info.
- **Check-only mode:** type, effect and ownership checking without code generation, powering the
  language server and agents (rationale §2.18).
- **Test impact analysis:** the query graph knows which tests depend on a change, so only those run.

## 5. Profiles

**Recommended default dev profile:**
- **dependencies optimised** and fetched from the cache;
- **your workspace** built with fast, unoptimised code generation and full debug info.

Hashiverse needed a manual override to get this; in Moddil it is the default. Per-mod overrides remain
available (for example, optimising one hot mod of your own in dev builds).

Other profiles: `release` (full optimisation, LTO), `profiling` (release plus debug info), `fuzz`.

## 6. Budgets

Initial targets, tracked in CI as benchmarks. A regression is a bug.

| Operation | Target |
|---|---|
| Keystroke to diagnostics (language server) | < 100 ms |
| Edit one function body → runnable test | < 1 s, independent of project size |
| Interface change | Proportional to direct dependents, not project size |
| Clean debug build, warm dependency cache | Comparable to Go (exact figure to be set) |
| Release build | Slower is acceptable; always cacheable |

Supporting tools:
- a public performance dashboard, as perf.rust-lang.org does for Rust;
- `moddil build --timings`, showing time per mod and per query, so authors can see which generics or
  comptime code are expensive.

## 7. Agents

Agents rebuild dozens of times per task (rationale §2.18), so the following matter even more for them
than for humans:
- check-only mode for fast feedback on types, effects and ownership;
- test impact analysis;
- the shared cache;
- structured diagnostics streamed as soon as they are available.

## 8. Fit with the reference examples

| Example | What it needs |
|---|---|
| [Game](examples/01-multiplayer-game.md) | Hot reload of game rules and rendering without restarting the client |
| [HFT](examples/02-hft-platform.md) | A fast dev loop; long release builds are acceptable if cached |
| [Web store](examples/03-web-store.md) | Large-team iteration; sub-second rebuilds after a one-line change |
| [Hashiverse](examples/04-hashiverse.md) | Fast hash and crypto code in dev builds without manual overrides; tests in the default profile |

## 9. Decisions to lock early (recommendations)

1. No cycles between mods (§3.1).
2. Explicit interfaces and interface artefacts at mod boundaries (§3.2).
3. No cross-mod inlining in debug builds (§3.3).
4. Witness tables in debug, specialisation in release, with identical semantics (§3.5).
5. Canonical-AST hashing for cache keys (§3.9).
6. Optimised dependencies as the default dev profile (§5).
7. Published budgets, enforced in CI (§6).

## 10. Open questions

1. Are cycles allowed between modules inside one mod?
2. How slow can generic code be in debug builds before it is unacceptable? Should hot generics be
   specialised even in debug builds?
3. How far should hot reload go: function bodies only, or signature and data-layout changes too?
4. How much should the registry serve prebuilt binaries, and what are the trust implications?

## Prior art

- Go: package DAG, export data, fast compiler.
- Zig: incremental compilation, lazy analysis, in-place binary patching.
- Rust: Cranelift backend, incremental compilation, parallel front end; rust-analyzer and Salsa.
- Swift: value witness tables, specialisation.
- OCaml: separate interface files.
- Jai, D, Delphi: fast compilers.
- Bazel, Buck2, sccache, Nix binary cache.
- `lld`, `mold`.
- perf.rust-lang.org.

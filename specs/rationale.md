# Moddil Rationale: Why a New Language?

> **Status:** living index for **Moddil**. This is not a specification and makes no syntax commitments.
> It states the thesis, lists the problem areas with a short summary of each, and points to the
> subdocument that holds the detail. The subdocuments are authoritative.

## Index

Status legend:

- 🟡 **started**: brainstorm or proposal; nothing is decided yet.
- 🟠 **questions**: the direction is decided; open questions remain.
- 🟢 **complete**: decided, with no open questions.

**Problem areas** (each links to its summary below)

| §                                                         | Area                                   | Status       | Detail                                                           |
| --------------------------------------------------------- | -------------------------------------- | ------------ | ---------------------------------------------------------------- |
| [2.1](#21-effects-and-capabilities-as-a-unifying-idea)    | Effects and capabilities               | 🟡 started   | [`effects-and-capabilities.md`](effects-and-capabilities.md)     |
| [2.2](#22-memory-and-resource-management)                 | Memory and resource management         | 🟠 questions | [`memory-and-resources.md`](memory-and-resources.md)             |
| [2.3](#23-error-handling)                                 | Error handling                         | 🟠 questions | [`error-handling.md`](error-handling.md)                         |
| [2.4](#24-null-and-invalid-states)                        | Null and invalid states                | 🟠 questions | [`null-safety.md`](null-safety.md)                               |
| [2.5](#25-numerics-and-data)                              | Numerics and data                      | 🟡 started   | [`numerics.md`](numerics.md), [`units.md`](units.md)             |
| [2.6](#26-metaprogramming)                                | Metaprogramming                        | 🟠 questions | [`comptime.md`](comptime.md)                                     |
| [2.7](#27-correctness-tooling-contracts-and-verification) | Contracts and verification             | 🟡 started   | [`correctness.md`](correctness.md)                               |
| [2.8](#28-concurrency-across-cpus)                        | Concurrency across CPUs                | 🟠 questions | [`concurrency.md`](concurrency.md), [`closures.md`](closures.md) |
| [2.9](#29-concurrency-across-gpus-and-accelerators)       | GPUs and accelerators                  | 🟠 questions | [`accelerators.md`](accelerators.md)                             |
| [2.10](#210-distributed-and-multi-tier-programs)          | Distributed and multi-tier programs    | 🟠 questions | [`distribution.md`](distribution.md)                             |
| [2.11](#211-determinism-and-reproducibility)              | Determinism and reproducibility        | 🟡 started   | [`determinism.md`](determinism.md)                               |
| [2.12](#212-observability-and-debuggability)              | Observability and debuggability        | 🟠 questions | [`observability.md`](observability.md)                           |
| [2.13](#213-interoperability-with-c-and-rust)             | Interoperability with C and Rust       | 🟠 questions | [`interop.md`](interop.md)                                       |
| [2.14](#214-build-times)                                  | Build times                            | 🟡 started   | [`build-times.md`](build-times.md)                               |
| [2.15](#215-cross-compilation-and-deployment)             | Cross-compilation and deployment       | 🟠 questions | [`targets.md`](targets.md), [`freestanding.md`](freestanding.md) |
| [2.16](#216-package-management-and-supply-chain-security) | Packages and supply-chain security     | 🟡 started   | [`packages-and-supply-chain.md`](packages-and-supply-chain.md)   |
| [2.17](#217-tooling-uniformity)                           | Tooling uniformity                     | 🟠 questions | [`toolchain.md`](toolchain.md), [`docs-and-tests.md`](docs-and-tests.md) |
| [2.18](#218-llms-as-first-class-programmers)              | LLMs as first-class programmers        | 🟠 questions | [`agent-interface.md`](agent-interface.md)                       |
| [2.19](#219-diffs-code-review-and-version-control)        | Diffs, code review and version control | 🟡 started   | [`semantic-vcs.md`](semantic-vcs.md)                             |
| [2.20](#220-language-evolution)                           | Language evolution                     | 🟡 started   | [`evolution.md`](evolution.md)                                   |

**Other documents**

| Document                                 | Covers                                                                 | Status       | Summary here                              |
| ---------------------------------------- | ---------------------------------------------------------------------- | ------------ | ----------------------------------------- |
| [`requirements.md`](requirements.md)     | Firm requirements R1–R3 in full, and non-goals                         | 🟠 questions | [§5](#5-requirements-and-non-goals)       |
| [`mods.md`](mods.md)                     | Mods, the core/mod boundary, standard-mod catalogue (R2)               | 🟡 started   | [§5.1](#51-firm-requirements-decided)     |
| [`bootstrap.md`](bootstrap.md)           | Bootstrap compiler, self-hosting, backends, WASM engine support (R1)   | 🟡 started   | [§5.1](#51-firm-requirements-decided)     |
| [`closures.md`](closures.md)             | Lambdas that borrow, fail and suspend; escaping closures               | 🟠 questions | [§2.8](#28-concurrency-across-cpus)       |
| [`units.md`](units.md)                   | Units of measure: `100<cm>`, open dimensions, declared units           | 🟡 started   | [§2.5](#25-numerics-and-data)             |
| [`freestanding.md`](freestanding.md)     | Kernels and firmware: profiles, interrupts, MMIO, emulator testing     | 🟡 started   | [§2.15](#215-cross-compilation-and-deployment) |
| [`prior-art.md`](prior-art.md)           | Prior-art matrix across languages, and the reference list              | reference    | [§6](#6-prior-art-matrix)                 |
| [`examples/`](examples/README.md)        | Four reference systems used to test every decision                     | 🟡 started   | [§7](#7-open-questions-and-next-steps-for-moddil) |

---

## 0. Preamble

Every serious language exists because it solved something better than what came before it:

- **C** made a portable abstraction over the machine that became the world's ABI.
- **C++** showed that zero-cost abstractions and RAII can scale to huge systems.
- **Rust** proved that memory safety and data-race freedom can be checked at compile time without a GC.
- **Go** showed that simplicity, fast builds, and one blessed toolchain are features in their own right.
- **Erlang/Elixir** made failure a normal, designed-for event, using isolated processes and supervisors.
- **Haskell/OCaml/Koka** demonstrated what types and effects can say about a program.
- **Zig** treated cross-compilation, explicit allocation, and C interop as first-class concerns.
- **Swift** shipped real-world automatic reference counting and serious C++ interop.
- **Unison** asked what happens if code is identified by its content rather than by its text.

Moddil should not be a remix of syntax. It is only justified if the *combination* of problems below
cannot be solved well by evolving an existing language, either because the solutions need guarantees
that existing languages cannot add without breaking their ecosystems, or because the target user has
changed.

**What this document is not:** a grammar, a feature list, or a promise. Where a snippet appears it only
illustrates a point.

---

## 1. Thesis

Three shifts are happening together, and no existing language was designed for all of them:

1. **The author is changing.** More and more code is written by LLM agents. Humans type less, but they
   must *understand, review and approve* more. The bottleneck moves from writing code to verifying it.
   Languages built to save keystrokes (implicit conversions, clever macros, overloads, magic)
   now cost the human who has to review the result.

2. **The machine is changing.** "A computer" now means many CPU cores, one or more GPUs, and often other
   accelerators (NPUs, TPUs, DSPs), with non-uniform memory between them. Most languages still treat
   anything beyond a single CPU thread as a library concern.

3. **The program is a fleet.** A deployed system is a group of cooperating processes: browser
   client, API servers, message brokers, background workers, databases. Languages describe one
   process at a time; the system as a whole is held together by hand-maintained contracts,
   YAML and hope.

All three shifts point the same way: **semantics that are explicit and checked by machines.** That is
Moddil's reason to exist. A language where effects, resources, capabilities, concurrency and memory placement are visible in the types and
checked by the compiler is easier for an LLM to write correctly, easier for a human to review, easier to
schedule onto heterogeneous hardware, easier to split safely across machines, and safer to pull dependencies into.

---

## 2. Problem Areas

The areas are ordered from the language core outward: semantics (effects, memory, errors, types, numerics, metaprogramming, correctness), then execution (CPU, GPU, distributed, determinism, observability), then the surrounding system (interop, builds, deployment, packages, tooling), and finally the human and agent workflow (LLMs, review, evolution). The order reflects dependencies, not priority: every area matters equally. Effects come first because most other areas build on them.

Each area below gives only:

- its **status** and a link to the **subdocument**, which holds the problem statement, prior art,
  decisions and directions in full;
- a short **summary** of the problem and of what is decided or proposed;
- the **open questions**, copied from the subdocument. The subdocument's list is the master copy.

---

### 2.1 Effects and capabilities as a unifying idea

🟡 **started** · Detail: [`effects-and-capabilities.md`](effects-and-capabilities.md)

Signatures say what a function takes and returns, but not what it *does*: I/O, allocation,
suspension, failure, nondeterminism, GPU work, unsafe code. One mechanism, effects plus capability
values, underlies most other areas:

- suspension without function colouring (§2.8) and GPU-safe kernel subsets (§2.9);
- resource safety with linear types (§2.2) and visible failure (§2.3);
- capability-scoped dependencies (§2.16) and reviewable signatures (§2.18, §2.19);
- handlers that swap real I/O for fakes in tests (§2.11) and provide implicit context.

Direction: effects inferred inside a module and explicit at boundaries; capabilities as values;
handlers compiled to zero-cost code where possible.

**Open questions** (same list as [`effects-and-capabilities.md`](effects-and-capabilities.md) §5)

- Can effect systems be made ergonomic for mainstream programmers? No mainstream language has done
  it yet; this is both the biggest risk and the biggest opportunity.
- Effect system design: row-polymorphic (Koka) vs capability-passing vs hybrid.
  [`determinism.md`](determinism.md) §4 recommends a hybrid: scoped handlers plus second-class
  capability values.

---

### 2.2 Memory and resource management

🟠 **questions** · Detail: [`memory-and-resources.md`](memory-and-resources.md)

Every existing approach trades something away: manual management is unsafe, tracing GC costs pauses
and memory, RAII is unenforced, reference counting is slow when atomic, and Rust's lifetimes leak
into APIs. Most languages also manage only memory, not files, sockets, locks or transactions.

Decided:

- **Unique ownership and RAII by default** for every resource, enforced by the compiler. Drop is
  deterministic, never suspends and never fails.
- **Second-class borrows with no lifetime annotations**: parameter modes `let`, `mut` and `sink`,
  plus *view types* for structs and containers that hold borrows.
- **`.to_rc()` is the only way to share a pointer.** Contents are immutable, and biased reference
  counting keeps it nearly free on the owning thread.
- **Affine by default, linear on request**: transactions and fallible `close` must be consumed.
- **Zero-copy I/O** by moving owned buffers and parsing through views.

**Open questions** (same list as [`memory-and-resources.md`](memory-and-resources.md) §10)

- Exact syntax for parameter modes, views and `from`.
- Allocators: how does the allocator capability (Zig-style) interact with `.to_rc()`
  and arenas? Does an `Rc` remember its allocator? (Proposed answer in
  [`freestanding.md`](freestanding.md) §4.)
- Should `Weak` be core or a mod? (It needs header support, so probably core.)
- Linear values on error paths: what is the least noisy explicit handling?
- Can the borrow check give diagnostics good enough for LLM agents (§2.18) — e.g.
  machine-applicable "insert `.to_rc()` here" fixes?
- Is there still a place for an optional per-task/per-actor collector, or does this model make it
  unnecessary?

---

### 2.3 Error handling

🟠 **questions** · Detail: [`error-handling.md`](error-handling.md)

Exceptions hide control flow, error codes get ignored, `Result` types are noisy, and panics are
coarse. Composing error types across libraries is painful.

Decided:

- Rust-style `?` and try blocks; errors do not unwind.
- **R3** (§5.1): no unhandled errors, checked at every entry point.
- A clear split between recoverable errors and bugs (traps).
- **Cleanup cannot fail silently or fatally**: drop has an empty effect set; cleanup that can fail
  is a linear type with an explicit `close`/`commit`; a second error on an error path is
  *suppressed* onto the primary error, never lost.

Still a direction: errors as an effect (`fail(E)`) with inferred error sets.

**Open questions** (same list as [`error-handling.md`](error-handling.md) §12)

1. Are explicit discards of errors allowed, and do they need a reason (§4.4 there)?
2. Do failed contract assertions trap or raise an error (§2 there)? (Recommendation: trap; see
   [`correctness.md`](correctness.md) §5.)
3. Task scopes: first error and cancel siblings, or collect all errors (§4.5 there)?
4. Can a trap terminate only its task rather than the process, and how are its resources freed (§8 there)?
   If so, linear types would also need an infallible, non-suspending `abandon` for that path (e.g. a
   transaction drops its connection so the database rolls back). Cancellation does not need it
   ([`concurrency.md`](concurrency.md) §9).
5. Should a resumable recovery effect ("retry"/"default") exist as a separate effect?
6. How is context attached to structured errors (§5 there)?
7. Is `Option` part of the same machinery (`?` on `Option`, as in Rust)?
8. Should errors from the `remote` effect (§2.10) have a standard shape (timeout, unreachable,
   incompatible version), and how are errors serialised across tiers?
9. Checked-exception fatigue (Java) vs inference: how much of an error set should be inferred?

---

### 2.4 Null and invalid states

🟠 **questions** · Detail: [`null-safety.md`](null-safety.md)

Null references are the best-known case of a wider problem: types that admit values the domain does
not (unchosen zero values, unchecked ranges, mixed-up IDs, objects used in the wrong state).

Decided:

- **No null in safe code.** Absence is `Option<T>`, an ordinary sum type in a mod, with Swift-style
  sugar: `T?`, `if let`, `let … else`, `a?.b`, `x ?? d`. There is no implicit unwrap.
- **Absence is data; failure is an effect**, so `Option` is the only "maybe" type in everyday code.
- **Guaranteed niches**: `Option<&T>`, `Option<Rc<T>>` and range-typed integers cost no extra space.
- **Definite initialisation**: no zero values and no uninitialised reads.
- **Beyond null**: cheap newtypes, pattern/range types, refinement types, and typestate through
  linear types.

**Open questions** (same list as [`null-safety.md`](null-safety.md) §4)

- **What does postfix `?` mean?** In Rust it propagates `None`/`Err`; in Swift it is chaining.
  - If `fail` is an effect, failure may propagate implicitly. The `call()?` in the snippets of
    [`memory-and-resources.md`](memory-and-resources.md) would then disappear.
  - Separately: should `opt?` inside a function returning `T?` propagate `None`?
  - Resolve together with §2.3.
- How much refinement checking is static vs dynamic, and what does it cost in compile time (§2.14)?
- Layout of nested options (`T??`): how far should niche packing go?

---

### 2.5 Numerics and data

🟡 **started** · Detail: [`numerics.md`](numerics.md), [`units.md`](units.md) (units of measure)

Silent integer overflow, undefined behaviour, floating-point surprises and no units.

- Decided ([`error-handling.md`](error-handling.md) §2): integer arithmetic wraps with no undefined
  behaviour; floats follow IEEE 754; integer division by zero and `MIN / -1` trap; checked and
  saturating operators live in a mod.
- Proposed: F#-style units of measure (`100<cm>`, `f64<m/s>`) with an open set of dimensions
  ([`units.md`](units.md)); arbitrary-width integers and packed bitfields
  ([`freestanding.md`](freestanding.md) §6).
- Directions: SIMD vectors and tensors as first-class types (§2.9); arbitrary-precision integers in
  the standard library.

**Open questions** (same list as [`numerics.md`](numerics.md) §4)

None recorded yet. Units of measure have their own list in [`units.md`](units.md) §10.

**Open questions** (same list as [`units.md`](units.md) §10)

1. Confirm the lexing rule: no space between a literal and its `<unit>`.
2. Glob imports of unit packs (§7 there).
3. Angles: are radians and degrees units of a dimensionless-but-distinct "angle" kind?
4. Rational exponents (`m^(1/2)`), needed for some formulas: allow, or only integers?
5. The common-unit rule for unannotated mixed expressions: finer unit, or left operand?
6. Integer representations with fractional scales (for example `i64<mm>` from `cm`): exact-only
   conversion, or a fixed-point scale in the type?

---

### 2.6 Metaprogramming

🟠 **questions** · Detail: [`comptime.md`](comptime.md)

Macros are powerful but break tooling, readability, caching and LLM understanding, and proc macros
run arbitrary code at build time.

Decided:

- **No macro language.** Metaprogramming is ordinary Moddil code run by the compiler.
- **Comptime is pure**: deterministic, fuel-limited, sandboxed and cached by input hash.
- **Generics stay trait-bounded** and are checked at definition.
- **Static reflection** (`reflect(T)`) and **typed derives** built through a construction API,
  never token streams.
- **Tagged string literals for DSLs** (`fmt"…"`, `re"…"`, `sql"…"`), checked at compile time.
- Every expansion is visible through `explain` and in semantic diffs.

**Open questions** (same list as [`comptime.md`](comptime.md) §13)

- **Where does comptime run?** Options:
  - an interpreter inside the compiler (Zig; fast start, slow for heavy work);
  - compiling comptime code to WASM and running it in Wasmtime (fast for heavy work; type values
    must cross the boundary);
  - a hybrid of the two.
- **Allocation:** what allocator does comptime use, and how does comptime data become static data in
  the binary?
- **Budget:** what is the default fuel and the per-mod compile-time budget (§2.14)?
- **Ordering:** can one derive depend on another's output (e.g. `Hash` relying on `Eq`), and how are
  cycles detected?
- **Reflection and privacy:** exactly which private details may a derive in another mod see?
- **IDE responsiveness:** comptime must be incremental enough to run on every keystroke for agents
  (§2.18).
- **Construction API:** how expressive is it, and does it stay stable across editions (§2.20)?

---

### 2.7 Correctness tooling: contracts and verification

🟡 **started** · Detail: [`correctness.md`](correctness.md)

Tests show the presence of bugs, not their absence, and specifications live in comments or nowhere.

Proposed:

- **v1**: contracts as syntax (checked at run time in debug builds), validated newtypes, property
  testing with contracts as oracles, fuzzing, mutation testing, a Miri-style checker and
  interleaving tests.
- **Designed for now, built later**: proofs written in Moddil itself (SMT-backed), assurance levels
  per mod, refinement types and model checking.

**Open questions** (same list as [`correctness.md`](correctness.md) §7)

1. Is the verifier part of the core toolchain or a mod (`moddil/verify`)?
2. Which SMT solver(s), and how are solver versions pinned for reproducible proofs?
3. How much annotation burden is acceptable before L1/L2 stop being used?
4. Should wrapping arithmetic have a "prove no overflow" mode for mods that want it?
5. How are contracts on effectful operations expressed (for example, "this handler is called at most
   once")?

---

### 2.8 Concurrency across CPUs

🟠 **questions** · Detail: [`concurrency.md`](concurrency.md), [`closures.md`](closures.md) (lambdas)

Shared-memory parallelism is still the source of the hardest bugs, async has split ecosystems into
sync and async halves, and cancellation is an afterthought almost everywhere.

Decided:

- **Data-race freedom by construction**, built on §2.2: owned values move into tasks, shared borrows
  are lent to many children, `mut` to exactly one, and the only shared mutable state is sync cells.
  There are **no `Send`/`Sync` traits**.
- **Structured concurrency is the only way to start a task**, so children can borrow from their
  parent's frame.
- **No function colouring**: suspension is an effect, and the backend chooses the lowering.
- **Runtimes are mods** that handle the scheduling effect (R2).
- **Construction may suspend; drop may not.** Async teardown is an explicit `close()` on a linear
  type.
- **Cancellation is implicit and cooperative**, with two-level shutdown (stop, then cancel)
  delivered innermost scope first. All I/O is promptly cancellable and cancel-safe.
- Closures are non-escaping by default and may borrow, fail and suspend
  ([`closures.md`](closures.md)).

**Open questions** (same list as [`concurrency.md`](concurrency.md) §7)

- **Preemption:** guaranteed (BEAM-style, via compiler-inserted yield points) or cooperative?
  Affects tail latency (HFT, game tick) and fairness. *Recommended* (§6.1 there): compiler-inserted safe
  points in loops and function entries, which `pinned` executors can disable.
- ~~**Cancellation and linear values.**~~ Resolved: cancellation is cooperative and observed only at
  suspension points, and the cancellation path must consume linear values like the error path (§9 there).
- **Deadlock:** this model prevents data races, not deadlocks. Lock ordering by type? A lint? Lock
  levels (as in some kernels)? Or steer toward message passing?
- **Async drop:** drop may not suspend (§8 there). Is that enough for resources whose clean shutdown needs
  I/O (e.g. flushing a TLS stream)? Probably: they become linear with an explicit `close`.
- **Priority inversion** under the scheduler effect: does the core effect interface carry priority?
- **Long-lived tasks:** what is the idiomatic shape for a server's top-level scope (accept loop with
  per-connection children) so that it reads naturally, not as a workaround for "no fire-and-forget"?

**Open questions** (same list as [`closures.md`](closures.md) §11)

1. Non-local `return` and labelled `break` from non-escaping lambdas (§7 there).
2. Default closure parameter mode, stated or inferred (§3 there).
3. Showing suspension points: inlay hints or an `await` marker (§9 there).
4. Rules and syntax for methods that declare which fields of `self` they touch (§6 there).

---

### 2.9 Concurrency across GPUs and accelerators

🟠 **questions** · Detail: [`accelerators.md`](accelerators.md)

GPU programming is fragmented by vendor, uses a different language from host code, and leaves data
movement between memory spaces manual and unchecked.

Decided:

- **One language, three levels**, all ordinary Moddil restricted by a compiler-checked `kernel`
  effect set: tensor expressions, tile kernels (the default) and SIMT kernels.
- **Devices are capability values**; buffers are branded by their device, so mixing devices is a
  compile error. A CPU pool is a device too.
- **Launches are structured tasks**; the end of the scope is the fence.
- **Race-free kernels by default**; arbitrary scatter writes need `unsafe`.
- **Algorithm and schedule are separate**; schedules are comptime data per target.
- **Runtimes are mods**: WebGPU first, CUDA second, plus a CPU simulator for tests.

**Open questions** (same list as [`accelerators.md`](accelerators.md) §14)

- **Autodiff:** a comptime transformation (as Slang and Enzyme do) or a feature of `moddil/tensor`?
- **Multi-GPU collectives** (all-reduce etc.): a library built on placement (§2.10)?
- Should tensor/array syntax become core, or stay in a mod?
- How close can the CPU simulator get to bit-exact results with GPU floating point?
- WebGPU feature gaps (subgroups, f16) and their fallbacks.
- **Brands:** exact rules — how far they propagate, and how they interact with views and generics.
- Hot-plug, and device loss in the middle of a run.
- Scheduling policy when a role's device is busy or shared between processes.

---

### 2.10 Distributed and multi-tier programs

🟠 **questions** · Detail: [`distribution.md`](distribution.md)

Almost no real system is one process. Today a feature spans several codebases and languages, held
together by hand-maintained contracts that drift, with mixed versions in production and implicit
trust boundaries.

Decided: *the language owns the program's meaning across boundaries; existing tools own where and
how it runs.*

- **Layer A, in the language** (v0): components with checked placement (`@client`, `@server`, …); a
  `remote` effect that never looks local; wire contracts generated from types with version-skew
  checking; `Untrusted<T>` and `Secret<T>`; whole-system runs in one process or under simulation.
- **Layer B, thin adapter mods** (v1 onwards): static binaries and OCI images; a topology file;
  adapters that emit Compose, Kubernetes, Nomad and systemd formats.
- **Infrastructure is bound, never provisioned.**
- **Layer C, never**: orchestration, autoscaling, service mesh, provisioning (non-goals, §5.2).

**Open questions** (same list as [`distribution.md`](distribution.md) §17)

- Where deployed contracts are recorded: in the repository (a lock file) or a registry?
- **Choreographies**: write a multi-party protocol once and project it to each participant — when,
  and how much?
- Databases as requirements: how schema migrations are versioned and checked alongside code.
- Independent team deploys: can one team ship its component without rebuilding the world?
- How much rollout safety (e.g. canary analysis) should Moddil check, versus leave to the platform?
- How unreliable or unordered channels (game snapshots over WebTransport) are expressed in types.

---

### 2.11 Determinism and reproducibility

🟡 **started** · Detail: [`determinism.md`](determinism.md)

Concurrent and distributed bugs are hard to reproduce.

Proposed:

- The core language is deterministic; **all nondeterminism enters through effects** (time,
  randomness, I/O, scheduling), with strict IEEE floating point.
- **Deterministic simulation testing as a built-in mode**, with seed-based replay.
- **Scoped handlers** for per-node and per-subsystem injection.
- **Record and replay**, and **reproducible builds**.

**Open questions** (same list as [`determinism.md`](determinism.md) §12)

1. Should this become R4?
2. Insertion-ordered maps by default, or seeded hash order?
3. How much of `moddil/math` must be correctly rounded, and at what speed cost?
4. GPU determinism: per kernel, per mod, or global?
5. Record format and retention: per task, per request, sampled?
6. How are non-Moddil peers brought into simulation beyond WASM components?

---

### 2.12 Observability and debuggability

🟠 **questions** · Detail: [`observability.md`](observability.md)

Optimised code is hard to debug, async stacks are unreadable, tracing is bolted on, logging is slow
and stringly typed, secrets leak into logs, and production bugs cannot be reproduced.

Decided:

- **`log`, `span` and `metric` are write-only ambient effects**; handlers are mods and cannot change
  results. There is no global logger.
- **Events are interned at compile time**; text is rendered later.
- **Spans follow the structured-concurrency tree** and cross `remote` calls automatically.
- **`Secret<T>` values cannot reach any event.**
- **Task-aware stacks**, a **flight recorder** with deterministic replay, and **live introspection**
  behind a capability.
- **Release builds stay debuggable.**

**Open questions** (same list as [`observability.md`](observability.md) §13)

- **Continuous profiling** as a handler, with time attributed by effect (network, locks, GPU)?
- Sampling policy for spans and recordings.
- Recording volume and retention; encrypting recordings, or excluding secret-typed effect results.
- Replaying a whole fleet: combining recordings from several processes.
- Schema evolution of structured events across versions.
- Overhead targets per level (e.g. nanoseconds per interned event on the hot path).

---

### 2.13 Interoperability with C and Rust

🟠 **questions** · Detail: [`interop.md`](interop.md)

Nobody adopts a language that cannot use existing code, yet C headers carry no ownership or
nullability information, C++ interop is very expensive, and Rust has no stable ABI.

Decided:

- **Only the C ABI and Rust are supported by the language.** C++ is reached through C shims.
- **C in both directions with zero overhead**: direct header import, C export with generated headers.
- **The toolchain compiles C**, hermetically and for any target.
- **Annotation overlays** add ownership, nullability, errors and effects to C APIs.
- **Rust through a generated bridge** over the C ABI.
- **WASM components** are the sandboxed path for untrusted code.
- **Foreign code is visible**: raw bindings are `unsafe`, and `ffi` is a capability.

**Open questions** (same list as [`interop.md`](interop.md) §12)

- How stable is rustdoc's JSON format, and how much generator maintenance follows Rust releases?
- How far can generics cross the Rust bridge beyond explicit instantiations?
- If crABI stabilises, can the bridge use it instead of the C ABI?
- Overlay distribution and trust: who may publish overlays, and how are they audited?
- Python interop (raised in the HFT example): out of scope, or through Python's C API as a mod?
- Size of the bundled clang in the toolchain download.

---

### 2.14 Build times

🟡 **started** · Detail: [`build-times.md`](build-times.md)

Slow builds damage the edit-compile-test loop for humans, and are worse for agents that iterate
dozens of times per task.

Proposed:

- **Language rules that decide build speed**: no cycles between mods, explicit interface artefacts,
  witness tables in debug builds and specialisation in release builds.
- **A lazy, query-based compiler** with a shared cache and no link step in dev builds.
- **Two backends**: a fast one for the dev loop, LLVM for release.
- **Live update** of a running debug program.
- **Four cache layers**, optimised dependencies by default, and **published budgets**.

**Open questions** (same list as [`build-times.md`](build-times.md) §10)

1. Are cycles allowed between modules inside one mod?
2. How slow can generic code be in debug builds before it is unacceptable? Should hot generics be
   specialised even in debug builds?
3. Live update (§4.1 there) proposes: bodies, signatures and new items live; layout changes through
   migration hooks or scope restart plus replay. Open: which layout migrations can be automatic, and
   whether production hot upgrades are ever a goal.
4. How much should the registry serve prebuilt binaries, and what are the trust implications?
5. Can debug-mode dictionary passing and release-mode specialisation produce *identical
   semantics*, so the difference is purely performance? *Proposed answer:* yes, if everything
   layout-dependent lives in the witness table, as Swift does (§3 there).
6. What is the budget? *Proposed:* < 100 ms keystroke to diagnostics, < 1 s from a function-body
   edit to a runnable test regardless of project size (§6 there).

---

### 2.15 Cross-compilation and deployment

🟠 **questions** · Detail: [`targets.md`](targets.md), [`freestanding.md`](freestanding.md) (bare metal; proposal)

Shared libraries make deployment fragile, and building for another OS usually needs that OS, a VM
or a hand-assembled sysroot.

Decided:

- **Every toolchain is a cross-compiler** for every tier-1 target (Linux, macOS, Windows, WASM),
  with zero setup.
- **Static by default**: no third-party shared libraries, ever.
- **No libc where the OS allows**; opt-in dynamic glibc targets with a pinned minimum version.
- **Byte-identical output on any host.**
- **Signing and packaging from any host.**

Proposed: a freestanding profile for kernels, bootloaders and firmware, verified in emulators
([`freestanding.md`](freestanding.md)).

**Open questions** (same list as [`targets.md`](targets.md) §13)

- **Licensing review** of bundling macOS headers and link stubs.
- Notarising from CI without a Mac, and how Apple credentials are held.
- Scope of iOS and Android packaging.
- Which tier-2 targets (Windows on ARM, FreeBSD, RISC-V), and when they become tier 1.
- The default CPU feature level per architecture.
- Which glibc versions to ship stubs for.

**Open questions** (same list as [`freestanding.md`](freestanding.md) §11)

1. Add a fifth, embedded reference example (§9.4 there)?
2. Should `moddil/rt-embedded` default to preemptive or cooperative scheduling?
3. Does stack-depth analysis apply to all freestanding code, or only to L1 tasks?
4. Tier assignments for bare-metal targets ([`targets.md`](targets.md) §5).
5. Floating point in interrupt handlers (lazy FPU state saving).

---

### 2.16 Package management and supply-chain security

🟡 **started** · Detail: [`packages-and-supply-chain.md`](packages-and-supply-chain.md)

Package managers made reuse trivially easy, and made the dependency graph the largest attack surface
in modern software: build-time code execution, ambient authority, and transitive dependencies that
nobody reviews.

Directions:

- **Capability-scoped dependencies**, enforced by the compiler: a JSON parser that imports `net`
  fails to build. Upgrades show **capability diffs**.
- **No arbitrary code at install time**; build logic is declarative or sandboxed in WASM.
- **Reproducible builds by default**, hash-pinned lockfiles, minimal version selection and a
  transparency log.
- **A built-in audit trail**, and a small but broad standard library.

**Open questions** (same list as [`packages-and-supply-chain.md`](packages-and-supply-chain.md) §5)

- How to handle capabilities through FFI (C code can do anything)? Probably: FFI *is* a capability
  (`ffi`/`unsafe`) and auditors treat it as high risk.
- Content-addressed definitions (Unison) vs versioned packages, or both?
- Central registry vs decentralised (Go's URLs plus proxy)?

---

### 2.17 Tooling uniformity

🟠 **questions** · Detail: [`toolchain.md`](toolchain.md), [`docs-and-tests.md`](docs-and-tests.md); see also [`agent-interface.md`](agent-interface.md)

One official formatter, test runner, doc generator, package manager, LSP, benchmark harness and
linter, shipped with the compiler, removes ecosystem fragmentation. The compiler is a library and a
service, and every tool uses the same front end.

Proposed ([`toolchain.md`](toolchain.md)): one `moddil` binary and one manifest; every tool is a
client of the warm compiler daemon; a canonical formatter whose only setting is line width; lints
built into the compiler, with custom lints as pure comptime mods; a task-aware debugger with time
travel; a built-in profiler; the toolchain version pinned in the manifest and fetched
automatically; no tool-specific config files.

Decided: docs and tests live next to the code.

- `///` docs are parsed and **checked**; effects, capabilities, error sets, contracts and traps are
  **generated**.
- **Examples are tests** with expected results, run under the deterministic simulation runtime.
- **Tests name their subject** (`test "…" for withdraw { }`); property tests, fuzz targets, proofs
  and benchmarks attach the same way.
- Only affected tests run, with results cached by content hash.
- `note` blocks record the *why* for maintainers.
- **Overviews map the code**: mod, module (`//!`) and group overviews carry the big picture for
  humans and agents. Overviews are required for every module with public items; groups are declared
  once (`group levels "Price levels"`) and **every public item** joins at least one with
  `@group(…)`; private items inherit the groups of the items that use them, shown by the language
  server rather than written in the source. Links are checked, invariants cite the contract or test
  behind them, and the tour example runs.

**Open questions** (same list as [`docs-and-tests.md`](docs-and-tests.md) §13)

- Syntax for attaching tests to methods, trait implementations and generic instantiations.
- Shared fixtures and test-only helpers: how they are declared and kept out of release builds.
- Default doc-coverage thresholds per assurance level.
- Where notes and provenance live: in the source, or in a sidecar keyed by definition ID?
- Benchmark stability on shared CI machines.
- Should the semantic diff flag a module whose public interface changed but whose overview wasn't
  edited?

---

### 2.18 LLMs as first-class programmers

🟠 **questions** · Detail: [`agent-interface.md`](agent-interface.md)

Languages assume the person who writes the code also understands it. In an LLM-heavy workflow the
writer is a model and the human is the reviewer, architect and approver. Current languages optimise
for the writer and against the reviewer, and a new language also starts with no training data.

Decided:

- **Verbosity is a view**: the source is explicit only at mod boundaries (effects, error sets,
  contracts, capabilities); a reviewer view shows inferred effects, types and error sets inline.
- **An agent server**: the compiler is a service over LSP and MCP, for queries (signatures, effects,
  call graph, test subjects, installed-version docs, `explain`) and actions (fixes, refactor
  intents, affected tests, replays).
- **Diagnostics built for agents**: stable codes, one root cause, machine-applicable fixes with a
  confidence level, relevant signatures inline.
- **"Did you mean?" everywhere**: suggestions from scope and the installed API, ranked by type;
  other languages' habits; missing imports; exact hints for renamed APIs (version drift).
- **The cold start**: familiar syntax, a reference small enough for a model's context, and a
  corpus of checked examples.
- **Agent-written code runs safely** (pure comptime, simulated tests, declared capabilities), and
  humans review semantic diffs, untested changes and mutation scores.

Directions: provenance in version control (recommended), a prefix-checkable grammar for constrained
decoding (recommended), translation tools from Rust and TypeScript, `note` blocks for the *why*
(decided in [`docs-and-tests.md`](docs-and-tests.md)).

**Open questions** (same list as [`agent-interface.md`](agent-interface.md) §7)

- Should provenance live in version control (recommended, §6), in the source, or in a sidecar?
- How far should constrained decoding go: grammar only, or names and types per statement?
- What size budget should the context-sized reference have, and how is it kept current?
- Who curates the cross-language synonym tables and foreign-syntax recognisers?
- Is MCP the right agent protocol long term, or one adapter among several?

---

### 2.19 Diffs, code review and version control

🟡 **started** · Detail: [`semantic-vcs.md`](semantic-vcs.md)

Version control works on lines of text, but programs are trees with meaning. Reviewers see noise
(formatting, renames, moves) and miss signal (a new side effect, a changed error path). As agents
write more code, review becomes the bottleneck.

Directions:

- **The canonical AST is the diff unit.**
- **Refactors are recorded as intents** ("rename `foo` → `bar`"), and reviewers approve the intent.
- **A semantic change summary per PR**, computed by the compiler: API changes, effect and capability
  deltas, new `unsafe` or FFI, new dependencies.
- **LLM-written review summaries are checked** against the computed semantic diff.
- **Definition-level history**, working with Git, not replacing it.

**Open questions** (same list as [`semantic-vcs.md`](semantic-vcs.md) §5)

- Stable identity for definitions: hash of content (Unison) vs persistent IDs vs name-based?
- Is a custom VCS layer justified, or can everything be done as Git metadata + tooling?

---

### 2.20 Language evolution

🟡 **started** · Detail: [`evolution.md`](evolution.md)

Languages either freeze (C) or break their ecosystems (Python 2→3).

Direction: editions from day one, with **automated migrations** in the toolchain that agents can run
and humans can review as intents (§2.19).

**Open questions** (same list as [`evolution.md`](evolution.md) §4)

None recorded yet.

---

## 3. Cross-Cutting Themes

When the problem areas are laid side by side, four ideas keep recurring:

1. **Explicitness that machines enforce.** Effects, capabilities, resources, memory spaces and errors
   are *in the types*, checked by the compiler, and mostly inferred inside a module. Explicit for the
   reader; cheap for the writer (especially when the writer is a model).

2. **One canonical representation.** A single formatting and a canonical AST give smaller diffs,
   cleaner training data, semantic review, faster caching and simpler tools.

3. **Safe by default, with explicit escape hatches.** Unsafe, FFI, manual memory and wrapping
   arithmetic are always possible, but always *visible*: in signatures, in package metadata and in PR
   summaries. The same goes for **cost**: the default is zero-cost (owned values and borrows), and
   anything more expensive is spelled out where it happens (`.to_rc()`, a sync cell, a `remote`
   call).

4. **Placement is one concept at every scale.** Thread → core → GPU → machine → tier: work is placed
   and coordinated by the same structured-concurrency model, and *where data lives* is part of its
   type, whether that is GPU memory (§2.9) or a client browser (§2.10). Algorithms are separate from
   schedules, and logical programs are separate from physical topologies. Crossing a boundary is
   always explicit.

An optional fifth: **the compiler is a service for agents and humans.** Its structured output is as
important as the binaries it produces.

---

## 4. Tensions and Trade-offs

Being honest about where goals collide:

| Tension                                            | Notes                                                                                                                                                                                            |
| -------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Explicitness vs brevity                            | Effects in every signature could be noisy. Mitigate with inference inside modules and explicit effects only at boundaries.                                                                       |
| Compile-time safety vs build speed                 | Borrow checking, effect inference and verification all cost time. Keep the expensive checks incremental and per-module. Second-class borrows help: the borrow check is local to each function. |
| No lifetimes vs expressiveness                     | Second-class borrows and view types remove lifetime annotations, but lose structs with several independent lifetimes and borrows stored in long-lived data. Owning views, rc and indices fill the gap. |
| Visible sharing vs single-threaded convenience     | rc contents are immutable and shared mutation needs sync cells, so a mutable graph used by one thread still pays (cheap) atomic costs. Arenas, indices and library collections are the idiomatic answer. |
| Monomorphisation vs build time vs runtime speed    | Dual-mode generics (§2.14) is promising but unproven at scale.                                                                                                                                    |
| Strict capabilities vs ergonomics                  | Passing capabilities everywhere is tedious. Need good defaults (e.g., `main` receives all; libraries receive nothing unless asked).                                                              |
| Canonical syntax vs expressiveness                 | One way to do things limits DSLs and clever code, which is usually good, and sometimes painful.                                                                                                  |
| Easy interop vs safety guarantees                  | Every FFI call weakens guarantees. Making it visible is the compromise.                                                                                                                          |
| Novelty vs adoptability                            | Effect systems and linear types are unfamiliar. Each novel feature must pay for itself.                                                                                                          |
| GPU portability vs peak performance                | Portable code is rarely fastest; schedules per target are the escape valve.                                                                                                                      |
| Location transparency vs explicitness              | One language across tiers is attractive, but hiding the network repeats the RPC mistakes of the 1990s. Moddil chooses explicit boundaries.                                                       |
| Language vs infrastructure scope                   | Every step toward describing infrastructure risks turning Moddil into a cloud platform.                                                                                                          |
| Whole-program view vs independent deploys          | Whole-system checking is powerful, but teams must still ship components separately.                                                                                                              |
| WASM as a required target vs native-first features | Threads and GPU access are weaker on WASM, and stack switching does not ship in browsers yet, so browsers need a JSPI-based lowering in the meantime. Every core feature needs a WASM implementation strategy, even if it is slower. |
| Wrapping arithmetic vs correctness | Unchecked, wrapping integers are fast and predictable but can silently produce wrong values. Checked operators and debug sanitisers are opt-in. |
| Small core vs compiler-checked guarantees | R2 wants a tiny core, but anything the compiler must check (ownership, effects, capabilities, placement, contracts) has to be in it. The core is "small" only relative to its guarantees. |
| "For LLMs" vs "for humans"                         | Must not produce a language only machines can comfortably write. Humans still need to read, debug and occasionally write it.                                                                     |

---

## 5. Requirements and Non-Goals

Full text: [`requirements.md`](requirements.md).

### 5.1 Firm requirements (decided)

These are no longer open for debate. Every other design decision must be compatible with them.

**R1. WebAssembly is a first-class compilation target.** WASM is a tier-one target from the first
release, tested in CI like native targets: core WASM, WASI and the Component Model, and the browser.
It is needed for multi-tier programs (§2.10), it fits the capability model (§2.1, §2.16), and it is
the sandbox for untrusted code. Concurrency semantics are target-independent; the backend lowers
suspension to stack switching (preferred), JSPI (the browser baseline) or a CPS transform. Details:
[`requirements.md`](requirements.md) §1 and [`bootstrap.md`](bootstrap.md).

**R2. Small core; functionality lives in mods.** The core language is deliberately very small.
Collections, networking, crypto, the async runtime, GPU support and UI are provided by **mods**
(packages; a *module* is a namespace inside a mod). A feature belongs in the core only if the
compiler must check it, if other mods need it to be defined at all, or if it cannot be expressed
efficiently in a mod. Runtimes are mods that provide handlers for core effects. Details:
[`mods.md`](mods.md).

**R3. No crash from an unhandled error.** Every error the language knows about must be handled. An
unhandled error at any entry point, or a silently dropped result, is a compile-time error, and there
are no fire-and-forget tasks. Traps (out of memory, stack overflow, division by zero, out-of-bounds
indexing) are out of scope. Details: [`error-handling.md`](error-handling.md).

**Open questions** (same list as [`requirements.md`](requirements.md) §3)

- R1: should WASI runtimes without stack switching be supported, which would need the
  CPS/Asyncify-style lowering? (Also in [`bootstrap.md`](bootstrap.md) §9.)
- Non-goals: is a REPL or script mode desirable?
- Should determinism become R4 ([`determinism.md`](determinism.md) §12)?

**Open questions** (same list as [`mods.md`](mods.md) §8)

- **How many batteries?** Go ships a broad standard library (HTTP, crypto, JSON); Rust ships a minimal
  one. With strong supply-chain controls (§2.16), a broad `moddil/` set reduces risky third-party
  dependencies. Which way should Moddil lean?
- **Distribution**: should standard mods ship with the toolchain, or be fetched and pinned like
  any other mod?
- **Registry**: one central registry with a `moddil/` namespace, or decentralised (Go-style URLs plus a
  proxy and checksum database)?
- **Naming**: is "mod" too easily confused with Rust's `mod` (a module)? The distinction between mod
  and module needs to stay clear in docs and tooling.
- **Coherence**: how are trait/instance overlap rules (Rust's orphan rule) handled across mods?

**Open questions** (same list as [`bootstrap.md`](bootstrap.md) §9)

- Stack switching may change shape before standardisation, or arrive late in Safari.
- Should the MIR be designed so that MLIR can consume it later (GPU/tensor work)?
- C backend: worth it for bootstrapping and portability?
- Should WASI runtimes without stack switching be supported (lowering 4)?
- LLVM version pinning and upgrade cadence.
- Is Moddil-core (the stage 0 subset) small enough to build quickly, yet large enough to write a
  pleasant compiler in?

### 5.2 Non-goals (draft)

- Maximum syntactic novelty. Familiar syntax wins unless there is a strong reason.
- Replacing every language. Scripting, notebooks and quick glue may remain better served elsewhere
  (open question: is a REPL/script mode desirable?).
- C++ interop in the language. C++ is reached through C shims ([`interop.md`](interop.md) §10).
- A custom VCS that replaces Git (tooling on top of Git first).
- A cloud platform, orchestrator or service mesh. Moddil emits artefacts and manifests for existing
  ones.
- Transparent remote calls.
- Runtime reflection / dynamic metaprogramming.

---

## 6. Prior-Art Matrix

The matrix comparing fifteen languages across the problem areas, and the reference list, are in
[`prior-art.md`](prior-art.md).

**Observation:** no row is strong across the board, and **no language at all** is strong in "semantic
diffs" or treats "LLM-friendliness" as a stated design goal. Distributed programming is well served
only in niches (Erlang, Unison, Ballerina/Wing, research multitier and choreographic languages), and
none of those combine it with systems-level performance or GPU support. Those three columns, together
with effects and capabilities as a unifying mechanism, are where the opportunity is largest.

---

## 7. Open Questions and Next Steps for Moddil

Open questions for a single area are listed under that area in §2. The questions here cut across
areas.

**Strategic questions**
1. **Primary target domain** for v0? Candidates: systems / infrastructure services; ML and
   numeric (GPU-heavy); embedded; general applications. The choice reshapes priorities (e.g., GPU is
   central for ML, peripheral for services).
2. **Who is the first user?** An agent-assisted team writing services? A numerics group? The answer
   sets the bar for ergonomics vs guarantees.
3. **Adoption path**: embed in existing C/C++/Rust codebases one module at a time, through C export
   and the generated Rust bridge (§2.13)?
4. **Is multi-tier deployment (§2.10) a v0 goal?** **Decided:** the language layer (components,
   placement, `remote`, contracts, single-process and simulated runs) is v0; images, Compose,
   Kubernetes and skew checks are v1; other adapters later ([`distribution.md`](distribution.md) §15).
5. **Mobile and UI**: shared logic under native UI shells, or a Moddil UI mod? See
   [`examples/03-web-store.md`](examples/03-web-store.md).

The **reference examples** in [`examples/`](examples/README.md) (multiplayer game, HFT platform, web
store, Hashiverse) are the yardstick for every decision. A feature no example needs is a candidate for
removal.

**Technical questions**
6. **Bootstrap compiler and back ends — proposed** in [`bootstrap.md`](bootstrap.md): stage 0 in
   Rust, then self-host; Cranelift (debug) + LLVM (release) + own WASM emitter + naga/LLVM for GPU;
   Wasmtime as the reference WASM runtime first.
7. **Memory core — decided** in [`memory-and-resources.md`](memory-and-resources.md): unique
   ownership, second-class borrows and view types, `.to_rc()` with biased RC, affine + linear
   resources (summary in §2.2).
8. Effect system design: row-polymorphic (Koka) vs capability-passing vs hybrid.
   [`determinism.md`](determinism.md) §4 recommends a hybrid: scoped handlers plus second-class
   capability values.
9. Canonical AST storage: text with canonical printing, or a structured store (Unison-style) with text
   as a view?

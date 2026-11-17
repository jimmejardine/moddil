# Moddil Rationale: Why a New Language?

> **Status:** brainstorm / living document for **Moddil**. This is not a specification and makes no syntax commitments.
> Its job is to list the problems clearly, credit what already works, and narrow down where a new
> language could actually earn its place.
>
> Where a decision has been made, this document gives a short summary and links to the detailed
> companion document (see the map below). The companion documents are authoritative for detail.

**Document map**

| Document | Covers | Status |
|---|---|---|
| [`rationale.md`](rationale.md) | Problems, prior art, direction (this document) | Living |
| [`memory-and-resources.md`](memory-and-resources.md) | Ownership, borrows and view types, `.to_rc()`, linear resources, zero-copy I/O (§2.2) | Decided direction |
| [`concurrency.md`](concurrency.md) | CPU concurrency: sharing rules, structured tasks, sync cells, runtime contract (§2.8) | Decided direction (CPU); GPU to come |
| [`mods.md`](mods.md) | Mods, the core/mod boundary, standard-mod catalogue (R2) | Decided direction |
| [`error-handling.md`](error-handling.md) | `?`, try blocks, results vs effects, R3 (no unhandled errors), traps, traces (§2.3) | Partly decided; survey |
| [`comptime.md`](comptime.md) | Compile-time evaluation, reflection, typed derives, tagged-literal DSLs instead of macros (§2.6) | Decided direction |
| [`bootstrap.md`](bootstrap.md) | Bootstrap compiler, self-hosting, backends, WASM engine support (R1) | Proposed |
| [`examples/`](examples/README.md) | Four reference systems used to test every decision | Written |

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

Each area follows the same template:

- **Problem**: what goes wrong today.
- **State of the art**: who does it best, and why.
- **What still hurts**: the remaining gaps.
- **Decision** (once made): a short summary, with a link to the companion document.
- **Directions**: ideas still to explore (not decisions).
- **Open questions**

---

### 2.1 Effects and capabilities as a unifying idea

- **Problem.** Signatures say what a function takes and returns, but not what it *does*: I/O,
  allocation, suspension, failure, nondeterminism, GPU work, unsafe code.
- **State of the art.** Koka, Effekt, Eff (algebraic effects and handlers); OCaml 5 effect handlers;
  Unison abilities; Haskell monads/effect libraries; object-capability languages (E, Austral, Monte),
  WASI capabilities.
- **Why it matters here.** One mechanism could solve several problems above:
  - concurrency without colouring (suspension is an effect, §2.8)
  - GPU-safe subsets (kernels are code with a restricted effect set, §2.9)
  - resource safety (effects plus linear types, §2.2)
  - supply-chain security (capabilities, §2.16)
  - reviewability (effects in signatures, §2.18, §2.19)
  - testability (handlers swap real I/O for fakes, §2.11)
  - implicit context (config, database pools) provided by handlers instead of threading context
    objects through every call ([`memory-and-resources.md`](memory-and-resources.md) §4)
- **Directions.** Effect *inference* inside a module, explicit effects at module/package boundaries;
  capabilities as values; handlers compiled to zero-cost code where possible (Koka's evidence passing).
- **Open question.** Can effect systems be made ergonomic for mainstream programmers? No mainstream
  language has done it yet; this is both the biggest risk and the biggest opportunity.

---

### 2.2 Memory and resource management

**Problem.** Every approach trades something away:

| Approach                                         | Gives you                        | Costs you                                                                      |
| ------------------------------------------------ | -------------------------------- | ------------------------------------------------------------------------------ |
| Manual (C)                                       | Total control                    | Use-after-free, leaks, double frees: most CVEs                                 |
| Tracing GC (Java, Go, C#)                        | Simplicity, safety               | Pauses, memory overhead, non-deterministic destruction                         |
| RAII (C++)                                       | Deterministic, zero-cost cleanup | Only if followed consistently. Nothing enforces it; dangling references remain |
| Reference counting (Swift, Python, `shared_ptr`) | Deterministic, incremental       | Atomic RC is slow under contention; cycles leak; extra traffic on every copy   |
| Ownership/borrowing (Rust)                       | Safe, zero-cost, deterministic   | Learning curve; some data structures (graphs, doubly-linked lists) are awkward |

**State of the art.**
- **Rust**: the reference point for safe, GC-free memory management.
- **Modern GCs** (ZGC, Shenandoah, Go's concurrent GC) have made pauses sub-millisecond for many
  workloads. "GC = random delays" is less true than it was, but throughput and memory overhead remain.
- **Perceus** (Koka, Lean 4): compile-time reference counting with reuse analysis. Most RC operations
  are removed, and functional code can update in place.
- **Lobster**: compile-time RC elision with ownership inference.
- **Vale**: generational references, so memory-safety checks happen at runtime but cheaply, without a
  borrow checker.
- **Zig/Odin**: explicit allocators passed around, arenas everywhere, with very simple semantics.
- **Pony ORCA**: per-actor GC with no global pauses.

**What still hurts.**
- RAII is brilliant but *opt-in by convention*; nothing stops a raw `new` in C++.
- Most languages only manage **memory**. File handles, sockets, locks, GPU buffers and transactions
  are managed with `defer`, `using`, `with` or finalisers, which are easy to forget.
- Atomic RC makes multithreaded code slower even when objects are never shared.
- Rust lifetimes leak into APIs and make refactoring expensive.

**Decision** (detail: [`memory-and-resources.md`](memory-and-resources.md)).
- **Unique ownership and RAII by default**, for every resource, enforced by the compiler. Values
  are moved or borrowed. Drop is deterministic, never suspends and never fails.
- **Borrows are second-class; there are no lifetime annotations anywhere.**
  - Parameter modes `let` (shared), `mut` (exclusive) and `sink` (move), as in Hylo. A shared
    borrow freezes its source.
  - *View types* (structs and containers holding borrows, with one inferred region) cover context
    objects, custom iterators and arena-backed graphs.
  - Functions may return borrows derived from their parameters.
  - Accepted limits: no structs with two independent lifetimes, and no borrows stored in long-lived
    owned data.
- **`.to_rc()` is the only way to share a pointer**, so the cost is visible where it is paid. `Weak`
  handles break cycles.
  - rc contents are immutable; shared mutation goes only through sync cells.
  - **Biased reference counting**: one rc type, always thread-safe, and nearly free while it stays
    on its owning thread. Compile-time RC elision removes more.
- **Affine by default, linear on request.** Linear types (transactions, fallible `close`) must be
  explicitly consumed, and cannot be rc'd.
- **Zero-copy I/O** comes from moving owned buffers: DMA buffers are owned values, parsed through
  validated views, *owning views* (buffer + offsets) or rc slices.

**Directions.**
- **Allocators as a capability**: the allocator is visible (Zig-style), and per-scope arenas are cheap
  and idiomatic.
- Machine-applicable diagnostics for the borrow check (e.g. "insert `.to_rc()` here"), aimed at
  agents (§2.18).

**Open questions** (more in [`memory-and-resources.md`](memory-and-resources.md) §10).
- How do allocators interact with `.to_rc()` and arenas? Does an `Rc` remember its allocator?
- What is the least noisy way to consume linear values on error paths (§2.3)?
- Is there still a place for an optional per-task/per-actor collector, or does the model above make
  it unnecessary?

---

### 2.3 Error handling

> Expanded in [`error-handling.md`](error-handling.md). Decided: Rust-style `?`, try blocks, and
> **R3** (§5.1): no unhandled errors.

- **Problem.** Exceptions hide control flow; error codes get ignored; `Result` types are explicit but
  noisy; panics/abort are coarse. Composing error types across libraries is painful (Rust's
  `thiserror`/`anyhow` split shows this).
- **State of the art.** Rust `Result` + `?`; Swift typed `throws`; Zig error unions with inferred error
  sets and error return traces; Go explicit `error` values; Erlang "let it crash" plus supervisors.
- **Directions.** Errors as an *effect* (`fail(E)`) so they are visible in signatures but cheap to
  propagate; inferred error sets (Zig) to reduce boilerplate; mandatory handling; error return traces
  for free; a clear split between *recoverable errors* and *bugs* (contract violations → abort the
  task, not the process, in structured concurrency). Error propagation must also respect linear
  values (§2.2): a path that propagates an error while holding an open transaction has to consume it.
  Absence is *not* an error: "not found" is an `Option` (§2.4), and `or_fail` converts explicitly.
- **Open question.** Checked-exception fatigue (Java) vs inference: how much should be inferred?

---

### 2.4 Null and invalid states

**Problem.** Null references (Hoare's "billion-dollar mistake"). More generally: types that admit
values the domain does not, such as zero values nobody chose, unchecked ranges, mixed-up IDs, and
objects used in the wrong state.

**State of the art.** Every language that has tackled null is converging on *absence in the type,
plus compiler-checked narrowing*, in one of two styles:

| Style | Languages | Strength | Weakness |
|---|---|---|---|
| Wrapper sum type `Option<T>` | Rust, Haskell (`Maybe`), OCaml, Scala, Zig (`?T`) | Ordinary type; composes; nests (`Option<Option<T>>` ≠ `Option<T>`) | Ceremony without sugar |
| Nullable union `T \| null` + flow typing | Kotlin, TypeScript (`strictNullChecks`), Dart 3, C# 8, Java (Valhalla null-restricted types, JSpecify) | Ergonomic; can be retrofitted onto a language with null | `T??` collapses to `T?`: "absent" vs "present but null" is lost |

- **Swift** is the synthesis most new designs copy. `T?` is sugar for a real `Optional<T>` sum type,
  so it nests correctly, and it adds `if let`, `guard let`, optional chaining `a?.b` and `x ?? d`.
- **Rust** has exactly one `Option`, kept separate from `Result`: absence is a normal outcome,
  failure is an error.
  - Raw pointers can be null only in `unsafe` code.
  - The **niche optimisation** makes `Option<&T>` pointer-sized, with `None` as address 0.
  - Ergonomics keep improving: `?` on `Option`, `let … else`, `if let` chains (2024 edition).
  - The *pattern types* work (`u32 is 1..`) generalises niches.
- **The frontier is invalid states in general:**
  - definite initialisation (no Go-style zero values);
  - refinement types (Liquid Haskell, F*);
  - newtypes;
  - typestate.

**Decision.**
- **No null in safe code.** Absence is `Option<T>` = `None | Some(T)`. It is an ordinary sum type in a
  core-adjacent mod (R2); the compiler adds only sugar and layout guarantees.
- **Swift-style sugar:** `T?` means `Option<T>`, and nests (`T??` ≠ `T?`). It comes with:
  - exhaustive `match`, `if let` and `let … else`;
  - optional chaining `a?.b`;
  - defaulting `x ?? d`.

  There is no implicit unwrap. `unwrap()` is an explicit, visible panic point.
- **Absence is data; failure is an effect.** `fail(E)` (§2.3) plays the role of Rust's `Result`,
  so `Option` is the only "maybe" type in everyday code.
  - `opt.or_fail(e)` turns absence into failure.
  - Catching a failure can produce an `Option`.
- **Guaranteed niches:** `Option<&T>`, `Option<Rc<T>>`, owned pointers, non-zero and range-typed
  integers, and enums with spare values are all the same size as `T`. Nullable C pointers import as
  `Option<…>` (§2.13).
- **Fits the ownership model:**
  - `Option<&T>`, or an `Option` holding a view, is itself a view
    ([`memory-and-resources.md`](memory-and-resources.md) §4).
  - Moving out of an `Option` is an ordinary move (`take()`).
- **Definite initialisation:** no zero values, and no reads of uninitialised memory in safe code.
- **Invalid states beyond null:**
  - **Cheap newtypes:** zero-cost and distinct, with no implicit conversion (`UserId` ≠ `OrderId`).
  - **Pattern/range types** (`u32 in 1..`): values that are excluded become free niches for `Option`.
  - **Refinement types** (`Int where 0 <= self < 100`): checked statically where provable,
    dynamically otherwise. They share machinery with contracts (§2.7).
  - **Typestate through linear types** (§2.2): states are types, and transitions consume the old
    state, e.g. `Conn<Unauthed>` → `Conn<Authed>`.

Illustration only:
```
type UserId = newtype u64
type Port   = u16 in 1..                          // 0 excluded: Port? is still 2 bytes

fn email_of(users: let Map<UserId, User>, id: UserId) -> &str? {
    users.get(id)?.profile?.email                 // optional chaining: absence flows through
}

fn connect(host: let str, port: Port?) ! {net, fail(NetError)} {
    let addr = resolve(host) ?? return            // default/early exit on absence
    let Some(p) = port else { fail(NetError.NoPort) }
    …
}
```

**Open questions.**
- **What does postfix `?` mean?** In Rust it propagates `None`/`Err`; in Swift it is chaining.
  - If `fail` is an effect, failure may propagate implicitly. The `call()?` in the snippets of
    [`memory-and-resources.md`](memory-and-resources.md) would then disappear.
  - Separately: should `opt?` inside a function returning `T?` propagate `None`?
  - Resolve together with §2.3.
- How much refinement checking is static vs dynamic, and what does it cost in compile time (§2.14)?
- Layout of nested options (`T??`): how far should niche packing go?

---

### 2.5 Numerics and data

- **Problem.** Silent integer overflow (C), undefined behaviour, float surprises, no units.
- **Directions.** **Decided** ([`error-handling.md`](error-handling.md) §2): integer arithmetic
  wraps (two's complement, no UB, no checks); floats follow IEEE 754; integer division by zero and
  `MIN / -1` trap, as in Rust; explicit checked/saturating operators live in a mod;
  units of measure (F#) as an optional type feature; SIMD vectors and tensors as first-class types
  (ties into §2.9); arbitrary-precision integers in the standard library.

---

### 2.6 Metaprogramming

**Problem.** Macros are powerful but break tooling, readability, caching and LLM understanding
(C preprocessor, C++ templates, Rust proc macros). Proc macros also run arbitrary code at build time.

**State of the art.**
- **Zig `comptime`**: ordinary code run at compile time, with no separate macro language. Types are
  values and reflection is built in. But generics are duck-typed, and comptime cannot add
  declarations (no `derive`).
- **Rust**: hygienic `macro_rules!` and very powerful proc macros (`serde`, `sqlx`), which are opaque,
  slow to build and unsandboxed.
- **Others**: C++ `constexpr` and C++26 static reflection; Swift's sandboxed, typed macros;
  Lisp/Racket macros.

**Decision** (detail: [`comptime.md`](comptime.md)).
- **No macro language.** Metaprogramming is ordinary Moddil code run by the compiler.
- **Comptime is pure**: an empty effect set, deterministic, fuel-limited, sandboxed and cached by
  input hash (§2.14). External inputs come only through declared `embed` files.
- **Generics stay trait-bounded** and are checked at definition. Comptime is for values (sizes,
  tables, format strings) and for code generation.
- **Static reflection**: `reflect(T)`, read-only, respecting visibility.
- **Typed generation at the annotation site.** `@derive(serde.Serialize)` calls a comptime function
  that returns a typed impl built through a construction API, never token streams.
- **Tagged string literals for DSLs**: `fmt"…"`, `re"…"`, `sql"…"`. These are checked at compile time,
  with interpolations as typed holes, and `sql` checks against an embedded schema rather than a
  live database.
- **Every expansion is visible**, through `explain` (§2.18) and in semantic diffs (§2.19).

**Open questions** (more in [`comptime.md`](comptime.md) §13).
- Does comptime run in an interpreter inside the compiler, as WASM, or a hybrid?
- What are the default compile-time budget and the per-mod limits?

---

### 2.7 Correctness tooling: contracts and verification

- **Problem.** Tests show the presence of bugs, not their absence; specifications live in comments or
  nowhere.
- **State of the art.** Ada/SPARK, Eiffel (design by contract), Dafny, Verus (Rust), Liquid Haskell,
  Lean 4, Prusti/Kani for Rust, property-based testing (QuickCheck, Hypothesis).
- **Directions.** Contracts are syntax, not library calls; checked at runtime in debug, optionally
  proven by an SMT-backed verifier for critical modules; property tests generated from contracts.
  This becomes much more valuable with LLMs, which can write proofs and code against a fixed spec.

---

### 2.8 Concurrency across CPUs

**Problem.** Shared-memory parallelism is still the main source of the hardest bugs: data races,
deadlocks, priority inversion, and memory-ordering mistakes. Async programming has split many
ecosystems into "sync" and "async" halves.

**State of the art.**
- **Rust**: `Send`/`Sync` traits plus ownership give compile-time freedom from data races. This is a
  real breakthrough.
- **Erlang/BEAM**: share-nothing processes, message passing, supervisors, and preemptive scheduling of
  millions of lightweight processes. It has the best story for fault tolerance.
- **Go**: goroutines and channels make concurrency cheap and ergonomic, and there are no coloured
  functions.
- **Pony**: reference capabilities (`iso`, `val`, `ref`, `tag`, …) give actors data-race freedom by
  construction, plus a per-actor GC (ORCA) with no stop-the-world pauses.
- **Structured concurrency**: Trio (Python), Swift's `TaskGroup`, Kotlin coroutine scopes, and Java's
  virtual threads plus `StructuredTaskScope`. A task's lifetime is bounded by its lexical scope, so
  cancellation and errors propagate properly.

**What still hurts.**
- **Function colouring** (Bob Nystrom's "What color is your function?"). In Rust, C#, JS and Python,
  `async` spreads through the codebase, and libraries split into sync and async variants. Rust async adds
  `Pin`, `Send` bounds across `.await`, and executor lock-in (Tokio vs others).
- Go has no compile-time protection against races. The race detector is dynamic and optional.
- Erlang's isolation costs copying, and its raw compute performance is weak.
- Rust's guarantees come with a learning curve that is still the most common complaint about the
  language.
- Cancellation is an afterthought almost everywhere.

**Decision** (detail: [`concurrency.md`](concurrency.md)).
- **Data-race freedom by construction**, built on the memory model of §2.2:
  - owned values are *moved* into tasks;
  - shared borrows may be lent to many child tasks, a `mut` borrow to exactly one;
  - `Rc` is always thread-safe and its contents are immutable;
  - the only shared mutable state is thread-safe *sync cells* (`Atomic`, `AtomicRc`, `Mutex`, …).
- Therefore **no `Send`/`Sync` traits**: every type can be sent between tasks.
- **Structured concurrency is the only way to start a task.** Every task has a parent scope and ends
  before it, which is what lets children borrow from their parent's frame with no `'static` bounds
  and no `Arc` cloning (Rust's `thread::scope`, as the only mode).
- **No function colouring.** Suspension is an *effect* (§2.1); the backend chooses the lowering
  (stack switching, JSPI, …; see R1).
- **Runtimes are mods** that handle the scheduling effect (R2). They also poll the biased-RC merge
  queue (§2.2).

**Directions.**
- Actors, channels and thread-per-core sharding as library patterns built on these primitives, not
  as the only model.
- Deterministic scheduling mode for tests (see §2.11).
- Lints for holding a lock across a suspension point.

**Open questions** (tracked in [`concurrency.md`](concurrency.md) §7).
- Should preemption be guaranteed (BEAM-style) or cooperative?
- What happens to a linear value (e.g. an open transaction) in a task that is cancelled?
- The model prevents data races, not deadlocks. Lock ordering, lock levels, lints, or steering
  toward message passing?

---

### 2.9 Concurrency across GPUs and accelerators

**Problem.** GPU programming is fragmented by vendor, uses a different language (or dialect) from host
code, and makes data movement between memory spaces manual and error-prone. Most programmers never
touch it; those who do are often tied to one vendor.

**State of the art.**
- **CUDA**: mature, fast, huge ecosystem, but NVIDIA-only, C++-dialect, with a separate compilation model.
- **SYCL / oneAPI, OpenCL, HIP**: portability attempts with varying success.
- **Metal / Vulkan compute / WebGPU (WGSL)**: platform-bound compute shaders.
- **Triton**: a Python DSL for tiled GPU kernels. It shows that block-level abstractions can match
  hand-written CUDA for many kernels.
- **Halide**: separates the *algorithm* (what to compute) from the *schedule* (tiling, vectorisation,
  parallelism, placement). This is a key idea.
- **Futhark**: a functional data-parallel language that compiles to GPU and proves that high-level
  array programs can compete.
- **JAX / XLA, MLIR**: compiler infrastructure that lowers high-level array programs to many back ends.
- **Mojo**: aims at Python ergonomics plus systems performance plus accelerator targeting via MLIR.

**What still hurts.**
- Host and device code live in different worlds: different types, different debuggers, different build steps.
- Memory spaces (host, device, shared, unified) are invisible in most type systems, so you only discover
  an accidental copy or an invalid pointer at runtime.
- Portable performance is still mostly a myth. Schedules are tuned per device by hand.
- Synchronisation between CPU and GPU (streams, fences, events) is manual and unchecked.

**Directions.**
- **One language, many targets.** Kernels are ordinary functions restricted to a subset (no allocation,
  no unbounded recursion, …) that the compiler checks: a "kernel-safe" effect set.
- **Memory space in the type**: `Buffer<f32, @device(gpu0)>` vs `Buffer<f32, @host>`. Transfers are
  explicit operations; aliasing across spaces is a compile error. The same idea already covers NIC
  DMA buffers (`@dma(nic0)`) in the zero-copy I/O design
  ([`memory-and-resources.md`](memory-and-resources.md) §6), and device buffers follow the same
  ownership rules as any other resource.
- **Algorithm/schedule separation** (Halide-style) as a language feature, so the same algorithm can
  carry several schedules per target, and LLMs or autotuners can propose schedules without touching the
  algorithm.
- **GPU work participates in structured concurrency**: a kernel launch is a task whose completion is a
  value you can wait on, so fences and streams become scoped tasks.
- Lower through **MLIR / SPIR-V / PTX / Metal IR** instead of building backends from scratch.

**Open questions.**
- How much of the CPU language is allowed in kernels? A strict subset, or a separate sub-language?
- Is a single-source model realistic across NVIDIA, AMD, Apple and Intel, or should we target a
  portable IR and accept a performance gap?
- Should tensor/array types be built into the language (like Fortran/APL) or left to libraries?

---

### 2.10 Distributed and multi-tier programs

**Problem.** Almost no real system is one process. A single feature, such as "user uploads a photo",
touches a browser client, an API server, a message queue, a worker, object storage and a database.
Today that means:
- **N codebases in M languages** (TypeScript client, Go/Java/Python server, consumers, SQL, plus
  Terraform/Helm/YAML), each with its own build and deploy.
- **Hand-maintained contracts** between them (OpenAPI, protobuf, JSON shapes) that drift silently.
- **Mixed versions in production**: during a rolling deploy, old and new code talk to each other, and
  nothing checks that they are compatible.
- **Partial failure** (timeouts, retries, duplicates, ordering) handled ad hoc in every service.
- **Implicit trust boundaries**: client code runs on hostile machines, but nothing in the code says so.

**State of the art.**
- **Multitier ("tierless") languages**: Links, Hop.js, Ur/Web, Eliom/Ocsigen, ScalaLoci. One program
  with client and server parts; the compiler splits it and generates the communication.
- **Choreographic programming**: Choral, HasChor, Pirouette. A multi-party protocol is written *once*
  from a global viewpoint and projected into the code for each participant. Deadlock freedom and
  protocol conformance hold by construction.
- **Erlang/OTP and Akka**: distribution as a native concept, with location-aware process identifiers
  and supervision across nodes.
- **Microsoft Orleans**: virtual actors that the runtime places and moves for you.
- **Service Weaver** (Google): write a *modular monolith*, then decide at deploy time which components
  run in which processes. The same code runs as one binary locally or as many services in production.
- **Unison Cloud**: code is shipped to remote nodes by hash; there is no deployment step in the
  usual sense.
- **Winglang**: separates *preflight* code (defines infrastructure) from *inflight* code (runs on it),
  in one language.
- **Ballerina**: a language whose types and syntax are built around network services and data.
- **Durable execution**: Temporal, Restate, Azure Durable Functions. Workflows survive crashes and
  restarts.
- **Web frameworks** moving the same way: React Server Components and server actions, Electric
  Clojure, tRPC (types shared end to end).
- **Research**: Berkeley's Hydro/Bloom (dataflow languages for distributed systems).
- **Infrastructure as code**: Pulumi (real languages), Terraform (declarative).

**What still hurts.**
- **Transparent RPC is a trap.** Making a remote call look like a local one hides latency, partial
  failure and concurrency. This is the central lesson of Waldo et al., *A Note on Distributed
  Computing* (1994), and of Deutsch's "fallacies of distributed computing". Many multitier systems
  repeated the mistake.
- Frameworks that blur client and server make it easy to **leak secrets or trust client input** by
  moving one line of code.
- **Schema evolution** across independently deployed versions is still manual discipline.
- Infrastructure definitions live apart from the code that depends on them, and drift.
- Running and testing the *whole system* locally is slow and flaky.

**Directions.**
- **Placement is part of the type/effect system.** Code and data carry a location: `@client`,
  `@server`, `@worker`, `@edge`, …. This is the same idea as GPU memory spaces in §2.9, scaled up
  from "device" to "machine".
- **Crossing a boundary is never invisible.** A remote call is an *effect* (`remote`), with its own
  failure, timeout and cancellation semantics, and it appears in signatures and PR summaries. The
  rule is "one language, explicit boundaries", not "one address space".
- **Wire contracts generated by the compiler** from the types at each boundary, plus **compatibility
  checking across versions**: the compiler compares the new contract with what is deployed and
  rejects changes that would break a rolling deploy (ties into semantic diffs, §2.19).
- **Choreographies** for multi-party protocols (client ↔ server ↔ worker), projected to each
  participant.
- **Trust boundaries in the type system.** Data arriving from `@client` is *untrusted* until
  validated; values marked secret cannot flow to `@client` (information-flow checking). Capabilities
  (§2.16, §2.1) decide which tier may touch which resources.
- **Separate the logical program from the physical topology** (the Service Weaver idea). The code
  defines components and placements; a declarative **deployment manifest** maps them to processes and
  machines. The same program runs as one process for development and tests, or as many services in
  production.
- **The compiler emits artefacts, not infrastructure.** Native binaries, WASM for the browser and
  WASI components for edge/serverless (R1, §5.1),
  container images, plus a manifest that existing orchestrators (Kubernetes, serverless platforms,
  plain VMs) can consume.
- **Whole-system deterministic simulation** (§2.11): every tier in one process, with network, time and
  failures simulated and replayable from a seed.
- **Durable workflows** as an optional effect for long-running, crash-surviving processes.
- **Messaging as structured concurrency** across machines: a request that fans out to workers is a
  task scope; cancellation and errors propagate across the network.

**Open questions.**
- How far into infrastructure should Moddil go? Describe resources (as Winglang does), or only emit
  artefacts plus a manifest for existing tools?
- How does a Moddil system talk to non-Moddil services? Probably by generating and consuming
  OpenAPI/protobuf/AsyncAPI at the edges.
- How are databases modelled: as a tier, a capability, or an external service with a typed schema?
- What belongs in the language and what in a library/framework? Placement and the `remote` effect
  seem like language features; specific protocols and brokers seem like libraries.
- Whole-program view vs independent teams: can one team deploy its component without rebuilding the
  world? Version-compatibility checking has to make that safe.

---

### 2.11 Determinism and reproducibility

- **Problem.** Concurrent and distributed bugs are hard to reproduce.
- **State of the art.** FoundationDB and TigerBeetle deterministic simulation testing; `rr` record and
  replay; Loom (Rust) model checking for concurrency.
- **Directions.** Since time, randomness, I/O and scheduling are effects/capabilities, a test harness
  can replace them with deterministic versions, giving **deterministic simulation testing as a built-in
  mode** with seed-based replay.

---

### 2.12 Observability and debuggability

- **Problem.** Optimised code is hard to debug; async stacks are unreadable; tracing is bolted on.
- **Directions.** Structured tracing as a language-level effect; async/task-aware stack traces;
  record-and-replay in dev builds; good debug info even in release mode.

---

### 2.13 Interoperability with C, C++ and Rust

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

**Directions.**
- **Zero-overhead C ABI** in both directions, and **direct import of C headers** (Zig-style), with a
  bundled C compiler/cross-compiler.
- **Annotation overlays** for imported C APIs: ownership (owned / borrowed / rc), nullability
  (nullable pointers become `Option<…>`, §2.4), thread safety, and capabilities, ideally shareable
  across projects (like TypeScript's DefinitelyTyped).
- **C++**: a defined, documented subset (POD types, non-template functions, explicit instantiations,
  simple classes) via a Clang-based importer; no promise of full template interop.
- **Rust**: via `extern "C"` + a richer **interface description** (shared IDL that both sides generate
  from, carrying ownership and simple generics). Track Rust's `crabi` / stable-ABI efforts.
- **Foreign code is a capability**: calling FFI requires `ffi`/`unsafe` in the effect set, so it is
  visible in signatures, PR summaries (§2.19) and package audits (§2.16).
- **Export to C** cleanly, so Moddil can be adopted *inside* existing C/C++/Rust projects
  one module at a time.

**Open questions.**
- How much C++ is worth supporting? Carbon and Swift spent years on this.
- Should the language *be able to compile C* (as Zig does) to own the build end-to-end?

---

### 2.14 Build times

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

**Directions.**
- **Make the module boundary the unit of compilation and caching.** Interfaces are separate from
  implementations (an interface hash change triggers a rebuild of dependents; an implementation
  change does not).
- **Content-addressed, query-based compiler** (rust-analyzer/Salsa style) where every artefact is
  cached by hash and can be shared through a remote cache.
- **Two backends**: a fast, unoptimised backend (Cranelift-like, or a custom one) for the dev loop, and
  LLVM/MLIR for release.
- **Avoid monomorphisation blow-up by default**: dictionary passing / dynamic dispatch for generics in
  debug builds, specialisation in release builds (as Swift does with witness tables).
- **Bounded compile-time execution**: compile-time code is deterministic, sandboxed, has no I/O, and
  is cacheable ([`comptime.md`](comptime.md) §3).
- **Hot reload** of function bodies in dev builds.

**Open questions.**
- Can debug-mode dictionary passing and release-mode specialisation produce *identical semantics*,
  so the difference is purely performance?
- Target: what is the budget? (e.g., "incremental rebuild of a 1 MLOC project under 1 s").

---

### 2.15 Cross-compilation and deployment

- **State of the art.** Zig (cross-compile to anything from anything, bundled libc), Go (static
  binaries, `GOOS`/`GOARCH`).
- **Directions.** Cross-compilation as a default capability of the toolchain; static binaries;
  WebAssembly (with the component model / WASI) as a **required** first-class target, which also
  fits the capability model naturally (see §5.1, R1).

---

### 2.16 Package management and supply-chain security

**Problem.** Package managers made code reuse trivially easy, and in doing so made the dependency
graph the largest attack surface in modern software.

**State of the art.**
- **Cargo**: lockfiles, semver, workspaces, features, `cargo-vet`/`cargo-crev` for audit trails.
  The ergonomics are excellent.
- **Go modules**: minimal version selection (deterministic, no SAT solver), a public checksum database
  (`sum.golang.org`), a module proxy, and **no install scripts**.
- **Nix / Guix**: fully reproducible, content-addressed builds.
- **Deno**: runtime permissions (`--allow-net`, `--allow-read`, …).
- **Unison**: code identified by hash, so there is no "dependency hell" between versions because
  definitions are immutable.
- **Sigstore / SLSA / in-toto**: signing and provenance attestations.
- **pnpm / Yarn PnP**: content-addressed stores and strictness about phantom dependencies.

**What still hurts.**
- **Build-time code execution**: npm `postinstall`, Python `setup.py`, Rust `build.rs` and proc macros
  all run arbitrary code on the developer's machine at install or build time.
- **Ambient authority**: any dependency, however deep, can open sockets, read `~/.ssh`, or spawn
  processes. A left-pad library has the same power as your web server.
- Typosquatting, account takeover, maintainer burnout, and long-con attacks (the `xz-utils` backdoor,
  `event-stream`, `ua-parser-js`).
- Transitive dependency counts in the hundreds or thousands; nobody reviews them.
- Binary artefacts in packages that don't match the source.

**Directions.**
- **Capability-scoped dependencies.** A package declares the capabilities it needs (`net`, `fs`,
  `env`, `ffi`, `unsafe`, `spawn`, `time`, …), and **the compiler enforces it**: a JSON parser that
  imports `net` fails to build. Capabilities are passed as values (object-capability style), so a
  library only gets what its caller hands it. (Prior art: E, Austral, WASI, Deno.)
- **Capability diffs on upgrade**: "`foo` 1.4 → 1.5 now requests `net`" is a red flag shown by the tool.
- **No arbitrary code at install time.** Build logic is declarative or runs in a WASM sandbox
  (R1, §5.1) with no network or file access beyond its inputs. Comptime code, which replaces proc
  macros, is pure and sandboxed ([`comptime.md`](comptime.md) §10).
- **Reproducible builds by default**; packages are published from source and the registry verifies
  the build.
- **Hash-pinned lockfiles, minimal version selection, transparency log.**
- **Audit trail built in** (cargo-vet style): "who reviewed this version, and was it a human or an
  agent?"
- **Small, broad standard library** to reduce trivial dependencies.

**Open questions.**
- How to handle capabilities through FFI (C code can do anything)? Probably: FFI *is* a capability
  (`ffi`/`unsafe`) and auditors treat it as high risk.
- Content-addressed definitions (Unison) vs versioned packages, or both?
- Central registry vs decentralised (Go's URLs plus proxy)?

---

### 2.17 Tooling uniformity

- **Lesson from Go and Rust.** One official formatter, test runner, doc generator, package manager,
  LSP, benchmark harness and linter, shipped with the compiler, removes endless ecosystem fragmentation.
- **Directions.** The compiler is a library and a service; every tool (formatter, LSP, agent API,
  semantic diff, docs) uses the same front end.

---

### 2.18 LLMs as first-class programmers

**Problem.** Languages assume the person who writes the code also understands it. In an LLM-heavy
workflow, the writer is a model and the human is the *reviewer, architect and approver*. Current
languages optimise for the writer and against the reviewer.

**What LLMs struggle with today.**
- **Non-local semantics**: implicit conversions, overload resolution, operator overloading, Scala
  implicits, C++ ADL, monkey patching, global state. A model (and a human) has to see far away to
  understand a single line.
- **Many ways to say the same thing**: style variation makes training data noisier and diffs bigger.
- **Opaque macros**: generated code that neither model nor reviewer can see.
- **Weak feedback loops**: compiler diagnostics written for humans, not structured for agents; slow
  builds limiting iterations.
- **Hallucinated APIs**: names that look plausible but don't exist. Fast and precise feedback is the cure.
- **Version drift**: the model learned an old API; nothing tells it the API changed.

**What humans struggle with when reviewing LLM code.**
- "What does this change *actually* do?" Side effects are invisible in signatures.
- "Is this safe?" Nothing marks which code touches the network, filesystem, or `unsafe`.
- "Why was it written like this?" Intent is lost; only the result remains.
- Big diffs from mechanical changes hide the few lines that matter.

**State of the art.**
- **Go**: one formatter, a small language, few ways to do things. LLMs write Go well.
- **Rust**: precise, structured compiler errors with suggested fixes; strong types catch hallucinations.
- **TypeScript**: types + LSP give models a tight feedback loop.
- **Dafny / Lean / Verus**: specifications that a model can try to satisfy and a checker can verify.
- **Language servers / tree-sitter**: queryable structure for tooling.

**Directions.**
- **Canonical form.** Exactly one formatting; ideally the source is a printed view of a canonical
  AST. No style debates, minimal diffs.
- **Local reasoning.** You can understand a line by reading it plus the signatures it calls. No
  implicit conversions, limited overloading, hygienic and *visible* metaprogramming (the expansion can
  always be shown).
- **Effects and capabilities in signatures.** A reviewer can see at a glance that a function does
  I/O, allocates, can fail, can suspend, or runs on a GPU (§2.1). Illustration only:
  ```
  fn fetch_user(id: UserId) -> User ! {net, fail(HttpError)}
  ```
- **Contracts and properties next to code.** Pre/post-conditions, invariants and property-based tests
  are first-class syntax, so a model can be asked to "make these pass" and a human can review the
  *spec* instead of the implementation.
- **Machine-first compiler API.** Diagnostics as structured data with stable codes and machine-applicable
  fixes; a queryable semantic model (types, effects, call graph) exposed as a service; fast incremental
  checking.
- **Intent and provenance.** First-class places for *why* (design notes, linked issues) and *who/what*
  (human, model, model version, prompt reference) that survive refactors, unlike comments, which drift.
- **Stable, small surface syntax** that changes rarely, so training data stays valid. Editions handle
  evolution (§2.20).
- **Explainability tooling**: `explain` commands that show the expanded, desugared, effect-annotated
  view of any expression.

**Open questions.**
- How verbose can a language be before humans reject it, given they will mostly *read* it? Is
  explicit-but-verbose acceptable if humans rarely type it?
- Should provenance live in the source, in version control metadata, or in a sidecar?
- Can the language be designed so that type checking and effect checking are fast enough to run after
  every token an agent generates (constrained decoding)?

---

### 2.19 Diffs, code review and version control

**Problem.** Version control operates on lines of text, but programs are trees with meaning. Reviewers
see noise (formatting, renames, moved code) and miss signal (a new side effect, a changed error path, a
performance regression). As agents produce more code, review becomes the bottleneck.

**State of the art.**
- **Git**: universal, fast, but line-based and snapshot-based; merges are textual.
- **Difftastic / SemanticDiff / GumTree**: AST-aware diffs that ignore formatting and track moves.
- **Unison**: code is stored by hash; names are metadata. Renames are free and never conflict.
- **Pijul / Darcs**: patch theory, so patches commute and merges are more predictable.
- **Jujutsu (jj)**: first-class conflicts, working copy as a commit, easy history rewriting.
- **Stacked diffs** (Phabricator, Gerrit, Graphite, Sapling): small, reviewable, dependent changes.
- **Refactoring-aware review tools** (RefactoringMiner, IDE "show refactors") detect renames and moves.

**What still hurts.**
- A rename across 200 files is a 200-file diff.
- Reformatting obscures history (`git blame` breaks).
- Diffs show *textual* change, not *behavioural* change.
- Reviewers can't easily ask "which public APIs changed? which functions gained side effects? did
  allocation or complexity change?"
- Merge conflicts on code that is semantically independent.

**Directions.**
- **Canonical AST as the diff unit.** Since formatting is canonical (§2.18), whitespace and style
  diffs disappear. Tooling diffs trees, not lines.
- **Refactors as intents.** A rename, move or extract-function is recorded as a single operation
  ("rename `foo` → `bar`") that the tool replays, rather than as hundreds of line edits. Reviewers
  approve the intent, not every touched line.
- **Semantic change summary per PR**, computed by the compiler:
  - public API / signature changes
  - **effect and capability deltas** ("`parse()` now performs `fs`")
  - new `unsafe` / FFI
  - new dependencies and their capabilities
  - contract changes
  - optional: allocation / complexity / benchmark deltas
- **LLM-written review summaries that are checked** against the computed semantic diff, so the summary
  cannot claim something the diff does not show.
- **Definition-level history**: blame and history per function/type, surviving moves and renames.
- Work *with* Git (it is not going away), but store enough structure (e.g., stable node IDs, rename
  maps) to enable all of the above. Consider Unison-style hashing of definitions as an internal model.

**Open questions.**
- Stable identity for definitions: hash of content (Unison) vs persistent IDs vs name-based?
- Is a custom VCS layer justified, or can everything be done as Git metadata + tooling?

---

### 2.20 Language evolution

- **Problem.** Languages either freeze (C) or break their ecosystems (Python 2→3).
- **State of the art.** Rust editions (per-crate, interoperable); Go 1 compatibility promise plus
  `go fix`; Swift's evolution process.
- **Directions.** Editions from day one; **automated migrations** as part of the toolchain, which agents
  can run and humans can review as intents (§2.19).

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

### 5.1 Firm requirements (decided)

These are no longer open for debate. Every other design decision must be compatible with them.

**R1. WebAssembly is a first-class compilation target.**

WASM is not a port done later. It is a tier-one target from the first release, tested in CI like
native targets. Reasons:
- **Multi-tier programs (§2.10)** need it: the browser tier compiles to WASM, and edge/serverless
  platforms increasingly run WASM components.
- **It fits the capability model (§2.16, §2.1).** A WASM module has no ambient authority; it can only
  call the imports it is given. WASI and the Component Model are object-capability systems.
- **It is the sandbox for untrusted code**: build-time logic and compile-time evaluation (§2.14, §2.6),
  package build steps (§2.16), plugins, and even imported C libraries compiled to WASM and isolated
  (as RLBox does in Firefox, §2.13).
- **Portability and determinism**: one artefact that runs anywhere, with deterministic execution
  available for simulation testing (§2.11).

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
- **Concurrency without function colouring (§2.8)** needs suspendable stacks. *Revised after checking
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
  tail calls) where available; error handling (§2.3) must not depend on unwinding.
- **GPU (§2.9)** in the browser goes through WebGPU/WGSL, so kernel lowering needs a WGSL path.
- **FFI (§2.13)** on WASM means C code compiled to WASM and linked as components, not native calls.

**R2. Small core; functionality lives in mods.**

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

**R3. No crash from an unhandled error.**

Every error the language knows about (the `fail` effect / `Result`) must be handled. An unhandled
error at any entry point (`main`, task bodies, exported/FFI functions, tier endpoints, handlers) or a
silently dropped result is a **compile-time error**. There are **no fire-and-forget tasks**: every
task is awaited by its enclosing scope. Out of scope for R3: traps such as out of memory, stack
overflow, integer division by zero and out-of-bounds indexing, which are crashes as in Rust.

Details: [`error-handling.md`](error-handling.md).

### 5.2 Non-goals (draft)

- Maximum syntactic novelty. Familiar syntax wins unless there is a strong reason.
- Replacing every language. Scripting, notebooks and quick glue may remain better served elsewhere
  (open question: is a REPL/script mode desirable?).
- Full C++ template interop.
- A custom VCS that replaces Git (tooling on top of Git first).
- A cloud platform, orchestrator or service mesh. Moddil emits artefacts and manifests for existing
  ones.
- Transparent remote calls.
- Runtime reflection / dynamic metaprogramming.

---

## 6. Prior-Art Matrix

Legend: ✓ strong · ◐ partial · ✗ weak/absent. These are rough judgements, intended as a starting
point for debate.

| Language | CPU concurrency | GPU | Memory / resources | Build times | Packages / supply chain | LLM-friendliness | Semantic diffs | C/C++/Rust interop | Distributed / multi-tier |
|---|---|---|---|---|---|---|---|---|---|
| C | ✗ manual | ◐ via CUDA C/OpenCL | ✗ manual | ◐ fast-ish, headers | ✗ none standard | ◐ simple but unsafe | ✗ | ✓ *is* the ABI | ✗ |
| C++ | ◐ libraries, no race safety | ✓ CUDA/SYCL/HIP | ◐ RAII unenforced | ✗ slow | ✗ fragmented (vcpkg/Conan) | ✗ huge, non-local semantics | ✗ | ✓ C / ◐ Rust | ✗ |
| Rust | ✓ Send/Sync; ◐ async colouring | ◐ emerging | ✓ ownership; ◐ learning curve | ✗ slow | ✓ Cargo; ◐ build.rs/proc macros | ✓ great diagnostics | ✗ | ✓ C / ◐ C++ | ◐ WASM client + server, separate builds |
| Go | ✓ goroutines; ✗ race safety | ✗ | ◐ low-pause GC | ✓ fast | ✓ MVS, checksum DB | ✓ small, one format | ✗ | ◐ cgo overhead | ◐ Service Weaver |
| Zig | ◐ manual | ◐ SPIR-V/PTX targets emerging | ◐ explicit allocators, no safety proof | ✓ fast, incremental | ◐ young | ◐ explicit, little magic | ✗ | ✓ best C story | ✗ |
| Swift | ✓ structured concurrency, actors | ◐ Metal (separate language) | ◐ ARC, atomic costs | ◐ | ◐ SwiftPM | ◐ | ✗ | ✓ C / ✓ C++ | ◐ distributed actors |
| Mojo | ◐ | ✓ MLIR-based targeting | ◐ ownership | ? | ◐ young | ◐ Python-like | ✗ | ◐ Python, C | ✗ |
| Odin / Jai | ◐ | ✗ | ◐ allocators, arenas | ✓ very fast | ◐ | ◐ | ✗ | ✓ C | ✗ |
| Erlang / Elixir | ✓ processes, supervisors | ✗ | ✓ per-process GC | ◐ | ✓ Hex | ◐ | ✗ | ◐ NIFs are risky | ✓ native distribution; ◐ trusted clusters only |
| Haskell / OCaml / Koka | ◐ / ✓ OCaml 5 effects | ◐ Futhark-like DSLs | ◐ GC; ✓ Perceus (Koka) | ◐ | ◐ | ◐ strong types, steep learning | ✗ | ◐ | ◐ Eliom (OCaml), HasChor (Haskell) |
| Unison | ◐ abilities | ✗ | ◐ GC | ✓ cache by hash | ✓ content-addressed | ◐ | ✓ by design | ✗ | ✓ ships code by hash |
| Carbon | ? | ? | ◐ planned safety | ? | ? | ? | ✗ | ✓ C++ (goal) | ✗ |
| Vale | ◐ | ✗ | ✓ generational refs | ? | ? | ◐ | ✗ | ◐ | ✗ |
| Pony | ✓ ref capabilities | ✗ | ✓ ORCA GC | ◐ | ◐ | ✗ niche | ✗ | ◐ C | ✗ |
| Ballerina / Winglang | ◐ | ✗ | ◐ GC | ◐ | ◐ | ◐ | ✗ | ✗ | ✓ network/cloud-aware by design |

**Observation:** no row is strong across the board, and **no language at all** is strong in "semantic
diffs" or treats "LLM-friendliness" as a stated design goal. Distributed programming is well served
only in niches (Erlang, Unison, Ballerina/Wing, research multitier and choreographic languages), and
none of those combine it with systems-level performance or GPU support. Those three columns, together
with effects and capabilities as a unifying mechanism, are where the opportunity is largest.

---

## 7. Open Questions and Next Steps for Moddil

**Strategic questions**
1. **Primary target domain** for v0? Candidates: systems / infrastructure services; ML and
   numeric (GPU-heavy); embedded; general applications. The choice reshapes priorities (e.g., GPU is
   central for ML, peripheral for services).
2. **Who is the first user?** An agent-assisted team writing services? A numerics group? The answer
   sets the bar for ergonomics vs guarantees.
3. **Adoption path**: embed in existing C/C++/Rust codebases one module at a time (§2.13)?
4. **Is multi-tier deployment (§2.10) a v0 goal**, or something the design must *allow* but only
   delivers later? Even if it comes later, placement has to be in the type system from the start.
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
9. Canonical AST storage: text with canonical printing, or a structured store (Unison-style) with text
   as a view?

**Documents written** (see the document map at the top)
- `specs/memory-and-resources.md`: ownership, borrows and views, rc, linear resources, zero-copy I/O (§2.2)
- `specs/concurrency.md`: CPU part — sharing rules, structured concurrency, runtime contract (§2.8)
- `specs/mods.md`: mods, core/mod boundary, standard-mod catalogue (R2)
- `specs/error-handling.md`: `?`, try blocks, R3, outcome classes, traces (§2.3); survey of prior art
- `specs/comptime.md`: compile-time evaluation, reflection, typed derives, tagged literals (§2.6)
- `specs/bootstrap.md`: stage 0, self-hosting, backends, WASM support status (R1)
- `specs/examples/`: the four reference systems

**Documents still to write**
- `specs/effects-and-capabilities.md`: the unifying mechanism (§2.1). Natural next step: both
  written specs lean on effects (suspension, scheduling, implicit context).
- `specs/concurrency.md`, GPU part: kernels, memory spaces, launches as scoped tasks (§2.9)
- `specs/packages-and-supply-chain.md`: §2.16
- `specs/agent-interface.md`: compiler-as-a-service for LLMs (§2.18)
- `specs/semantic-vcs.md`: diffs and review (§2.19)
- `specs/interop.md`: §2.13, including how C pointers map onto ownership, borrows and rc
- `specs/distribution.md`: placement, the `remote` effect, wire contracts, topology manifests (§2.10)
- `specs/requirements.md`: firm requirements (R1 WASM, R2 small core, …) and non-goals, grown from §5
- `specs/wasm.md`: target details: WASI/Component Model mapping, browser/JS interop, stack switching

---

## Appendix: References and Prior Art

*To be expanded with links during research.*

- **Concurrency:** Nystrom, "What Color is Your Function?" (2015); N. J. Smith, "Notes on structured
  concurrency, or: Go statement considered harmful" (2018); Pony reference capabilities and the ORCA
  GC; Java JEP 444 (virtual threads) and JEP 453/480 (structured concurrency); Rust
  `std::thread::scope` and rayon.
- **GPU:** Ragan-Kelley et al., *Halide* (PLDI 2013); Tillet et al., *Triton* (MAPL 2019); Henriksen
  et al., *Futhark* (PLDI 2017); MLIR (Lattner et al., CGO 2021); SYCL specification.
- **Memory:** Reinking et al., *Perceus: Garbage Free Reference Counting with Reuse* (PLDI 2021);
  Vale generational references; Hylo (Val) mutable value semantics and parameter conventions; Mojo
  argument conventions and origins; Swift non-escaping closures; Rust Polonius; Choi, Shull,
  Torrellas, *Biased Reference Counting* (PACT 2018) and CPython PEP 703; Austral and Linear
  Haskell (linear types); Rust `bytes::Bytes` and `yoke`; FlatBuffers, Cap'n Proto, SBE.
- **Build systems:** Mokhov, Mitchell, Peyton Jones, *Build Systems à la Carte* (ICFP 2018); Salsa;
  Bazel / Buck2.
- **Supply chain:** Go modules and Cox's "Our Software Dependency Problem" (2019); SLSA; Sigstore;
  `cargo-vet`; analysis of the `xz-utils` backdoor (CVE-2024-3094); Miller, *Robust Composition*
  (object capabilities, 2006); Austral language.
- **Effects:** Leijen, *Koka* papers; Effekt language; OCaml 5 effect handlers; Unison abilities.
- **Version control:** Pijul theory of patches; Jujutsu; Difftastic; GumTree (Falleri et al., 2014);
  Unison codebase model.
- **Verification:** Dafny; Verus; SPARK; Liquid Haskell; Kani.
- **Null and invalid states:** Hoare, "Null References: The Billion Dollar Mistake" (QCon 2009);
  Swift `Optional`; Kotlin null safety; TypeScript `strictNullChecks`; Dart sound null safety; C# 8
  nullable reference types; Java Project Valhalla null-restricted types and JSpecify; Rust niche
  optimisation and pattern types; F* and Liquid Haskell refinement types; typestate (Strom and
  Yemini, 1986).
- **Metaprogramming:** Zig `comptime`; Rust proc macros, `syn`, `serde`, `sqlx`; C++26 static
  reflection (P2996); D CTFE; Jai `#run`/`#insert`; Nim macros; Swift macros; Scala 3 inline and
  quotes; JavaScript tagged templates.
- **Interop:** Zig `@cImport` / `zig cc`; Swift C++ interoperability; Carbon language goals; `cxx`;
  `crubit`; Rust `crabi` proposal.
- **Determinism:** FoundationDB simulation testing; TigerBeetle VOPR; `rr`.
- **WebAssembly:** Haas et al., *Bringing the Web up to Speed with WebAssembly* (PLDI 2017); WASI and
  the Component Model / WIT (Bytecode Alliance); stack-switching and JSPI proposals; WASM GC
  proposal; RLBox (sandboxing C libraries via WASM in Firefox).
- **Distribution:** Waldo, Wyant, Wollrath, Kendall, *A Note on Distributed Computing* (1994);
  Deutsch, "Fallacies of distributed computing"; Weisenburger, Wirth, Salvaneschi, *A Survey of
  Multitier Programming* (ACM Computing Surveys, 2020); Montesi, *Choreographic Programming*
  (PhD thesis, 2013) and Choral; HasChor; Links; Ur/Web; Eliom; ScalaLoci; Service Weaver
  ("Towards Modern Development of Cloud Applications", HotOS 2023); Orleans; Winglang; Ballerina;
  Temporal; Hydro (Berkeley).

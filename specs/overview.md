# Moddil Overview: Why a New Language?

> **Status:** living index for **Moddil**. This is not a specification and makes no syntax commitments.
> It states the thesis, lists the problem areas with a short summary of each, and points to the
> subdocument that holds the detail. The subdocuments are authoritative.

## Index

Status legend:

| Marker                 | Meaning                                                                                                                    |
| ---------------------- | -------------------------------------------------------------------------------------------------------------------------- |
| ⚪ **not started**     | No content beyond a heading                                                                                                |
| 🔴 **superficial**     | A brainstorm or stub; little is decided                                                                                    |
| 🟠 **major questions** | A direction exists, but open questions could still change the core design, or the document is a proposal not yet confirmed |
| 🟡 **minor questions** | Decided; the remaining questions are details                                                                               |
| 🟢 **complete**        | Decided, with no open questions                                                                                            |

Rule of thumb: 🟠 if an open question could change language semantics or a requirement, or the
document is still a proposal; 🟡 if what is left is detail, tooling or operations.

**Contents** (each row links to its section; detail documents are authoritative)

| Section                                                                                | Status             | Detail                                                                                                                                              |
| -------------------------------------------------------------------------------------- | ------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------- |
| [Preamble](#preamble)                                                                  | —                  | —                                                                                                                                                   |
| [Thesis](#thesis)                                                                      | —                  | —                                                                                                                                                   |
| [Effects and capabilities](#effects-and-capabilities-as-a-unifying-idea)               | 🟠 major questions | [`effects-and-capabilities.md`](effects-and-capabilities.md)                                                                                        |
| [Memory and resource management](#memory-and-resource-management)                      | 🟡 minor questions | [`memory-and-resources.md`](memory-and-resources.md)                                                                                                |
| [Error handling](#error-handling)                                                      | 🟠 major questions | [`error-handling.md`](error-handling.md)                                                                                                            |
| [Null and invalid states](#null-and-invalid-states)                                    | 🟡 minor questions | [`null-safety.md`](null-safety.md)                                                                                                                  |
| [Numerics and data](#numerics-and-data)                                                | 🟠 major questions | [`numerics.md`](numerics.md), [`units.md`](units.md)                                                                                                |
| [Metaprogramming](#metaprogramming)                                                    | 🟡 minor questions | [`comptime.md`](comptime.md)                                                                                                                        |
| [Contracts and verification](#correctness-tooling-contracts-and-verification)          | 🟠 major questions | [`correctness.md`](correctness.md)                                                                                                                  |
| [Concurrency across CPUs](#concurrency-across-cpus)                                    | 🟡 minor questions | [`concurrency.md`](concurrency.md), [`closures.md`](closures.md)                                                                                    |
| [GPUs and accelerators](#concurrency-across-gpus-and-accelerators)                     | 🟡 minor questions | [`accelerators.md`](accelerators.md)                                                                                                                |
| [Distributed and multi-tier programs](#distributed-and-multi-tier-programs)            | 🟡 minor questions | [`distribution.md`](distribution.md)                                                                                                                |
| [Determinism and reproducibility](#determinism-and-reproducibility)                    | 🟠 major questions | [`determinism.md`](determinism.md)                                                                                                                  |
| [Observability and debuggability](#observability-and-debuggability)                    | 🟡 minor questions | [`observability.md`](observability.md)                                                                                                              |
| [Interoperability with C and Rust](#interoperability-with-c-and-rust)                  | 🟡 minor questions | [`interop.md`](interop.md)                                                                                                                          |
| [Build times](#build-times)                                                            | 🟠 major questions | [`build-times.md`](build-times.md)                                                                                                                  |
| [Cross-compilation and deployment](#cross-compilation-and-deployment)                  | 🟡 minor questions | [`targets.md`](targets.md), [`freestanding.md`](freestanding.md)                                                                                    |
| [Packages and supply-chain security](#package-management-and-supply-chain-security)    | 🟡 minor questions | [`packages-and-supply-chain.md`](packages-and-supply-chain.md)                                                                                      |
| [Tooling uniformity](#tooling-uniformity)                                              | 🟡 minor questions | [`toolchain.md`](toolchain.md), [`docs-and-tests.md`](docs-and-tests.md)                                                                            |
| [LLMs as first-class programmers](#llms-as-first-class-programmers)                    | 🟡 minor questions | [`agent-interface.md`](agent-interface.md)                                                                                                          |
| [Diffs, code review and version control](#diffs-code-review-and-version-control)       | 🟡 minor questions | [`semantic-vcs.md`](semantic-vcs.md)                                                                                                                |
| [Language evolution](#language-evolution)                                              | 🟡 minor questions | [`evolution.md`](evolution.md)                                                                                                                      |
| [Surface syntax and paradigms](#surface-syntax-and-paradigms)                          | 🟡 minor questions | [`syntax.md`](syntax.md), [`paradigms.md`](paradigms.md)                                                                                            |
| [Type system core](#type-system-core)                                                  | ⚪ not started     | —                                                                                                                                                   |
| [Modules, visibility and program structure](#modules-visibility-and-program-structure) | ⚪ not started     | —                                                                                                                                                   |
| [`unsafe` and the safety boundary](#unsafe-and-the-safety-boundary)                    | ⚪ not started     | —                                                                                                                                                   |
| [Text and strings](#text-and-strings)                                                  | ⚪ not started     | —                                                                                                                                                   |
| [Values: copy, clone and equality](#values-copy-clone-and-equality)                    | ⚪ not started     | —                                                                                                                                                   |
| [Standard library shape](#standard-library-shape)                                      | ⚪ not started     | —                                                                                                                                                   |
| [Data interchange and persistence](#data-interchange-and-persistence)                  | ⚪ not started     | —                                                                                                                                                   |
| [UI, mobile and the browser](#ui-mobile-and-the-browser)                               | ⚪ not started     | —                                                                                                                                                   |
| [Program lifecycle](#program-lifecycle)                                                | ⚪ not started     | —                                                                                                                                                   |
| [Cross-cutting themes](#cross-cutting-themes)                                          | —                  | —                                                                                                                                                   |
| [Tensions and trade-offs](#tensions-and-trade-offs)                                    | —                  | —                                                                                                                                                   |
| [Firm requirements (R1–R3)](#firm-requirements-decided)                                | 🟠 major questions | [`requirements.md`](requirements.md), [`bootstrap.md`](bootstrap.md) (R1), [`mods.md`](mods.md) (R2), [`error-handling.md`](error-handling.md) (R3) |
| [Non-goals](#non-goals-draft)                                                          | 🟡 minor questions | [`requirements.md`](requirements.md)                                                                                                                |
| [Prior-art matrix](#prior-art-matrix)                                                  | reference          | [`prior-art.md`](prior-art.md)                                                                                                                      |
| [Open questions and next steps](#open-questions-and-next-steps-for-moddil)             | 🟠 major questions | [`examples/README.md`](examples/README.md) (reference systems)                                                                                      |
| [Working on these specs](#working-on-these-specs)                                      | —                  | [`tools/check_spec_links.py`](../tools/check_spec_links.py)                                                                                         |

---

## Preamble

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

**Syntax is deliberately not where Moddil innovates:** its surface syntax is Rust's, with a short
list of justified deltas ([`syntax.md`](syntax.md)). Snippets in every spec follow it.

Moddil should not be a remix of syntax. It is only justified if the *combination* of problems below
cannot be solved well by evolving an existing language, either because the solutions need guarantees
that existing languages cannot add without breaking their ecosystems, or because the target user has
changed.

**What this document is not:** a grammar, a feature list, or a promise. Where a snippet appears it only
illustrates a point.

---

## Thesis

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

## Problem Areas

The areas are ordered from the language core outward: semantics (effects, memory, errors, types, numerics, metaprogramming, correctness), then execution (CPU, GPU, distributed, determinism, observability), then the surrounding system (interop, builds, deployment, packages, tooling), and finally the human and agent workflow (LLMs, review, evolution). The order reflects dependencies, not priority: every area matters equally. Effects come first because most other areas build on them. Areas 2.22–2.30 came out of a gap review and are not started yet; each will get a subdocument when it is worked on.

Each area below gives only:

- its **status** and a link to the **subdocument**, which holds the problem statement, prior art,
  decisions and directions in full;
- a short **summary** of the problem and of what is decided or proposed;
- the **open questions**, copied from the subdocument. The subdocument's list is the master copy.

---

### Effects and capabilities as a unifying idea

🟠 **major questions** · Detail: [`effects-and-capabilities.md`](effects-and-capabilities.md)

Signatures say what a function takes and returns, but not what it *does*: I/O, suspension, failure,
nondeterminism, GPU work, unsafe code. One mechanism, effects plus capability values, underlies most
other areas:

- suspension without function colouring ([Concurrency across CPUs](#concurrency-across-cpus)) and GPU-safe kernel subsets ([Concurrency across GPUs and accelerators](#concurrency-across-gpus-and-accelerators));
- resource safety with linear types ([Memory and resource management](#memory-and-resource-management)) and visible failure ([Error handling](#error-handling));
- capability-scoped dependencies ([Package management and supply-chain security](#package-management-and-supply-chain-security)) and reviewable signatures ([LLMs as first-class programmers](#llms-as-first-class-programmers), [Diffs, code review and version control](#diffs-code-review-and-version-control));
- handlers that swap real I/O for fakes in tests ([Determinism and reproducibility](#determinism-and-reproducibility)) and provide implicit context.

Decided:

- **Effects are interfaces; handlers are scoped implementations** (`with time = sim.clock(n) { … }`),
  inherited by spawned tasks. A hybrid design: scoped handlers by default, **second-class capability
  values** when two instances are needed at once.
- **Handler tiers**: user effects get function-like (tail-resumptive) and abortive handlers;
  one-shot continuations are reserved for core and standard effects (`suspend`, generators); never
  multi-shot, which would duplicate linear values.
- **Authority comes only from root capabilities** that `main` receives; libraries perform, wrap and
  narrow effects but never create authority. No ambient authority.
- **Inferred inside a module, explicit on `pub` signatures**, with **no aliases**; effects are coarse,
  with finer limits from narrowing handlers. Allocation, bugs and non-termination are not effects.
- **Unannotated closure parameters are implicitly effect-polymorphic.**
- Drop, comptime, kernels, workflows and freestanding profiles are all effect-set constraints.
- Statically known handlers compile to direct calls.

**Open questions** (same list as [`effects-and-capabilities.md`: Open questions](effects-and-capabilities.md#open-questions))

- Without aliases, do public signatures of high-level functions (a request handler with `net`,
  `fs`, `time`, `rand`, `suspend` and `fail(…)`) become too long? Revisit once the reference
  examples are written in Moddil.
- Should generators (`yield`) be a standard one-shot effect, and should user effects ever get
  one-shot continuations?
- Is `spawn` a separate effect from `suspend`, or implied by it?
- How exactly are effect requirements and handler stacks surfaced, so that dynamic handler lookup is
  never surprising (LSP, `explain`)?

---

### Memory and resource management

🟡 **minor questions** · Detail: [`memory-and-resources.md`](memory-and-resources.md)

Every existing approach trades something away: manual management is unsafe, tracing GC costs pauses
and memory, RAII is unenforced, reference counting is slow when atomic, and Rust's lifetimes leak
into APIs. Most languages also manage only memory, not files, sockets, locks or transactions.

Decided:

- **Unique ownership and RAII by default** for every resource, enforced by the compiler. Drop is
  deterministic, never suspends and never fails.
- **Second-class borrows with no lifetime annotations**: parameters are `&T`, `&mut T` or `T` (move),
  plus *view types* for structs and containers that hold borrows.
- **`.to_rc()` is the only way to share a pointer.** Contents are immutable, and biased reference
  counting keeps it nearly free on the owning thread.
- **Affine by default, linear on request**: transactions and fallible `close` must be consumed.
- **Zero-copy I/O** by moving owned buffers and parsing through views.

**Open questions** (same list as [`memory-and-resources.md`: Open questions](memory-and-resources.md#open-questions))

- Exact syntax for parameter modes, views and `from`.
- Allocators: how does the allocator capability (Zig-style) interact with `.to_rc()`
  and arenas? Does an `Rc` remember its allocator? (Proposed answer in
  [`freestanding.md`: Memory without an operating system](freestanding.md#memory-without-an-operating-system).)
- Should `Weak` be core or a mod? (It needs header support, so probably core.)
- Linear values on error paths: what is the least noisy explicit handling?
- Can the borrow check give diagnostics good enough for LLM agents ([LLMs as first-class programmers](#llms-as-first-class-programmers)) — e.g.
  machine-applicable "insert `.to_rc()` here" fixes?
- Is there still a place for an optional per-task/per-actor collector, or does this model make it
  unnecessary?

---

### Error handling

🟠 **major questions** · Detail: [`error-handling.md`](error-handling.md)

Exceptions hide control flow, error codes get ignored, `Result` types are noisy, and panics are
coarse. Composing error types across libraries is painful.

Decided:

- Rust-style `?` and try blocks; errors do not unwind.
- **R3** ([Firm requirements (decided)](#firm-requirements-decided)): no unhandled errors, checked at every entry point.
- A clear split between recoverable errors and bugs (traps).
- **Cleanup cannot fail silently or fatally**: drop has an empty effect set; cleanup that can fail
  is a linear type with an explicit `close`/`commit`; a second error on an error path is
  *suppressed* onto the primary error, never lost.

Still a direction: errors as an effect (`fail(E)`) with inferred error sets.

**Open questions** (same list as [`error-handling.md`: Open questions](error-handling.md#open-questions))

1. Are explicit discards of errors allowed, and do they need a reason ([`error-handling.md`: Entry points must handle everything (R3)](error-handling.md#entry-points-must-handle-everything-r3))?
2. Do failed contract assertions trap or raise an error ([`error-handling.md`: Three outcome classes](error-handling.md#three-outcome-classes))? (Recommendation: trap; see
   [`correctness.md`: Decisions to lock early](correctness.md#decisions-to-lock-early).)
3. Task scopes: first error and cancel siblings, or collect all errors ([`error-handling.md`: Tasks and concurrency](error-handling.md#tasks-and-concurrency))?
4. Can a trap terminate only its task rather than the process, and how are its resources freed ([`error-handling.md`: Performance: why C++ exceptions are slow, and why Moddil's errors aren't](error-handling.md#performance-why-c-exceptions-are-slow-and-why-moddils-errors-arent))?
   If so, linear types would also need an infallible, non-suspending `abandon` for that path (e.g. a
   transaction drops its connection so the database rolls back). Cancellation does not need it
   ([`concurrency.md`: Cancellation and shutdown](concurrency.md#cancellation-and-shutdown)).
5. Should a resumable recovery effect ("retry"/"default") exist as a separate effect?
6. How is context attached to structured errors ([`error-handling.md`: Error types](error-handling.md#error-types))?
7. Is `Option` part of the same machinery (`?` on `Option`, as in Rust)?
8. Should errors from the `remote` effect ([Distributed and multi-tier programs](#distributed-and-multi-tier-programs)) have a standard shape (timeout, unreachable,
   incompatible version), and how are errors serialised across tiers?
9. Checked-exception fatigue (Java) vs inference: how much of an error set should be inferred?

---

### Null and invalid states

🟡 **minor questions** · Detail: [`null-safety.md`](null-safety.md)

Null references are the best-known case of a wider problem: types that admit values the domain does
not (unchosen zero values, unchecked ranges, mixed-up IDs, objects used in the wrong state).

Decided:

- **No null in safe code.** Absence is `Option<T>` (no `T?` sugar), an ordinary sum type in a mod,
  with `if let`, `let … else`, `?` (propagates, as in Rust), `try { a?.b?.c }` for chaining within an
  expression, and `unwrap_or`. There is no implicit unwrap.
- **Absence is data; failure is an effect**, so `Option` is the only "maybe" type in everyday code.
- **Guaranteed niches**: `Option<&T>`, `Option<Rc<T>>` and range-typed integers cost no extra space.
- **Definite initialisation**: no zero values and no uninitialised reads.
- **Beyond null**: cheap newtypes, pattern/range types, refinement types, and typestate through
  linear types.

**Open questions** (same list as [`null-safety.md`: Open questions](null-safety.md#open-questions))

- ~~What does postfix `?` mean?~~ **Resolved:** it propagates, as in Rust; chaining within an
  expression is `try { a?.b?.c }` ([`null-safety.md`](null-safety.md)).
- How much refinement checking is static vs dynamic, and what does it cost in compile time ([Build times](#build-times))?
- Layout of nested options (`Option<Option<T>>`): how far should niche packing go?

---

### Numerics and data

🟠 **major questions** · Detail: [`numerics.md`](numerics.md), [`units.md`](units.md) (units of measure)

Silent integer overflow, undefined behaviour, unsigned-index underflow, lossy conversions,
floating-point surprises, floats used for money, and no units.

- Decided ([`numerics.md`](numerics.md)):
  - **integer overflow traps in every build**; wrapping is explicit (`+%`, `Wrapping<T>`);
  - **lengths and indices are a signed `int`** (i64 on every target);
  - **exact literals** (Go-style), checked when they get a type;
  - **only lossless widening is implicit**; lossy conversions are named, and there is no `as`;
  - strict IEEE floats with `total_cmp` and a totally ordered `Finite` refinement;
  - **`Fixed<S>`** (scale in the type) and `BigDecimal` for money;
  - `Simd<T, N>` in the core; endian types, packed structs and a comptime `Soa<T>` for layout.
- Proposed: F#-style units of measure (`100<cm>`, `f64<m/s>`) with an open set of dimensions
  ([`units.md`](units.md)).

**Open questions** (same list as [`numerics.md`: Open questions](numerics.md#open-questions))

- How are units written on parameterised types (`Fixed<4>` in US dollars: `Fixed<4><USD>`?)?
- Should `Fixed` and `BigDecimal` have a default rounding mode, or must it always be stated?
- Should overflow checks ever be disabled per mod or per block (an `unchecked` region), or only
  replaced by explicit `+%`?

**Open questions** (same list as [`units.md`: Open questions](units.md#open-questions))

1. Confirm the lexing rule: no space between a literal and its `<unit>`.
2. Glob imports of unit packs ([`units.md`: Imports](units.md#imports)).
3. Angles: are radians and degrees units of a dimensionless-but-distinct "angle" kind?
4. Rational exponents (`m^(1/2)`), needed for some formulas: allow, or only integers?
5. The common-unit rule for unannotated mixed expressions: finer unit, or left operand?
6. Integer representations with fractional scales (for example `i64<mm>` from `cm`): exact-only
   conversion, or a fixed-point scale in the type?

---

### Metaprogramming

🟡 **minor questions** · Detail: [`comptime.md`](comptime.md)

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

**Open questions** (same list as [`comptime.md`: Open questions](comptime.md#open-questions))

- **Where does comptime run?** Options:
  - an interpreter inside the compiler (Zig; fast start, slow for heavy work);
  - compiling comptime code to WASM and running it in Wasmtime (fast for heavy work; type values
    must cross the boundary);
  - a hybrid of the two.
- **Allocation:** what allocator does comptime use, and how does comptime data become static data in
  the binary?
- **Budget:** what is the default fuel and the per-mod compile-time budget ([Build times](#build-times))?
- **Ordering:** can one derive depend on another's output (e.g. `Hash` relying on `Eq`), and how are
  cycles detected?
- **Reflection and privacy:** exactly which private details may a derive in another mod see?
- **IDE responsiveness:** comptime must be incremental enough to run on every keystroke for agents
  ([LLMs as first-class programmers](#llms-as-first-class-programmers)).
- **Construction API:** how expressive is it, and does it stay stable across editions ([Language evolution](#language-evolution))?

---

### Correctness tooling: contracts and verification

🟠 **major questions** · Detail: [`correctness.md`](correctness.md)

Tests show the presence of bugs, not their absence, and specifications live in comments or nowhere.

Proposed:

- **v1**: contracts as syntax (checked at run time in debug builds), validated newtypes, property
  testing with contracts as oracles, fuzzing, mutation testing, a Miri-style checker and
  interleaving tests.
- **Designed for now, built later**: proofs written in Moddil itself (SMT-backed), assurance levels
  per mod, refinement types and model checking.

**Open questions** (same list as [`correctness.md`: Open questions](correctness.md#open-questions))

1. Is the verifier part of the core toolchain or a mod (`moddil/verify`)?
2. Which SMT solver(s), and how are solver versions pinned for reproducible proofs?
3. How much annotation burden is acceptable before L1/L2 stop being used?
4. Should a mod be able to require proof that no overflow trap can occur?
5. How are contracts on effectful operations expressed (for example, "this handler is called at most
   once")?

---

### Concurrency across CPUs

🟡 **minor questions** · Detail: [`concurrency.md`](concurrency.md), [`closures.md`](closures.md) (lambdas)

Shared-memory parallelism is still the source of the hardest bugs, async has split ecosystems into
sync and async halves, and cancellation is an afterthought almost everywhere.

Decided:

- **Data-race freedom by construction**, built on [Memory and resource management](#memory-and-resource-management): owned values move into tasks, shared borrows
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

**Open questions** (same list as [`concurrency.md`: Open questions](concurrency.md#open-questions))

- **Preemption:** guaranteed (BEAM-style, via compiler-inserted yield points) or cooperative?
  Affects tail latency (HFT, game tick) and fairness. *Recommended* ([`concurrency.md`: Tasks, threads and executors](concurrency.md#tasks-threads-and-executors)): compiler-inserted safe
  points in loops and function entries, which `pinned` executors can disable.
- ~~**Cancellation and linear values.**~~ Resolved: cancellation is cooperative and observed only at
  suspension points, and the cancellation path must consume linear values like the error path ([`concurrency.md`: Cancellation and shutdown](concurrency.md#cancellation-and-shutdown)).
- **Deadlock:** this model prevents data races, not deadlocks. Lock ordering by type? A lint? Lock
  levels (as in some kernels)? Or steer toward message passing?
- **Async drop:** drop may not suspend ([`concurrency.md`: Construction may suspend; drop may not](concurrency.md#construction-may-suspend-drop-may-not)). Is that enough for resources whose clean shutdown needs
  I/O (e.g. flushing a TLS stream)? Probably: they become linear with an explicit `close`.
- **Priority inversion** under the scheduler effect: does the core effect interface carry priority?
- **Long-lived tasks:** what is the idiomatic shape for a server's top-level scope (accept loop with
  per-connection children) so that it reads naturally, not as a workaround for "no fire-and-forget"?

**Open questions** (same list as [`closures.md`: Open questions](closures.md#open-questions))

1. Non-local `return` and labelled `break` from non-escaping lambdas ([`closures.md`: Control flow out of a lambda](closures.md#control-flow-out-of-a-lambda)).
2. Default closure parameter mode, stated or inferred ([`closures.md`: Closure parameters: `Fn`, `FnMut`, `FnOnce`](closures.md#closure-parameters-fn-fnmut-fnonce)).
3. Showing suspension points: inlay hints or an `await` marker ([`closures.md`: Seeing suspension points](closures.md#seeing-suspension-points)).
4. Rules and syntax for methods that declare which fields of `self` they touch ([`closures.md`: Capturing parts of `self`](closures.md#capturing-parts-of-self)).

---

### Concurrency across GPUs and accelerators

🟡 **minor questions** · Detail: [`accelerators.md`](accelerators.md)

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

**Open questions** (same list as [`accelerators.md`: Open questions](accelerators.md#open-questions))

- **Autodiff:** a comptime transformation (as Slang and Enzyme do) or a feature of `moddil/tensor`?
- **Multi-GPU collectives** (all-reduce etc.): a library built on placement ([Distributed and multi-tier programs](#distributed-and-multi-tier-programs))?
- Should tensor/array syntax become core, or stay in a mod?
- How close can the CPU simulator get to bit-exact results with GPU floating point?
- WebGPU feature gaps (subgroups, f16) and their fallbacks.
- **Brands:** exact rules — how far they propagate, and how they interact with views and generics.
- Hot-plug, and device loss in the middle of a run.
- Scheduling policy when a role's device is busy or shared between processes.

---

### Distributed and multi-tier programs

🟡 **minor questions** · Detail: [`distribution.md`](distribution.md)

Almost no real system is one process. Today a feature spans several codebases and languages, held
together by hand-maintained contracts that drift, with mixed versions in production and implicit
trust boundaries.

Decided: *the language owns the program's meaning across boundaries; existing tools own where and
how it runs.*

- **Layer A, in the language** (v0): components with checked placement (`client`, `server`, …); a
  `remote` effect that never looks local; wire contracts generated from types with version-skew
  checking; `Untrusted<T>` and `Secret<T>`; whole-system runs in one process or under simulation.
- **Layer B, thin adapter mods** (v1 onwards): static binaries and OCI images; a topology file;
  adapters that emit Compose, Kubernetes, Nomad and systemd formats.
- **Infrastructure is bound, never provisioned.**
- **Layer C, never**: orchestration, autoscaling, service mesh, provisioning (non-goals, [Non-goals (draft)](#non-goals-draft)).

**Open questions** (same list as [`distribution.md`: Open questions](distribution.md#open-questions))

- Where deployed contracts are recorded: in the repository (a lock file) or a registry?
- **Choreographies**: write a multi-party protocol once and project it to each participant — when,
  and how much?
- Databases as requirements: how schema migrations are versioned and checked alongside code.
- Independent team deploys: can one team ship its component without rebuilding the world?
- How much rollout safety (e.g. canary analysis) should Moddil check, versus leave to the platform?
- How unreliable or unordered channels (game snapshots over WebTransport) are expressed in types.

---

### Determinism and reproducibility

🟠 **major questions** · Detail: [`determinism.md`](determinism.md)

Concurrent and distributed bugs are hard to reproduce.

Proposed:

- The core language is deterministic; **all nondeterminism enters through effects** (time,
  randomness, I/O, scheduling), with strict IEEE floating point.
- **Deterministic simulation testing as a built-in mode**, with seed-based replay.
- **Scoped handlers** for per-node and per-subsystem injection.
- **Record and replay**, and **reproducible builds**.

**Open questions** (same list as [`determinism.md`: Open questions](determinism.md#open-questions))

1. Should this become R4?
2. Insertion-ordered maps by default, or seeded hash order?
3. How much of `moddil/math` must be correctly rounded, and at what speed cost?
4. GPU determinism: per kernel, per mod, or global?
5. Record format and retention: per task, per request, sampled?
6. How are non-Moddil peers brought into simulation beyond WASM components?

---

### Observability and debuggability

🟡 **minor questions** · Detail: [`observability.md`](observability.md)

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

**Open questions** (same list as [`observability.md`: Open questions](observability.md#open-questions))

- **Continuous profiling** as a handler, with time attributed by effect (network, locks, GPU)?
- Sampling policy for spans and recordings.
- Recording volume and retention; encrypting recordings, or excluding secret-typed effect results.
- Replaying a whole fleet: combining recordings from several processes.
- Schema evolution of structured events across versions.
- Overhead targets per level (e.g. nanoseconds per interned event on the hot path).

---

### Interoperability with C and Rust

🟡 **minor questions** · Detail: [`interop.md`](interop.md)

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

**Open questions** (same list as [`interop.md`: Open questions](interop.md#open-questions))

- How stable is rustdoc's JSON format, and how much generator maintenance follows Rust releases?
- How far can generics cross the Rust bridge beyond explicit instantiations?
- If crABI stabilises, can the bridge use it instead of the C ABI?
- Overlay distribution and trust: who may publish overlays, and how are they audited?
- Python interop (raised in the HFT example): out of scope, or through Python's C API as a mod?
- Size of the bundled clang in the toolchain download.

---

### Build times

🟠 **major questions** · Detail: [`build-times.md`](build-times.md)

Slow builds damage the edit-compile-test loop for humans, and are worse for agents that iterate
dozens of times per task.

Proposed:

- **Language rules that decide build speed**: no cycles between mods, explicit interface artefacts,
  witness tables in debug builds and specialisation in release builds.
- **A lazy, query-based compiler** with a shared cache and no link step in dev builds.
- **Two backends**: a fast one for the dev loop, LLVM for release.
- **Live update** of a running debug program.
- **Four cache layers**, optimised dependencies by default, and **published budgets**.

**Open questions** (same list as [`build-times.md`: Open questions](build-times.md#open-questions))

1. Are cycles allowed between modules inside one mod?
2. How slow can generic code be in debug builds before it is unacceptable? Should hot generics be
   specialised even in debug builds?
3. Live update ([`build-times.md`: Live update (debug builds)](build-times.md#live-update-debug-builds)) proposes: bodies, signatures and new items live; layout changes through
   migration hooks or scope restart plus replay. Open: which layout migrations can be automatic, and
   whether production hot upgrades are ever a goal.
4. How much should trusted shared caches serve prebuilt dependency binaries, and what are the trust
   implications? (There is no registry: [`packages-and-supply-chain.md`](packages-and-supply-chain.md).)
5. Can debug-mode dictionary passing and release-mode specialisation produce *identical
   semantics*, so the difference is purely performance? *Proposed answer:* yes, if everything
   layout-dependent lives in the witness table, as Swift does ([`build-times.md`: Language rules that decide build speed](build-times.md#language-rules-that-decide-build-speed)).
6. What is the budget? *Proposed:* < 100 ms keystroke to diagnostics, < 1 s from a function-body
   edit to a runnable test regardless of project size ([`build-times.md`: Budgets](build-times.md#budgets)).

---

### Cross-compilation and deployment

🟡 **minor questions** · Detail: [`targets.md`](targets.md), [`freestanding.md`](freestanding.md) (bare metal; proposal)

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

**Open questions** (same list as [`targets.md`: Open questions](targets.md#open-questions))

- **Licensing review** of bundling macOS headers and link stubs.
- Notarising from CI without a Mac, and how Apple credentials are held.
- Scope of iOS and Android packaging.
- Which tier-2 targets (Windows on ARM, FreeBSD, RISC-V), and when they become tier 1.
- The default CPU feature level per architecture.
- Which glibc versions to ship stubs for.

**Open questions** (same list as [`freestanding.md`: Open questions](freestanding.md#open-questions))

1. Add a fifth, embedded reference example ([`freestanding.md`: Proving the profile end to end](freestanding.md#proving-the-profile-end-to-end))?
2. Should `moddil/rt-embedded` default to preemptive or cooperative scheduling?
3. Does stack-depth analysis apply to all freestanding code, or only to L1 tasks?
4. Tier assignments for bare-metal targets ([`targets.md`: Target names and tiers](targets.md#target-names-and-tiers)).
5. Floating point in interrupt handlers (lazy FPU state saving).

---

### Package management and supply-chain security

🟡 **minor questions** · Detail: [`packages-and-supply-chain.md`](packages-and-supply-chain.md)

Package managers made reuse trivially easy, and made the dependency graph the largest attack surface
in modern software: build-time code execution, ambient authority, hijacked accounts publishing
malicious versions, and transitive dependencies that nobody reviews.

Decided:

- **Plain git, nothing else.** Mods live in ordinary git repositories; no registry, mirror, checksum
  log or index owns names or is needed to build. **Names are URLs** (`github.com/x/y`), with optional
  vanity domains.
- **Versions are semver tags, pinned by hash.** The lockfile records a content hash and signing key per
  mod (trust on first use); vendoring is built in.
- **One version per major** (the major is in the path, `x/v2`), with **minimal version selection**.
  **Semver is compiler-checked** at publish and again at update.
- **Randomised cooldown**: a new version is eligible after 7–28 days, a delay derived from
  hash(project, dependency, version), so adoption is staggered like a canary. Overrides are explicit.
- **Capability-scoped dependencies**: a ceiling per mod in the manifest plus capabilities as values;
  upgrades show **capability diffs**; `ffi` and `unsafe` are high-risk capabilities.
- **No code at fetch or build time**; **the source is the release** (no tarballs); a **built-in audit
  trail** recorded in git, saying whether a human or an agent reviewed each version.
- **Local sources**: local git repositories (still hash-pinned), user-level source overrides to
  mirrors or clones, and `moddil.work` workspaces for live working copies during development.

**Open questions** (same list as [`packages-and-supply-chain.md`: Open questions](packages-and-supply-chain.md#open-questions))

- **Trustworthy release dates.** Tag and commit dates are set by the author, so they can be
  backdated to skip the cooldown. Recommended: measure age from the later of the tag date and when
  this project first saw the tag (recorded by `outdated` and `update`). A first `add` still has to
  trust the tag date.
- How are security advisories distributed with no central service?
- How are typosquatting and look-alike URLs caught without a naming authority?
- Pre-1.0: is each `0.x` minor a major (Cargo), or does `v0` promise nothing (Go)?
- Do standard mods ship with the toolchain, or are they fetched like any other mod
  ([`mods.md`: Open questions](mods.md#open-questions))?

---

### Tooling uniformity

🟡 **minor questions** · Detail: [`toolchain.md`](toolchain.md), [`docs-and-tests.md`](docs-and-tests.md); see also [`agent-interface.md`](agent-interface.md)

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
  `#[group(…)]`; private items inherit the groups of the items that use them, shown by the language
  server rather than written in the source. Links are checked, invariants cite the contract or test
  behind them, and the tour example runs.

**Open questions** (same list as [`docs-and-tests.md`: Open questions](docs-and-tests.md#open-questions))

- Syntax for attaching tests to methods, trait implementations and generic instantiations.
- Shared fixtures and test-only helpers: how they are declared and kept out of release builds.
- Default doc-coverage thresholds per assurance level.
- Where notes and provenance live: in the source, or in a sidecar keyed by definition ID?
- Benchmark stability on shared CI machines.
- Should the semantic diff flag a module whose public interface changed but whose overview wasn't
  edited?

---

### LLMs as first-class programmers

🟡 **minor questions** · Detail: [`agent-interface.md`](agent-interface.md)

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

**Open questions** (same list as [`agent-interface.md`: Open questions](agent-interface.md#open-questions))

- Should provenance live in version control (recommended, [Prior-Art Matrix](#prior-art-matrix)), in the source, or in a sidecar?
- How far should constrained decoding go: grammar only, or names and types per statement?
- What size budget should the context-sized reference have, and how is it kept current?
- Who curates the cross-language synonym tables and foreign-syntax recognisers?
- Is MCP the right agent protocol long term, or one adapter among several?

---

### Diffs, code review and version control

🟡 **minor questions** · Detail: [`semantic-vcs.md`](semantic-vcs.md)

Version control works on lines of text, but programs are trees with meaning. Reviewers see noise
(formatting, renames, moves) and miss signal (a new side effect, a changed error path). As agents
write more code, review becomes the bottleneck.

Decided:

- **Git is assumed**, with **no required metadata** (no commit trailers, notes or special comments).
- **`moddil diff` compares any two revisions**: uncommitted work against `HEAD`, a branch against
  `main`, two tags.
- **Definition identity is inferred**: qualified names, rename and move detection on canonical ASTs,
  and a content hash per definition.
- **A concise report**, most important first: the required **semver bump**; behavioural changes
  (signatures, effects, capabilities, error sets, contracts, `unsafe`/FFI, wire compatibility);
  dependencies; refactors collapsed and marked **verified behaviour-preserving** where provable;
  affected tests.
- **A suggested PR description** generated from the report, for the user or an LLM to enrich, with
  `--check-description` flagging unsupported claims.
- Optional: Git diff and merge drivers, definition-level `log` and `blame`, split suggestions, and
  CI review policies keyed on semantic change.

**Open questions** (same list as [`semantic-vcs.md`: Open questions](semantic-vcs.md#open-questions))

- Rename-detection thresholds, and how confidence is shown.
- How far the optional merge driver should go.
- Whether `diff` runs affected tests by default or only on request.

---

### Language evolution

🟡 **minor questions** · Detail: [`evolution.md`](evolution.md)

Languages either freeze (C) or break their ecosystems (Python 2→3).

Decided:

- **Editions per mod**; mods of different editions work together, because mod interface artefacts
  are edition-neutral. Semantic changes apply only inside a mod that opts into the new edition.
- **A compatibility promise** within an edition; at most one small edition every three years.
- **Verified migrations:** `moddil migrate` rewrites code and checks it is equivalent; the change is
  reviewed with `moddil diff` ([Diffs, code review and version control](#diffs-code-review-and-version-control)).
- **Libraries ship migration rules** (`#[deprecated(replace_with = …)]`) applied by `moddil fix`.
- **No unstable features in published mods** (no nightly-only ecosystem).
- **A proposal process** that requires a motivating reference example, a migration, and an
  ecosystem-wide build and test run.

**Open questions** (same list as [`evolution.md`: Open questions](evolution.md#open-questions))

- The exact edition cadence.
- May an edition change runtime semantics at all, or only syntax and defaults?
- The minimum deprecation window.
- Governance: a foundation or a core team, and who accepts proposals.
- Long-term-support toolchains.

---

### Surface syntax and paradigms

🟡 **minor questions** · Detail: [`syntax.md`](syntax.md), [`paradigms.md`](paradigms.md)

Syntax is not where Moddil innovates, but every snippet, agent and reader depends on it being stable
and familiar. The questions are which language to borrow from, where to differ, and which
programming styles the language supports.

Decided:

- **Rust syntax as the baseline.** Deltas only where semantics differ: effects with `with`
  (`fn f() -> T with net, fail(E)`), named, default and opt-in name-only arguments, units
  (`100<cm>`), contracts as clauses, `linear` and `view` types. Lifetimes, `async`/`.await`,
  `Send`/`Sync`, `Pin`, `move` and macros disappear.
- **Inferred private signatures**: private functions and closures may omit types and get the most
  general signature; public mod-boundary functions write theirs.
- **`Option<T>` only** (no `T?`); `?` propagates as in Rust, and `try { a?.b?.c }` chains within an
  expression. **One refinement form**, `where` (ranges as `where self in 1..`).
- **Anonymous records** `{ min: f64, max: f64 }`, without width subtyping.
- **Functional and OO styles both supported**; **no inheritance** (traits with supertraits and default
  methods, plus delegation). Eager evaluation with lazy iterators; GATs now, higher-kinded types
  later; methods rather than computed properties; key paths later through reflection.
- **Stable binary boundaries**: the C ABI and WASM components; a Swift-style resilience mode is
  deferred.

**Open questions** (same lists as [`syntax.md`: Open questions](syntax.md#open-questions) and [`paradigms.md`: Open questions](paradigms.md#open-questions))

- Default arguments on trait methods and their implementations.
- Revisit higher-kinded types once GAT-based libraries show what is missing.
- A Swift-style resilience mode for native binary libraries.
- The key-path API on top of static reflection.

---

### Type system core

⚪ **not started** · Detail: none yet

Moddil borrows Rust's traits, generics and sum types, but the core rules of the type system have not
been decided in their own right, and several other areas depend on them: coherence decides whether
two mods can implement the same trait for the same type, and the generics strategy decides both
build times and debug-build speed.

**Touched on in:** [`syntax.md`: Mapping](syntax.md#mapping), [`paradigms.md`](paradigms.md),
[`comptime.md`: Generics are bounded](comptime.md#generics-are-bounded), [`build-times.md`: Language rules that decide build speed](build-times.md#language-rules-that-decide-build-speed), [`mods.md`: Open questions](mods.md#open-questions).

**Questions to settle:**

- Traits and **coherence across mods**: Rust's orphan rule, or something looser?
- Associated types and GATs (higher-kinded types are deferred, [Surface syntax and paradigms](#surface-syntax-and-paradigms)).
- Generics compiled by monomorphisation, dictionary passing, or both by build profile.
- Variance, `dyn` safety, and exhaustiveness checking of sum types.

---

### Modules, visibility and program structure

⚪ **not started** · Detail: none yet

The mod boundary already carries a lot of meaning (explicit effects, capability ceilings, required
groups, semver checks), but the module system itself is still Rust's by default: visibility levels,
how files map to modules, and how code varies by target are undecided.

**Touched on in:** [`syntax.md`: Mapping](syntax.md#mapping), [`docs-and-tests.md`: Overviews: mod, module and group docs](docs-and-tests.md#overviews-mod-module-and-group-docs),
[`build-times.md`: Open questions](build-times.md#open-questions), [`mods.md`](mods.md).

**Questions to settle:**

- Visibility levels (`pub`, `pub(mod)`, private) and what each exposes.
- Are cycles allowed between modules inside one mod?
- Conditional compilation per target and profile, and Cargo-style feature flags.
- The exact meaning of the mod boundary for effects, groups, capabilities and semver.

---

### `unsafe` and the safety boundary

⚪ **not started** · Detail: none yet

`unsafe` is mentioned throughout as a capability and an escape hatch, but its semantics are
undefined: what undefined behaviour remains inside it, what aliasing rules raw pointers must obey,
and what the Miri-style checker actually checks.

**Touched on in:** [`memory-and-resources.md`](memory-and-resources.md),
[`interop.md`: The safety boundary](interop.md#the-safety-boundary), [`correctness.md`](correctness.md),
[`effects-and-capabilities.md`: The effect catalogue](effects-and-capabilities.md#the-effect-catalogue).

**Questions to settle:**

- Which undefined behaviour remains inside `unsafe`, and how it is documented.
- The aliasing model for raw pointers (cf. Rust's Stacked and Tree Borrows).
- What the Miri-style checker checks, and when it runs.
- How `unsafe` code is reviewed and audited across mods.

---

### Text and strings

⚪ **not started** · Detail: none yet

Every reference example, and almost every program an agent writes, handles text, yet string types,
encoding guarantees and formatting have no design of their own.

**Touched on in:** [`memory-and-resources.md`: Zero-copy I/O](memory-and-resources.md#zero-copy-io) (rc slices),
[`comptime.md`: Tagged string literals (embedded DSLs)](comptime.md#tagged-string-literals-embedded-dsls) (`fmt"…"`), [`mods.md`: Initial standard-mod catalogue](mods.md#initial-standard-mod-catalogue) (`moddil/text`).

**Questions to settle:**

- String types: owned, borrowed and rc slices, and how they convert.
- UTF-8 guarantees, byte strings, and OS strings (paths, environment).
- Indexing: bytes, code points or grapheme clusters.
- Formatting, and localisation (messages, numbers, dates).

---

### Values: copy, clone and equality

⚪ **not started** · Detail: none yet

The theme "cost is visible" ([Cross-Cutting Themes](#cross-cutting-themes)) has not yet been applied to everyday values: which types copy
implicitly, whether duplicating a value is always explicit, and how equality, hashing and ordering
are defined and derived.

**Touched on in:** [`memory-and-resources.md`](memory-and-resources.md),
[`paradigms.md`](paradigms.md), [`numerics.md`: Floats](numerics.md#floats) (float ordering).

**Questions to settle:**

- Which types copy implicitly (a `Copy`-like rule), and whether `clone` is always explicit.
- Derived `Eq`, `Hash` and `Ord`; structural vs nominal equality.
- Hashing and determinism (seeded hashing, [`determinism.md`: Sources of nondeterminism](determinism.md#sources-of-nondeterminism)).

---

### Standard library shape

⚪ **not started** · Detail: none yet

The catalogue of standard mods exists, but not the shape of their core interfaces, which every
library builds on.

**Touched on in:** [`mods.md`: Initial standard-mod catalogue](mods.md#initial-standard-mod-catalogue), [`mods.md`: Open questions](mods.md#open-questions).

**Questions to settle:**

- How many batteries: a broad standard library (Go) or a minimal one (Rust)?
- The iterator protocol, and how it relates to generators.
- The collections surface (`Vec`, maps, sets) and its naming.
- The I/O stream model: readers, writers and buffers over effects.

---

### Data interchange and persistence

⚪ **not started** · Detail: none yet

Serialisation, schema evolution and databases appear in several places (standard mods, distributed
contracts, the web store), but there is no single design for how data leaves and re-enters a
program.

**Touched on in:** [`mods.md`: Initial standard-mod catalogue](mods.md#initial-standard-mod-catalogue) (`moddil/serde`, `moddil/sql`),
[`distribution.md`](distribution.md) (contracts), [`examples/03-web-store.md`](examples/03-web-store.md).

**Questions to settle:**

- Serialisation and derives; binary and text formats.
- Schema evolution, shared with distributed contracts.
- Zero-copy formats and validated views.
- Typed SQL, migrations and connection pools.

---

### UI, mobile and the browser

⚪ **not started** · Detail: none yet

Two reference examples need user interfaces, and the browser is a required target (R1), yet there is
no decision on how Moddil builds UIs, or how it talks to JavaScript and the DOM. Interop covers C and
Rust only.

**Touched on in:** [Open Questions and Next Steps for Moddil](#open-questions-and-next-steps-for-moddil) Q5, [`examples/03-web-store.md`](examples/03-web-store.md),
[`examples/01-multiplayer-game.md`](examples/01-multiplayer-game.md), [`targets.md`: Mobile](targets.md#mobile).

**Questions to settle:**

- A Moddil UI mod, or shared logic under native UI shells?
- JS and DOM interop for `wasm32-browser`.
- Mobile packaging for Android and iOS.

---

### Program lifecycle

⚪ **not started** · Detail: none yet

How a program starts, is configured and stops is spread across several areas: root capabilities
arrive in `main`, shutdown is layered, and traps terminate something. These pieces need one coherent
design.

**Touched on in:** [`concurrency.md`: Cancellation and shutdown](concurrency.md#cancellation-and-shutdown) (shutdown),
[`effects-and-capabilities.md`: Authority comes only from root capabilities](effects-and-capabilities.md#authority-comes-only-from-root-capabilities) (root capabilities),
[`error-handling.md`: Three outcome classes](error-handling.md#three-outcome-classes) (traps).

**Questions to settle:**

- The entry point and which root capabilities it receives.
- Configuration, command-line arguments and environment variables.
- Signals and exit codes.
- Does a trap terminate the task or the whole process?

---

## Cross-Cutting Themes

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
   type, whether that is GPU memory ([Concurrency across GPUs and accelerators](#concurrency-across-gpus-and-accelerators)) or a client browser ([Distributed and multi-tier programs](#distributed-and-multi-tier-programs)). Algorithms are separate from
   schedules, and logical programs are separate from physical topologies. Crossing a boundary is
   always explicit.

An optional fifth: **the compiler is a service for agents and humans.** Its structured output is as
important as the binaries it produces.

---

## Tensions and Trade-offs

Being honest about where goals collide:

| Tension                                            | Notes                                                                                                                                                                                            |
| -------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Explicitness vs brevity                            | Effects in every signature could be noisy. Mitigate with inference inside modules and explicit effects only at boundaries.                                                                       |
| Compile-time safety vs build speed                 | Borrow checking, effect inference and verification all cost time. Keep the expensive checks incremental and per-module. Second-class borrows help: the borrow check is local to each function. |
| No lifetimes vs expressiveness                     | Second-class borrows and view types remove lifetime annotations, but lose structs with several independent lifetimes and borrows stored in long-lived data. Owning views, rc and indices fill the gap. |
| Visible sharing vs single-threaded convenience     | rc contents are immutable and shared mutation needs sync cells, so a mutable graph used by one thread still pays (cheap) atomic costs. Arenas, indices and library collections are the idiomatic answer. |
| Monomorphisation vs build time vs runtime speed    | Dual-mode generics ([Build times](#build-times)) is promising but unproven at scale.                                                                                                                                    |
| Strict capabilities vs ergonomics                  | Passing capabilities everywhere is tedious. Need good defaults (e.g., `main` receives all; libraries receive nothing unless asked).                                                              |
| Canonical syntax vs expressiveness                 | One way to do things limits DSLs and clever code, which is usually good, and sometimes painful.                                                                                                  |
| Easy interop vs safety guarantees                  | Every FFI call weakens guarantees. Making it visible is the compromise.                                                                                                                          |
| Novelty vs adoptability                            | Effect systems and linear types are unfamiliar. Each novel feature must pay for itself.                                                                                                          |
| GPU portability vs peak performance                | Portable code is rarely fastest; schedules per target are the escape valve.                                                                                                                      |
| Location transparency vs explicitness              | One language across tiers is attractive, but hiding the network repeats the RPC mistakes of the 1990s. Moddil chooses explicit boundaries.                                                       |
| Language vs infrastructure scope                   | Every step toward describing infrastructure risks turning Moddil into a cloud platform.                                                                                                          |
| Whole-program view vs independent deploys          | Whole-system checking is powerful, but teams must still ship components separately.                                                                                                              |
| WASM as a required target vs native-first features | Threads and GPU access are weaker on WASM, and stack switching does not ship in browsers yet, so browsers need a JSPI-based lowering in the meantime. Every core feature needs a WASM implementation strategy, even if it is slower. |
| Overflow checks vs speed | Integer overflow traps in every build, which costs typically 1–5% before checks are elided by range analysis. Wrapping is always available but explicit (`+%`, `Wrapping<T>`). |
| Small core vs compiler-checked guarantees | R2 wants a tiny core, but anything the compiler must check (ownership, effects, capabilities, placement, contracts) has to be in it. The core is "small" only relative to its guarantees. |
| "For LLMs" vs "for humans"                         | Must not produce a language only machines can comfortably write. Humans still need to read, debug and occasionally write it.                                                                     |

---

## Requirements and Non-Goals

Full text: [`requirements.md`](requirements.md).

### Firm requirements (decided)

🟠 **major questions** · Detail: [`requirements.md`](requirements.md), [`bootstrap.md`](bootstrap.md) (R1), [`mods.md`](mods.md) (R2), [`error-handling.md`](error-handling.md) (R3)

These are no longer open for debate. Every other design decision must be compatible with them.

**R1. WebAssembly is a first-class compilation target.** WASM is a tier-one target from the first
release, tested in CI like native targets: core WASM, WASI and the Component Model, and the browser.
It is needed for multi-tier programs ([Distributed and multi-tier programs](#distributed-and-multi-tier-programs)), it fits the capability model ([Effects and capabilities as a unifying idea](#effects-and-capabilities-as-a-unifying-idea), [Package management and supply-chain security](#package-management-and-supply-chain-security)), and it is
the sandbox for untrusted code. Concurrency semantics are target-independent; the backend lowers
suspension to stack switching (preferred), JSPI (the browser baseline) or a CPS transform. Details:
[`requirements.md`: Firm requirements (decided)](requirements.md#firm-requirements-decided) and [`bootstrap.md`](bootstrap.md).

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

**Open questions** (same list as [`requirements.md`: Open questions](requirements.md#open-questions))

- R1: should WASI runtimes without stack switching be supported, which would need the
  CPS/Asyncify-style lowering? (Also in [`bootstrap.md`: Risks and open questions](bootstrap.md#risks-and-open-questions).)
- Should determinism become R4 ([`determinism.md`: Open questions](determinism.md#open-questions))?

**Open questions** (same list as [`mods.md`: Open questions](mods.md#open-questions))

- **How many batteries?** Go ships a broad standard library (HTTP, crypto, JSON); Rust ships a minimal
  one. With strong supply-chain controls ([Package management and supply-chain security](#package-management-and-supply-chain-security)), a broad `moddil/` set reduces risky third-party
  dependencies. Which way should Moddil lean?
- **Distribution**: should standard mods ship with the toolchain, or be fetched and pinned like
  any other mod?
- **Naming**: is "mod" too easily confused with Rust's `mod` (a module)? The distinction between mod
  and module needs to stay clear in docs and tooling.
- **Coherence**: how are trait/instance overlap rules (Rust's orphan rule) handled across mods?

**Open questions** (same list as [`bootstrap.md`: Risks and open questions](bootstrap.md#risks-and-open-questions))

- Stack switching may change shape before standardisation, or arrive late in Safari.
- Should the MIR be designed so that MLIR can consume it later (GPU/tensor work)?
- C backend: worth it for bootstrapping and portability?
- Should WASI runtimes without stack switching be supported (lowering 4)?
- LLVM version pinning and upgrade cadence.
- Is Moddil-core (the stage 0 subset) small enough to build quickly, yet large enough to write a
  pleasant compiler in?

### Non-goals (draft)

🟡 **minor questions** · Detail: [`requirements.md`](requirements.md)

- Maximum syntactic novelty. Familiar syntax wins unless there is a strong reason.
- Replacing every language. Scripting, notebooks and quick glue stay with other languages.
- **A REPL, script mode or notebook mode (decided).** Moddil is compiled and checked as a whole.
  Quick experiments use attached tests and examples-as-tests (`moddil test`,
  [`docs-and-tests.md`](docs-and-tests.md)) and live update in debug builds
  ([`build-times.md`: Live update (debug builds)](build-times.md#live-update-debug-builds)). Quick learning is served by an in-browser playground
  that re-runs the whole program on every edit ([`toolchain.md`: Playground: learning without a REPL](toolchain.md#playground-learning-without-a-repl)).
- C++ interop in the language. C++ is reached through C shims ([`interop.md`: C++ through C shims](interop.md#c-through-c-shims)).
- A custom VCS that replaces Git (tooling on top of Git first).
- A cloud platform, orchestrator or service mesh. Moddil emits artefacts and manifests for existing
  ones.
- Transparent remote calls.
- Runtime reflection / dynamic metaprogramming.

---

## Prior-Art Matrix

reference · Detail: [`prior-art.md`](prior-art.md)

The matrix comparing fifteen languages across the problem areas, and the reference list, are in
[`prior-art.md`](prior-art.md).

**Observation:** no row is strong across the board, and **no language at all** is strong in "semantic
diffs" or treats "LLM-friendliness" as a stated design goal. Distributed programming is well served
only in niches (Erlang, Unison, Ballerina/Wing, research multitier and choreographic languages), and
none of those combine it with systems-level performance or GPU support. Those three columns, together
with effects and capabilities as a unifying mechanism, are where the opportunity is largest.

---

## Open Questions and Next Steps for Moddil

🟠 **major questions** · Detail: [`examples/README.md`](examples/README.md)

Open questions for a single area are listed under that area in [Problem Areas](#problem-areas). The questions here cut across
areas.

**Strategic questions**
1. **Primary target domain** for v0? Candidates: systems / infrastructure services; ML and
   numeric (GPU-heavy); embedded; general applications. The choice reshapes priorities (e.g., GPU is
   central for ML, peripheral for services).
2. **Who is the first user?** An agent-assisted team writing services? A numerics group? The answer
   sets the bar for ergonomics vs guarantees.
3. **Adoption path**: embed in existing C/C++/Rust codebases one module at a time, through C export
   and the generated Rust bridge ([Interoperability with C and Rust](#interoperability-with-c-and-rust))?
4. **Is multi-tier deployment ([Distributed and multi-tier programs](#distributed-and-multi-tier-programs)) a v0 goal?** **Decided:** the language layer (components,
   placement, `remote`, contracts, single-process and simulated runs) is v0; images, Compose,
   Kubernetes and skew checks are v1; other adapters later ([`distribution.md`: Staging](distribution.md#staging)).
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
   resources (summary in [Memory and resource management](#memory-and-resource-management)).
8. Effect system design. **Decided:** a hybrid of scoped handlers and second-class capability
   values ([`effects-and-capabilities.md`](effects-and-capabilities.md), [Effects and capabilities as a unifying idea](#effects-and-capabilities-as-a-unifying-idea)).
9. Canonical AST storage: text with canonical printing, or a structured store (Unison-style) with text
   as a view?
10. **Minimum viable language**: which language features are in v0 and which come later? (Separate
    from the compiler milestones in [`bootstrap.md`: Milestones](bootstrap.md#milestones).)
11. **How is the language defined?** A prose reference, a formal semantics, or the MIR interpreter as
    the executable reference? How are conformance tests organised?

---

## Working on these specs

For people and LLM agents editing the specs:

- **Link by heading name, not by number.** Sections are not numbered, so they can be reordered and
  rewritten freely. Cross-reference with a heading link: `[Live update (debug builds)](build-times.md#live-update-debug-builds)`
  in another document, or `[Error handling](#error-handling)` within this one. Anchors follow
  GitHub's slug rules (lower-case, punctuation removed, spaces become hyphens). Stable identifiers
  that are not positions (R1–R3, D1–D10, example file numbers) stay as they are.
- **Run the link checker after every edit:**

  ```text
  python tools/check_spec_links.py            # check only; exit code 1 on any problem
  python tools/check_spec_links.py --align    # also re-align this document's Contents table
  ```

  It checks that every relative link (file and heading anchor) in `specs/` resolves, and that each
  status in the [Contents](#index) table matches the status line under its section. Renaming a
  heading breaks the links to it; the checker lists them.
- **Keep statuses in two places in step:** the Contents table and the status line under each section
  (`🟡 **minor questions** · Detail: …`), using the legend at the top.
- **Detail documents are authoritative.** This overview summarises; when a decision changes, update
  the detail document first, then its summary here.
- **Code blocks are illustrative** and follow [`syntax.md`](syntax.md) (Rust-based syntax).


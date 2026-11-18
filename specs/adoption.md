# Adoption

> **Status:** decided direction, details open. Listed in the [index](README.md). Snippets are
> **illustration only**.

## Summary

| Topic | Decision |
|---|---|
| **Pitch** | **"Moddil: the programming language with armour, not baggage."** Rust's safety without Rust's pain. The mascot is the **armadillo** ([Positioning: armour, not baggage](#positioning-armour-not-baggage)). |
| **First audiences** | **Backend and infrastructure teams**, and **Python teams that need speed** ([First audiences](#first-audiences)). |
| **Python** | `moddil bind --lang python` is the **first binding target, in v0**: native extension modules, `.pyi` stubs and wheels ([Python first](#python-first)). |
| **Migration** | `moddil migrate rust` is an official, mechanical, test-verified translator for the common subset. TypeScript and other languages get an **agent-assisted** workflow ([Migrating existing code](#migrating-existing-code)). |
| **Simplicity** | "Simpler than Rust" is a measured claim, with metrics published per release ([Simpler than Rust](#simpler-than-rust)). |

## Problem

A language succeeds only if programmers choose it. Moddil's guarantees matter only if people write
code in it, and a new language starts with no users, no libraries, no answers on the web, and almost no
training data for the models that now write much of the code. Adoption has to be designed, not
hoped for: a sharp reason to try the language, a way in that does not require rewriting everything,
and an experience that makes people stay.

## How languages were adopted

| Language | What drove adoption | Lesson for Moddil |
|---|---|---|
| **Rust** | Memory safety without a GC; Cargo; excellent error messages; Firefox, then systems and infrastructure | A real guarantee plus delightful tooling beats a steep learning curve, but the curve still costs users |
| **Go** | Simplicity, fast builds, one toolchain; Docker and Kubernetes as flagship software | Flagship software in the target domain recruits the audience |
| **TypeScript** | An incremental superset of JavaScript: adopt one file at a time | The way in matters more than the destination |
| **Kotlin** | Seamless Java interop; Android's endorsement | Interop with what teams already run removes the risk |
| **Swift** | A platform mandate (Apple) | Not available to Moddil |
| **Zig** | `zig cc`: useful as a C toolchain before anyone writes Zig | Be useful to people who have not adopted you yet |
| **Mojo** | "Python, but fast" | Python users are a huge audience looking for speed |
| **PyO3 and maturin** | Many teams first used Rust to speed up one Python module | A native-extension path is a proven route in |

**Lessons:** an incremental way in, interop with what people already run, flagship software in the
target domain, and tooling that delights from the first command.

## Positioning: armour, not baggage

**Decision: the tagline is "Moddil: the programming language with armour, not baggage", and the
mascot is the armadillo.** The pitch is **Rust's safety without Rust's pain**: Moddil keeps the armour
and leaves the baggage behind.

| Armour kept | Baggage dropped |
|---|---|
| Memory safety and data-race freedom without a GC ([`memory.md`](memory.md)) | Lifetime annotations: borrows are second-class ([`memory.md`: Borrowing](memory.md#borrowing)) |
| No crash from an unhandled error ([R3](requirements.md#firm-requirements-decided)) | `async`/`.await` and coloured functions: suspension is an effect ([`concurrency.md`](concurrency.md)) |
| Effects and capabilities in signatures ([`effects-and-capabilities.md`](effects-and-capabilities.md)) | `Send`, `Sync`, `Pin`, `'static` and `move` ([`concurrency.md`: No `Send` / `Sync`](concurrency.md#no-send--sync), [`closures.md`](closures.md)) |
| Checked contracts and refinements ([`correctness.md`](correctness.md)) | `macro_rules!` and procedural macros: comptime instead ([`comptime.md`](comptime.md)) |
| Capability-scoped dependencies ([`packages-and-supply-chain.md`](packages-and-supply-chain.md)) | Build scripts and code run at fetch time ([`packages-and-supply-chain.md`: No code at fetch or build time](packages-and-supply-chain.md#no-code-at-fetch-or-build-time)) |
| Overflow and bounds checks ([`numerics.md`](numerics.md)) | Orphan-rule workarounds: bridge mods ([`types.md`: Coherence: one impl per trait and type](types.md#coherence-one-impl-per-trait-and-type)) |
| Deterministic, replayable programs ([`determinism.md`](determinism.md)) | Slow builds: fast debug builds and live update ([`build-times.md`](build-times.md)) |

Three secondary reasons, each a pitch to a particular reader:

- **The language agents write best:** explicit semantics, one canonical form, structured diagnostics
  and verified refactors ([`agent-interface.md`](agent-interface.md)).
- **One language for the whole system:** browser, server, workers and GPU in one typed program with
  generated contracts ([`distribution.md`](distribution.md)).
- **Testable by construction:** time, network and randomness are effects, so deterministic simulation
  testing comes free ([`determinism.md`: Deterministic simulation testing](determinism.md#deterministic-simulation-testing)).

## First audiences

**Decision: two first audiences.**

| Audience | Today they use | The itch | What must be ready on day one |
|---|---|---|---|
| **Backend and infrastructure teams**: services, networking, data pipelines, distributed systems | Go, Rust, TypeScript, Java | Untestable concurrency and I/O; async Rust's complexity; slow builds; supply-chain fear | `moddil/net` and HTTP, `sql`, `serde`, `time`, the simulation runtime, deploy adapters for containers ([`distribution.md`](distribution.md)) |
| **Python teams that need speed**: data, ML and scientific code with a slow hot loop | PyO3 and Rust, Cython, C++ with pybind11, Numba | Rewriting a hot path means learning Rust or C++, or fighting a build system | Python bindings, wheels, numpy interop ([Python first](#python-first)), `moddil/simd`, later `moddil/tensor` |

Why these two:

- Backend and infrastructure code is where effects, deterministic simulation, fast builds and
  supply-chain controls pay off most, and two of the reference examples (the
  [web store](examples/03-web-store.md) and [Hashiverse](examples/04-hashiverse.md)) live there.
- Python's audience is enormous, and its "this function is too slow" moment is a natural, low-risk
  first contact: one module, inside an existing project, with no rewrite.

## Simpler than Rust

What a newcomer to Moddil never has to learn, compared with Rust:

- lifetime parameters and elision rules;
- `async`/`.await`, `Pin`, futures, and choosing an async runtime;
- `Send`, `Sync` and other auto traits;
- `move` closures, `Arc::clone` rituals and `'static` bounds for tasks;
- `macro_rules!`, procedural macros and `build.rs`;
- orphan-rule workarounds and object-safety rules ([`types.md`](types.md));
- numeric `as` casts with silent truncation ([`numerics.md`](numerics.md)).

**Proposal: simplicity is measured, not asserted.** Each release publishes:

- the **number of concepts** needed to write, test and ship a small HTTP service, compared with Rust
  and Go;
- the **compile-error rate** on a standard set of newcomer tasks, for people and for agents
  ([`agent-interface.md`](agent-interface.md));
- the **time to a first working service** from `moddil new`.

## Python first

**Decision: Python bindings are in v0, and Python is the first `moddil bind` target.**

```rust
// fast_stats.mdl
#[export(python)]
pub fn rolling_mean(xs: &[f64], window: int) -> Vec<f64> { … }
```

```python
import numpy as np
from fast_stats import rolling_mean       # a native extension module built by moddil

prices = np.load("ticks.npy")
smooth = rolling_mean(prices, window=50)  # numpy array in, no copy; list or array out
```

- `moddil build --wheel` produces wheels for each target platform using the CPython **stable ABI**, so
  one wheel serves several Python versions; `.pyi` stubs give editors and type checkers full types.
- **Zero-copy numpy**: slices of numbers map to the buffer protocol, so arrays cross without copying.
- **Errors become exceptions**: each `fail` type becomes a Python exception class carrying the typed
  error ([`error-handling.md`](error-handling.md)). Traps become a distinct exception and never kill the
  interpreter silently.
- **Async:** functions that `suspend` become awaitables usable from `asyncio`; cancellation crosses both
  ways, as with the other bindings ([`ui.md`: Generated bindings](ui.md#generated-bindings)).
- **The GIL:** Moddil code releases the GIL while it runs, and runs its own tasks on its own executor;
  free-threaded CPython builds are supported where available.
- **Placement in the roadmap:** Python bindings ship at M3, with the first optimised native release
  ([`roadmap.md`: Milestones](roadmap.md#milestones)), before the Swift, Kotlin and TypeScript
  bindings.

## Migrating existing code

**Decision: an official Rust translator, and an agent-assisted path for everything else.**

### `moddil migrate rust`

Rust and Moddil share most syntax, so translation of the common subset is mechanical:

| Rust | Moddil |
|---|---|
| Lifetime parameters and annotations | Removed; borrows are second-class. Structs holding borrows become view types, or are flagged |
| `async fn`, `.await` | Ordinary functions with the `suspend` effect; `.await` removed |
| `fn f() -> Result<T, E>` | `fn f() -> T with fail(E)`, where the result is not stored as a value |
| `Arc<T>`, `Rc<T>`, `.clone()` on them | `Rc<T>` and `.share()` ([`values.md`](values.md)) |
| `Mutex`, `RwLock`, atomics | The same, from `moddil/sync` |
| `Send`, `Sync`, `'static` bounds, `move` | Removed |
| `#[derive(…)]` | Kept; derives are comptime ([`comptime.md`](comptime.md)) |
| `macro_rules!` | Translated to comptime functions where the pattern is simple; otherwise flagged |
| `unsafe` blocks | Kept, with the capability declared ([`unsafe.md`](unsafe.md)) |
| `x as u8` | `x.truncate::<u8>()` or `x.to::<u8>()?`, chosen by the value range the compiler can prove |

- **Unsupported constructs** (complex procedural macros, self-referential structs, some trait-object
  patterns) produce TODO diagnostics with a pointer to the original code; nothing is silently dropped.
- **Verification:** the crate's tests are translated too and must pass. The report lists what was
  translated mechanically, what was flagged, and the test results.
- The **generated Rust bridge** lets translated and untranslated crates work together during a
  migration ([`interop.md`](interop.md)).

### TypeScript and other languages: agent-assisted

TypeScript, Python and Go are too far from Moddil (a GC, dynamic shapes, exceptions) for a purely
mechanical translation. The workflow:

1. The tool translates **structure and types**: modules, functions, interfaces into traits, unions
   into enums, exceptions into `fail` types.
2. An **agent proposes ownership**: which values are owned, borrowed or shared, using the compiler's
   diagnostics as feedback ([`agent-interface.md`](agent-interface.md)).
3. The **original tests verify**: they are translated, or run against both versions through bindings.
4. **Staged:** one module at a time, with bindings connecting translated and original code (for
   TypeScript, the WASM and TS bindings in [`ui.md`](ui.md)).

## Adopting inside existing codebases

No one should have to rewrite a system to try Moddil:

| Existing codebase | Way in |
|---|---|
| Python | A native extension module ([Python first](#python-first)) |
| C or C++ | C export with generated headers; the toolchain compiles the C side too ([`interop.md`: The C ABI, both directions](interop.md#the-c-abi-both-directions)) |
| Rust | The generated Rust bridge, or `moddil migrate rust` one crate at a time ([`interop.md`](interop.md)) |
| TypeScript and the browser | A WASM core with generated TS bindings ([`ui.md`: The browser](ui.md#the-browser)) |
| Swift, Kotlin | A shared core under native apps ([`ui.md`](ui.md)) |
| Anything over the network | A Moddil service with generated contracts ([`distribution.md`](distribution.md)) |

## The LLM cold start

Models write well only in languages they have seen. Moddil's plan is in
[`agent-interface.md`: Solving the cold start](agent-interface.md#solving-the-cold-start); adoption adds:

- **The translators produce training data.** `moddil migrate rust` turns real, tested Rust into real,
  tested Moddil, and agent-assisted migrations add more.
- **A context-sized reference** written for agents, small enough to include in a prompt.
- **The playground** gives people and agents instant feedback without installing anything
  ([`toolchain.md`: Playground: learning without a REPL](toolchain.md#playground-learning-without-a-repl)).

## Learning materials

- **The book**: the language from first principles, with every example runnable in the playground.
- **Moddil for Rust developers**: the baggage dropped, chapter by chapter, with the migration table.
- **Moddil for Python developers**: ownership explained for people who have never managed memory,
  starting from a single extension function.
- **The reference examples as tutorials**: the [web store](examples/03-web-store.md) and
  [Hashiverse](examples/04-hashiverse.md) built step by step.

## Flagship software

- The **Hashiverse client** (milestones M3 and M5, [`roadmap.md`: Milestones](roadmap.md#milestones)) is the
  first real application, and it is used by a real network.
- The four [reference examples](examples/README.md) are published as complete, idiomatic programs.
- **A widely used Python package** accelerated with Moddil would do more for adoption than any feature;
  finding one is a goal for the first year.

## Measuring adoption

- Weekly active projects building with `moddil`, and wheels built with `moddil build --wheel`.
- Published mods, and how many are depended on by others.
- Newcomer compile-error rates and time to a first service ([Simpler than Rust](#simpler-than-rust)).
- Agent success rate on a standard task suite, compared with Rust and Go.

## Open questions

1. **The Rust migration subset:** which constructs are translated mechanically in v0, and what success
   rate on popular crates is the target?
2. **Simplicity metrics:** which newcomer tasks, and what targets relative to Rust and Go?
3. **Free-threaded Python:** how do Moddil's executors and Python's threads interact once the GIL is
   gone?
4. **Name, logo and trademark:** registering "Moddil" and protecting the armadillo
   ([`governance.md`](governance.md)).
5. **Can TypeScript migration ever be mechanical**, for example for TypeScript written in a functional,
   immutable style?

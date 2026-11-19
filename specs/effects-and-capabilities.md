# Effects and Capabilities

> **Status:** decided direction, open questions remain. Indexed in [`README.md`](README.md).
> Snippets are **illustration only**.

## Summary

| Rule | One line |
|---|---|
| **Effects are traits passed implicitly** | There is no `effect` keyword. Any trait can be listed after `with`; the implementation is found by trait type in the dynamic scope, installed with `with Time = sim.clock(n) { … }` ([Effects are traits passed implicitly](#effects-are-traits-passed-implicitly)). |
| **Named when used, bare when passed through** | `with time: Time` binds a name for code that calls it; `with Time` only passes the need on; private pass-through functions are inferred. |
| **Effects propagate; plumbing doesn't** | A need shows up in every caller's effect set, but never as a parameter, an argument, a struct field or a change to higher-order code ([What an effect saves](#what-an-effect-saves)). |
| **Handlers are impls or anonymous impls** | `impl Links for MemoryLinks` for a reusable handler; `impl Time { … }` as an expression for a one-off that captures locals. One instance is shared by the block's tasks ([Defining handlers](#defining-handlers)). |
| **Control from the caller; dependencies captured at construction** | Trait methods declare `suspend` and `fail`; anything else a handler needs (`Db`, the outer `Net`) is captured when the handler is built, so swapping a handler never changes callers' signatures ([Effects inside handlers](#effects-inside-handlers)). |
| **Three control effects are built in** | `suspend`, `spawn` and `fail(E)` are keywords, not traits; `spawn` is separate from `suspend` ([The effect catalogue](#the-effect-catalogue)). |
| **Handler tiers** | User traits get function-like (tail-resumptive) handlers, and abortive ones once their spelling is settled. One-shot continuations are reserved for `suspend` and generators. Resuming twice is never allowed. |
| **Authority comes only from root capabilities** | `main` declares the root capabilities it needs (`NetRoot`, `FsRoot`, `Devices`, …); real OS handlers are built only from those. Libraries perform, wrap and narrow effects, but never create authority. |
| **Capability values for two at once** | A handler can also be passed as a second-class value (`fn bridge(a: &dyn Net, b: &dyn Net)`). |
| **Inferred inside, explicit at `pub`** | Effects are inferred inside a mod and written on `pub` signatures; no aliases. |
| **Implicit effect polymorphism** | An unannotated closure parameter means "whatever `f` does, I do too"; `with E` is written only to name or constrain. |
| **One mechanism for restricted code** | Drop, kernels, comptime, workflows and freestanding profiles are all effect-set constraints. |
| **Zero cost when handlers are known** | Effect calls compile to direct calls; abortive handlers to ordinary returns. |

## Problem

- **Signatures say what a function takes and returns, but not what it *does***: I/O, suspension,
  failure, nondeterminism, GPU work, unsafe code. A reviewer has to read the whole body, and every
  function it calls.
- **Ambient authority**: any function anywhere can open a socket or read `~/.ssh`
  ([`packages-and-supply-chain.md`: Problem](packages-and-supply-chain.md#problem)).
- **Coloured functions**: `async`, `Result` and `const` each split APIs into copies
  ([`error-handling.md`: Why `Result`-only languages duplicate higher-order APIs](error-handling.md#why-result-only-languages-duplicate-higher-order-apis), [`closures.md`: Rust pain points and Moddil's answers](closures.md#rust-pain-points-and-moddils-answers)).
- **Testability by hand**: mocking frameworks, `Context` objects threaded through every call, and
  hand-written "provider" traits (Hashiverse's `TimeProvider`, `TransportFactory`, `ClientStorage`).

## State of the art

| System | What it does well | Limits |
|---|---|---|
| **Koka** | Row-polymorphic effects; handlers; evidence passing makes most handlers cheap | Research language; multi-shot resumption complicates resources |
| **Effekt** | Effects as **second-class capabilities**: lexical, fast, simple to type; objects capture capabilities | Research language |
| **OCaml 5** | One-shot continuations in a production runtime; schedulers written as libraries | Effects are **untyped**: nothing in signatures |
| **Unison abilities**, **Flix** | Effects in signatures for mainstream-shaped code | Small ecosystems |
| **Scala 3 `using`/`given`** | Implicit parameters resolved by type; capture checking (experimental) | Implicits are not tracked as effects; resolution by import is hard to follow |
| **Haskell effect libraries** (mtl, polysemy, effectful) | Expressive | Heavy types; performance traps |
| **ZIO, Effect-TS** | A typed error and environment channel | Library-level; everything is wrapped in an effect value |
| **Swift `rethrows`**, **Rust keyword generics** | Polymorphism over one effect (throwing, `async`/`const`) | One effect at a time |
| **Java checked exceptions** | Failure in signatures | The lesson: no polymorphism, so lambdas and generics broke, and people wrapped everything in unchecked exceptions |
| **E, Austral, WASI** | Object capabilities: authority only by holding a reference | Austral and E are small; WASI covers the host boundary only |

## Decisions

### Effects are traits passed implicitly

**Decided: there is no separate `effect` construct. An effect is any trait listed after `with`.**

```rust
trait Time {
    fn now(&self) -> Instant;
}

fn deadline(after: Duration) -> Instant with time: Time {   // calls it, so names it
    time.now() + after
}

pub fn schedule(job: Job) with Time, spawn {                // only passes the need on
    let at = deadline(5<s>);
    …
}

with Time = sim.clock(node) {          // installs a handler for this block
    schedule(job)                      // everything inside, however deep, uses sim.clock
}
```

- **Found by trait type.** `with Time = value { … }` installs `value` (whose type implements `Time`)
  for the block, and the innermost one wins
  ([`determinism.md`: Handlers are scoped, not global](determinism.md#handlers-are-scoped-not-global)).
  This is Scala's `using`/`given`, resolved by dynamic scope rather than by imports.
- **Named when used, bare when passed through.** `with time: Time` binds `time` (a second-class
  `&dyn Time`, [Capability values](#capability-values)) for code that calls it. `with Time` only
  states the need. Inside a mod, pass-through functions need neither: the need is inferred
  ([Signatures and inference](#signatures-and-inference)).
- **Coherence is unaffected.** A handler is an ordinary impl on its own type (or an anonymous impl),
  so the one-impl-per-trait-and-type rule still holds
  ([`types.md`: Coherence: one impl per trait and type](types.md#coherence-one-impl-per-trait-and-type)). What varies by scope
  is the *value* installed, not the impl.
- Spawned tasks **inherit** their scope's handlers; structured concurrency guarantees the handlers
  outlive them.
- A handler can **interpose**: forward to the enclosing handler while recording, delaying, failing
  or asserting ([Effects inside handlers](#effects-inside-handlers)).
- Trait names are `UpperCamel` (`Net`, `Fs`, `Db`); the names bound in signatures are by convention
  the same in `snake_case` (`net`, `fs`, `db`).

#### What an effect saves

Effects **do propagate**: if `deadline` needs `Time`, every function that reaches it has `Time` in
its effect set, up to the `with` that installs a handler. That visibility is the point. What an
effect avoids is the plumbing a trait passed as a value needs:

| Adding a clock deep in `main → serve → handle → route → deadline` | Trait passed as a value (Rust today) | Effect |
|---|---|---|
| Signatures on the path | Each gains a parameter `clock: &dyn Time` or a generic `C: Time` | Each gains `Time` in its effect set |
| …written in source | **Every** function on the path | Only **`pub`** functions; private ones are inferred and shown by the editor |
| Call sites | Every call passes `clock` along | Unchanged |
| Structs that hold it | `Server { clock: Arc<dyn Time>, … }`, and constructors take it | Unchanged |
| Higher-order code in between (`visit`, a router taking callbacks) | Changed to accept and forward it, or the callback captures it | Unchanged: closures carry their effects through ([Effect polymorphism](#effect-polymorphism)) |
| Using a fake in a test | Build and pass a different value down the chain | `with Time = sim.clock() { … }` around the call |
| Switching to a handler that itself needs `Db` | Every signature on the path changes | Nothing on the path changes ([Effects inside handlers](#effects-inside-handlers)) |

This is compiler-checked dependency injection: it replaces mocking frameworks, `Context` objects and
hand-written provider traits, at the cost of one name per effect in public signatures.

### Defining handlers

**Decided: a reusable handler is an `impl`; a one-off handler is an anonymous impl expression.**

```rust
trait Links {
    fn get(&self, code: &Code) -> Option<Target> with suspend;
    fn put(&self, code: Code, target: Target) with suspend, fail(LinkError);
}

// reusable: an ordinary impl
struct MemoryLinks { map: Mutex<Map<Code, Target>> }

impl Links for MemoryLinks {
    fn get(&self, code: &Code) -> Option<Target> with suspend { … }
    fn put(&self, code: Code, target: Target) with suspend, fail(LinkError) { … }
}

with Links = MemoryLinks::new() { serve(&listener) }

// one-off: an anonymous impl captures locals like a closure
let mut calls = 0;
with Time = impl Time {
    fn now(&self) -> Instant { calls += 1; t0 }
} {
    run_once()
}
assert(calls == 3);
```

- **An anonymous impl** (`impl Trait { … }` as an expression) implements a trait for an unnamed type
  and captures its environment like a closure, as Kotlin's `object :` and Java's anonymous classes
  do. It is usable anywhere a trait value is wanted, not only after `with`. Because a handler never
  outlives its `with` block, it may **borrow** locals ([Capability values](#capability-values)).
- **One instance is shared.** Methods take `&self`, and every task spawned inside the `with` block
  uses the same handler, so a handler's mutable state lives in sync cells
  ([`concurrency.md`: Shared mutable state](concurrency.md#shared-mutable-state)). An anonymous impl
  that captures a local by `&mut` (like `calls` above) is rejected if the block spawns tasks, the same
  rule as for closures ([`closures.md`](closures.md)).
- **Methods declare their control effects.** A trait method may carry `with suspend` and `fail(E)`;
  those are what callers see ([Effects inside handlers](#effects-inside-handlers)).

### Effects inside handlers

**Decided: control effects follow the caller; everything else a handler needs is captured when the
handler is built.**

```rust
// a production handler for `Links`, backed by SQL: a view, because it holds a capability
view SqlLinks { db: &dyn Db }

impl SqlLinks {
    fn new() -> SqlLinks with db: Db { SqlLinks { db } }      // captures the Db in scope
}

impl Links for SqlLinks {
    fn put(&self, code: Code, target: Target) with suspend, fail(LinkError) {
        try { self.db.execute(sql"INSERT INTO links (code, target) VALUES ({code}, {target})")? } catch {
            Unique(_) => fail(LinkError::Taken(code)),
            e         => fail(LinkError::Storage(e)),          // DbError mapped into the declared error
        }
    }
    …
}

fn main(…) -> ExitCode {
    with Db = pool {
        with Links = SqlLinks::new() {     // `Db` is needed here, where the handler is built…
            serve(&listener)               // …not by `serve` or anything that uses `Links`
        }
    }
}
```

- **Control effects follow the caller.** `suspend` pauses the caller's task and `fail` returns
  through the caller's stack, so they belong to the trait method: a method that may suspend or fail
  declares it, and every caller sees it. A handler cannot raise an error its method doesn't declare;
  it maps it into a declared one (`LinkError::Storage`) or handles it.
- **Other dependencies are captured at construction**, as a closure captures its environment
  (Effekt's model). `SqlLinks::new()` runs where `Db` is installed and keeps it. A value that holds a
  capability is second-class, a [view type](memory.md#view-types), so it cannot outlive the scope
  that provided `Db`. Callers of `Links` never see `Db`, and **swapping a handler never changes a
  caller's signature**.
- **Interposition falls out.** A recorder built inside a scope with a `Net` handler captures that
  outer `Net` and forwards to it, so a recorder, a fault injector or a rate limiter is an ordinary
  handler ([`determinism.md`: Handlers are scoped, not global](determinism.md#handlers-are-scoped-not-global)).
- **No authority is created.** The code that builds the handler must itself be allowed to use `Db`,
  by its signature, its mod's ceiling and its own callers' handlers
  ([Authority comes only from root capabilities](#authority-comes-only-from-root-capabilities)).
  A handler only moves where an effect is paid for, never whether it is allowed.
- **Restricted contexts still apply.** A `drop` has an empty effect set (D9), so it cannot use
  `Links` at all, whatever the handler would do ([Restricted code is one mechanism](#restricted-code-is-one-mechanism)).

### The effect catalogue

| Kind | Effects | Notes |
|---|---|---|
| **Control (built in)** | `fail(E)`, `suspend`, `spawn` | Keywords, not traits: they act on the caller's stack and task ([`error-handling.md`](error-handling.md), [`concurrency.md`](concurrency.md)) |
| **Standard traits** | `Time`, `Rand`, `Net`, `Fs`, `Stdin`, `Stdout`, `Stderr`, `Env`, `Process`, `Gpu`, `Remote`, `Introspect`, `Locale`, `Dom` | Traits in standard mods; handlers in runtime and simulation mods ([`runtime.md`: Pluggable runtimes without ecosystem splits](runtime.md#pluggable-runtimes-without-ecosystem-splits)) |
| **Capabilities with no handler to swap** | `ffi`, `unsafe` | Declared and visible to every consumer; treated as high risk ([`interop.md`: The safety boundary](interop.md#the-safety-boundary)) |
| **Ambient, write-only** | `log`, `span`, `metric` | Never written in signatures; cannot feed values back ([`observability.md`](observability.md)) |
| **User-defined** | any trait | Context such as a database pool (`Db`), configuration, feature flags ([Directions (not yet decided)](#directions-not-yet-decided)) |

**`spawn` is its own effect (decided).** Starting tasks is separate from suspending: `drop`,
GPU kernels and freestanding profiles without a scheduler must be able to forbid `spawn` while still
allowing `suspend` (or forbid both). A function that opens a `scope` and spawns into it lists
`spawn`; one that only waits on I/O lists `suspend`
([`concurrency.md`: Tasks and borrowing](concurrency.md#tasks-and-borrowing)).

**Not effects:**
- **allocation**: a capability chosen per profile ([`freestanding.md`](freestanding.md));
- **bugs and traps**: contract violations abandon the task ([`error-handling.md`](error-handling.md));
- **non-termination**.

**Effects are coarse.** There is one `Fs` trait, not separate traits for reading and writing. Finer
limits come from **narrowing a handler** (`fs.read_only("/data")` returns a narrower `Fs` handler),
not from more traits. This keeps signatures short ([Signatures and inference](#signatures-and-inference)).

**No umbrella effects.** Each effect is listed on its own, and none implies another: there is no
`Io` trait. A function that uses the network and may suspend writes `with Net, suspend`.

### Handler tiers

| Tier | What the handler can do | Who can define it | Compiles to |
|---|---|---|---|
| **Tail-resumptive** | Return a value to the caller, like a function | Anyone: an ordinary trait method | A direct (or indirect) call |
| **Abortive** | Never resume; unwind to the handler, like an exception | Built in for `fail`; for user traits, open ([Open questions](#open-questions)) | An ordinary return (`fail`, [`error-handling.md`: Results and effects](error-handling.md#results-and-effects)) |
| **One-shot continuation** | Capture the rest of the computation and resume it **once**, later | Built in only: `suspend`, generators | Stack switching ([`runtime.md`: Runtime contract](runtime.md#runtime-contract), [`compiler.md`: WASM concurrency lowering](compiler.md#wasm-concurrency-lowering)) |

- **No multi-shot resumption**, ever. Resuming twice would run the rest of the computation twice,
  duplicating linear values and running drops twice ([`memory.md`](memory.md)).
- Almost every use (I/O, context, mocking, simulation, recording) is tail-resumptive, so user
  effects are ordinary trait calls and easy to reason about.

### Authority comes only from root capabilities

```rust
fn main(net: NetRoot, files: FsRoot, clock: Clock, devs: Devices) {
    with Net  = net,
         Fs   = files.read_only("/data"),
         Time = clock {
        app::run()
    }
}

// a library: uses Fs, but can only see what its caller allows
pub fn load_plugin(p: &Plugin) with fs: Fs, fail(PluginError) {
    with Fs = fs.only(p.dir()) {        // narrow before running untrusted code
        p.init()?
    }
}
```

- **Only `main` receives root capabilities**, and it declares exactly which ones it uses
  ([`lifecycle.md`: `main` declares its root capabilities](lifecycle.md#main-declares-its-root-capabilities)). Real OS handlers are built
  only from them, so a library can never create a network or file-system handler of its own.
- A library may:
  - **use** the effects its signature declares (and its mod's ceiling allows,
    [`packages-and-supply-chain.md`: Capability-scoped dependencies](packages-and-supply-chain.md#capability-scoped-dependencies));
  - **interpose** on a handler already in scope;
  - **narrow** a handler before passing control on.
- So there is **no ambient authority**: what code can do is bounded by its signature, its mod's
  ceiling and the handlers its callers chose.
- Tests and simulations build handlers from simulation values rather than root capabilities, so the same
  code runs against fakes ([`determinism.md`](determinism.md)).

Capabilities are a **security boundary**, not only an audit aid: code in a mod without `unsafe` or
`ffi` can use only the authority it is passed ([`security.md`: The guarantee: capabilities as a boundary](security.md#the-guarantee-capabilities-as-a-boundary)).

### Capability values

When code needs **two instances of the same trait at once** (a bridge between two networks, a test
asserting on two nodes), handlers are passed as values (Effekt's capability-passing style):

```rust
fn bridge(a: &dyn Net, b: &dyn Net) with suspend { … }
```

These values, and the names bound by `with net: Net`, are **second-class**: like Moddil's borrows,
they cannot escape their scope, which keeps them cheap and guarantees the handler outlives its uses.
A struct that holds one is a [view type](memory.md#view-types).

### Signatures and inference

- **Inferred inside a mod**, **explicit on `pub` signatures** ([LLMs as first-class programmers](agent-interface.md)). The reviewer view
  shows inferred effects inline ([`agent-interface.md`: Verbosity is a view](agent-interface.md#verbosity-is-a-view)).
- A function that **calls** a trait's methods binds a name (`with net: Net`), even when private;
  one that only reaches it through callees needs nothing inside a mod and `with Net` at `pub`.
- An effect set is a **set**: order doesn't matter, and a function with fewer effects can be used
  where more are allowed.
- **No effect aliases.** Every effect is listed in a public signature, so a reader never has to look
  up what `Io` means. Coarse effects ([The effect catalogue](#the-effect-catalogue)) keep the lists short; whether they stay short enough is
  open ([Open questions](#open-questions)).

### Effect polymorphism

An **unannotated closure parameter is implicitly polymorphic**: the function performs whatever the
closure performs.

```rust
fn visit<T>(xs: &[T], f: impl Fn(&T)) {        // no `with`: whatever f does, visit does
    for x in xs { f(x) }
}

visit(&orders, |o| send(o)?)                    // this call is `with Net, suspend, fail(SendError)`
visit(&orders, |o| total += o.qty)              // this one is pure
```

- `with E` is written only to **name or constrain** the effects, e.g. to say that two closures share
  effects, or that `f` must not suspend:
  `fn map<T, U, E>(xs: &[T], f: impl FnMut(&T) -> U with E) -> Vec<U> with E`.
- **Stored closures state their effects** ([`closures.md`: Stored (escaping) closures](closures.md#stored-escaping-closures)):
  `Box<dyn Fn(Event) with suspend, fail(HandlerError)>`.
- This generalises Swift's `rethrows` to every effect, and avoids Java's checked-exception trap.

### Restricted code is one mechanism

| Context | Allowed effects |
|---|---|
| `drop` | None, so neither `suspend` nor `spawn` ([`error-handling.md`](error-handling.md) D9) |
| Comptime | None: pure and sandboxed ([`comptime.md`](comptime.md)) |
| GPU kernels | A restricted set: no `suspend`, no `spawn`, no allocation, no I/O ([`accelerators.md`](accelerators.md)) |
| Durable workflows | Replayable effects only ([`determinism.md`: Record and replay](determinism.md#record-and-replay)) |
| Freestanding profiles | No OS effects; no `spawn` without a scheduler ([`freestanding.md`](freestanding.md)) |

Each is an effect-set constraint, checked by the same compiler pass and reported with the same
diagnostics.

### Compilation

- **Handlers known statically** (production `main`): calls through an effect become direct calls,
  by monomorphisation or Koka-style evidence passing. Production pays nothing for testability.
- **Handlers that capture other handlers** carry them like a closure's environment
  ([Effects inside handlers](#effects-inside-handlers)); when everything is known statically this is
  still direct calls.
- **Abortive handlers** compile to ordinary returns ([`error-handling.md`: Results and effects](error-handling.md#results-and-effects)).
- **One-shot continuations** use stack switching natively and WASM stack switching or JSPI in the
  browser ([`compiler.md`: WASM concurrency lowering](compiler.md#wasm-concurrency-lowering)).
- **Dynamic handlers** (tests, simulations) pass a hidden `&dyn Trait` argument, which is cheap
  enough for tests.

### Ergonomics: the biggest risk

No mainstream language has made a typed effect system popular. The measures:
- **One concept:** effects are traits, so there is nothing new to learn about declaring or
  implementing them; only `with` is new.
- **Inference** inside modules; effects are written only at `pub` boundaries.
- **Implicit polymorphism** for closures ([Effect polymorphism](#effect-polymorphism)), so higher-order code reads like Rust.
- **Ambient observability**: logging never pollutes signatures.
- **Coarse effects**, with narrowing done by handlers.
- **The reviewer view** shows inferred effects without putting them in the file.
- **Agent-grade diagnostics**, e.g. "`parse` uses `Net` (via `fetch_schema`, line 40), which
  `acme/json`'s ceiling does not allow".

## Fit check against the reference examples

| Example | Uses |
|---|---|
| [Multiplayer game](examples/01-multiplayer-game.md) | Simulation handlers for `Net` and `Time` in rollback tests; the restricted kernel set for GPU work |
| [HFT](examples/02-hft-platform.md) | Statically known handlers in production (zero cost); recorded `Net` and `Time` for whole-day replay |
| [Web store](examples/03-web-store.md) | A user-defined `Db` trait instead of threading a pool; narrowed `Fs` for uploads |
| [Hashiverse](examples/04-hashiverse.md) | `TimeProvider`, `TransportFactory` and `ClientStorage` stay traits but are used after `with`, so the `RuntimeServices` plumbing disappears; the in-memory transport becomes a handler |

## Directions (not yet decided)

- **Generators** as a built-in one-shot `yield` effect, so iterators can be written as loops.
- **Context traits** in place of `Context` objects: a database pool, configuration or a request ID
  provided by a handler.

## Open questions

- Without aliases, do public signatures of high-level functions (a request handler with `Net`,
  `Fs`, `Time`, `Rand`, `suspend` and `fail(…)`) become too long? Revisit once the reference
  examples are written in Moddil.
- Should generators (`yield`) be a built-in one-shot effect, and should user traits ever get
  one-shot continuations?
- **User-defined abortive handlers:** how are they written? Perhaps a trait method returning `!`,
  whose handler supplies the value of the whole `with` block ([Handler tiers](#handler-tiers)).
- May a trait with a single method be installed from a plain closure (`with Time = || t0`), or is
  an anonymous impl short enough ([Defining handlers](#defining-handlers))?
- **Is every trait usable after `with`?** `with Display` is harmless but meaningless. Should a trait
  opt in, or is any trait whose methods take `&self` allowed?
- **Generic traits as effects:** `with store: Store<User>` and `with Store<Order>` in one scope are
  two different needs. Is each instantiation installed separately?
- How exactly are effect requirements and handler stacks surfaced, so that dynamic handler lookup is
  never surprising (LSP, `explain`)?

## Prior art

Leijen, *Koka* papers and "Generalized Evidence Passing for Effect Handlers" (2021); Brachthäuser et
al., *Effekt* (effects as capabilities, 2020); Sivaramakrishnan et al., "Retrofitting Effect
Handlers onto OCaml" (2021); Unison abilities; Flix; Scala 3 `using`/`given` and capture checking;
Miller, *Robust Composition* (object capabilities, 2006); Austral; WASI; Swift `rethrows`; Rust
keyword generics.

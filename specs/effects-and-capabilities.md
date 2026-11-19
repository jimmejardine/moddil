# Effects and Capabilities

> **Status:** decided direction, open questions remain. Indexed in [`README.md`](README.md).
> Snippets are **illustration only**.

## Summary

| Rule | One line |
|---|---|
| **Effects are interfaces; handlers are scoped implementations** | `effect time { fn now() -> Instant }`; `with time` in a signature; `with time = sim.clock(n) { … }` installs a handler for a block. |
| **Handler tiers** | User effects get function-like (tail-resumptive) and abortive handlers. One-shot continuations are reserved for core and standard effects (`suspend`, generators). Resuming twice is never allowed. |
| **Authority comes only from root capabilities** | `main` declares the root capabilities it needs (`NetRoot`, `FsRoot`, `Devices`, …); real OS handlers are built only from those. Libraries perform, wrap and narrow effects, but never create authority. |
| **Capability values for two at once** | A handler can also be passed as a second-class value (`fn bridge(a: &Net, b: &Net)`). |
| **Inferred inside, explicit at `pub`** | Effects are inferred inside a mod and written on `pub` signatures; no aliases. |
| **Implicit effect polymorphism** | An unannotated closure parameter means "whatever `f` does, I do too"; `with E` is written only to name or constrain. |
| **One mechanism for restricted code** | Drop, kernels, comptime, workflows and freestanding profiles are all effect-set constraints. |
| **Zero cost when handlers are known** | Effect operations compile to direct calls; abortive handlers to ordinary returns. |

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
| **Effekt** | Effects as **second-class capabilities**: lexical, fast, simple to type | Research language |
| **OCaml 5** | One-shot continuations in a production runtime; schedulers written as libraries | Effects are **untyped**: nothing in signatures |
| **Unison abilities**, **Flix** | Effects in signatures for mainstream-shaped code | Small ecosystems |
| **Haskell effect libraries** (mtl, polysemy, effectful) | Expressive | Heavy types; performance traps |
| **Scala 3 capture checking** | Capabilities tracked in types | Experimental |
| **ZIO, Effect-TS** | A typed error and environment channel | Library-level; everything is wrapped in an effect value |
| **Swift `rethrows`**, **Rust keyword generics** | Polymorphism over one effect (throwing, `async`/`const`) | One effect at a time |
| **Java checked exceptions** | Failure in signatures | The lesson: no polymorphism, so lambdas and generics broke, and people wrapped everything in unchecked exceptions |
| **E, Austral, WASI** | Object capabilities: authority only by holding a reference | Austral and E are small; WASI covers the host boundary only |

## Decisions

### Effects are interfaces; handlers are scoped implementations

```rust
effect time {
    fn now() -> Instant
}

fn deadline(ms: u64) -> Instant with time {
    time.now() + ms<ms>
}

with time = sim.clock(node) {          // installs a handler for this block
    run_node()                         // everything inside, however deep, uses sim.clock
}
```

- A handler is installed **around a block**, and the innermost one wins
  ([`determinism.md`: Handlers are scoped, not global](determinism.md#handlers-are-scoped-not-global)).
- Spawned tasks **inherit** their scope's handlers; structured concurrency guarantees the handlers
  outlive them.
- A handler can **interpose**: forward to the enclosing handler while recording, delaying, failing
  or asserting. Mocks, spies, fault injection and recording all compose by nesting.
- This is compiler-checked dependency injection. It replaces mocking frameworks, `Context` objects
  and hand-written provider traits.

### The effect catalogue

| Kind | Effects | Notes |
|---|---|---|
| **Core** | `fail(E)`, `suspend`, `spawn` | Defined by the language ([`error-handling.md`](error-handling.md), [`concurrency.md`](concurrency.md)) |
| **Standard** | `time`, `rand`, `net`, `fs`, `stdin`, `stdout`, `stderr`, `env`, `process`, `gpu`, `remote`, `introspect`, `locale`, `dom` | Interfaces in standard mods; handlers in runtime and simulation mods ([`runtime.md`: Pluggable runtimes without ecosystem splits](runtime.md#pluggable-runtimes-without-ecosystem-splits)) |
| **Capabilities with no handler to swap** | `ffi`, `unsafe` | Declared and visible to every consumer; treated as high risk ([`interop.md`: The safety boundary](interop.md#the-safety-boundary)) |
| **Ambient, write-only** | `log`, `span`, `metric` | Never written in signatures; cannot feed values back ([`observability.md`](observability.md)) |
| **User-defined** | anything | Context such as a database pool, configuration, feature flags ([Directions (not yet decided)](#directions-not-yet-decided)) |

**Not effects:**
- **allocation**: a capability chosen per profile ([`freestanding.md`](freestanding.md));
- **bugs and traps**: contract violations abandon the task ([`error-handling.md`](error-handling.md));
- **non-termination**.

**Effects are coarse.** There is one `fs`, not `fs.read` and `fs.write`. Finer limits come from
**narrowing a handler** (`fs.read_only("/data")`), not from more effect names. This keeps
signatures short ([Signatures and inference](#signatures-and-inference)).

**No umbrella effects.** Each effect is listed on its own, and none implies another: there is no
`io` effect. A function that uses the network and may suspend writes `with net, suspend`.

### Handler tiers

| Tier | What the handler can do | Who can define it | Compiles to |
|---|---|---|---|
| **Tail-resumptive** | Return a value to the caller, like a function | Anyone | A direct (or indirect) call |
| **Abortive** | Never resume; unwind to the handler, like an exception | Anyone | An ordinary return (`fail`, [`error-handling.md`: Results and effects](error-handling.md#results-and-effects)) |
| **One-shot continuation** | Capture the rest of the computation and resume it **once**, later | Core and standard effects only: `suspend`, generators | Stack switching ([`runtime.md`: Runtime contract](runtime.md#runtime-contract), [`compiler.md`: WASM concurrency lowering](compiler.md#wasm-concurrency-lowering)) |

- **No multi-shot resumption**, ever. Resuming twice would run the rest of the computation twice,
  duplicating linear values and running drops twice ([`memory.md`](memory.md)).
- Almost every use (I/O, context, mocking, simulation, recording) is tail-resumptive, so user
  effects are cheap and easy to reason about.

### Authority comes only from root capabilities

```rust
fn main(net: NetRoot, files: FsRoot, clock: Clock, devs: Devices) {
    with net  = net,
         fs   = files.read_only("/data"),
         time = clock {
        app::run()
    }
}

// a library: performs fs, but can only see what its caller allows
fn load_plugin(p: &Plugin) with fs, fail(PluginError) {
    with fs = fs.only(p.dir()) {        // narrow before running untrusted code
        p.init()?
    }
}
```

- **Only `main` receives root capabilities**, and it declares exactly which ones it uses
  ([`lifecycle.md`: `main` declares its root capabilities](lifecycle.md#main-declares-its-root-capabilities)). Real OS handlers are built
  only from them, so a library can never create a network or file-system handler of its own.
- A library may:
  - **perform** the effects its signature declares (and its mod's ceiling allows,
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

When code needs **two instances of the same effect at once** (a bridge between two networks, a test
asserting on two nodes), a handler is passed as a value (Effekt's capability-passing style):

```rust
fn bridge(a: &Net, b: &Net) with suspend { … }
```

These values are **second-class**: like Moddil's borrows, they cannot escape their scope, which
keeps them cheap and guarantees the handler outlives its uses.

### Signatures and inference

- **Inferred inside a mod**, **explicit on `pub` signatures** ([LLMs as first-class programmers](agent-interface.md)). The reviewer view
  shows inferred effects inline ([`agent-interface.md`: Verbosity is a view](agent-interface.md#verbosity-is-a-view)).
- An effect set is a **set**: order doesn't matter, and a function with fewer effects can be used
  where more are allowed.
- **No effect aliases.** Every effect is listed in a public signature, so a reader never has to look
  up what `io` means. Coarse effects ([The effect catalogue](#the-effect-catalogue)) keep the lists short; whether they stay short enough is
  open ([Open questions](#open-questions)).

### Effect polymorphism

An **unannotated closure parameter is implicitly polymorphic**: the function performs whatever the
closure performs.

```rust
fn visit<T>(xs: &[T], f: impl Fn(&T)) {        // no `with`: whatever f does, visit does
    for x in xs { f(x) }
}

visit(&orders, |o| send(o)?)                    // this call is `with net, suspend, fail(SendError)`
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
| `drop` | None ([`error-handling.md`](error-handling.md) D9) |
| Comptime | None: pure and sandboxed ([`comptime.md`](comptime.md)) |
| GPU kernels | A restricted set: no suspension, no allocation, no I/O ([`accelerators.md`](accelerators.md)) |
| Durable workflows | Replayable effects only ([`determinism.md`: Record and replay](determinism.md#record-and-replay)) |
| Freestanding profiles | No OS effects ([`freestanding.md`](freestanding.md)) |

Each is an effect-set constraint, checked by the same compiler pass and reported with the same
diagnostics.

### Compilation

- **Handlers known statically** (production `main`): effect operations become direct calls, by
  monomorphisation or Koka-style evidence passing. Production pays nothing for testability.
- **Abortive handlers** compile to ordinary returns ([`error-handling.md`: Results and effects](error-handling.md#results-and-effects)).
- **One-shot continuations** use stack switching natively and WASM stack switching or JSPI in the
  browser ([`compiler.md`: WASM concurrency lowering](compiler.md#wasm-concurrency-lowering)).
- **Dynamic handlers** (tests, simulations) pass a hidden handler argument, which is cheap enough for
  tests.

### Ergonomics: the biggest risk

No mainstream language has made a typed effect system popular. The measures:
- **Inference** inside modules; effects are written only at `pub` boundaries.
- **Implicit polymorphism** for closures ([Effect polymorphism](#effect-polymorphism)), so higher-order code reads like Rust.
- **Ambient observability**: logging never pollutes signatures.
- **Coarse effects**, with narrowing done by handlers.
- **The reviewer view** shows inferred effects without putting them in the file.
- **Agent-grade diagnostics**, e.g. "`parse` performs `net` (via `fetch_schema`, line 40), which
  `acme/json`'s ceiling does not allow".

## Fit check against the reference examples

| Example | Uses |
|---|---|
| [Multiplayer game](examples/01-multiplayer-game.md) | Simulation handlers for `net` and `time` in rollback tests; the restricted kernel set for GPU work |
| [HFT](examples/02-hft-platform.md) | Statically known handlers in production (zero cost); recorded `net` and `time` for whole-day replay |
| [Web store](examples/03-web-store.md) | A user-defined `db` context effect instead of threading a pool; narrowed `fs` for uploads |
| [Hashiverse](examples/04-hashiverse.md) | `TimeProvider`, `TransportFactory` and `ClientStorage` become effects; the in-memory transport becomes a handler |

## Directions (not yet decided)

- **Generators** as a standard one-shot `yield` effect, so iterators can be written as loops.
- **Context effects** in place of `Context` objects: a database pool, configuration or a request ID
  provided by a handler.

## Open questions

- Without aliases, do public signatures of high-level functions (a request handler with `net`,
  `fs`, `time`, `rand`, `suspend` and `fail(…)`) become too long? Revisit once the reference
  examples are written in Moddil.
- Should generators (`yield`) be a standard one-shot effect, and should user effects ever get
  one-shot continuations?
- Is `spawn` a separate effect from `suspend`, or implied by it?
- How exactly are effect requirements and handler stacks surfaced, so that dynamic handler lookup is
  never surprising (LSP, `explain`)?

## Prior art

Leijen, *Koka* papers and "Generalized Evidence Passing for Effect Handlers" (2021); Brachthäuser et
al., *Effekt* (effects as capabilities, 2020); Sivaramakrishnan et al., "Retrofitting Effect
Handlers onto OCaml" (2021); Unison abilities; Flix; Scala 3 capture checking; Miller, *Robust
Composition* (object capabilities, 2006); Austral; WASI; Swift `rethrows`; Rust keyword generics.

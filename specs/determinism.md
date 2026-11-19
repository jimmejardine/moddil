# Determinism and Reproducibility

> **Status:** complete: decided, with no open questions. Indexed in [`README.md`](README.md). Snippets are
> **illustration only**.

## Summary

| Rule | One line |
|---|---|
| **R4: deterministic by construction** | Pure Moddil code under deterministic handlers gives bit-identical results on every tier-1 target ([`requirements.md`](requirements.md#r4-deterministic-by-construction)). |
| **Nondeterminism only through effects** | Time, randomness, I/O, scheduling and the environment are effects or root capabilities; evaluation and drop order are specified. |
| **One `nondet` effect** | Fast-math, relaxed SIMD, hardware approximations, float atomics and unordered reductions perform `nondet`; the defaults are deterministic ([The `nondet` effect](#the-nondet-effect)). |
| **Correctly rounded maths** | Every `f32`/`f64` function in `moddil/math` is correctly rounded; `math::fast` is faster and still deterministic ([Deterministic maths](#deterministic-maths)). |
| **Secure, seedable randomness** | Every `Rand` handler is a CSPRNG: the OS in production, seeded ChaCha20 in simulation ([Randomness](#randomness)). |
| **Simulation is a standard mod** | `moddil/runtime-sim`: a seeded scheduler, virtual time, a simulated network and disk, and fault injection ([Deterministic simulation testing](#deterministic-simulation-testing)). |
| **Per-task recording** | Effect results and scheduling decisions are logged per task, in the flight recorder's ring buffer or in full per scope ([Record and replay](#record-and-replay)). |
| **Reproducible builds** | The same source, lockfile and toolchain give bit-identical artefacts ([Reproducible builds](#reproducible-builds)). |

## The principle

**Decided, as firm requirement R4
([`requirements.md`: R4](requirements.md#r4-deterministic-by-construction)):** *the core language is
deterministic, and every source of nondeterminism enters through an effect or a root capability.*

Pure code run under deterministic handlers then produces **bit-identical** results on every target.
The exceptions are a closed, visible list: the `ffi` capability, `unsafe` code that breaks the rules,
and the [`nondet` effect](#the-nondet-effect).

What it enables:
- deterministic simulation testing of whole systems ([Deterministic simulation testing](#deterministic-simulation-testing));
- exact record and replay of production runs ([Record and replay](#record-and-replay));
- statically checked durable workflows ([Record and replay](#record-and-replay));
- cross-platform lockstep, such as a game simulation running identically in the browser and on the
  server ([Cross-platform determinism](#cross-platform-determinism)).

## Sources of nondeterminism

| Source | Typical languages | Moddil |
|---|---|---|
| Evaluation order | Unspecified in C and C++ | Specified left to right; drop order specified |
| Time, randomness, environment, process IDs, I/O | Ambient global functions | Effects and root capabilities only ([`lifecycle.md`: `main` declares its root capabilities](lifecycle.md#main-declares-its-root-capabilities)) |
| Scheduling, `select` fairness | Hidden inside the runtime | Scheduling belongs to the runtime handler; random choices draw from `Rand` |
| Hash-map iteration order | Rust randomises the seed; Go deliberately randomises iteration | The hash seed comes from `Rand`: protection against hash-flooding in production, a fixed seed in simulation. Maps iterate in seeded hash order ([`values.md`](values.md#map-iteration-order-seeded-hash-order)); `OrderedMap` and `TreeMap` when order matters; tests use a fresh, printed seed. |
| Pointer addresses | Observable (hashing or sorting by address) | Safe code cannot turn an address into an integer ([`unsafe.md`: Strict provenance](unsafe.md#strict-provenance)) |
| Floating point | FMA contraction, x87 extended precision, fast-math reordering, flush-to-zero, differing `libm`, WASM NaN bit patterns, relaxed SIMD | **Strict IEEE 754**: no implicit FMA, no reordering. Correctly rounded `moddil/math` ([Deterministic maths](#deterministic-maths)). NaN canonicalisation where observable. Fast-math and relaxed SIMD perform [`nondet`](#the-nondet-effect). |
| Platform integer width | `usize` is 32-bit on wasm32, 64-bit natively | `int` (lengths, indices, counts) is 64-bit on every target ([`numerics.md`: Integer types](numerics.md#integer-types)). `usize` exists only for FFI and memory-level code, and a lint flags it in cross-target code. |
| Parallel float reductions, GPU atomics | Order of additions varies | Fixed-order reductions by default; float atomics and unordered reductions perform [`nondet`](#the-nondet-effect) |
| Native FFI | Anything | Behind the `ffi` capability; outside the guarantee unless wrapped by effects |
| Locale | Process-wide settings change number formats, case mapping and sorting (C `setlocale`, Java `Locale.getDefault()`) | Formatting and case mapping are locale-independent; locale-dependent code performs the `Locale` effect ([`text.md`: Localisation: the `Locale` effect](text.md#localisation-the-locale-effect)) |

## The `nondet` effect

**Decided: all opt-in nondeterminism performs one built-in effect, `nondet`.** Like `ffi` and
`unsafe`, it has no handler to swap
([`effects-and-capabilities.md`: The effect catalogue](effects-and-capabilities.md#the-effect-catalogue)).

| Performs `nondet` | Deterministic default |
|---|---|
| Fast-math blocks (reassociation, contraction to FMA) | Strict IEEE 754 |
| Relaxed SIMD instructions | Portable `Simd<T, N>` operations |
| Hardware approximations (`rsqrt` estimates) | `moddil/math` and `math::fast` |
| Float atomics on GPUs and CPUs | Integer atomics (integer addition is associative) |
| Unordered parallel or GPU reductions | Fixed-order (tree) reductions |

```rust
kernel fn splat(points: &[Point], grid: &mut [f32]) with nondet {   // float atomics: order varies
    for p in points { atomic_add(&mut grid[p.cell], p.mass) }
}
```

- It is **declared per function or kernel**, never by a global flag or build setting. Like every
  effect, it is inferred inside a mod and written on `pub` signatures.
- **Forbidden where replay or lockstep matters:** durable workflows, lockstep game simulation and
  any scope that requires replay reject it, through the same effect-set check as other restricted
  code ([`effects-and-capabilities.md`: Restricted code is one mechanism](effects-and-capabilities.md#restricted-code-is-one-mechanism)).
- The recorder flags code that performs `nondet` as not replayable
  ([Record and replay](#record-and-replay)).

## Deterministic maths

**Decided: every `f32` and `f64` elementary function in `moddil/math` is correctly rounded**
(`exp`, `log`, `pow`, trigonometric and hyperbolic functions and their inverses), as CORE-MATH has
shown can be done at speeds close to existing `libm`s. A correctly rounded result is unique, so it is
bit-identical on every target and every implementation.

- **`math::fast`** gives portable software versions with a stated error bound (a few ulp). They are
  faster and **still deterministic**, because Moddil ships the same implementation for every target.
- Only **hardware-specific approximations** perform `nondet`.

## Randomness

**Decided: one `Rand` trait, and every `Rand` handler is cryptographically secure.**

- In production, the handler is built from the **`Entropy`** root capability (the OS CSPRNG,
  [`lifecycle.md`: `main` declares its root capabilities](lifecycle.md#main-declares-its-root-capabilities)).
- In simulation, it is a **seeded ChaCha20**, so keys and nonces are reproducible. Simulated keys
  never leave a simulation.
- **Fast non-cryptographic generators** (games, Monte Carlo) are plain values seeded from `Rand`, so
  they are deterministic by construction.

## Deterministic simulation testing

`moddil/runtime-sim` ([`runtime.md`: Runtime contract](runtime.md#runtime-contract)) is a
first-class standard mod, and CI runs the standard library under it. It provides:
- a **seeded, single-threaded scheduler**;
- **virtual time**, advanced **only when every task is blocked** (quiescence), jumping straight to the
  next timer, so hours of simulated time run in milliseconds;
- a **simulated network**: latency, loss, reordering, duplication, partitions;
- a **simulated disk** with faults: torn writes, an `fsync` that lies, full disks;
- **fault-injection points** (FoundationDB's `BUGGIFY`) as an effect that does nothing in production.

**Whole systems in one process:** every placement or tier ([Distributed and multi-tier programs](distribution.md)) becomes a simulated node.
This covers all four [reference examples](examples/README.md).

**Searching for bugs:**
- run many seeds, with swarm testing (randomly varying which faults are enabled);
- coverage-guided seed selection;
- shrink failing seeds to minimal traces;
- check liveness after faults stop, not just safety (TigerBeetle's VOPR).

**Self-check:** every seed is run twice and the state hashes compared. Any difference is accidental
nondeterminism, caught immediately (TigerBeetle does this).

### The `Sim` API

**Decided**, in the shape used by the [tour](tour.md):

| Operation | Shape |
|---|---|
| Create a simulation and its nodes | `Sim::new(seed: 42)`, `sim.node()` |
| Per-node handlers | `sim.clock(node)`, `sim.rng(node)`, `sim.net.endpoint(node)`, `sim.disk(node)` |
| Faults | Interposing handlers (`drop_messages(10.percent)`), `sim.net.partition(a, b)`, `sim.net.heal()` |
| Standard faults | Network: drop, delay, duplicate, reorder, partition, heal. Disk: write errors, torn writes, lost `fsync`. Clocks: skew, jumps. Nodes: crash and restart. All seed-driven; more are added as interposing handlers |
| Running and stopping | `sim.run_until(condition)`; `sim.stop_all()` requests every node to stop |

### Peers not written in Moddil

**Decided: three ways in, in order of preference:**
1. **WASM components** run under the deterministic host ([`interop.md`: WASM components: the sandboxed path](interop.md#wasm-components-the-sandboxed-path));
2. a **Moddil model** of the peer's protocol, written as a handler;
3. **replay of recorded traffic** at the `Net` boundary.

Whole unmodified systems need a deterministic hypervisor such as Antithesis, outside the language.

### Limits

- A single-threaded simulation cannot explore weak-memory behaviour. Moddil code is data-race free,
  but the internals of sync cells and the runtime need Loom-style model checking in the standard
  library ([`correctness.md`: Systematic concurrency testing](correctness.md#systematic-concurrency-testing)).

## Handlers are scoped, not global

Effect handlers are installed **around a block** and apply to everything that runs inside it; the
innermost handler wins ([`effects-and-capabilities.md`](effects-and-capabilities.md)). This gives
test harnesses fine-grained control without passing services through constructors.

```rust
// production main: handlers built from root capabilities, installed once
fn main(net: NetRoot, clock: Clock, entropy: Entropy) {
    with Time = clock, Rand = entropy.rng(), Net = net {
        server.run()
    }
}

// test: six servers in one process, each with its own dependencies
let sim = Sim::new(seed: 42);
scope(|s| {
    for node in sim.nodes(6) {
        with Time    = sim.clock(node, skew: node.id * 5[ms]),
             Rand    = sim.rng(node),
             Net     = sim.net.endpoint(node),
             Storage = sim.disk(node) {
            s.spawn(|| server.run());    // unchanged production code
        }
    }
});
```

What this allows:
- **Per node:** the same code with different instances of its dependencies (clock skew, separate
  disks and RNG streams).
- **Per subsystem:** wrap one call, so that only that part sees the fault.
  ```
  with Net = drop_messages(30.percent) { replicate(batch)? }   // captures the outer Net
  ```
- **Interposition:** a handler can forward to the enclosing one while recording, delaying, failing or
  asserting. Typed mocks, spies, fault injection and recording all compose by nesting.
- **Mixed real and simulated:** for example a real filesystem with a simulated network.
- **Test-driven state:** advance node 3's clock only; partition nodes 1–2 from 3–5; heal later.
- **Inheritance:** spawned tasks inherit their parent scope's handlers unless they override them.
  Structured concurrency guarantees the handlers outlive the tasks.
- **Two instances side by side** (a bridge between networks, a test asserting on two nodes) use
  second-class capability values, `fn bridge(a: &dyn Net, b: &dyn Net)`
  ([`effects-and-capabilities.md`: Two handlers of one trait](effects-and-capabilities.md#two-handlers-of-one-trait)).

**Performance:** handlers known at compile time (as in production `main`) become direct calls.
Production pays nothing for testability.

**Risk:** with dynamic scoping, which handler applies to a call can be non-obvious. Mitigations: the
language server shows the effect requirements at a call site and the handler stack in tests and
simulations; `moddil explain` shows the installed handlers.

## Record and replay

Because every source of nondeterminism is an effect, **logging effect results at handler boundaries
is enough to replay an execution exactly**. It works like `rr`, but at the language level, so it is
portable and much cheaper. Recorded runs also supply reproducible PGO profiles
([`performance.md`: Optimisation control](performance.md#optimisation-control)).

Uses:
- reproducing production incidents;
- time-travel debugging (as in rr, WinDbg TTD, Replay.io, Elm's debugger);
- replaying a whole trading day ([HFT example](examples/02-hft-platform.md));
- **replaying production traffic through a new code version**, with detection of where the runs
  diverge, as regression and shadow testing.

### Format and retention

**Decided: one effect log per task.**

- Each task's log holds its **effect results** and the **scheduling decisions** that affected it
  (wake order, `select` choices). Logs are linked through the task tree, so a request, a workflow or
  a whole process can be replayed from the logs of its tasks.
- The **flight recorder** keeps logs in a bounded ring buffer, opt-in per scope and sampleable per
  request, and dumps them on an error or trap
  ([`observability.md`: Flight recorder and replay](observability.md#flight-recorder-and-replay)).
- **Full recording** is opt-in per scope; volume and retention are configuration.
- **Secrets** are recorded through a redacting handler.
- Code that performs `nondet` or uses `ffi` without an effect wrapper is **flagged as not
  replayable**.

**Durable execution.** Temporal and Restate rely on deterministic replay of workflow code, and can
only check it at runtime. In Moddil a workflow's effect row may contain only replayable effects, and
never `nondet`, so **workflow determinism is checked by the compiler** (durable workflows,
[Distributed and multi-tier programs](distribution.md)).

## Reproducible builds

The same source, lockfile and toolchain produce **bit-identical artefacts**, a requirement of the
toolchain and of every mod release:
- no timestamps or build paths in outputs (path remapping);
- deterministic parallel code generation;
- pure, sandboxed comptime ([`comptime.md`](comptime.md));
- hash-pinned dependencies.

Anyone can rebuild a tagged version of a mod and compare the bytes ([Package management and supply-chain security](packages-and-supply-chain.md)). The bootstrap fixed point, once the
compiler is self-hosted ([`compiler.md`: Staged bootstrap](compiler.md#staged-bootstrap)), will use the same machinery. Prior art: Go toolchains are fully
reproducible from Go 1.21; Debian Reproducible Builds; Nix and Guix; Bazel's hermetic builds.

Tests run in a shuffled order and print the seed (Go's `-shuffle`), so order dependence is found and
reproducible.

## Cross-platform determinism

Bit-identical results across x86, ARM, WASM and GPU (for code without `nondet`), for game lockstep
([game example](examples/01-multiplayer-game.md)), HFT replay on different hardware, and consensus
systems. It follows from [Sources of nondeterminism](#sources-of-nondeterminism): strict floating
point, correctly rounded maths, and fixed-width integers.

## Prior art: Hashiverse

[Hashiverse](examples/04-hashiverse.md) already applies the core idea by hand, and is the strongest
real-world evidence for it.

**What it does** (`hashiverse-lib/src/tools/`):
- `RuntimeServices` bundles `Arc<dyn TimeProvider>`, `Arc<dyn TransportFactory>` and
  `Arc<dyn PowGenerator>`, threaded through every constructor. Tests swap in an in-memory transport, a
  virtual clock and a single-threaded PoW generator, so a whole network runs in one test binary.
- `TimeProvider` has real, scaled and manual implementations. `ManualTimeProvider` keeps a min-heap
  of wakers, and a driver task jumps the clock to the next wake time, so simulated days run in
  milliseconds.

**What it shows:**
1. The pattern works and is valuable.
2. **Hand-written seams are incomplete by construction.** Randomness is not behind a trait: free
   functions call the OS RNG directly, at about 38 call sites (RPC, Kademlia, keys, PoW, posting).
   Tokio's multi-threaded scheduling is not under test control either. So tests are deterministic in
   time but not in randomness or interleaving. With effects, using `Rand` without a handler does not
   compile.
3. **Plumbing cost:** `Arc<dyn …>` through constructors, a boxed future per `sleep`, and a services
   bundle that grows with each new service. Effects remove it.
4. **Heuristic time driving:** the driver calls `yield_now()` hoping other tasks register their sleeps
   first, which is racy on a multi-threaded runtime. In Moddil the scheduler and the clock are one
   handler, advancing time only at quiescence.
5. **Cryptographic randomness:** keys need real entropy in production and reproducible entropy in
   simulation, which [Randomness](#randomness) provides.
6. **Next steps Hashiverse doesn't have yet,** and a Moddil port gets for free: seeded scheduling,
   record and replay, and the determinism self-check.

## Costs

- Strict floating point forbids implicit FMA and fast-math, so some numeric code is slower. Fast-math
  is available, but performs `nondet`.
- Correctly rounded maths is slightly slower than the fastest `libm`s; `math::fast` closes most of
  the gap.
- Recording adds overhead (hence the ring buffer and per-scope opt-in).
- A simulation is only as faithful as its models.

## Fit with the reference examples

| Example | What determinism gives it |
|---|---|
| [Game](examples/01-multiplayer-game.md) | Identical simulation on browser WASM and native server; whole system simulated with latency and packet loss; `nondet` kept out of the lockstep code |
| [HFT](examples/02-hft-platform.md) | Bit-identical backtest and replay of a trading day; replay of production traffic through new strategy versions |
| [Web store](examples/03-web-store.md) | Simulated rolling deploys with mixed versions; reproducible order-flow tests |
| [Hashiverse](examples/04-hashiverse.md) | The existing harness, plus deterministic randomness and interleavings, record and replay, and the self-check |

## Resolved questions

Decided in the open-question review:

1. **The `Sim` API** is decided as [above](#the-sim-api), with its standard faults.
2. **Requiring replay:** durable workflows and lockstep code require it automatically. Any other
   scope opts in, with `scope(…, replayable: true)` or `#[replayable]` on a function, and the
   compiler then rejects [`nondet`](#the-nondet-effect) inside it. Typical users: consensus code,
   ledgers, trading strategies.
3. **GPU determinism holds across vendors** for code without `nondet`: strict IEEE, correctly
   rounded maths and fixed-order reductions give the same bits on any GPU and on the CPU simulator.
   Tensor-core paths whose accumulation order a vendor leaves open, and fast intrinsics, perform
   `nondet` ([`accelerators.md`](accelerators.md)).

## Open questions

None at present.

## Background

The problem and the state of the art that led to this design.

- **Problem.** Concurrent and distributed bugs are hard to reproduce.
- **State of the art.** FoundationDB and TigerBeetle deterministic simulation testing; `rr` record and
  replay; Loom (Rust) model checking for concurrency.
- **Directions.** Since time, randomness, I/O and scheduling are effects/capabilities, a test harness
  can replace them with deterministic versions, giving **deterministic simulation testing as a built-in
  mode** with seed-based replay.

## Prior art

- FoundationDB simulation testing and `BUGGIFY`; TigerBeetle VOPR; Antithesis.
- rr and Pernosco; WinDbg Time Travel Debugging; Replay.io; Elm's time-travel debugger.
- Temporal; Restate.
- Loom; shuttle; tokio's paused clock.
- Effekt (capability passing); Koka (evidence passing); OCaml 5 effect handlers.
- Debian Reproducible Builds; Go reproducible toolchains (Go 1.21); Nix; Guix; Bazel.
- CORE-MATH; crlibm; Wasmtime NaN canonicalisation; PyTorch deterministic algorithms.
- Hashiverse `RuntimeServices` and `ManualTimeProvider`.

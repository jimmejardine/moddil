# Determinism and Reproducibility

> **Status:** proposal. Indexed in [`README.md`](README.md). Nothing here is decided;
> recommendations are marked **Recommended**. Syntax is illustrative.

## The principle

**Recommended:** *the core language is deterministic, and every source of nondeterminism enters
through an effect.*

Pure code run under deterministic handlers then produces **bit-identical** results on every target.
Everything in this document either follows from that principle or lists an exception. Candidate for a
firm requirement, **R4: deterministic by construction** (open, see [Costs](#costs)).

What it enables:
- deterministic simulation testing of whole systems ([Deterministic simulation testing](#deterministic-simulation-testing));
- exact record and replay of production runs ([Record and replay](#record-and-replay));
- statically checked durable workflows ([Record and replay](#record-and-replay));
- cross-platform lockstep, such as a game simulation running identically in the browser and on the
  server ([Cross-platform determinism](#cross-platform-determinism)).

## Sources of nondeterminism

| Source | Typical languages | Moddil proposal |
|---|---|---|
| Evaluation order | Unspecified in C and C++ | Specified left to right; drop order specified |
| Time, randomness, environment, process IDs, I/O | Ambient global functions | Effects and capabilities only |
| Scheduling, `select` fairness | Hidden inside the runtime | Scheduling is an effect; random choices draw from `rand` |
| Hash-map iteration order | Rust randomises the seed; Go deliberately randomises iteration | The hash seed comes from the `rand` effect: protection against hash-flooding in production, a fixed seed in simulation. Maps iterate in seeded hash order (decided, [`values.md`](values.md#map-iteration-order-seeded-hash-order)); `OrderedMap` and `TreeMap` when order matters; tests use a fresh, printed seed. |
| Pointer addresses | Observable (hashing or sorting by address) | Safe code cannot turn an address into an integer |
| Floating point | FMA contraction, x87 extended precision, fast-math reordering, flush-to-zero, differing `libm`, WASM NaN bit patterns, relaxed SIMD | **Strict IEEE 754 by default**: no implicit FMA, no reordering. A deterministic `moddil/math` with correctly rounded functions (CORE-MATH style). NaN canonicalisation where observable. Fast-math and relaxed SIMD only by explicit opt-in, **marked nondeterministic** so they are excluded from replayable code. |
| Platform integer width | `usize` is 32-bit on wasm32, 64-bit natively | `int` (lengths, indices, counts) is 64-bit on every target ([`numerics.md`: Integer types](numerics.md#integer-types)). `usize` exists only for FFI and memory-level code, and a lint flags it in cross-target code. |
| Parallel float reductions, GPU atomics | Order of additions varies | Deterministic reductions by default where cheap; GPU determinism opt-in (cf. PyTorch `use_deterministic_algorithms`) |
| Native FFI | Anything | Behind the `ffi` capability; outside the guarantee unless modelled |
| Locale | Process-wide settings change number formats, case mapping and sorting (C `setlocale`, Java `Locale.getDefault()`) | Formatting and case mapping are locale-independent; locale-dependent code performs the `locale` effect ([`text.md`: Localisation: the `locale` effect](text.md#localisation-the-locale-effect)) |

## Deterministic simulation testing

`moddil/runtime-sim` ([`concurrency.md`: Runtime contract](concurrency.md#runtime-contract)) provides:
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

**Self-check (easy win):** run every seed twice and compare state hashes. Any difference is accidental
nondeterminism, caught immediately (TigerBeetle does this).

**Limits:**
- A single-threaded simulation cannot explore weak-memory behaviour. Moddil code is data-race free,
  but the internals of sync cells and the runtime need Loom-style model checking in the standard
  library ([`correctness.md`: Systematic concurrency testing](correctness.md#systematic-concurrency-testing)).
- Non-Moddil peers (for example Hashiverse's Rust servers) only take part if compiled to WASM
  components and run under a deterministic host.
- Prior art for whole unmodified systems: Antithesis, a deterministic hypervisor.

## Handlers are scoped, not global

Effect handlers are installed **around a block** and apply to everything that runs inside it; the
innermost handler wins. This gives test harnesses fine-grained control without passing services
through constructors.

```rust
// production main: handlers installed once
with time = os::clock, rand = os::rng, net = os::net {
    server.run()
}

// test: six servers in one process, each with its own dependencies
let sim = Sim::new(seed: 42);
scope(|s| {
    for node in sim.nodes(6) {
        with time    = sim.clock(node, skew: node.id * 5<ms>),
             rand    = sim.rng(node),
             net     = sim.net.endpoint(node),
             storage = sim.disk(node) {
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
  with net = drop_messages(30.percent, outer: net) { replicate(batch)? }
  ```
- **Interposition:** a handler can forward to the enclosing one while recording, delaying, failing or
  asserting. Typed mocks, spies, fault injection and recording all compose by nesting.
- **Mixed real and simulated:** for example a real filesystem with a simulated network.
- **Test-driven state:** advance node 3's clock only; partition nodes 1–2 from 3–5; heal later.
- **Inheritance:** spawned tasks inherit their parent scope's handlers unless they override them.
  Structured concurrency guarantees the handlers outlive the tasks.

**Two instances side by side.** For code that genuinely needs two clocks or two networks at once (a
bridge between networks, a test asserting on two nodes), a handler can also be a **named capability
value** passed explicitly (Effekt's capability-passing style):

```rust
fn bridge(a: &Net, b: &Net) { … }
```

These capabilities are **second-class**: like Moddil's borrows, they cannot escape their scope.

**Decided:** a hybrid effect design. Effects appear in signatures; handlers are installed by scope
by default; explicit capability values are used where needed
([`effects-and-capabilities.md`: Decisions](effects-and-capabilities.md#decisions)).

**Performance:** handlers known at compile time (as in production `main`) become direct calls
(monomorphisation or Koka-style evidence passing). Production pays nothing for testability.

**Risk:** with dynamic scoping, which handler applies to a call can be non-obvious. Mitigations: the
language server shows the effect requirements at a call site and the handler stack in tests and
simulations; `moddil explain` shows the installed handlers.

## Record and replay

Because every source of nondeterminism is an effect, **logging effect results at handler boundaries
is enough to replay an execution exactly**. It works like `rr`, but at the language level, so it is
portable and much cheaper.

Uses:
- reproducing production incidents;
- time-travel debugging (as in rr, WinDbg TTD, Replay.io, Elm's debugger);
- replaying a whole trading day ([HFT example](examples/02-hft-platform.md));
- **replaying production traffic through a new code version**, with detection of where the runs
  diverge, as regression and shadow testing.

Recording can be selective per scope or task to control volume.

**Durable execution.** Temporal and Restate rely on deterministic replay of workflow code, and can
only check it at runtime. In Moddil a workflow's effect row may contain only replayable effects, so
**workflow determinism is checked by the compiler** (durable workflows, [Distributed and multi-tier programs](distribution.md)).

## Reproducible builds

The same source, lockfile and toolchain produce **bit-identical artefacts**:
- no timestamps or build paths in outputs (path remapping);
- deterministic parallel code generation;
- pure, sandboxed comptime ([`comptime.md`](comptime.md));
- hash-pinned dependencies.

Anyone can rebuild a tagged version of a mod and compare the bytes ([Package management and supply-chain security](packages-and-supply-chain.md)). The bootstrap fixed point
([`bootstrap.md`: Staged bootstrap](bootstrap.md#staged-bootstrap)–[`bootstrap.md`: Bootstrapping after self-hosting](bootstrap.md#bootstrapping-after-self-hosting)) uses the same machinery. Prior art: Go toolchains are fully
reproducible from Go 1.21; Debian Reproducible Builds; Nix and Guix; Bazel's hermetic builds.

Tests run in a shuffled order and print the seed (Go's `-shuffle`), so order dependence is found and
reproducible.

## Cross-platform determinism

Bit-identical results across x86, ARM, WASM and (opt-in) GPU, for game lockstep
([game example](examples/01-multiplayer-game.md)), HFT replay on different hardware, and consensus
systems. It follows from [Sources of nondeterminism](#sources-of-nondeterminism): strict floating point, deterministic math, and fixed-width integers.

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
   time but not in randomness or interleaving. With effects, using `rand` without a handler does not
   compile.
3. **Plumbing cost:** `Arc<dyn …>` through constructors, a boxed future per `sleep`, and a services
   bundle that grows with each new service. Effects remove it.
4. **Heuristic time driving:** the driver calls `yield_now()` hoping other tasks register their sleeps
   first, which is racy on a multi-threaded runtime. In Moddil the scheduler and the clock are one
   handler, advancing time only at quiescence.
5. **Cryptographic randomness:** keys need real entropy in production and reproducible entropy in
   simulation. **Recommended:** one `rand` effect with a `secure` sub-capability. Production handlers
   use the OS CSPRNG; the simulation handler uses a seeded CSPRNG (e.g. ChaCha20). Simulated keys must
   never leave simulation.
6. **Next steps Hashiverse doesn't have yet,** and a Moddil port gets for free: seeded scheduling,
   record and replay, and the determinism self-check.

## Costs

- Strict floating point forbids implicit FMA and fast-math, so some numeric code is slower. Fast-math
  is an explicit, visible opt-in.
- Recording adds overhead (hence selective recording).
- A simulation is only as faithful as its models.

## Decisions to lock early (recommendations)

1. The principle in [The principle](#the-principle), possibly as firm requirement **R4: deterministic by construction**.
2. Specified evaluation and drop order.
3. Strict IEEE floating point by default; deterministic `moddil/math`; nondeterministic floating
   point opt-in and visible.
4. Hash seeding via the `rand` effect; seeded hash order for maps (decided).
5. A hybrid effect design: scoped handlers plus second-class capability values ([Handlers are scoped, not global](#handlers-are-scoped-not-global)).
6. `moddil/runtime-sim` as a first-class standard mod, with CI running the standard library under it.
7. Record and replay at effect boundaries as a runtime feature; workflow determinism checked by effect
   rows.
8. Reproducible builds as a requirement of the toolchain and of every mod release.

## Fit with the reference examples

| Example | What determinism gives it |
|---|---|
| [Game](examples/01-multiplayer-game.md) | Identical simulation on browser WASM and native server; whole system simulated with latency and packet loss |
| [HFT](examples/02-hft-platform.md) | Bit-identical backtest and replay of a trading day; replay of production traffic through new strategy versions |
| [Web store](examples/03-web-store.md) | Simulated rolling deploys with mixed versions; reproducible order-flow tests |
| [Hashiverse](examples/04-hashiverse.md) | The existing harness, plus deterministic randomness and interleavings, record and replay, and the self-check |

## Open questions

1. Should this become R4?
2. How much of `moddil/math` must be correctly rounded, and at what speed cost?
3. GPU determinism: per kernel, per mod, or global?
4. Record format and retention: per task, per request, sampled?
5. How are non-Moddil peers brought into simulation beyond WASM components?

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

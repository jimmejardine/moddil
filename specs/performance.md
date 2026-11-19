# Performance model

> **Status:** decided direction, details open. Listed in the [index](README.md). Brings together
> performance decisions made in other areas and adds the rules for hot paths, checks, optimisation
> and benchmarks. Snippets are **illustration only**.

## Summary

| Topic | Decision |
|---|---|
| **Cost model** | Published per construct. **No construct hides an allocation, a lock, or a copy proportional to data size**; each appears in the source ([A published cost model](#a-published-cost-model)). |
| **Hot paths** | **`#[no_alloc]` and `#[no_lock]`** on functions or blocks, checked transitively through callees. "Never suspends" already follows from effect sets ([Hot-path guarantees](#hot-path-guarantees)). |
| **Checks** | Overflow and bounds checks are **never disabled globally**. The compiler elides what it can prove, `moddil perf` reports what remains, and opt-outs are local ([Checks and their elision](#checks-and-their-elision)). |
| **PGO** | **First-class, from replays:** profiles come from recorded or simulated runs and are versioned in git, so optimised builds stay reproducible ([Optimisation control](#optimisation-control)). |
| **Benchmarks** | A `#[budget(…)]` on **deterministic counters** on an attached `bench` block fails CI; wall-clock comparisons are statistical and only warn ([Benchmarks and budgets](#benchmarks-and-budgets)). |

## Problem

Moddil promises that cost is visible ([`vision.md`: Cross-cutting themes](vision.md#cross-cutting-themes)), and two reference examples
depend on it: the [HFT platform](examples/02-hft-platform.md) needs tick-to-trade latency competitive
with tuned C++, with zero allocation on the hot path, and the [multiplayer game](examples/01-multiplayer-game.md)
must hit its frame budget, including in debug builds. The relevant decisions are spread across memory,
values, numerics, concurrency and build times. What is missing is one model: what each construct costs,
how a programmer guarantees that a hot path stays fast, and how regressions are caught.

## Prior art

| Language or tool | Approach | Lesson |
|---|---|---|
| **C++** | Zero-cost abstractions | Hidden costs remain: implicit copies, exceptions, virtual calls through templates |
| **Rust** | Zero-cost abstractions, monomorphisation | Fast, but async state machines bloat, generic code compiles slowly, and release builds turn overflow checks off |
| **Swift** | ARC; `@_noAllocation` and `@_noLocks` performance annotations; `-Ounchecked` | Hot-path annotations work when the compiler checks them transitively; ARC traffic is the hidden cost |
| **Zig** | Explicit allocators; `ReleaseSafe` vs `ReleaseFast` | Explicit cost; build modes that change semantics |
| **Go** | Garbage collection | Simple, with tail-latency costs that real-time systems cannot accept |
| **Jai, Zig** | Built-in or library struct-of-arrays | Layout transformations without rewriting code |
| **AutoFDO, LLVM BOLT** | Profile-guided and post-link optimisation | 10–20% gains on large services; profiles are hard to keep reproducible |
| **iai, cachegrind** | Benchmarks on instruction counts instead of time | Deterministic counters make benchmark budgets reliable on noisy CI machines |

## A published cost model

**Decision: every construct has a documented cost, and no construct hides an allocation, a lock or a
copy whose size grows with the data.**

| Construct | Compiles to | Cost |
|---|---|---|
| Move | A memory copy of the value's own bytes, often elided | Proportional to the inline size only |
| `Copy` value | The same | Types are `Copy` only below a size cap ([`values.md`: Opt-in `Copy`, with a size cap](values.md#opt-in-copy-with-a-size-cap)) |
| `.clone()` | A deep copy | Explicit, and proportional to the data ([`values.md`](values.md)) |
| Borrow, view type | A pointer, or the value itself when small | None |
| `Rc<T>` | Header plus value on the heap | `.to_rc()` allocates; `.share()` is a biased increment; borrowing an rc is free ([`memory.md`: Cost table](memory.md#cost-table)) |
| Sync cell | Inline atomic or lock | One atomic operation or lock acquisition, always visible as a method call |
| Static call | A direct call, often inlined | None |
| Generic call | Debug: a witness-table call; release: a specialised copy | Debug: indirect calls; release: none, at a code-size cost ([`types.md`: Generics](types.md#generics)) |
| `dyn Trait` call | A witness-table call | One indirect call; no inlining ([`types.md`: `dyn`: existentials](types.md#dyn-existentials)) |
| Non-escaping closure | Inlined or specialised | None ([`closures.md`](closures.md)) |
| `Box<dyn Fn …>` | A heap allocation and an indirect call | Allocation at creation, an indirect call per use |
| Effect operation | A call to the installed handler | Static handlers are inlined; dynamic handlers are an indirect call ([`effects-and-capabilities.md`: Compilation](effects-and-capabilities.md#compilation)) |
| `suspend` and resume | A stack switch | Roughly a function call plus register save and restore ([`compiler.md`: WASM concurrency lowering](compiler.md#wasm-concurrency-lowering)) |
| Bounds and overflow checks | A compare and a branch to a trap | Usually 1–5% in integer-heavy code, less after elision ([Checks and their elision](#checks-and-their-elision)) |
| `fmt"…"` into a `String` | Formatting plus an allocation if longer than the inline capacity | Writing into an existing buffer avoids the allocation ([`text.md`](text.md)) |
| `Remote` call | Serialisation and a network round trip | Always an explicit effect ([`distribution.md`](distribution.md)) |

## Data layout

Decided in [`numerics.md`: Data layout](numerics.md#data-layout) and summarised here:

- The compiler **reorders fields** to minimise padding unless the type is `#[repr(C)]`.
- **Packed structs, bitfields and endian types** (`u32be`) for wire formats.
- A **comptime struct-of-arrays container**, so a `Vec<Particle>` can become a column per field
  without rewriting the code that uses it.

Added here:

- **Cache-line alignment:** `#[align(64)]` on a type or field.
- **False sharing:** sync cells used by different cores can be padded to a cache line
  (`CachePadded<Atomic<u64>>` in `moddil/sync`, [`concurrency.md`: Shared mutable state](concurrency.md#shared-mutable-state)).

## Hot-path guarantees

**Decision: `#[no_alloc]` and `#[no_lock]` are checked attributes.**

```rust
#[no_alloc, no_lock]
fn on_tick(book: &mut OrderBook, tick: &Tick, out: &mut SpscProducer<Order>) with fail(RiskError) {
    book.apply(tick);                                  // must itself be no_alloc and no_lock
    if let Some(order) = strategy::decide(book) {
        risk::check(&order)?;                          // fails with a typed error; no allocation
        out.push(order);                               // lock-free ring
    }
}
```

- **Transitive.** Every function the annotated code calls must be known not to allocate (or lock).
  The compiler infers these properties for every function and records them in the mod's interface
  artefact, so checking across mods costs no re-analysis ([`build-times.md`: Language rules that decide build speed](build-times.md#language-rules-that-decide-build-speed)).
  Public functions that promise the property write the attribute, so removing it is a breaking change
  that the semver check reports.
- **What counts:**
  - `no_alloc`: any call into an allocator, including `.to_rc()`, growing a `Vec` or `String`, and
    `Box::new`;
  - `no_lock`: mutexes, read-write locks, blocking system calls. Atomic operations and lock-free
    structures are allowed.
- **"Never suspends"** needs no attribute: a function whose effect set lacks `suspend` cannot suspend
  ([`effects-and-capabilities.md`](effects-and-capabilities.md)).
- **Foreign code:** C functions are assumed to allocate and lock unless an overlay says otherwise
  ([`interop.md`](interop.md)).
- **Traps still trap.** These attributes are about latency, not failure; a bounds or overflow trap
  inside a `no_alloc` function behaves as everywhere else.
- **Why attributes:** making allocation an effect would put `alloc` in almost every public signature,
  and running the hot path with the allocator capability withheld would only check the code reached
  in that one scope. Swift's `@_noAllocation` shows that checked attributes work.

## Kernel-bypass I/O: a worked example

With kernel bypass (DPDK, ef_vi/Onload, AF_XDP) the NIC writes packets by DMA straight into a ring in
the process's memory, without the kernel. Nothing here needs a new language feature: it combines
handlers, pinned executors, owned DMA buffers, contained `unsafe` and declared root capabilities.

**Three ways to notice a packet:**

| Wait mode | How | Latency | CPU |
|---|---|---|---|
| **Busy-poll** | A core spins on the RX ring's next descriptor | Hundreds of ns | One core at 100% |
| **Interrupt** | The NIC raises MSI-X, delivered to user space as an **eventfd** (VFIO with DPDK's RX interrupts, AF_XDP `need_wakeup` plus `poll`, ef_vi event-queue priming) | ~5–20 µs (wake-up and context switch) | Near zero when idle |
| **Adaptive** | Spin for N µs after the last packet, then arm the interrupt and sleep (as Linux NAPI and DPDK's power-aware examples do) | Busy-poll latency under load | Proportional to load |

On a hosted OS, user space never runs a real interrupt handler: an "interrupt" is an eventfd becoming
readable.

**The NIC queue is a `Net` handler.** A kernel-bypass runtime mod provides a handler bound to one
queue; application code reads packets exactly as it would from sockets, so the same feed handler runs
on kernel sockets, kernel bypass and the simulator ([`effects-and-capabilities.md`](effects-and-capabilities.md)).

```rust
fn main(net: NetRoot) {
    let rx = net.bypass("eth0", queue: 3, wait: Adaptive { spin: 20<us> })   // narrows NetRoot
    scope(|s| {
        s.spawn(|| feed_handler(&rx))
    }, on: pinned(3))                       // the core that also takes the queue's IRQ
}

#[no_alloc, no_lock]
fn feed_handler(rx: &RxQueue) with Net, suspend, fail(NetError) {
    loop {
        let batch = rx.recv_batch::<32>()?  // owned Packets from the DMA pool; zero copy
        for pkt in batch { on_packet(pkt) }
    }
}
```

- **The wait mode is the handler's configuration, not the code's.**
  - *Busy-poll*: `recv` spins on the descriptor ring on a `pinned` executor, which already offers
    busy-polling and real-time priority ([`runtime.md`: Tasks, threads and executors](runtime.md#tasks-threads-and-executors)).
    Each spin is a cancellation checkpoint, so stop and cancel still work.
  - *Interrupt*: the handler arms the queue's interrupt, registers its eventfd with the shared I/O
    reactor and suspends the task. The interrupt wakes the task, which drains the ring in a batch and
    re-arms.
  - *Adaptive*: spin, then arm and suspend.
  - The spin budget, interrupt coalescing (`rx-usecs`) and IRQ affinity are set in code or in the
    deployment topology. Production can busy-poll while a developer laptop uses interrupts, with no
    code change.
- **Packets are owned DMA buffers.** A `Packet` is a unique value over a DMA-pool slot, typed by its
  memory space; dropping it returns the slot to the ring; validation yields zero-copy views
  ([`memory.md`: Zero-copy I/O](memory.md#zero-copy-io), [`accelerators.md`](accelerators.md)).
- **`unsafe` lives in one small mod.** Descriptor rings, the AF_XDP UMEM mapping, volatile access and
  DPDK or ef_vi bindings use `unsafe` (and `ffi`), with safety contracts, `FromBytes` descriptors,
  the checker and human review ([`unsafe.md`](unsafe.md)). Applications hold neither capability.
  AF_XDP can be driven from pure Moddil through raw system calls.
- **Capabilities.** Opening a bypass queue narrows `NetRoot`, so it appears in `main`'s declared roots;
  the startup sandbox keeps exactly the privileges it needs (VFIO device access, or `CAP_NET_RAW` and
  `CAP_BPF` for AF_XDP) and drops the rest ([`lifecycle.md`: `main` declares its root capabilities](lifecycle.md#main-declares-its-root-capabilities)).
- **Replay and simulation.** A packet arrival is a `Net` effect result, so the flight recorder captures
  it and a trading day replays exactly; in tests a simulation handler delivers packets with modelled
  latency and loss ([`determinism.md`: Record and replay](determinism.md#record-and-replay)).
- **Bare metal.** In the freestanding profile there are real interrupt handlers, with a restricted
  effect set; they only signal, and a task drains the ring ([`freestanding.md`](freestanding.md)):

  ```rust
  #[interrupt(NIC_RX, priority = 3)]
  fn nic_rx() { RX_READY.signal() }        // no allocation, no suspension
  ```

## Checks and their elision

**Decision: overflow and bounds checks are never turned off for a whole build.** A program behaves the
same in debug and release, on every target ([`numerics.md`: Overflow traps](numerics.md#overflow-traps),
[`error-handling.md`](error-handling.md)).

- **Elision the compiler promises:** loops over `0..xs.len()` and iterators over slices; indices
  already compared against a length; values whose refinement proves the range
  (`u8 where self <= 100`); slices whose lengths are asserted once before a loop.
- **Visibility:** `moddil perf checks` lists the checks that remain in each function, with the reason
  each could not be removed. A team can review the list for its hot functions, and the language
  server can show remaining checks inline.
- **Local opt-outs** are visible in the code:
  - wrapping operators and `Wrapping<T>` where wrapping is the intended meaning;
  - `get_unchecked` inside `unsafe`, with a stated safety contract ([`unsafe.md`](unsafe.md));
  - overflow checks removed by stating the range (`requires`, `assert`, refinements). Arithmetic
    itself is never unchecked ([`numerics.md`: Overflow traps](numerics.md#overflow-traps)).

## Optimisation control

- **Inlining hints:** `#[inline]`, `#[inline(never)]` and `#[cold]`, as in Rust.
- **LTO** in release builds; no cross-mod inlining in debug builds ([`build-times.md`: Profiles](build-times.md#profiles)).
- **PGO from replays (decided):**
  - `moddil build --pgo` uses profiles gathered from **recorded production runs or simulated runs**
    ([`determinism.md`: Record and replay](determinism.md#record-and-replay)): the HFT platform replays a trading day; the game
    runs its simulated clients;
  - profiles are **stored in git** next to the code and included in the build cache key, so an
    optimised build is still reproducible;
  - a stale profile (one recorded against older code) is reported, never silently used.
- **Post-link optimisation** (BOLT-style layout of hot code) is an option for the release profile.

## Debug-build speed

Games and simulations must stay usable in debug builds:

- **dependencies are optimised by default**, and only the workspace is built unoptimised
  ([`build-times.md`: Profiles](build-times.md#profiles));
- a mod (or one hot function) can be optimised in debug builds with an `opt-level` override;
- **target:** the reference game holds its frame budget in a default debug build. This is tracked in
  CI as a benchmark.

## Benchmarks and budgets

**Decision: budgets on deterministic counters fail CI; wall-clock results only warn.**

```rust
#[budget(allocations: 0, instructions: 2_000)]
bench "one tick" for on_tick {
    let (mut book, tick, mut out) = fixtures::market();     // setup: not measured
    measure(|| on_tick(&mut book, &tick, &mut out));
}
```

- `moddil bench` (from `moddil/bench`) runs benchmarks and compares them with the previous commit
  statistically.
- **Counters are deterministic:** allocations and syscalls are counted by the runtime; instructions
  through hardware counters, or an instruction-counting emulator where none are available. A budget on
  a counter fails CI reliably, even on shared machines.
- **Compile-time performance assertions** sit beside budgets: `#[vectorise]` fails the release build
  if a loop was not vectorised, and `#[no_alloc]` if a function can allocate
  ([`stdlib.md`: Iteration](stdlib.md#iteration)).
- **Wall-clock time** is reported with confidence intervals and warns on regression, but does not fail
  the build: timing on CI machines is too noisy.
- Results are recorded per commit, so a regression can be traced to the change that caused it
  ([`semantic-vcs.md`](semantic-vcs.md)).

## Fit with the reference examples

| Example | What it needs | Moddil's answer |
|---|---|---|
| [HFT platform](examples/02-hft-platform.md) | Predictable tick-to-trade latency; zero allocation on the hot path | `#[no_alloc, no_lock]` hot loop; pinned executors; kernel-bypass reads, busy-polling in production and interrupt-driven in development; PGO from a replayed trading day; counter budgets in CI |
| [Multiplayer game](examples/01-multiplayer-game.md) | Frame budget in release and debug; cache-friendly entity data | Struct-of-arrays; optimised dependencies in debug; debug frame-rate benchmark |
| [Web store](examples/03-web-store.md) | Throughput per request; low memory per request | Allocation counters per request handler; visible rc and clone costs |
| [Hashiverse](examples/04-hashiverse.md) | Crypto and proof-of-work throughput | Constant-time crypto that is still vectorised; PGO from simulated network runs |

## Open questions

1. **Properties on closures and traits:** how does a parameter say "this closure must be `no_alloc`"
   (`impl Fn() + NoAlloc`, an attribute on the bound, or inference only)?
2. **Bounded execution:** is a checked `#[bounded]` (no unbounded loops or recursion) feasible and
   useful for real-time code?
3. **Portable counters:** which counters exist on every platform, including WASM, and what replaces
   hardware counters where they are missing?
4. **Automatic layout:** should the compiler ever choose a struct-of-arrays layout on its own, guided
   by a profile?
5. **Real-time scheduling:** what guarantees can pinned executors give
   ([`runtime.md`: Tasks, threads and executors](runtime.md#tasks-threads-and-executors))?

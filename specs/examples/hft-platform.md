# High-Frequency Trading Platform

## Overview

A trading platform with four groups of servers: market-data **collectors**, a **store** of historical
market data, a **research** platform that runs backtests and simulations on that data, and an
**execution** set of trade engines that run live algorithms against exchanges.

Its flagship idea: **the same strategy code runs unchanged in research and live trading.** Market
data, time and order routing are effects, and only their handlers differ.

## Topology

```
 exchanges ──feeds──►┌──────────────┐  normalised ticks  ┌──────────────┐
                     │  collector   │───────────────────►│   store      │
                     │ feed handlers│                    │ tick DB,     │
                     └──────┬───────┘                    │ columnar     │
                            │ live ticks (multicast/shm) └──────┬───────┘
                            ▼                                   │ historical data
 exchanges ◄─orders──┌──────────────┐                    ┌──────▼───────┐
                     │  execution   │  strategy code     │  research    │
                     │ live algos,  │◄═══ same mod ═════►│ backtests,   │
                     │ risk, gateway│                    │ sims (CPU+GPU)│
                     └──────────────┘                    └──────────────┘
```

| Placement | Role | Key property |
|---|---|---|
| `collector` | Connect to exchange feeds, decode, normalise, timestamp, publish | Lowest-latency ingest; never drop data |
| `store` | Persist ticks and order books; serve historical queries | Throughput, compression, columnar layout |
| `research` | Backtests, parameter sweeps, Monte Carlo, ML | Throughput; CPU clusters and GPUs |
| `execution` | Run live strategies, pre-trade risk checks, order gateway | Lowest and most predictable latency |

## Key flows

1. **Tick-to-trade**: feed packet arrives → decode → update book → strategy decides → risk check →
   order encoded → sent. The whole path should be measured in microseconds or less, with tight
   tail latency.
2. **Record**: every tick is also persisted to `store` with exchange and local timestamps.
3. **Backtest**: `research` replays historical data from `store` through the *same* strategy mod, with
   simulated time, a simulated order book and a simulated exchange (fills, latency, queue position).
4. **Promote**: a strategy version that passes research is promoted to `execution`. The review shows
   its semantic diff ([Diffs, code review and version control](../semantic-vcs.md)) and capability set.
5. **Replay**: a trading day can be replayed bit for bit for post-trade analysis and incident review.

## Moddil features this stresses

- **Effects and handlers as the core architecture ([Effects and capabilities as a unifying idea](../effects-and-capabilities.md), [Determinism and reproducibility](../determinism.md)).** A strategy is written against
  effect traits such as `MarketData`, `Time` and `Exchange`:
  - live handlers: kernel-bypass feed, hardware clock, exchange gateway;
  - research handlers: historical replay, simulated clock, simulated exchange. The simulator
    implements the same `Exchange` trait as the gateway; queue position, partial fills and latency
    models are its state, driven by simulated time.

  The strategy code is identical in both. This is the clearest demonstration of why effects belong in
  the core language.
- **Capabilities ([Package management and supply-chain security](../packages-and-supply-chain.md)).** Research code physically cannot reach the live `orders` capability; only a
  deployment to `execution` grants it. A third-party analytics mod cannot touch the network.
- **Predictable latency ([Memory: ownership, borrowing and sharing](../memory.md)).** No GC; arenas and pools; **`#[no_alloc]` and `#[no_lock]`** that the
  compiler checks on the hot path ([`performance.md`: Hot-path guarantees](../performance.md#hot-path-guarantees)); no hidden locks; core pinning and NUMA-aware placement.
  **Code places, deployment provisions:** code says where work runs and which memory it uses
  (`on: pinned(3)`, NUMA-local allocators, huge-page arenas); the deployment manifest provisions the
  machine (isolated cores, IRQ affinity, reserved huge pages); at startup the program checks that
  what it asked for was provisioned, and fails with a typed error if not
  ([`performance.md`: Kernel-bypass I/O: a worked example](../performance.md#kernel-bypass-io-a-worked-example)).
- **Lock-free concurrency ([Concurrency across CPUs](../concurrency.md)).** SPSC/MPMC rings between feed, strategy and gateway threads;
  thread-per-core designs; busy-polling.
- **C/C++ interop ([Interoperability with C and Rust](../interop.md)).** Kernel-bypass NIC libraries (e.g. DPDK, ef_vi/Onload-style APIs), exchange
  SDKs and FIX engines are typically C/C++. Calls must be zero-overhead.
- **GPU ([Concurrency across GPUs and accelerators](../accelerators.md)).** Research-side simulations, option pricing, ML training and inference.
- **Numerics ([Numerics and data](../numerics.md)).** Fixed-point/decimal prices, units (price vs quantity vs notional), SIMD for
  book updates and analytics, defined overflow behaviour.
- **Contracts ([Correctness tooling: contracts and verification](../correctness.md)).** Pre-trade risk limits as contracts: position limits, price collars, rate limits.
  These are checked at runtime, and ideally proven for the risk module.
- **Error handling ([Error handling](../error-handling.md)).** No unexpected panics in the trading path; failures are explicit and
  handled (e.g. a kill switch on any unknown state).
- **Review and audit ([LLMs as first-class programmers](../agent-interface.md), [Diffs, code review and version control](../semantic-vcs.md)).** Every change to a live algo is reviewed against a semantic diff;
  provenance (who or what wrote it, which model) is recorded for compliance.

## Success criteria

Initial targets, to be refined:
- Tick-to-trade latency competitive with tuned C++ for the same hardware and NIC, with p99/p99.9
  jitter reported, not just the median.
- Zero heap allocation on the hot path, **checked by the compiler**, not by convention.
- A backtest over a recorded day and a live replay of that day produce **bit-identical** decisions.
- Research code cannot be deployed to `execution` without explicitly granting `orders`; this is
  verified by the build.
- Store ingest keeps up with full-depth feeds of a busy exchange with headroom.

## Mods used

- Standard: `moddil/sync` (lock-free queues), `moddil/alloc` (arenas, pools), `moddil/simd`,
  `moddil/tensor`, `moddil/gpu`, `moddil/decimal`, `moddil/time` (hardware timestamps),
  `moddil/net` (UDP multicast, TCP), `moddil/timeseries`, `moddil/serde`, `moddil/sim`,
  `moddil/runtime` (thread-per-core variant), `moddil/trace`, `moddil/sql` (reference data).
- New: `hft/marketdata` (normalised book model), `hft/fix` (FIX protocol), `hft/risk`,
  exchange-specific feed and gateway mods, `hft/backtest` (simulated exchange).

## Interop and non-Moddil parts

- Kernel-bypass networking and NIC timestamping (C libraries).
- Exchange-provided SDKs (often C++).
- Possibly FPGA-accelerated feed handlers (out of scope for Moddil itself, but Moddil must talk to them).
- Research workflows still use Python notebooks: research mods are exported to Python as wheels
  ([`adoption.md`: Python first](../adoption.md#python-first)), and Moddil can call Python libraries
  through `moddil/python` ([`interop.md`](../interop.md)).

## Open questions

- **Bounded latency:** `#[no_alloc]` and `#[no_lock]` are checked attributes
  ([`performance.md`: Hot-path guarantees](../performance.md#hot-path-guarantees)). Can a latency
  bound on the hot path be checked too, or only measured?

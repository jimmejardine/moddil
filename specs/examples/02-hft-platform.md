# Example 2: High-Frequency Trading Platform

## Overview

A trading platform with four groups of servers: market-data **collectors**, a **store** of historical
market data, a **research** platform that runs backtests and simulations on that data, and an
**execution** set of trade engines that run live algorithms against exchanges.

Its flagship idea: **the same strategy code runs unchanged in research and live trading.** Market
data, time and order routing are effects, and only their handlers differ.

## Topology

```
 exchanges ──feeds──►┌──────────────┐  normalised ticks  ┌──────────────┐
                     │  @collector  │───────────────────►│   @store     │
                     │ feed handlers│                    │ tick DB,     │
                     └──────┬───────┘                    │ columnar     │
                            │ live ticks (multicast/shm) └──────┬───────┘
                            ▼                                   │ historical data
 exchanges ◄─orders──┌──────────────┐                    ┌──────▼───────┐
                     │  @execution  │  strategy code     │  @research   │
                     │ live algos,  │◄═══ same mod ═════►│ backtests,   │
                     │ risk, gateway│                    │ sims (CPU+GPU)│
                     └──────────────┘                    └──────────────┘
```

| Placement | Role | Key property |
|---|---|---|
| `@collector` | Connect to exchange feeds, decode, normalise, timestamp, publish | Lowest-latency ingest; never drop data |
| `@store` | Persist ticks and order books; serve historical queries | Throughput, compression, columnar layout |
| `@research` | Backtests, parameter sweeps, Monte Carlo, ML | Throughput; CPU clusters and GPUs |
| `@execution` | Run live strategies, pre-trade risk checks, order gateway | Lowest and most predictable latency |

## Key flows

1. **Tick-to-trade**: feed packet arrives → decode → update book → strategy decides → risk check →
   order encoded → sent. The whole path should be measured in microseconds or less, with tight
   tail latency.
2. **Record**: every tick is also persisted to `@store` with exchange and local timestamps.
3. **Backtest**: `@research` replays historical data from `@store` through the *same* strategy mod, with
   simulated time, a simulated order book and a simulated exchange (fills, latency, queue position).
4. **Promote**: a strategy version that passes research is promoted to `@execution`. The review shows
   its semantic diff (§2.19) and capability set.
5. **Replay**: a trading day can be replayed bit for bit for post-trade analysis and incident review.

## Moddil features this stresses

- **Effects and handlers as the core architecture (§2.1, §2.11).** A strategy is written against
  effects such as `market_data`, `clock` and `orders`:
  - live handlers: kernel-bypass feed, hardware clock, exchange gateway;
  - research handlers: historical replay, simulated clock, simulated exchange.

  The strategy code is identical in both. This is the clearest demonstration of why effects belong in
  the core language.
- **Capabilities (§2.16).** Research code physically cannot reach the live `orders` capability; only a
  deployment to `@execution` grants it. A third-party analytics mod cannot touch the network.
- **Predictable latency (§2.2).** No GC; arenas and pools; **a "no allocation" effect** that the
  compiler can check on the hot path; no hidden locks; core pinning and NUMA-aware placement.
- **Lock-free concurrency (§2.8).** SPSC/MPMC rings between feed, strategy and gateway threads;
  thread-per-core designs; busy-polling.
- **C/C++ interop (§2.13).** Kernel-bypass NIC libraries (e.g. DPDK, ef_vi/Onload-style APIs), exchange
  SDKs and FIX engines are typically C/C++. Calls must be zero-overhead.
- **GPU (§2.9).** Research-side simulations, option pricing, ML training and inference.
- **Numerics (§2.5).** Fixed-point/decimal prices, units (price vs quantity vs notional), SIMD for
  book updates and analytics, defined overflow behaviour.
- **Contracts (§2.7).** Pre-trade risk limits as contracts: position limits, price collars, rate limits.
  These are checked at runtime, and ideally proven for the risk module.
- **Error handling (§2.3).** No unexpected panics in the trading path; failures are explicit and
  handled (e.g. a kill switch on any unknown state).
- **Review and audit (§2.18, §2.19).** Every change to a live algo is reviewed against a semantic diff;
  provenance (who or what wrote it, which model) is recorded for compliance.

## Success criteria

Initial targets, to be refined:
- Tick-to-trade latency competitive with tuned C++ for the same hardware and NIC, with p99/p99.9
  jitter reported, not just the median.
- Zero heap allocation on the hot path, **checked by the compiler**, not by convention.
- A backtest over a recorded day and a live replay of that day produce **bit-identical** decisions.
- Research code cannot be deployed to `@execution` without explicitly granting `orders`; this is
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
- Research workflows may still use Python notebooks; a Python binding for research mods would help
  adoption.

## Open questions

- Can a "no allocation" or "bounded latency" property be an effect the compiler checks, as part of
  the effect system rather than an external linter?
- How exactly do simulated-exchange semantics (queue position, partial fills, latency models) live
  alongside the real gateway behind one effect interface?
- How far should Moddil go in controlling hardware (core pinning, NUMA, huge pages): runtime mod or
  deployment manifest?
- Is Python interop needed for research users, and if so, at what priority?

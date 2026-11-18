# Reference Examples

> **Status:** brainstorm. These are design drivers, not code (yet).

Moddil is designed against four real, multi-tier systems. Every proposed language feature or
standard mod should be able to point at **at least one example that needs it**. A feature that no
example needs is a candidate for removal.

Each example will eventually become:
1. a **runnable reference implementation** written in Moddil;
2. a **benchmark** (latency, throughput, binary size, build time);
3. a **test** of the language's claims, for example "the same code runs in simulation and production".

| # | Example | Domain | Document |
|---|---|---|---|
| 1 | Multiplayer game | Real-time, browser | [`01-multiplayer-game.md`](01-multiplayer-game.md) |
| 2 | HFT platform | Finance, ultra-low latency | [`02-hft-platform.md`](02-hft-platform.md) |
| 3 | Web store | Commerce, web + mobile | [`03-web-store.md`](03-web-store.md) |
| 4 | Hashiverse | Decentralised P2P social network | [`04-hashiverse.md`](04-hashiverse.md) |

All four use the same template: *Overview → Topology → Key flows → Moddil features this stresses →
Success criteria → Mods used → Interop and non-Moddil parts → Open questions.*

## Coverage: examples × problem areas

Rows link to each area's document; all areas are listed in the [index](../README.md).
● = central to the example · ○ = exercised · blank = not significant.

| Problem area | Game | HFT | Store | Hashiverse |
|---|---|---|---|---|
| [Effects and handlers](../effects-and-capabilities.md) | ○ | ● (sim ↔ live) | ○ | ● (pluggable transport/time/storage) |
| [Memory / resources](../memory.md) | ○ | ● (zero-alloc hot path) | ○ | ○ |
| [Error handling](../error-handling.md) | ○ | ● | ○ | ○ |
| [Numerics](../numerics.md) | ○ | ● (decimal, SIMD) | ○ (money) | ○ (crypto) |
| [Contracts / verification](../correctness.md) | | ● (risk checks) | ○ | ○ |
| [CPU concurrency](../concurrency.md) | ● | ● | ○ | ● (parallel PoW) |
| [GPU](../accelerators.md) | ● (WebGPU) | ● (research) | | |
| [Distributed / multi-tier](../distribution.md) | ● | ● | ● | ● (P2P) |
| [Deterministic simulation](../determinism.md) | ● | ● | ○ | ● |
| [C/C++/Rust interop](../interop.md) | ○ | ● (kernel bypass, exchange SDKs) | ○ (mobile platform APIs) | ● (existing Rust code) |
| [Build times](../build-times.md) | ○ | ○ | ● (large app team) | ○ |
| [Supply chain / capabilities](../packages-and-supply-chain.md) | ○ | ● (`order` capability) | ● (`payments`) | ● (untrusted peers) |
| [LLM-friendliness](../agent-interface.md) | ○ | ○ | ● (CRUD volume) | ○ |
| [Semantic diffs / review](../semantic-vcs.md) | ○ | ● (audited algos) | ● | ○ |
| R1 WASM | ● (browser client) | | ● (web client) | ● (browser client) |
| Mobile (iOS/Android) | | | ● | |

No problem area is left unexercised. [GPU](../accelerators.md) is the thinnest; if research workloads in the HFT
example are not enough, a fifth GPU-heavy example (e.g. ML inference) may be added.

## Coverage: examples × standard mods

See [`../stdlib.md`: Initial standard-mod catalogue](../stdlib.md#initial-standard-mod-catalogue) for the full catalogue. Mods marked **new** are example-specific
mods the example would create.

| Mod | Game | HFT | Store | Hashiverse |
|---|---|---|---|---|
| `moddil/net`, `moddil/rpc` | ● | ● | ● | ● |
| `moddil/runtime`, `-wasm`, `-sim` | ● | ● | ● | ● |
| `moddil/serde` | ● | ● | ● | ● |
| `moddil/sync` | ● | ● | | ● |
| `moddil/alloc` | ○ | ● | | |
| `moddil/crypto` | | | ○ | ● |
| `moddil/simd`, `moddil/tensor` | ○ | ● | | ○ |
| `moddil/gpu` | ● | ● | | |
| `moddil/decimal` | | ● | ● | |
| `moddil/sql` | | ○ | ● | |
| `moddil/timeseries` | | ● | | |
| `moddil/web` | ● | | ● | ● |
| `moddil/storage` | | | ○ | ● |
| **new** | `game/netcode`, `game/chat` | `hft/marketdata`, `hft/risk`, `hft/fix` | `store/catalog`, `store/payments` | `hashiverse/protocol`, `moddil/dht`?, `moddil/pow`? |

The order in which the examples are built is in [`roadmap.md`: Order of the reference examples](../roadmap.md#order-of-the-reference-examples).

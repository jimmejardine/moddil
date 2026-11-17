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

Section numbers refer to [`../rationale.md`](../rationale.md).
● = central to the example · ○ = exercised · blank = not significant.

| Problem area | Game | HFT | Store | Hashiverse |
|---|---|---|---|---|
| §2.1 CPU concurrency | ● | ● | ○ | ● (parallel PoW) |
| §2.2 GPU | ● (WebGPU) | ● (research) | | |
| §2.3 Memory / resources | ○ | ● (zero-alloc hot path) | ○ | ○ |
| §2.4 Build times | ○ | ○ | ● (large app team) | ○ |
| §2.5 Supply chain / capabilities | ○ | ● (`order` capability) | ● (`payments`) | ● (untrusted peers) |
| §2.6 LLM-friendliness | ○ | ○ | ● (CRUD volume) | ○ |
| §2.7 Semantic diffs / review | ○ | ● (audited algos) | ● | ○ |
| §2.8 C/C++/Rust interop | ○ | ● (kernel bypass, exchange SDKs) | ○ (mobile platform APIs) | ● (existing Rust code) |
| §2.9 Distributed / multi-tier | ● | ● | ● | ● (P2P) |
| R1 WASM | ● (browser client) | | ● (web client) | ● (browser client) |
| §3.1 Error handling | ○ | ● | ○ | ○ |
| §3.2 Effects and handlers | ○ | ● (sim ↔ live) | ○ | ● (pluggable transport/time/storage) |
| §3.5 Contracts / verification | | ● (risk checks) | ○ | ○ |
| §3.6 Deterministic simulation | ● | ● | ○ | ● |
| §3.10 Numerics | ○ | ● (decimal, SIMD) | ○ (money) | ○ (crypto) |
| Mobile (iOS/Android) | | | ● | |

No problem area is left unexercised. §2.2 GPU is the thinnest; if research workloads in the HFT
example are not enough, a fifth GPU-heavy example (e.g. ML inference) may be added.

## Coverage: examples × standard mods

See [`../mods.md`](../mods.md) §6 for the full catalogue. Mods marked **new** are example-specific
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
| `moddil/ui` | ○ | | ● | ○ |
| **new** | `game/netcode`, `game/chat` | `hft/marketdata`, `hft/risk`, `hft/fix` | `store/catalog`, `store/payments` | `hashiverse/protocol`, `moddil/dht`?, `moddil/pow`? |

## Proposed order

1. **Hashiverse client.** A real protocol with real users, with existing Rust servers to interoperate
   against. It exercises WASM, interop, crypto and effects-as-pluggable-traits.
2. **Multiplayer game.** Multi-tier, browser, real-time, WebGPU.
3. **Web store.** Everyday ergonomics, databases and the mobile question.
4. **HFT platform.** The hardest latency requirements and the flagship "same code in simulation and live"
   demonstration.

The order is open for discussion. HFT has the strongest single demonstration of effects, but the
largest scope.

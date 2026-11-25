# Reference Examples

Design drivers, not code (yet).

Moddil is designed against eight programs in two groups:

- **Flagship showcases:** programs chosen to **market** Moddil to its first audiences
  ([`adoption.md`: First audiences](../adoption.md#first-audiences)): a GPU fractal fly-through in
  the browser, a high-performance web app with a Svelte 5 UI over a Moddil core, and a
  GPU-accelerated Python library.
- **Design drivers:** four real, multi-tier systems and one bare-metal firmware. They exist to
  test the design.

Both groups count for the rule that every proposed language feature or standard mod should be able
to point at **at least one example that needs it**; a feature that no example needs is a candidate
for removal. The examples are not numbered, because the order in which they are built changes; it
lives in [`roadmap.md`: Order of the reference examples](../roadmap.md#order-of-the-reference-examples).

Each example will eventually become:
1. a **runnable reference implementation** written in Moddil;
2. a **benchmark** (latency, throughput, binary size, build time);
3. a **test** of the language's claims, for example "the same code runs in simulation and production".

**Flagship showcases**

| Example | Domain | Document |
|---|---|---|
| Mandelbulb flight | Fly through a 3D fractal, ray-marched on the GPU in the browser; the first to be built | [`mandelbulb.md`](mandelbulb.md) |
| fumoca | Monte Carlo spreadsheet, Svelte 5 UI over a Moddil WASM core | [`fumoca.md`](fumoca.md) |
| Talk | Text to speech for Python on any GPU | [`talk.md`](talk.md) |

**Design drivers**

| Example | Domain | Document |
|---|---|---|
| Multiplayer game | Real-time, browser | [`multiplayer-game.md`](multiplayer-game.md) |
| HFT platform | Finance, ultra-low latency | [`hft-platform.md`](hft-platform.md) |
| Web store | Commerce, web + mobile | [`web-store.md`](web-store.md) |
| Hashiverse | Decentralised P2P social network | [`hashiverse.md`](hashiverse.md) |
| Sensor firmware | Embedded, bare metal | [`sensor-firmware.md`](sensor-firmware.md) |

All eight use the same template: *Overview → Topology → Key flows → Moddil features this stresses →
Success criteria → Mods used → Interop and non-Moddil parts → Open questions.* The showcases add
*The pitch* after the overview: what the program shows a newcomer, in a few lines.

## Coverage: examples × problem areas

Rows link to each area's document; all areas are listed in the [index](../README.md).
● = central to the example · ○ = exercised · blank = not significant.

| Problem area | Mandelbulb | fumoca | Talk | Game | HFT | Store | Hashiverse | Firmware |
|---|---|---|---|---|---|---|---|---|
| [Effects and handlers](../effects-and-capabilities.md) | ○ (`Dom`, `Js`, `Gpu`) | ○ | ● (synthesis is `Gpu` only) | ○ | ● (sim ↔ live) | ○ | ● (pluggable transport/time/storage) | ○ |
| [Memory / resources](../memory.md) | | ○ | ○ | ○ | ● (zero-alloc hot path) | ○ | ○ | ● (no heap, static buffers) |
| [Error handling](../error-handling.md) | | ○ (errors as cell values) | ○ | ○ | ● | ○ | ○ | ○ |
| [Numerics](../numerics.md) | ● (f32 maths shared by CPU and GPU) | ● (f64 CPU, f32 GPU, RNG) | ● (f16, tensors) | ○ | ● (decimal, SIMD) | ○ (money) | ○ (crypto) | ○ (fixed point, filters) |
| [Contracts / verification](../correctness.md) | | | | | ● (risk checks) | ○ | ○ | ○ (stack bounds) |
| [CPU concurrency](../concurrency.md) | | ● (worker pool) | ○ (CPU pool) | ● | ● | ○ | ● (parallel PoW) | ● (interrupts, static tasks) |
| [GPU](../accelerators.md) | ● (ray-marching kernel) | ● (WebGPU kernels) | ● (tensor graph, tile kernels) | ● (WebGPU) | ● (research) | | | |
| [Distributed / multi-tier](../distribution.md) | | | | ● | ● | ● | ● (P2P) | |
| [Deterministic simulation](../determinism.md) | ○ (golden images) | ● (seeded replay, CPU/GPU cross-check) | ○ (golden audio) | ● | ● | ○ | ● | ● (host simulation) |
| [C/C++/Rust interop](../interop.md) | ○ (Svelte overlay) | ● (TypeScript UI, storage providers) | ● (Python) | ○ | ● (kernel bypass, exchange SDKs) | ○ (mobile platform APIs) | ● (existing Rust code) | ○ (vendor C, SVD) |
| [Build times](../build-times.md) | | | | ○ | ○ | ● (large app team) | ○ | |
| [Supply chain / capabilities](../packages-and-supply-chain.md) | | | ● (no `Net` while speaking) | ○ | ● (`order` capability) | ● (`Payments`) | ● (untrusted peers) | ○ (no `Net`, no `Fs`) |
| [LLM-friendliness](../agent-interface.md) | | | | ○ | ○ | ● (CRUD volume) | ○ | |
| [Semantic diffs / review](../semantic-vcs.md) | | | | ○ | ● (audited algos) | ● | ○ | |
| R1 WASM | ● | ● | ○ (browser demo) | ● (browser client) | | ● (web client) | ● (browser client) | |
| Mobile (iOS/Android) | | | | | | ● | | |

No problem area is left unexercised. [GPU](../accelerators.md), once the thinnest, is now central to
every showcase: the Mandelbulb's ray marching, fumoca's Monte Carlo kernels and Talk's ML inference.

## Coverage: examples × standard mods

See [`../stdlib.md`: Initial standard-mod catalogue](../stdlib.md#initial-standard-mod-catalogue) for the full catalogue. Mods marked **new** are example-specific
mods the example would create.

| Mod | Mandelbulb | fumoca | Talk | Game | HFT | Store | Hashiverse | Firmware |
|---|---|---|---|---|---|---|---|---|
| `moddil/net`, `moddil/rpc` | | | ○ (download) | ● | ● | ● | ● | |
| `moddil/runtime`, `-wasm`, `-sim` | ● (`-wasm`) | ● (`-wasm`) | ● | ● | ● | ● | ● | ● (`rt-embedded`, `-sim`) |
| `moddil/serde` | | ● | | ● | ● | ● | ● | ● |
| `moddil/sync` | | ○ | | ● | ● | | ● | ● |
| `moddil/alloc` | | | | ○ | ● | | | |
| `moddil/crypto` | | | ○ (weight hashes) | | | ○ | ● | |
| `moddil/simd`, `moddil/tensor` | | | ● | ○ | ● | | ○ | |
| `moddil/gpu` | ● | ● | ● | ● | ● | | | |
| `moddil/decimal` | | | | | ● | ● | | |
| `moddil/sql` | | | | | ○ | ● | | |
| `moddil/timeseries` | | | | | ● | | | |
| `moddil/web` | ● | ● | ○ | ● | | ● | ● | |
| `moddil/storage` | | | | | | ○ | ● | |
| **new** | `mandelbulb/render` | `fumoca/formula`, `fumoca/sim`, `fumoca/gpu` | `talk/g2p`, `talk/kokoro` | `game/netcode`, `game/chat` | `hft/marketdata`, `hft/risk`, `hft/fix` | `store/catalog`, `store/payments` | `hashiverse/protocol`, `moddil/dht`?, `moddil/pow`? | `sensor/hal`, `sensor/frames` |

The order in which the examples are built is in [`roadmap.md`: Order of the reference examples](../roadmap.md#order-of-the-reference-examples).

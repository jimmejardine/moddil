# Mods: Small Core, Big Library

> **Status:** brainstorm. Implements requirement **R2** in [`overview.md`: Firm requirements (decided)](overview.md#firm-requirements-decided).

## Idea

The Moddil **core language is very small**. Almost all functionality, including collections,
networking, crypto, runtimes, GPU support and UI, comes from **mods**:

- **Mod**: Moddil's unit of distribution, like a Rust crate or a Go module. A mod has a name, a
  version, a manifest, and one or more modules.
- **Module**: a namespace/file unit *inside* a mod.
- **Standard mods**: mods published under the `moddil/` namespace by the project itself. They play the
  role of the C++ STL and Boost combined, or Rust's `std` plus the handful of crates everyone uses
  (`serde`, `tokio`, `regex`, …).
- **Community mods**: anyone can write and publish mods, including alternatives to standard ones
  (a different runtime, a different collections library).

Why a small core:
- **The language stays learnable**, for humans and LLMs ([overview: LLMs as first-class programmers](overview.md#llms-as-first-class-programmers)). The stable surface is small.
- **Evolution happens in libraries**, which can be versioned, forked and replaced. The core changes
  rarely ([overview: Language evolution](overview.md#language-evolution)).
- **Build times** ([overview: Build times](overview.md#build-times)): the core does not drag in code you don't use.
- **No privileged magic**: if a standard mod can do it, a community mod can do it too.

## What a mod is

A mod is the unit of:
- **Versioning**: semver, with editions for the language ([overview: Language evolution](overview.md#language-evolution)).
- **Compilation and caching** ([overview: Build times](overview.md#build-times)): interface hash vs implementation hash (see
  [`build-times.md`: Language rules that decide build speed](build-times.md#language-rules-that-decide-build-speed)).
- **Capabilities** ([overview: Package management and supply-chain security](overview.md#package-management-and-supply-chain-security)): the manifest declares which capabilities the mod may require (`net`, `fs`,
  `ffi`, `gpu`, `unsafe`, …). The compiler rejects any use beyond them.
- **Targets**: native, WASM (R1), GPU-kernel-safe, and so on. A mod declares which targets it supports.
- **Audit** ([overview: Package management and supply-chain security](overview.md#package-management-and-supply-chain-security)): reviews and attestations attach to a mod version.

Illustrative manifest (syntax not decided):

```toml
[mod]
name = "moddil/net"
version = "0.3.0"
edition = "2027"

[capabilities]
requires = ["net", "time"]

[targets]
supported = ["native", "wasm-wasi", "wasm-browser"]
```

## The core / mod boundary

Something belongs in the **core** only if at least one of these is true:

1. **The compiler must check it.** Guarantees that a library cannot enforce: ownership and linearity,
   effects, capabilities, placement, contracts.
2. **Other mods need it to be defined at all.** Primitive types, modules, generics, compile-time
   evaluation and reflection.
3. **It cannot be expressed efficiently in a mod.** Intrinsics, the C ABI, SIMD primitives, atomics.

Candidate core list (to be argued over):

| Core | Why core |
|---|---|
| Primitive types, structs, sum types, generics | Needed to define anything |
| Ownership, borrowing / linearity | Compiler-checked ([overview: Memory and resource management](overview.md#memory-and-resource-management)) |
| Effects and handlers (definition and checking) | Compiler-checked; the extension mechanism for everything else ([overview: Effects and capabilities as a unifying idea](overview.md#effects-and-capabilities-as-a-unifying-idea)) |
| Capabilities | Compiler-checked ([overview: Package management and supply-chain security](overview.md#package-management-and-supply-chain-security)) |
| Placement annotations | Compiler-checked ([overview: Concurrency across GPUs and accelerators](overview.md#concurrency-across-gpus-and-accelerators), [overview: Distributed and multi-tier programs](overview.md#distributed-and-multi-tier-programs)) |
| Structured concurrency *interface* (task scopes, cancellation, the `suspend` effect) | Compiler-checked; must be shared by all runtimes |
| Contracts | Compiler-checked ([overview: Correctness tooling: contracts and verification](overview.md#correctness-tooling-contracts-and-verification)) |
| Compile-time evaluation, static reflection | Needed to define mods ([overview: Metaprogramming](overview.md#metaprogramming)) |
| Intrinsics: atomics, SIMD primitives, C ABI / FFI | Not expressible otherwise |

Everything else is a mod, **including the scheduler, the allocator implementations, I/O and the
async runtime**.

## Pluggable runtimes without ecosystem splits

**Lesson from Rust.** `async` is in the language, but the executor is not, and there is no standard
interface for spawning, timers or I/O. The result: libraries tied to tokio, async-std or smol, and an
ecosystem that partly fragmented.

**Moddil's approach.** The *interfaces* are core effects: `suspend`, `spawn` (scoped), `time`, `net`,
`fs`, `rand`, `log`. *Implementations* are mods that provide **handlers** for those effects:

- `moddil/runtime`: the standard multi-threaded work-stealing runtime.
- `moddil/runtime-wasm`: browser/WASI runtime using stack switching and JSPI (R1).
- `moddil/runtime-sim`: deterministic simulation runtime ([overview: Determinism and reproducibility](overview.md#determinism-and-reproducibility)).
- A community `fastrt` (the tokio-equivalent): a different scheduler, io_uring, thread-per-core, etc.

Because libraries only use the *effects*, **every library works on every runtime**. Swapping the
runtime is a choice in `main` (or in the deployment manifest, [overview: Distributed and multi-tier programs](overview.md#distributed-and-multi-tier-programs)), not a choice made by the whole
dependency tree.

The same mechanism generalises the pluggable traits that real projects build by hand. Hashiverse,
for example, has `TransportFactory`, `TimeProvider`, `ClientStorage` and `KeyLockerManager` traits plus
an in-memory transport for tests (see [`examples/04-hashiverse.md`](examples/04-hashiverse.md)). In
Moddil these are effects, and the test versions are handlers.

## Tiers of mods

| Tier | Namespace | Bar | Examples |
|---|---|---|---|
| Core-adjacent | `moddil/core-*` | Ships with the compiler; tiny; changes only with editions; usable in the `freestanding` profile ([`freestanding.md`](freestanding.md)) | `core-fmt`, `core-collections` (Vec, Map), `core-text` |
| Standard | `moddil/*` | Curated, RFC process, capability review, audited, edition-aware | `moddil/net`, `moddil/crypto`, `moddil/gpu` |
| Community | anyone | Capability ceiling and checked semver ([`packages-and-supply-chain.md`](packages-and-supply-chain.md)) | `fastrt`, `hashiverse/protocol` |

**Promotion path:** a community mod that becomes a de facto standard can be proposed for `moddil/`
(the Boost → C++ standard library path). Standard mods can also be **deprecated** and moved out,
which a monolithic std cannot easily do.

## Initial standard-mod catalogue

This list is driven by the four reference examples in [`examples/`](examples/README.md).
Legend: G = multiplayer game, H = HFT platform, W = web store, X = Hashiverse.

| Mod | Contents | Needed by |
|---|---|---|
| `moddil/collections` | Vec, Map, Set, deque, ring buffers, persistent collections | G H W X |
| `moddil/text` | UTF-8 strings, Unicode, regex | G W X |
| `moddil/serde` | Serialisation framework; binary and JSON formats; schema evolution; zero-copy validated views ([`memory-and-resources.md`: Zero-copy I/O](memory-and-resources.md#zero-copy-io)) | G H W X |
| `moddil/fs` / `moddil/io` | Files, streams, buffers | H W X |
| `moddil/net` | TCP, UDP, TLS, HTTP/1-2-3, WebSocket, WebTransport | G H W X |
| `moddil/rpc` | Compiler-generated wire contracts, version compatibility ([overview: Distributed and multi-tier programs](overview.md#distributed-and-multi-tier-programs), [`distribution.md`](distribution.md)) | G H W X |
| `moddil/deploy-*` | Adapters from the topology file to Compose, Kubernetes, Nomad, systemd; discovery and secret-provider handlers ([`distribution.md`: Adapter mods](distribution.md#adapter-mods)) | G H W X |
| `moddil/time` | Clocks, monotonic time, time zones, high-resolution timestamps | G H W X |
| `moddil/runtime`, `-wasm`, `-sim` | Effect handlers for scheduling and I/O ([Pluggable runtimes without ecosystem splits](#pluggable-runtimes-without-ecosystem-splits)) | G H W X |
| `moddil/sync` | Sync cells (`Atomic<T>`, `AtomicRc<T>`, `Mutex`, `RwLock`), channels, lock-free queues (SPSC/MPMC) ([`concurrency.md`: Shared mutable state](concurrency.md#shared-mutable-state)) | G H X |
| `moddil/alloc` | Arenas, pools, bump allocators, allocator capability | G H |
| `moddil/crypto` | Ed25519, ML-DSA, FN-DSA, Blake2/3, SHA-2/3, ChaCha20-Poly1305, TLS primitives | W X |
| `moddil/simd`, `moddil/tensor` | Portable SIMD; n-d arrays and tensor expressions compiled to graphs or kernels ([`accelerators.md`: Level 1: tensor expressions](accelerators.md#level-1-tensor-expressions)) | G H X |
| `moddil/gpu` | Devices, buffers, launches; WebGPU/Vulkan/Metal/CUDA/ROCm and CPU-simulator handlers ([`accelerators.md`](accelerators.md)) | G H |
| `moddil/decimal` | `Fixed<S>` fixed-point (scale in the type) and `BigDecimal` ([`numerics.md`: Decimals for money](numerics.md#decimals-for-money)) | H W |
| `moddil/num` | `BigInt`, `Rational` | X |
| `moddil/units` | Unit packs: `si`, `information`, `currency`, `angle` ([`units.md`](units.md)) | G H W X |
| `moddil/alloc` | Allocator-backed collections and `Rc`, without an OS (`freestanding-alloc` profile) | all |
| `moddil/rt-embedded` | Bare-metal scheduler handler with static task slots ([`freestanding.md`: Concurrency without a runtime](freestanding.md#concurrency-without-a-runtime)) | embedded |
| `moddil/hal` | Hardware effects (`gpio`, `uart`, `timer`, `mmio`), SVD-generated register views, simulation handlers | embedded |
| `moddil/sql` | Typed SQL, schemas, migrations, connection pools | H W |
| `moddil/timeseries` | Columnar storage, compression, tick data | H |
| `moddil/sim` | Deterministic simulation: network, time, faults ([overview: Determinism and reproducibility](overview.md#determinism-and-reproducibility)) | G H W X |
| `moddil/test`, `moddil/prop`, `moddil/bench` | Test runner, property-based testing, fuzzing, mutation testing, benchmarks ([`correctness.md`](correctness.md), [`docs-and-tests.md`](docs-and-tests.md)) | G H W X |
| `moddil/verify` | Verifier front end: proof obligations, SMT back ends, assurance levels (open: core tool or mod) | H X |
| `moddil/log`, `moddil/trace` | Handlers for the write-only `log`/`span`/`metric` effects: pretty output, OTLP, Prometheus, flight recorder ([`observability.md`](observability.md)) | G H W X |
| `moddil/web` | Browser APIs, DOM, JS interop for the `wasm-browser` target | G W X |
| `moddil/storage` | Key-value storage: SQLite, IndexedDB, files | W X |
| `moddil/ui` | Cross-platform UI (**open question**, see [`examples/03-web-store.md`](examples/03-web-store.md)) | G W X |
| `moddil/dht`, `moddil/pow` | Kademlia DHT, proof-of-work | X (probably community mods, not standard) |

## Contribution and governance

- **RFCs** for new standard mods and breaking changes.
- **Capability review**: any increase in a standard mod's declared capabilities needs review.
- **Audit trail** ([overview: Package management and supply-chain security](overview.md#package-management-and-supply-chain-security)): standard mod releases are reproducibly built, signed, and reviewed. The
  review record says whether the reviewer was a human or an agent.
- **Wrappers are allowed**: a mod may wrap a C or Rust library ([overview: Interoperability with C and Rust](overview.md#interoperability-with-c-and-rust), [`interop.md`](interop.md)),
  but it carries the `ffi` capability, which is visible to every consumer.
- **Editions**: standard mods follow language editions, with automated migrations ([overview: Language evolution](overview.md#language-evolution)).

## Open questions

- **How many batteries?** Go ships a broad standard library (HTTP, crypto, JSON); Rust ships a minimal
  one. With strong supply-chain controls ([overview: Package management and supply-chain security](overview.md#package-management-and-supply-chain-security)), a broad `moddil/` set reduces risky third-party
  dependencies. Which way should Moddil lean?
- **Distribution**: should standard mods ship with the toolchain, or be fetched and pinned like
  any other mod?
- **Naming**: is "mod" too easily confused with Rust's `mod` (a module)? The distinction between mod
  and module needs to stay clear in docs and tooling.
- **Coherence**: how are trait/instance overlap rules (Rust's orphan rule) handled across mods?

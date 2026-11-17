# Mods: Small Core, Big Library

> **Status:** brainstorm. Implements requirement **R2** in [`rationale.md`](rationale.md) §5.1.

## 1. Idea

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
- **The language stays learnable**, for humans and LLMs (§2.18). The stable surface is small.
- **Evolution happens in libraries**, which can be versioned, forked and replaced. The core changes
  rarely (§2.20).
- **Build times** (§2.14): the core does not drag in code you don't use.
- **No privileged magic**: if a standard mod can do it, a community mod can do it too.

## 2. What a mod is

A mod is the unit of:
- **Versioning**: semver, with editions for the language (§2.20).
- **Compilation and caching** (§2.14): interface hash vs implementation hash.
- **Capabilities** (§2.16): the manifest declares which capabilities the mod may require (`net`, `fs`,
  `ffi`, `gpu`, `unsafe`, …). The compiler rejects any use beyond them.
- **Targets**: native, WASM (R1), GPU-kernel-safe, and so on. A mod declares which targets it supports.
- **Audit** (§2.16): reviews and attestations attach to a mod version.

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

## 3. The core / mod boundary

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
| Ownership, borrowing / linearity | Compiler-checked (§2.2) |
| Effects and handlers (definition and checking) | Compiler-checked; the extension mechanism for everything else (§2.1) |
| Capabilities | Compiler-checked (§2.16) |
| Placement annotations | Compiler-checked (§2.9, §2.10) |
| Structured concurrency *interface* (task scopes, cancellation, the `suspend` effect) | Compiler-checked; must be shared by all runtimes |
| Contracts | Compiler-checked (§2.7) |
| Compile-time evaluation, static reflection | Needed to define mods (§2.6) |
| Intrinsics: atomics, SIMD primitives, C ABI / FFI | Not expressible otherwise |

Everything else is a mod, **including the scheduler, the allocator implementations, I/O and the
async runtime**.

## 4. Pluggable runtimes without ecosystem splits

**Lesson from Rust.** `async` is in the language, but the executor is not, and there is no standard
interface for spawning, timers or I/O. The result: libraries tied to tokio, async-std or smol, and an
ecosystem that partly fragmented.

**Moddil's approach.** The *interfaces* are core effects: `suspend`, `spawn` (scoped), `time`, `net`,
`fs`, `rand`, `log`. *Implementations* are mods that provide **handlers** for those effects:

- `moddil/runtime`: the standard multi-threaded work-stealing runtime.
- `moddil/runtime-wasm`: browser/WASI runtime using stack switching and JSPI (R1).
- `moddil/runtime-sim`: deterministic simulation runtime (§2.11).
- A community `fastrt` (the tokio-equivalent): a different scheduler, io_uring, thread-per-core, etc.

Because libraries only use the *effects*, **every library works on every runtime**. Swapping the
runtime is a choice in `main` (or in the deployment manifest, §2.10), not a choice made by the whole
dependency tree.

The same mechanism generalises the pluggable traits that real projects build by hand. Hashiverse,
for example, has `TransportFactory`, `TimeProvider`, `ClientStorage` and `KeyLockerManager` traits plus
an in-memory transport for tests (see [`examples/04-hashiverse.md`](examples/04-hashiverse.md)). In
Moddil these are effects, and the test versions are handlers.

## 5. Tiers of mods

| Tier | Namespace | Bar | Examples |
|---|---|---|---|
| Core-adjacent | `moddil/core-*` | Ships with the compiler; tiny; changes only with editions | `core-fmt`, `core-collections` (Vec, Map), `core-text` |
| Standard | `moddil/*` | Curated, RFC process, capability review, audited, edition-aware | `moddil/net`, `moddil/crypto`, `moddil/gpu` |
| Community | anyone | Registry rules only (§2.16) | `fastrt`, `hashiverse/protocol` |

**Promotion path:** a community mod that becomes a de facto standard can be proposed for `moddil/`
(the Boost → C++ standard library path). Standard mods can also be **deprecated** and moved out,
which a monolithic std cannot easily do.

## 6. Initial standard-mod catalogue

This list is driven by the four reference examples in [`examples/`](examples/README.md).
Legend: G = multiplayer game, H = HFT platform, W = web store, X = Hashiverse.

| Mod | Contents | Needed by |
|---|---|---|
| `moddil/collections` | Vec, Map, Set, deque, ring buffers, persistent collections | G H W X |
| `moddil/text` | UTF-8 strings, Unicode, regex | G W X |
| `moddil/serde` | Serialisation framework; binary and JSON formats; schema evolution; zero-copy validated views ([`memory-and-resources.md`](memory-and-resources.md) §6) | G H W X |
| `moddil/fs` / `moddil/io` | Files, streams, buffers | H W X |
| `moddil/net` | TCP, UDP, TLS, HTTP/1-2-3, WebSocket, WebTransport | G H W X |
| `moddil/rpc` | Compiler-generated wire contracts, version compatibility (§2.10) | G H W X |
| `moddil/time` | Clocks, monotonic time, time zones, high-resolution timestamps | G H W X |
| `moddil/runtime`, `-wasm`, `-sim` | Effect handlers for scheduling and I/O (§4) | G H W X |
| `moddil/sync` | Sync cells (`Atomic<T>`, `AtomicRc<T>`, `Mutex`, `RwLock`), channels, lock-free queues (SPSC/MPMC) ([`concurrency.md`](concurrency.md) §4) | G H X |
| `moddil/alloc` | Arenas, pools, bump allocators, allocator capability | G H |
| `moddil/crypto` | Ed25519, ML-DSA, FN-DSA, Blake2/3, SHA-2/3, ChaCha20-Poly1305, TLS primitives | W X |
| `moddil/simd`, `moddil/tensor` | Portable SIMD; n-d arrays and tensor expressions compiled to graphs or kernels ([`accelerators.md`](accelerators.md) §8) | G H X |
| `moddil/gpu` | Devices, buffers, launches; WebGPU/Vulkan/Metal/CUDA/ROCm and CPU-simulator handlers ([`accelerators.md`](accelerators.md)) | G H |
| `moddil/decimal` | Fixed-point and decimal arithmetic | H W |
| `moddil/sql` | Typed SQL, schemas, migrations, connection pools | H W |
| `moddil/timeseries` | Columnar storage, compression, tick data | H |
| `moddil/sim` | Deterministic simulation: network, time, faults (§2.11) | G H W X |
| `moddil/test`, `moddil/prop`, `moddil/bench` | Test runner, property-based testing, fuzzing, mutation testing, benchmarks ([`correctness.md`](correctness.md)) | G H W X |
| `moddil/verify` | Verifier front end: proof obligations, SMT back ends, assurance levels (open: core tool or mod) | H X |
| `moddil/log`, `moddil/trace` | Handlers for the write-only `log`/`span`/`metric` effects: pretty output, OTLP, Prometheus, flight recorder ([`observability.md`](observability.md)) | G H W X |
| `moddil/web` | Browser APIs, DOM, JS interop for the `wasm-browser` target | G W X |
| `moddil/storage` | Key-value storage: SQLite, IndexedDB, files | W X |
| `moddil/ui` | Cross-platform UI (**open question**, see [`examples/03-web-store.md`](examples/03-web-store.md)) | G W X |
| `moddil/dht`, `moddil/pow` | Kademlia DHT, proof-of-work | X (probably community mods, not standard) |

## 7. Contribution and governance

- **RFCs** for new standard mods and breaking changes.
- **Capability review**: any increase in a standard mod's declared capabilities needs review.
- **Audit trail** (§2.16): standard mod releases are reproducibly built, signed, and reviewed. The
  review record says whether the reviewer was a human or an agent.
- **Wrappers are allowed**: a mod may wrap a C or Rust library (§2.13, [`interop.md`](interop.md)),
  but it carries the `ffi` capability, which is visible to every consumer.
- **Editions**: standard mods follow language editions, with automated migrations (§2.20).

## 8. Open questions

- **How many batteries?** Go ships a broad standard library (HTTP, crypto, JSON); Rust ships a minimal
  one. With strong supply-chain controls (§2.16), a broad `moddil/` set reduces risky third-party
  dependencies. Which way should Moddil lean?
- **Distribution**: should standard mods ship with the toolchain, or be fetched and pinned like
  any other mod?
- **Registry**: one central registry with a `moddil/` namespace, or decentralised (Go-style URLs plus a
  proxy and checksum database)?
- **Naming**: is "mod" too easily confused with Rust's `mod` (a module)? The distinction between mod
  and module needs to stay clear in docs and tooling.
- **Coherence**: how are trait/instance overlap rules (Rust's orphan rule) handled across mods?

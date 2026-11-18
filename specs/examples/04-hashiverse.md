# Example 4: Hashiverse Reference Client (and later Server)

## Overview

[Hashiverse](https://www.hashiverse.com) is an open-source, decentralised, peer-to-peer replacement for
X/Twitter. It is an existing, live project (owned by the Moddil author) with:

- a Rust workspace: `hashiverse-lib` (protocol), `hashiverse-server-lib`/`hashiverse-server`,
  `hashiverse-client-rust`, `hashiverse-cli`, and WASM, Python and Node.js client wrappers;
- a TypeScript/React web app that runs the WASM client inside a Web Worker;
- no central servers: peers find each other via a **Kademlia DHT**;
- **proof-of-work** in the RPC layer, in server identities and in feedback reporting;
- a serious crypto stack: Ed25519 plus post-quantum ML-DSA and FN-DSA signatures, Blake3, chained
  Blake2/Blake3/SHA-2/SHA-3/Whirlpool/Groestl/Skein for PoW hardness, ChaCha20-Poly1305;
- **pluggable traits** throughout (`TransportFactory`, `EnvironmentFactory`, `TimeProvider`,
  `ClientStorage`, `KeyLockerManager`), with an in-memory transport and a test harness that starts
  many servers and clients in one process.

Unlike the other three examples, this one is **not hypothetical**: a Moddil implementation must
interoperate with a running network built in another language.

## Recommendation: client first, server second

**Phase A: Moddil reference client.**
- **CLI/native client first, browser client second.** The early toolchain targets native and WASI on
  Wasmtime; the browser needs the JSPI lowering, which comes later (see
  [`../bootstrap.md`](../bootstrap.md) §5–§7).
- Smaller scope than a server.
- Must speak the protocol **on the wire** with existing Rust servers. This is a strong test of
  compiler-checked wire contracts against a protocol Moddil does not own (§2.13, §2.10).
- Exercises WASM in the browser (R1): Web Worker, IndexedDB storage, in-browser key management.
- Exercises CPU-heavy, parallel proof-of-work (§2.8) and crypto performance (SIMD, §2.5).
- Can start by calling `hashiverse-lib` through the C ABI (§2.13), then replace pieces with native
  Moddil one module at a time. That is exactly the incremental adoption path Moddil promises.

**Phase B: Moddil reference server.**
- Long-running and exposed to untrusted peers on the open internet.
- DHT routing and maintenance, storage, DDoS protection, PoW verification.
- Mixed-version P2P compatibility: the network always contains many server versions.
- Packaged as a static binary or container (§2.15) for volunteers to run.

## Topology

```
 ┌───────── browser ─────────┐       ┌───────── native ─────────┐
 │ client  (WASM, Web Worker)│       │ client  (CLI / desktop)   │
 │ IndexedDB, key locker     │       │ SQLite, on-disk keys      │
 └────────────┬──────────────┘       └────────────┬─────────────┘
              │ HTTPS RPC (PoW-stamped)           │
              ▼                                   ▼
        ┌───────────────────── P2P network ──────────────────────┐
        │  server   server   server   …  (Kademlia DHT, volunteers)│
        │   existing Rust servers  +  (phase B) Moddil servers     │
        └─────────────────────────────────────────────────────────┘
```

## Key flows

1. **Bootstrap**: client discovers peers, then builds and maintains a peer table (DHT).
2. **Post**: compose → sign (Ed25519 / ML-DSA) → compute PoW → send to the responsible peers.
3. **Read timeline**: recursive bucket traversal across peers; verify signatures; cache locally.
4. **Feedback/reporting**: PoW-protected reports.
5. **Key management**: keys locked locally (browser key locker or on-disk), never leaving the client.

## Moddil features this stresses

- **Effects as built-in pluggable traits (§2.1).** Hashiverse already does by hand what Moddil wants
  to make standard: transport, time, storage and key management are swappable traits with production
  and test implementations. In Moddil these become effects, and the implementations become handlers:
  `net` → HTTPS or in-memory; `time` → real or simulated; `storage` → IndexedDB, SQLite or memory;
  `keys` → browser locker or disk. This is concrete evidence for the effects design, drawn from a real
  codebase.
- **Deterministic simulation (§2.11).** Hashiverse's test harness (one primary, several secondary servers
  and many clients in one process) is exactly what `moddil/runtime-sim` should provide out of the box,
  with seeded replay and fault injection added.
- **WASM (R1).** Browser client in a Web Worker; Moddil's runtime-wasm with stack switching should remove
  the need for a separate worker-proxy layer written in TypeScript.
- **Interop with Rust (§2.13).** Calling `hashiverse-lib` from Moddil during migration; wire
  compatibility with Rust servers permanently.
- **Crypto and PoW performance (§2.5, §2.8).** Hash chains and signature checks must match Rust
  performance, natively and in WASM (SIMD).
- **Capabilities and untrusted input (§2.16, §2.10).** Every byte from a peer is untrusted. The crypto mods
  must not have `net` or `fs`.
- **Distributed without a centre (§2.10).** A P2P system has no `hub`; every node runs the same code.
  This tests whether Moddil's placement model fits symmetric topologies, not just client/server ones.

## Success criteria

- A Moddil client posts and reads on the **live Hashiverse network**, interoperating with Rust servers.
- It passes the equivalent of Hashiverse's integration scenarios (multi-server, multi-client) under
  `moddil/runtime-sim`.
- WASM bundle size and PoW throughput are comparable to the existing Rust WASM client.
- (Phase B) A Moddil server joins the network alongside Rust servers without either side noticing a
  difference.

## Mods used

- Standard: `moddil/crypto` (Ed25519, ML-DSA, FN-DSA, Blake2/3, SHA-2/3, ChaCha20-Poly1305),
  `moddil/net` (HTTPS), `moddil/rpc`, `moddil/serde`, `moddil/storage` (IndexedDB, SQLite),
  `moddil/web`, `moddil/runtime` / `-wasm` / `-sim`, `moddil/sync`, `moddil/simd`, `moddil/time`,
  `moddil/collections`.
- New: `hashiverse/protocol` (packet formats, RPC, post bundles), plus Kademlia DHT and PoW mods.
  These may be generic enough to become community mods (`dht`, `pow`), but they are probably not
  standard-library material.
- Crypto primitives without a Moddil implementation yet (Whirlpool, Groestl, Skein) can initially wrap
  existing C/Rust implementations behind the `ffi` capability.

## Interop and non-Moddil parts

- The existing Rust servers and clients (the network itself).
- The existing React web app: the Moddil WASM client could slot in behind the same TypeScript API as a
  drop-in replacement for `hashiverse-client-wasm`.

## Open questions

- Does the Moddil client reimplement the protocol from a spec, or is the Rust code the spec? A
  written protocol spec would benefit Hashiverse independently.
- Should the protocol's wire format be described once in a schema that both Rust and Moddil generate
  from?
- How does Moddil's placement model express symmetric P2P nodes (each node is both client and server)?

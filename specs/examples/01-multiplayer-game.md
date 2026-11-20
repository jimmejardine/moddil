# Example 1: Multiplayer Game

## Overview

A real-time multiplayer browser game. Players run a WASM client in the browser, chat in rooms, and
play matches coordinated by a central communications hub. The genre is deliberately unspecified; the
point is a real-time, authoritative-server architecture of the kind used by most modern online games.

## Topology

```
          ┌────────────────────────────── browser ──────────────────────────────┐
          │  client   (WASM + WebGPU)                                            │
          │  input → prediction → render        chat UI                          │
          └───────┬──────────────────────────────────────────┬──────────────────┘
                  │ WebTransport (unreliable+reliable)          │ WebSocket
                  ▼                                             ▼
          ┌───────────────┐   room events   ┌────────────────────────────┐
          │    hub        │◄───────────────►│   chat   (N instances)      │
          │ matchmaking,  │                 │ chatrooms, presence,        │
          │ session route,│                 │ moderation                  │
          │ authoritative │                 └────────────────────────────┘
          │ match state   │
          └───────────────┘
```

| Placement | Role | Scale |
|---|---|---|
| `client` | Input, client-side prediction and interpolation, rendering, chat UI | One per player, untrusted |
| `hub` | Matchmaking, session routing, authoritative simulation of matches, state relay | Sharded by match |
| `chat` | Chatrooms, presence, moderation, history | Horizontally scaled |

## Key flows

1. **Join**: client authenticates → hub assigns a match → client connects to the match's hub shard
   and the relevant chatroom.
2. **Play loop**: client sends inputs at a fixed rate → hub simulates authoritatively at a fixed tick →
   hub sends state snapshots/deltas → client reconciles its prediction.
3. **Chat**: messages go to `chat`, are moderated, then fanned out to room members. The hub notifies chat
   of match events (e.g. "match started").
4. **Disconnect/reconnect**: the session survives brief drops; state resynchronises.

## Moddil features this stresses

- **WASM in the browser (R1)**: binary size, start-up time, JSPI for suspending on browser APIs
  (WASM stack switching once browsers ship it).
- **GPU ([Concurrency across GPUs and accelerators](../accelerators.md))**: rendering and possibly particle/physics kernels through the WebGPU/WGSL path. The same
  kernel code could run server-side for authoritative physics.
- **Placement and the `Remote` effect ([Distributed and multi-tier programs](../distribution.md))**: the client/hub/chat split in one program. Unreliable vs
  reliable channels are distinguished in types: snapshots go over
  `Channel<Snapshot, Datagram, Unordered>` ([`distribution.md`: The `Remote` effect](../distribution.md#the-remote-effect)).
- **WebTransport only**: the game channel uses WebTransport, with no WebRTC fallback, so a browser
  or network without it cannot play; chat stays on WebSocket.
- **Choreographies ([Distributed and multi-tier programs](../distribution.md))**: the match lifecycle (join, ready, start, end) is written
  once and projected to `client`, `hub` and `chat` with `moddil/choreo`, once it exists after v1
  ([`distribution.md`: Choreographies (after v1)](../distribution.md#choreographies-after-v1)).
  Per-frame snapshot streaming is ordinary code.
- **Untrusted client ([Distributed and multi-tier programs](../distribution.md))**: all data from `client` is untrusted; the hub is authoritative. Anti-cheat
  starts with the type system refusing to let client-claimed state flow into authoritative state
  without validation.
- **Shared code across tiers**: game rules and the simulation step run in *both* `client` (prediction)
  and `hub` (authority). They must compile to WASM and native and give **bit-identical results**.
  Floating point is enough: strict IEEE 754 and a correctly rounded `moddil/math` give identical
  results on both ([`determinism.md`: Deterministic maths](../determinism.md#deterministic-maths)).
- **Structured concurrency ([Concurrency across CPUs](../concurrency.md))**: one task scope per match and per room. When a match ends, all its
  tasks end with it.
- **Deterministic simulation ([Determinism and reproducibility](../determinism.md))**: run hub + chat + N simulated clients in one process, with
  simulated latency, packet loss and disconnects, replayable from a seed.
- **Hot reload ([Build times](../build-times.md))**: change game rules or rendering without restarting the client.
- **Per-frame scratch memory ([`stdlib.md`: Scratch arenas](../stdlib.md#scratch-arenas))**: each client frame and hub tick
  allocates its temporary data in a `Scratch` arena that is reset at the end of the frame, with no
  per-object frees.
- **In-game inspector**: a debug overlay that lists and edits entity fields needs runtime type
  information (open: [`stdlib.md`: Open questions](../stdlib.md#open-questions)).

## Success criteria

Initial targets, to be refined:
- Browser WASM bundle: small enough for fast start (target in the low single-digit MB compressed),
  interactive within a couple of seconds on a mid-range laptop.
- Hub tick rate: 60 Hz per match with headroom; hundreds of matches per hub process.
- Prediction/authority code shared verbatim between client and hub, with identical results in a
  cross-target determinism test.
- Whole system runs in one process under `moddil/runtime-sim`, and a seeded run reproduces exactly.
- A chat server can be redeployed mid-game without dropping matches, and the compiler verifies wire
  compatibility between versions.

## Mods used

- Standard: `moddil/net` (WebTransport, WebSocket), `moddil/rpc`,
  `moddil/runtime` / `-wasm` / `-sim`, `moddil/serde` (compact binary), `moddil/gpu` (WebGPU),
  `moddil/simd`, `moddil/web`, `moddil/sync`, `moddil/collections`, `moddil/text`. The HUD and menus are
  GPU-rendered or DOM ([`ui.md`: Games and graphics](../ui.md#games-and-graphics)).
- New: `game/netcode` (prediction, interpolation, delta compression), `game/chat`.

## Interop and non-Moddil parts

- Browser APIs (WebGPU, WebTransport, input, audio) through `moddil/web`.
- Possibly an existing physics engine (C/C++/Rust) compiled to WASM and linked as a component ([Interoperability with C and Rust](../interop.md), R1).
- Authentication through a standard identity provider (OAuth/OIDC).

## Open questions

None at present.

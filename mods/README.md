# Standard mods

The **standard mods**, written in Moddil: `core-*`, `collections`, `text`, `net`, `serde` and the
rest of the catalogue ([`specs/stdlib.md`](../specs/stdlib.md)).

**Status:** M2 slices M2c to M2e: the mods the tour and Mandelbulb flight reach, each with its tests in its own `tests/`
project (so a mod that depends on one does not run them), run natively, on the MIR interpreter
and as WASM by `crates/moddil/tests/projects.rs`, one test for each project and runtime. A program names them as `moddil::<mod>` through the
umbrella mod [`moddil`](moddil/README.md), which re-exports them.

| Mod | What it has | Capabilities |
|---|---|---|
| [`collections`](collections/src/lib.moddil) | `Map` and `Set` over the `core` mod's hash maps, `Deque`, `RingBuffer`, `BinaryHeap`, the sorted `TreeMap` and `TreeSet`, the insertion-ordered `OrderedMap` and `OrderedSet`, `Interner` with typed `Symbol`s, and the persistent `PersistentList`, `PersistentVec`, `PersistentMap` and `PersistentSet` | |
| [`sync`](sync/src/lib.moddil) | `Mutex` (with a `Guard` view, and `with`), `RwLock`, `Condvar` and `Once`, over `UnsafeCell` and atomics; a waiting task parks (`task.park.1`) | `unsafe` |
| [`rand`](rand/src/lib.moddil) | the `Rand` effect, and `Seeded` (`seeded`) | `unsafe` |
| [`serde`](serde/src/lib.moddil) | `Serialize` and `Deserialize` with their derives (impl templates), `DecodeError`, JSON | |
| [`net`](net/src/lib.moddil) | the `Net` effect, `Port`, `Url`, `Conn` (bytes, over a simulated link or an OS connection), `Listener` (a selectable `accept`); [`http`](net/src/http.moddil): HTTP/1.1, `Router`, `Request`, `Response`, `serve_connection`, `post` with retries | `os` |
| [`sim`](sim/src/lib.moddil) | `Sim`, nodes, seeded randomness, a simulated network, `drop_messages` | `unsafe` |
| [`crypto`](crypto/src/lib.moddil) | SHA-256, SHA-512 and BLAKE3, and `hex`; not constant-time | |
| [`num`](num/src/lib.moddil) | `BigInt` and `Rational` | |
| [`time`](time/src/lib.moddil) | `Date` (proleptic Gregorian), `TimeOfDay`, `DateTime` and `Timestamp`, in UTC, with ISO 8601 and RFC 3339 text; no clock yet | |
| [`regex`](regex/src/lib.moddil) | `Regex`: linear-time matching (a Pike VM), groups and named groups, `find_all`, `replace_all`; backreferences and lookaround refused | |
| [`decimal`](decimal/src/lib.moddil) | `Fixed<S>` (the scale in the type) and `BigDecimal`, rounding by the prelude's `Round` | |
| [`runtime`](runtime/src/lib.moddil) | the root capabilities (`root`: `Args`, `Stdio`, `NetRoot`, `Entropy`, …), the terminal (`io`), `serve_until_stopped` and `Signal` | `os` |
| [`gpu`](gpu/src/lib.moddil) | `Rgba8`, `Tile` (with `each`: a pixel per GPU thread), `Image`, the `Gpu` effect, the CPU `simulator()`, the root capability `GpuRoot` (WebGPU in a browser), `launch` (draws an image and reads it back) and `present` (shows it, with no copy back) | `os` |
| [`web`](web/src/lib.moddil) | the root capability `Window`: each animation frame's input (keys, mouse, wheel) and size | `os` |
| [`moddil`](moddil/src/lib.moddil) | the umbrella: every mod above, re-exported as `moddil::<mod>`, with `moddil::root` and `moddil::io` from `runtime` | |

`Vec`, `String` and the hash maps are in the prelude and the `core` mod
([`reference/2027/`](../reference/2027/README.md)).

## Layout and releases

- One directory per mod (`mods/collections/`, `mods/net/`, …), each a real mod with its own manifest.
- **`core-*` ships inside the toolchain** and changes only with editions. **Every other mod** is an
  ordinary dependency: released with a path-prefixed tag (`mods/net/v0.3.1`), reached through a vanity
  name (`moddil.com/net`), and subject to cooldowns and capability ceilings like any third-party mod
  ([`specs/stdlib.md`: Distribution and versioning](../specs/stdlib.md#distribution-and-versioning)).
- Standard mods follow the extra rules in [`specs/governance.md`: Standard mods](../specs/governance.md#standard-mods):
  RFCs for new mods, capability review, reproducible signed releases.

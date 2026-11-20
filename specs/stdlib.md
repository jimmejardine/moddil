# Standard library and mods

> **Status:** complete: decided, with no open questions. Listed in the [index](README.md). The catalogue of
> standard mods is in [Initial standard-mod catalogue](#initial-standard-mod-catalogue); this document defines the shape of the
> core interfaces they share. Snippets are **illustration only**.

## Summary

| Topic | Decision |
|---|---|
| **Small core (R2)** | The core holds only what the compiler must check, what other mods need defined, or what cannot be expressed efficiently in a mod; everything else is a mod ([Small core, functionality in mods (R2)](#small-core-functionality-in-mods-r2)). |
| **Batteries** | **Broad, as separate mods.** Coverage at Go's level (networking, HTTP, crypto, serialisation, SQL, time, testing), delivered as many independently versioned `moddil/*` mods. Only the small `moddil/core-*` set is tied to the compiler ([Batteries and tiers](#batteries-and-tiers)). |
| **Distribution** | `core-*` ships with the compiler and changes only with editions. Every other `moddil/*` mod is fetched, hash-pinned and upgraded like any dependency ([Distribution and versioning](#distribution-and-versioning)). |
| **Iteration** | **One external `Iterator` protocol.** `next` may carry effects, so an iterator that suspends is also an async stream: there is no separate `Stream` trait ([Iteration](#iteration)). |
| **Generators** | **`gen` blocks and `gen fn` in v0**, compiled to state-machine iterators ([Generators](#generators)). |
| **Collections** | Rust's names and shapes; `Map` and `Set` traits for generic code; seeded hash order ([Collections](#collections)). |
| **I/O** | One set of `Read`/`Write` traits for every kind of I/O, with **borrowed buffers**, safe on completion-based kernels because cancellation is structured. No async drop: a resource is linear if skipping its graceful close loses data, affine with an abrupt drop otherwise ([I/O](#io)). |
| **HTTP** | A `Router` with paths checked at compile time and typed path parameters; handlers are plain functions whose errors must implement `IntoResponse` (R3) ([HTTP](#http)). |
| **Vectorisation** | Not promised; `#[vectorise]` asserts it where it matters and fails the build otherwise ([Iteration](#iteration)). |

## Problem

The standard library's core interfaces are what every other library agrees on. When they are wrong,
the ecosystem splits, and the split is permanent:

- Rust has `Iterator` and `Stream`, `std::io::Read` and `AsyncRead`, and two incompatible
  `AsyncRead`s (`tokio::io` and `futures::io`). Every adaptor exists twice or three times.
- C# has `IEnumerable` and `IAsyncEnumerable`; Swift has `Sequence` and `AsyncSequence`; Kotlin has
  `Sequence` and `Flow`.
- A minimal standard library pushes basic needs (HTTP, JSON, time zones) onto third-party packages,
  each one a supply-chain risk ([`packages-and-supply-chain.md`](packages-and-supply-chain.md)).
- A monolithic standard library ties every library fix to a compiler release and can never drop a
  mistake (Python's "dead batteries", Java's `Date`).

## Prior art

| Language | Approach | Lesson |
|---|---|---|
| **Go** | Broad std (HTTP, crypto, JSON); `io.Reader`/`io.Writer`; range-over-func iterators (1.23) | Broad and stable wins adoption; small, universal I/O interfaces compose everywhere |
| **Rust** | Minimal std; `Iterator` with lazy adaptors; async split into separate traits; `gen` blocks still unstable | The iterator design is excellent; the sync/async split is the cost of coloured functions |
| **Python** | Batteries included; generators | Generators make custom iteration trivial; old batteries become dead weight |
| **C#, Swift, Kotlin** | Separate sync and async sequence protocols | The same duplication as Rust, for the same reason |
| **Zig** | Allocators passed explicitly; small `std.io` | Explicit cost; less convenient defaults |
| **tokio-uring, monoio** | Owned buffers for io_uring | Needed in Rust because a future can be dropped while the kernel still writes into the buffer |

## Small core, functionality in mods (R2)

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
- **The language stays learnable**, for humans and LLMs ([LLMs as first-class programmers](agent-interface.md)). The stable surface is small.
- **Evolution happens in libraries**, which can be versioned, forked and replaced. The core changes
  rarely ([Language evolution](evolution.md)).
- **Build times** ([Build times](build-times.md)): the core does not drag in code you don't use.
- **No privileged magic**: if a standard mod can do it, a community mod can do it too.

### The core / mod boundary

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
| Ownership, borrowing / linearity | Compiler-checked ([Memory: ownership, borrowing and sharing](memory.md)) |
| Effects and handlers (definition and checking) | Compiler-checked; the extension mechanism for everything else ([Effects and capabilities as a unifying idea](effects-and-capabilities.md)) |
| Capabilities | Compiler-checked ([Package management and supply-chain security](packages-and-supply-chain.md)) |
| Placement annotations | Compiler-checked ([Concurrency across GPUs and accelerators](accelerators.md), [Distributed and multi-tier programs](distribution.md)) |
| Structured concurrency *interface* (task scopes, cancellation, the `suspend` effect) | Compiler-checked; must be shared by all runtimes |
| Contracts | Compiler-checked ([Correctness tooling: contracts and verification](correctness.md)) |
| Compile-time evaluation, static reflection | Needed to define mods ([Metaprogramming](comptime.md)) |
| Intrinsics: atomics, SIMD primitives, C ABI / FFI | Not expressible otherwise |

Everything else is a mod, **including the scheduler, the allocator implementations, I/O and the
async runtime**.

## Batteries and tiers

**Decision: broad batteries, delivered as separate mods.**

| Tier | Contents | Bar | Tied to |
|---|---|---|---|
| **`moddil/core-*`** | Primitives, `Option`/`Result`, slices, `str`, `char`, `Iterator`, formatting traits, conversion traits; with an allocator: `Vec`, `String`, `Box`, `Rc` and the core collections | Tiny; changes only with editions; usable in the freestanding profiles ([`freestanding.md`: Profiles](freestanding.md#profiles)) | The compiler and the edition |
| **`moddil/*`** | Everything else in the [catalogue](#initial-standard-mod-catalogue): `io`, `fs`, `net`, `time`, `text`, `i18n`, `serde`, `sql`, `crypto`, `sync`, `test`, … | Curated, RFC process, capability review, audited, edition-aware ([`governance.md`](governance.md)) | Nothing: each mod has its own version |
| **Community** | Anything else, including alternatives to standard mods (`fastrt`, `hashiverse/protocol`) | Capability ceiling and checked semver ([`packages-and-supply-chain.md`](packages-and-supply-chain.md)) | Nothing |

A mod belongs in `moddil/*` when at least one of these holds:

1. **Most programs need it**, or a [reference example](examples/README.md) does.
2. **It is security-sensitive** (crypto, TLS, parsing untrusted input), where one audited version is
   worth more than choice.
3. **It is an interoperability contract** that other mods must agree on (`serde`'s traits, the
   `log` and `span` effects, HTTP types).

Unused mods cost nothing: they are not fetched, compiled or linked. Standard mods can be deprecated and
moved out without a compiler release.

**Promotion path:** a community mod that becomes a de facto standard can be proposed for `moddil/`
(the Boost → C++ standard library path). Standard mods can also be **deprecated** and moved out,
which a monolithic std cannot easily do.

## Initial standard-mod catalogue

This list is driven by the four reference examples in [`examples/`](examples/README.md).
Legend: G = multiplayer game, H = HFT platform, W = web store, X = Hashiverse.

| Mod | Contents | Needed by |
|---|---|---|
| `moddil/collections` | Vec, Map, Set, deque, ring buffers, persistent collections, `Interner<T>` | G H W X |
| `moddil/text` | Unicode algorithms: graphemes, word and sentence breaks, display width, normalisation, case folding ([`text.md`](text.md)) | G W X |
| `moddil/regex` | Linear-time regular expressions; `re"…"` compiled at compile time ([`text.md`: Regular expressions](text.md#regular-expressions)) | W X |
| `moddil/regex-backtrack` | Backreferences and lookaround, with a step limit on every match | Opt-in |
| `moddil/i18n` | The `Locale` effect: localised numbers, dates, plurals, collation, MessageFormat 2 ([`text.md`: Localisation: the `Locale` effect](text.md#localisation-the-locale-effect)) | G W X |
| `moddil/serde` | Serialisation framework; binary and JSON formats; schema evolution; zero-copy validated views ([`memory.md`: Zero-copy I/O](memory.md#zero-copy-io)) | G H W X |
| `moddil/fs` / `moddil/io` | Files, streams, buffers | H W X |
| `moddil/net` | TCP, UDP, TLS, HTTP/1-2-3, WebSocket, WebTransport | G H W X |
| `moddil/rpc` | Compiler-generated wire contracts, version compatibility ([Distributed and multi-tier programs](distribution.md), [`distribution.md`](distribution.md)) | G H W X |
| `moddil/deploy-*` | Adapters from the topology file to Compose, Kubernetes, Nomad, systemd; discovery and secret-provider handlers ([`distribution.md`: Adapter mods](distribution.md#adapter-mods)) | G H W X |
| `moddil/time` | Clocks, monotonic time, time zones, high-resolution timestamps | G H W X |
| `moddil/platform` | Device-service effects (camera, notifications, payments, keychain) with shell-provided handlers ([`ui.md`: Platform APIs as shell-provided handlers](ui.md#platform-apis-as-shell-provided-handlers)) | G W X |
| `moddil/runtime`, `-wasm`, `-sim` | Effect handlers for scheduling and I/O ([Pluggable runtimes without ecosystem splits](runtime.md#pluggable-runtimes-without-ecosystem-splits)) | G H W X |
| `moddil/sync` | Sync cells (`Atomic<T>`, `AtomicRc<T>`, `Mutex`, `RwLock`), channels, lock-free queues (SPSC/MPMC) ([`concurrency.md`: Shared mutable state](concurrency.md#shared-mutable-state)) | G H X |
| `moddil/alloc` | Arenas, pools, bump allocators, scratch arenas ([Scratch arenas](#scratch-arenas)), allocator capability | G H |
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
| `moddil/sim` | Deterministic simulation: network, time, faults ([Determinism and reproducibility](determinism.md)) | G H W X |
| `moddil/test`, `moddil/prop`, `moddil/bench` | Test runner, property-based testing, fuzzing, mutation testing, benchmarks ([`correctness.md`](correctness.md), [`docs.md`](docs.md)) | G H W X |
| `moddil/verify` | Verifier front end: proof obligations, SMT back ends, assurance levels (open: core tool or mod) | H X |
| `moddil/log`, `moddil/trace` | Handlers for the write-only `log`/`span`/`metric` effects: pretty output, OTLP, Prometheus, flight recorder ([`observability.md`](observability.md)) | G H W X |
| `moddil/web` | Bindings generated from WebIDL; the `Dom` effect and a simulated DOM; JS interop for `wasm32-browser` ([`ui.md`: The browser](ui.md#the-browser)) | G W X |
| `moddil/storage` | Key-value storage: SQLite, IndexedDB, files | W X |
| `moddil/dht`, `moddil/pow` | Kademlia DHT, proof-of-work | X (probably community mods, not standard) |

## Distribution and versioning

**Decision: the core ships with the compiler; the rest is pinned like any dependency.**

- **`core-*` ships in the toolchain** and changes only with editions
  ([`evolution.md`](evolution.md)). It is what the compiler itself knows about: `Option`, `Iterator`,
  the `for` loop protocol, formatting.
- **Every other `moddil/*` mod is an ordinary dependency.** It is named by URL, pinned by hash,
  selected by minimal version selection, and subject to the randomised cooldown and capability
  ceilings ([`packages-and-supply-chain.md`](packages-and-supply-chain.md)). A fix to `moddil/crypto` ships the day it is
  ready, without a compiler release, and a project upgrades it deliberately.
- **Offline first use:** the toolchain carries a cache of current standard mod versions, so
  `moddil new` and a first build work without a network. The cache is a convenience; the lockfile
  remains authoritative.
- **Editions:** standard mods declare the editions they support, and edition migrations cover them
  like any other mod ([`evolution.md`](evolution.md)).

## The prelude

**Decision:** the prelude is small, and contains no effects:

- types: `Option`, `Result`, `Vec`, `String`, `Box`, `Rc`;
- traits: `Iterator`, `IntoIterator`, `Clone`, `Copy`, `Eq`, `Ord`, `Hash`, `Display`, `Debug`,
  `Default`, `From`, `Into`, `TryFrom`, `Fn`, `FnMut`, `FnOnce`, `Drop`, and `FromArrayLiteral`
  ([Collections](#collections)).

Everything else is imported with `use`. Effects such as `Fs` and `Net` are never ambient: they arrive
from root capabilities ([`effects-and-capabilities.md`: Authority comes only from root capabilities](effects-and-capabilities.md#authority-comes-only-from-root-capabilities)).

## Iteration

**Decision: one external iterator protocol, for every kind of sequence.**

```rust
trait Iterator {
    type Item;
    fn next(&mut self) -> Option<Self::Item>;     // may perform effects, inferred per impl
}
```

- **Effects make streams unnecessary.** An iterator over a socket has
  `next() with suspend, fail(NetError)`. It is used with the same `for` loop and the same adaptors as
  an iterator over a `Vec`, because adaptors are effect-polymorphic
  ([`effects-and-capabilities.md`: Effect polymorphism](effects-and-capabilities.md#effect-polymorphism)):

  ```rust
  for line in conn.lines().filter(|l| !l.is_empty()) {   // each next() may suspend and fail
      handle(line?)?;
  }
  ```

  There is no `Stream`, `AsyncIterator` or `StreamExt`, and no second copy of any adaptor.
- **Iterators are views.** An iterator borrows its collection, which is frozen while the iterator
  lives ([`memory.md`: View types](memory.md#view-types)). Adaptors take a view and return a view.
- **Lending iterators** (items that borrow from the iterator itself, such as windows over a buffer)
  use generic associated types ([`types.md`: Traits](types.md#traits)).
- **Internal iteration stays.** Collections also offer `visit`, which takes a closure
  ([`closures.md`](closures.md)). It is simpler to implement for trees and concurrent maps, and lets
  the collection keep its own locks or traversal stack. `for` and adaptors use `Iterator`; `visit` is
  an alternative, not a replacement.
- **Vectorisation is asserted, not promised** (decided). Release builds vectorise adaptor chains
  over slices when they can, but the language does not guarantee it. Where it matters,
  `#[vectorise]` on a loop or function fails the release build if the backend did not vectorise it,
  and says why (possible aliasing, a call it could not inline, a trap edge). It is a performance
  contract you ask for, like a benchmark's `#[budget]` ([`performance.md`: Benchmarks and budgets](performance.md#benchmarks-and-budgets)).
- **Parallel iteration** is `parallel_map` and friends over structured tasks
  ([`concurrency.md`: Data parallelism: `parallel_map`](concurrency.md#data-parallelism-parallel_map)).

### Generators

**Decision: `gen` blocks and `gen fn` are in v0.**

```rust
fn evens(xs: &[int]) -> impl Iterator<Item = int> {
    gen {
        for &x in xs {
            if x % 2 == 0 { yield x; }
        }
    }
}

gen fn read_records(file: &mut File) -> Record {          // an iterator of Record
    for line in file.lines() {
        yield parse(line?)?;                              // effects flow into next()
    }
}
```

- A generator **compiles to a state machine** that implements `Iterator`. It needs no stack of its own
  and does not use stack switching, so it costs the same on every target, including WASM and
  freestanding ones.
- A generator is a **view** when it borrows (as `evens` borrows `xs`), with the same freezing rules
  as a hand-written iterator.
- Its body may perform **effects**; they become the effects of `next()`. A `?` inside a generator
  fails the `next()` call that reached it.
- A **linear** value held across a `yield` must be consumed on every path, including the one where
  the consumer stops early and drops the generator
  ([`resources.md`: Resources: affine and linear](resources.md#resources-affine-and-linear)); the compiler rejects a generator that could
  abandon one.

### Coroutines

**Decision: generators that receive a value on each resume are in v1.**

```rust
gen fn guard_ai(start: Pos) -> Action resume first: Frame {      // a Coroutine<Action, Frame, ()>
    let mut f = first;
    loop {
        while !f.sees_player() { f = yield Action::Patrol }      // one action per tick
        f = yield Action::Shout;
        for _ in 0..30 { f = yield Action::Chase(f.player_pos()) }
    }
}

// each game tick:
let action = guard.resume(frame);
```

- **Why:** logic that the caller steps from outside: game behaviour across frames, protocol state
  machines fed by an event loop Moddil doesn't control, simulation agents. Async I/O does not need
  coroutines; `suspend` covers it.
- **Type:** `Coroutine<Yield, Resume, Return>`. Only coroutines with `Resume = ()` implement
  `Iterator`, so every generator is a coroutine.
- **The first resume value** is bound by the `resume` parameter (`first` above); each later one is
  the value of the `yield` expression.
- **Same lowering as generators:** a state machine, with no stack of its own; the same linearity and
  effect rules.
- **Snapshots:** a coroutine is `Clone` when all the state it holds is, so rollback netcode can
  snapshot and restore it ([multiplayer game](examples/01-multiplayer-game.md)).

## Collections

Rust's names and shapes, in `core-*` (with an allocator) and `moddil/collections`:

| Type | Notes |
|---|---|
| `Vec<T>`, `Deque<T>` | As Rust's `Vec` and `VecDeque` |
| `HashMap<K, V>`, `HashSet<T>` | Keyed hasher; iteration in seeded hash order ([`values.md`: Map iteration order: seeded hash order](values.md#map-iteration-order-seeded-hash-order)) |
| `TreeMap<K, V>`, `TreeSet<T>` | Sorted; Rust's `BTreeMap` |
| `OrderedMap<K, V>`, `OrderedSet<T>` | Insertion order |
| `BinaryHeap<T>`, `RingBuffer<T>` | |
| `EnumSet<E>` | A bit set over a fieldless enum, Jai's `enum_flags`: bit *i* is the variant with index *i*, stored in the smallest unsigned integer that fits; `#[repr(u8)]`-style layout is guaranteed, so it fits packed structs, register maps and wire formats (decided) |
| Persistent collections | Structural sharing for immutable updates; standard, in `moddil/collections` (decided) |

- **Collection literals are typed by context (decided).** There is no `vec!`. `[a, b, c]` and
  `[x; n]` build whatever type the context expects, if it implements the prelude trait
  `FromArrayLiteral<T>` (`from_array<const N: int>(items: [T; N]) -> Self` and
  `from_repeat(x: T, n: int) -> Self where T: Clone`); with no expected type, the literal is a
  fixed array `[T; N]`. `Vec`, `Deque`, the sets and the maps implement it, a map from
  `[(k, v), …]` with later pairs winning as with `insert`, and user types can too:

  ```rust
  let v: Vec<int> = [1, 2, 3];
  let zeros: Vec<f64> = [0.0; n];                   // n need not be constant for a Vec
  let ages: HashMap<&str, int> = [("ada", 36), ("alan", 41)];
  total(&[a, b, c])                                 // the parameter's type decides
  ```
- **`Map` and `Set` traits** let generic code accept any map (the specs' snippets write
  `&Map<K, V>`).
- **Indexing:** `xs[i]` traps when out of bounds (decided, [`numerics.md`](numerics.md)); `xs.get(i)`
  returns `Option`.
- **Capacity is explicit**: `with_capacity`, `reserve`. What happens when allocation fails depends on
  the profile ([`freestanding.md`: Memory without an operating system](freestanding.md#memory-without-an-operating-system)).
- Shared, mutable collections are sync cells or lock-free structures in `moddil/sync`
  ([`concurrency.md`: Shared mutable state](concurrency.md#shared-mutable-state)).
- **Spare capacity is written, never read (decided).** `vec.spare()` gives a view of the
  uninitialised capacity that tracks how much of it has been written, like Rust's `BorrowedBuf`.
  It can be split, filled element by element, or filled by an I/O read; `commit` extends `len` by
  the written prefix only. Large I/O and frame buffers skip zeroing without `unsafe`, unlike Jai's
  `---`, and safe code never reads uninitialised memory ([`types.md`](types.md)).

## Scratch arenas

**Decided: `moddil/alloc` provides `Scratch`, a bump arena that is reset whole at the end of a
scope**, as Jai's temporary storage and `reset_temporary_storage()`.

```rust
let mut scratch = Scratch::with_capacity(4[MiB]);
loop {
    scratch.scope(|tmp| {                              // tmp: &Arena, valid for this frame only
        let mut visible = Vec::new_in(tmp);            // grows inside the arena
        tick(&mut world, &mut visible);
        render(&visible);
    });                                                // O(1) reset; nothing is freed one by one
}
```

- Allocations are **views tied to the scope** ([`memory.md`: Sharing: `.to_rc()`](memory.md#sharing-to_rc)), so the
  borrow checker rejects anything that would outlive the frame. There is no use-after-reset, and
  nothing in it can become an `Rc`.
- **Running out of room** grows the arena by chaining another block in hosted profiles. In
  freestanding profiles it fails with `OutOfMemory`
  ([`freestanding.md`: Memory without an operating system](freestanding.md#memory-without-an-operating-system)).
- `scope` takes `&mut self`, so scopes cannot overlap on one `Scratch`. Each task that needs one
  has its own.
- Debug builds poison reset memory and report the high-water mark per scope, so the capacity
  can be tuned.
- `Vec::new_in(tmp)` is explicit until [`memory.md`: Open questions](memory.md#open-questions)
  decides whether an arena can also be put in scope.

## I/O

**Decision: one set of I/O traits, with borrowed buffers.**

```rust
trait Read {
    fn read(&mut self, buf: &mut [u8]) -> int with suspend, fail(IoError);   // bytes read; 0 at end
}
trait Write {
    fn write(&mut self, buf: &[u8]) -> int with suspend, fail(IoError);
    fn flush(&mut self) with suspend, fail(IoError);
}
```

`BufRead`, `Seek` and the convenience methods (`read_exact`, `read_to_end`, `lines`, `write_all`)
follow Rust's `std::io`.

- **No sync/async split.** Suspension is an effect, so a file, a socket, a pipe and an in-memory
  buffer all implement the same traits. Code that reads from a `&mut impl Read` works whether the
  handler blocks a thread, uses epoll, or uses io_uring.
- **Borrowed buffers are safe with completion I/O.** With io_uring or IOCP, the kernel writes into
  the buffer after the call returns to the runtime. In Rust, a future can be dropped at that moment,
  freeing the buffer under the kernel, which is why Rust's io_uring libraries take owned buffers.
  In Moddil:
  - a task cannot be dropped while suspended; cancellation is cooperative and runs the task to its
    suspension point ([`lifecycle.md`: Cancellation and shutdown](lifecycle.md#cancellation-and-shutdown));
  - the runtime cancels the operation at the OS level and **waits for the kernel to release the
    buffer** before the `read` call returns ([`lifecycle.md`: Cancelling I/O: no "press Enter", no self-connect](lifecycle.md#cancelling-io-no-press-enter-no-self-connect));
  - the borrow on `buf` therefore ends only after the kernel is done with it.
- **Closing can fail, so it is explicit.** Files open for writing and buffered writers are linear:
  they must be `close`d or `abort`ed, and a flush error cannot be lost in a destructor
  ([`resources.md`: Resources: affine and linear](resources.md#resources-affine-and-linear)).
- **Graceful or abrupt close, chosen per type (decided).** Drop never suspends, and there is no
  async drop ([`resources.md`: Construction may suspend; drop may not](resources.md#construction-may-suspend-drop-may-not)):
  - if skipping the graceful close would **lose data** (a buffered file writer, a transaction), the
    type is **linear** and must be `close`d, `commit`ted or `abort`ed;
  - if it would only **lose politeness** (TLS's `close_notify`, a WebSocket close frame), the type
    is **affine**: `close()` is graceful and may suspend, and a drop closes abruptly (a TCP reset)
    without suspending.

  ```rust
  let w = fs.create(path)?;            // linear: must be closed or aborted
  w.write_all(data)?;
  w.close()?;

  let tls = tls::connect(conn)?;       // affine
  tls.write_all(request)?;
  tls.close()?;                        // graceful; dropping it instead closes abruptly
  ```

  Each type's documentation says which it is.
- **Zero-copy reads** hand out `Bytes` and `RcStr` slices of the runtime's buffers
  ([`memory.md`: Zero-copy I/O](memory.md#zero-copy-io), [`text.md`](text.md)).
- **Where I/O comes from:** `Fs` and `Net` are effects. Handlers live in the runtime mods (`moddil/runtime`
  for epoll, kqueue, io_uring and IOCP; `-wasm` for WASI and the browser; `-sim` for simulation), so
  the same code runs in production, in the browser and in deterministic tests
  ([`determinism.md`](determinism.md)).

## HTTP

**Decision: a router with compile-time-checked paths and typed parameters**, in `moddil/net::http`.

```rust
fn create(req: &Request) -> Response with links: Links, Rand, suspend, fail(LinkError | DecodeError) { … }
fn follow(code: Code) -> Response with links: Links, suspend { … }

impl IntoResponse for LinkError { … }      // Taken => 409
impl IntoResponse for DecodeError { … }    // 400

let app = Router::new()
    .post("/links", create)
    .get("/{code}", follow);

http::serve(listener, app)?;
```

- **Paths are checked at compile time.** Route strings are tagged literals
  ([`comptime.md`: Tagged string literals (embedded DSLs)](comptime.md#tagged-string-literals-embedded-dsls)):
  a malformed path, a duplicate route, or a `{param}` with no matching handler parameter is a build
  error.
- **Path parameters are typed.** `{code}` is converted with `TryFrom<&str>` to the handler's
  parameter type, so `follow` receives a validated `Code`; a value that does not convert answers
  `404` without reaching the handler.
- **Handlers are plain functions** taking the request and the path parameters and returning a
  `Response`. Their effects are inferred and become the router's effects; tests call them directly.
- **Every error has a response (R3).** A handler may `fail(E)` only if every variant of `E`
  implements `IntoResponse`. The bound is checked when the route is added, so nothing escapes the
  router unhandled ([`error-handling.md`: Entry points must handle everything (R3)](error-handling.md#entry-points-must-handle-everything-r3)).
- **Serving.** `http::serve(listener, app)` is the accept loop: one isolated scope per connection, and
  it returns when `stop_requested()` fires ([`lifecycle.md`: Two levels: stop, then cancel](lifecycle.md#two-levels-stop-then-cancel)).
  `http::serve_connection(conn, &app)` serves one connection for a hand-written loop.
- **Typed extractors (decided).** Besides path parameters, a handler's parameters can be
  extractors: `Query<T>`, `Header<"name", T>` and `Json<T>`. The router decodes and validates each
  one, answering `400` (or `404` for a path parameter) before the handler runs.
- **Middleware is a plain function (decided):** `fn(req: &Request, next: Next) -> Response`, attached
  with `router.wrap(…)`. Middleware can **install handlers** for everything inside it, so a handler
  asks for the current user as an effect instead of digging through request extensions:

  ```rust
  fn auth(req: &Request, next: Next) -> Response with Sessions, suspend {
      match sessions.user_for(req.header("authorization")) {
          Some(user) => with CurrentUser = user { next(req) },
          None       => Response::status(Status::Unauthorized),
      }
  }

  fn orders(q: Query<Page>) -> Response with user: CurrentUser, Db, suspend { … }

  let app = Router::new().get("/orders", orders).wrap(auth);
  ```

- **Streaming bodies (decided).** `req.body()` buffers up to a configured limit and fails with a typed
  `TooLarge` beyond it; `req.body_stream()` is a byte stream whose reads suspend. A response streams
  from a generator, `Response::stream(gen { for row in rows { yield row.to_csv() } })`. Backpressure
  comes from suspension.
- **Testing without a socket.** `app.handle(&Request::get("/x"))` runs the router in-process.
- **Client.** `http::get(url)` and `http::post(url, body: …, retries: 3, timeout: 5[s])` use `Net`;
  `http::Client` holds configuration such as pooling and default headers.

## Conversion and core traits

| Trait | Notes |
|---|---|
| `From`, `Into`, `TryFrom`, `TryInto` | As in Rust; `TryFrom` fails through `fail(E)` ([`error-handling.md`](error-handling.md)) |
| `Borrow` | Kept for keyed lookups: `map.get("k")` on a `Map<String, V>` |
| ~~`AsRef`~~ | Not in Moddil (decided): functions take `&str` or `&[T]`, and callers convert with unsizing or `.as_str()` |
| `Default` | As in Rust |
| `Display`, `Debug` | [`text.md`: Formatting](text.md#formatting) |
| `Clone`, `Copy`, `Eq`, `Ord`, `Hash` | [`values.md`](values.md) |
| `Drop` | Cannot suspend or fail ([`resources.md`: Construction may suspend; drop may not](resources.md#construction-may-suspend-drop-may-not)) |
| `Fn`, `FnMut`, `FnOnce` | [`closures.md`](closures.md) |

## Fit with the reference examples

| Example | What it needs from the standard library |
|---|---|
| [Multiplayer game](examples/01-multiplayer-game.md) | Collections and ring buffers in the frame loop, `net` (UDP, WebTransport), `serde` binary formats, the browser runtime |
| [HFT platform](examples/02-hft-platform.md) | Zero-copy reads, io_uring with borrowed buffers, lock-free queues, `decimal`, `timeseries` |
| [Web store](examples/03-web-store.md) | HTTP, `sql`, `serde` JSON, `i18n`, `crypto` for sessions; streams of rows as ordinary iterators |
| [Hashiverse](examples/04-hashiverse.md) | `crypto`, `net`, `storage`, simulation handlers; generators for protocol state machines |

## Open questions

- **Opt-in runtime type information.** `reflect(T)` exists only at compile time
  ([`comptime.md`: Static reflection](comptime.md#static-reflection)). In-game editors, inspectors and
  generic debug UIs need a runtime description of a value's fields; Jai gives every type a
  `Type_Info`. A `#[derive(Reflect)]` could emit a static `TypeInfo` and typed field accessors, paid
  for only by the types that opt in. Still open:
  - whether it is a standard mod or a community one;
  - how it respects field visibility;
  - whether an editor may mutate fields through it, or only read them.


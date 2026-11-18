# Standard library shape

> **Status:** decided direction, details open. Listed in the [index](README.md). The catalogue of
> standard mods is in [`mods.md`: Initial standard-mod catalogue](mods.md#initial-standard-mod-catalogue); this document defines the shape of the
> core interfaces they share. Snippets are **illustration only**.

## Summary

| Topic | Decision |
|---|---|
| **Batteries** | **Broad, as separate mods.** Coverage at Go's level (networking, HTTP, crypto, serialisation, SQL, time, testing), delivered as many independently versioned `moddil/*` mods. Only the small `moddil/core-*` set is tied to the compiler ([Batteries and tiers](#batteries-and-tiers)). |
| **Distribution** | `core-*` ships with the compiler and changes only with editions. Every other `moddil/*` mod is fetched, hash-pinned and upgraded like any dependency ([Distribution and versioning](#distribution-and-versioning)). |
| **Iteration** | **One external `Iterator` protocol.** `next` may carry effects, so an iterator that suspends is also an async stream: there is no separate `Stream` trait ([Iteration](#iteration)). |
| **Generators** | **`gen` blocks and `gen fn` in v0**, compiled to state-machine iterators ([Generators](#generators)). |
| **Collections** | Rust's names and shapes; `Map` and `Set` traits for generic code; seeded hash order ([Collections](#collections)). |
| **I/O** | One set of `Read`/`Write` traits for every kind of I/O, with **borrowed buffers**, safe on completion-based kernels because cancellation is structured ([I/O](#io)). |

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

## Batteries and tiers

**Decision: broad batteries, delivered as separate mods.**

| Tier | Contents | Tied to |
|---|---|---|
| **`moddil/core-*`** | Primitives, `Option`/`Result`, slices, `str`, `char`, `Iterator`, formatting traits, conversion traits; with an allocator: `Vec`, `String`, `Box`, `Rc` and the core collections | The compiler and the edition; usable in the freestanding profiles ([`freestanding.md`: Profiles](freestanding.md#profiles)) |
| **`moddil/*`** | Everything else in the [catalogue](mods.md#initial-standard-mod-catalogue): `io`, `fs`, `net`, `time`, `text`, `i18n`, `serde`, `sql`, `crypto`, `sync`, `test`, … | Nothing: each mod has its own version |
| **Community** | Anything else | Nothing |

A mod belongs in `moddil/*` when at least one of these holds:

1. **Most programs need it**, or a [reference example](examples/README.md) does.
2. **It is security-sensitive** (crypto, TLS, parsing untrusted input), where one audited version is
   worth more than choice.
3. **It is an interoperability contract** that other mods must agree on (`serde`'s traits, the
   `log` and `span` effects, HTTP types).

Unused mods cost nothing: they are not fetched, compiled or linked. Standard mods can be deprecated and
moved out without a compiler release ([`mods.md`: Tiers of mods](mods.md#tiers-of-mods)).

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

The prelude is small, and contains no effects:

- types: `Option`, `Result`, `Vec`, `String`, `Box`, `Rc`;
- traits: `Iterator`, `IntoIterator`, `Clone`, `Copy`, `Eq`, `Ord`, `Hash`, `Display`, `Debug`,
  `Default`, `From`, `Into`, `TryFrom`, `Fn`, `FnMut`, `FnOnce`, `Drop`.

Everything else is imported with `use`. Effects such as `fs` and `net` are never ambient: they arrive
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
  lives ([`memory-and-resources.md`: View types](memory-and-resources.md#view-types)). Adaptors take a view and return a view.
- **Lending iterators** (items that borrow from the iterator itself, such as windows over a buffer)
  use generic associated types ([`types.md`: Traits](types.md#traits)).
- **Internal iteration stays.** Collections also offer `visit`, which takes a closure
  ([`closures.md`](closures.md)). It is simpler to implement for trees and concurrent maps, and lets
  the collection keep its own locks or traversal stack. `for` and adaptors use `Iterator`; `visit` is
  an alternative, not a replacement.
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
  ([`memory-and-resources.md`: Resources: affine and linear](memory-and-resources.md#resources-affine-and-linear)); the compiler rejects a generator that could
  abandon one.

## Collections

Rust's names and shapes, in `core-*` (with an allocator) and `moddil/collections`:

| Type | Notes |
|---|---|
| `Vec<T>`, `Deque<T>` | As Rust's `Vec` and `VecDeque` |
| `HashMap<K, V>`, `HashSet<T>` | Keyed hasher; iteration in seeded hash order ([`values.md`: Map iteration order: seeded hash order](values.md#map-iteration-order-seeded-hash-order)) |
| `TreeMap<K, V>`, `TreeSet<T>` | Sorted; Rust's `BTreeMap` |
| `OrderedMap<K, V>`, `OrderedSet<T>` | Insertion order |
| `BinaryHeap<T>`, `RingBuffer<T>` | |
| Persistent collections | Structural sharing for immutable updates, in `moddil/collections` |

- **`Map` and `Set` traits** let generic code accept any map (the specs' snippets write
  `&Map<K, V>`).
- **Indexing:** `xs[i]` traps when out of bounds (decided, [`numerics.md`](numerics.md)); `xs.get(i)`
  returns `Option`.
- **Capacity is explicit**: `with_capacity`, `reserve`. What happens when allocation fails depends on
  the profile ([`freestanding.md`: Memory without an operating system](freestanding.md#memory-without-an-operating-system)).
- Shared, mutable collections are sync cells or lock-free structures in `moddil/sync`
  ([`concurrency.md`: Shared mutable state](concurrency.md#shared-mutable-state)).

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
    suspension point ([`concurrency.md`: Cancellation and shutdown](concurrency.md#cancellation-and-shutdown));
  - the runtime cancels the operation at the OS level and **waits for the kernel to release the
    buffer** before the `read` call returns ([`concurrency.md`: Cancelling I/O: no "press Enter", no self-connect](concurrency.md#cancelling-io-no-press-enter-no-self-connect));
  - the borrow on `buf` therefore ends only after the kernel is done with it.
- **Closing can fail, so it is explicit.** Files open for writing and buffered writers are linear:
  they must be `close`d or `abort`ed, and a flush error cannot be lost in a destructor
  ([`memory-and-resources.md`: Resources: affine and linear](memory-and-resources.md#resources-affine-and-linear)).
- **Zero-copy reads** hand out `Bytes` and `RcStr` slices of the runtime's buffers
  ([`memory-and-resources.md`: Zero-copy I/O](memory-and-resources.md#zero-copy-io), [`text.md`](text.md)).
- **Where I/O comes from:** `fs` and `net` are effects. Handlers live in the runtime mods (`moddil/runtime`
  for epoll, kqueue, io_uring and IOCP; `-wasm` for WASI and the browser; `-sim` for simulation), so
  the same code runs in production, in the browser and in deterministic tests
  ([`determinism.md`](determinism.md)).

## Conversion and core traits

| Trait | Notes |
|---|---|
| `From`, `Into`, `TryFrom`, `TryInto` | As in Rust; `TryFrom` fails through `fail(E)` ([`error-handling.md`](error-handling.md)) |
| `Default` | As in Rust |
| `Display`, `Debug` | [`text.md`: Formatting](text.md#formatting) |
| `Clone`, `Copy`, `Eq`, `Ord`, `Hash` | [`values.md`](values.md) |
| `Drop` | Cannot suspend or fail ([`concurrency.md`: Construction may suspend; drop may not](concurrency.md#construction-may-suspend-drop-may-not)) |
| `Fn`, `FnMut`, `FnOnce` | [`closures.md`](closures.md) |

## Fit with the reference examples

| Example | What it needs from the standard library |
|---|---|
| [Multiplayer game](examples/01-multiplayer-game.md) | Collections and ring buffers in the frame loop, `net` (UDP, WebTransport), `serde` binary formats, the browser runtime |
| [HFT platform](examples/02-hft-platform.md) | Zero-copy reads, io_uring with borrowed buffers, lock-free queues, `decimal`, `timeseries` |
| [Web store](examples/03-web-store.md) | HTTP, `sql`, `serde` JSON, `i18n`, `crypto` for sessions; streams of rows as ordinary iterators |
| [Hashiverse](examples/04-hashiverse.md) | `crypto`, `net`, `storage`, simulation handlers; generators for protocol state machines |

## Open questions

1. **The exact prelude:** should `Box`, `Rc` and the conversion traits be in it, or imported?
2. **`AsRef` and `Borrow`:** are they still needed once unsizing coercions and the `Map` trait exist
   ([`types.md`: No subtyping](types.md#no-subtyping))?
3. **I/O handles and async drop:** is linear `close` enough for every resource whose shutdown must
   suspend ([`concurrency.md`: Open questions](concurrency.md#open-questions))?
4. **Coroutines:** generators that receive a value on each resume (`let x = yield y`), later?
5. **Persistent collections:** standard or community?
6. **Iterator fusion guarantees:** should the compiler promise that adaptor chains over slices
   vectorise in release builds, as a documented performance contract?

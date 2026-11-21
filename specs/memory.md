# Memory: ownership, borrowing and sharing

> **Status:** complete: decided, with no open questions. Indexed in [`README.md`](README.md) and is the
> foundation for CPU concurrency in [`concurrency.md`](concurrency.md). Destruction, cleanup and linear
> types are in [`resources.md`](resources.md). Snippets are **illustration
> only**; no syntax is committed.

## Summary

| Rule | One line |
|---|---|
| **Unique ownership by default** | Every value has one owner; destruction is deterministic (RAII) and cannot be bypassed. |
| **Move or borrow** | Values are moved, or lent as a shared (`&`) or exclusive (`&mut`) borrow. A shared borrow freezes the source. |
| **Borrows are second-class** | Borrows live in parameters, locals and *view types*. They are never stored in owned data. **No lifetime parameters exist.** |
| **`.to_rc()` to share** | The only way to hold more than one pointer to a value is to convert it to a reference-counted handle. The cost is visible at that point. |
| **rc is immutable inside** | Shared mutation only through thread-safe *sync cells* (`Mutex`, `Atomic<T>`, …). |
| **Biased RC** | rc is always thread-safe, and nearly free when it stays on one thread. |
| **Affine + linear resources** | Ordinary values auto-drop; *linear* values must be explicitly consumed ([`resources.md`](resources.md)). |
| **No special constructors** | A value exists only once a literal has supplied every field. "Constructors" are ordinary functions, which may suspend or fail. |
| **Destructors are rare and infallible** | Drop glue is generated; a hand-written `fn drop(self)` lives with its type and cannot fail or suspend. `defer` follows the same rules; `errdefer` may fail, and its error is suppressed ([`resources.md`](resources.md)). |

Together these give memory safety and data-race freedom without a GC, without `Send`/`Sync`
traits and without lifetime annotations ([`concurrency.md`: No `Send` / `Sync`](concurrency.md#no-send--sync)).

## Ownership and RAII

- Every value has exactly one owner. When the owner goes out of scope the value is dropped,
  in reverse declaration order. Drops also run on early return, error propagation and task
  cancellation.
- This applies to **every resource**: memory, file handles, sockets, locks, GPU buffers, NIC ring
  slots. There is no raw `new`/`free` outside `unsafe`.
- Moving transfers ownership; the source can no longer be used (checked at compile time).
- **There are no special constructors.**
  - A value comes into existence only through a literal that supplies every field, e.g.
    `Conn { socket, session }`. This follows from definite initialisation ([`types.md`: Optional values and invalid states](types.md#optional-values-and-invalid-states)).
  - The literal is pure: it cannot suspend or fail.
  - With private fields, only the type's own module can write the literal, so its creator
    functions enforce the invariants.
  - Creator functions (`connect`, `open`, `new`) are ordinary functions. They may suspend, fail and
    acquire resources.
  - There is no `this` before the literal, so a partially constructed object can never be observed.
  - If creation fails or is cancelled midway, the resources acquired so far are owned locals and
    are dropped. Linear ones must be consumed on that path ([Resources: affine and linear](resources.md#resources-affine-and-linear)).
  - The asymmetry is deliberate: creation may suspend, drop may not. Async teardown is a linear
    type's explicit `close` or `commit` ([Resources: affine and linear](resources.md#resources-affine-and-linear), [`resources.md`: Construction may suspend; drop may not](resources.md#construction-may-suspend-drop-may-not)).
- **Drop must not suspend and must not fail.** The compiler checks this: a drop implementation has
  an empty effect set. Finalisation that can fail or needs a decision is a linear type ([Resources: affine and linear](resources.md#resources-affine-and-linear), [`resources.md`: Cleanup that can fail](resources.md#cleanup-that-can-fail)).
- Unique ownership covers more than people expect: singly linked lists, trees, ASTs with owned
  children and builder patterns need no rc at all.

## Borrowing

### Parameter modes

Parameter modes use Rust's spelling (decided, [`syntax.md`](syntax.md)); borrows are second-class,
so no lifetimes appear.

| Parameter | Meaning | Caller's value during the call |
|---|---|---|
| `x: &T` | Shared, read-only borrow | Readable, frozen |
| `x: &mut T` | Exclusive borrow | Inaccessible |
| `x: T` | Move (ownership transferred) | Gone |

Because a shared borrow's address cannot be observed or stored, the compiler may pass it **by value or
by pointer**, whichever is cheaper (as Swift and Hylo do). Small types travel in registers.

### Rules

- Any number of shared borrows, or exactly one `&mut` borrow, at any time (aliasing XOR mutation).
- A source cannot be mutated, moved or dropped while a borrow of it is live.
- Checking is **intraprocedural** and flow-sensitive (NLL-style): a borrow ends at its last use.
  Function signatures carry everything a caller needs, so checking stays local and incremental
  ([Build times](build-times.md)).
- **Dereferencing follows Rust (decided):** field access looks through a borrow (`r.x`);
  operators, comparisons and writes through a borrow need `*` (`*r + 1`, `*a == *b`, `*r = v`).
  Using a `&mut` borrow lends it again (an implicit reborrow) instead of using it up.
- **Non-escaping closures** may capture borrows (Swift's model). Escaping closures capture by move
  or rc. Details in [`closures.md`](closures.md).
- **Diagnostics built for agents** (decided): every borrow error has one root cause and a ranked
  menu of machine-applicable fixes (clone, `.to_rc()`, a view, splitting the borrow by field, a
  move), each with a confidence level and a short cost note. The fix success rate is measured with
  the agent simplicity benchmarks ([`adoption.md`: Simpler than Rust](adoption.md#simpler-than-rust)).

### Returning borrows

A function may return a borrow or a view ([View types](#view-types)) derived from its borrowed parameters:

- **Methods**: the result derives from `self`.
- **Free functions**: the result derives from *all* borrowed parameters (conservative default).
- **Optional** precision: `-> T from a` (or `from a, b`) names a *parameter*, never an abstract
  region. This spelling is decided.

```rust
fn get(&self, key: &K) -> Option<&V>                        // derives from self
fn longer(a: &str, b: &str) -> &str                         // derives from a and b
fn first_word(text: &str, sep: &str) -> &str from text
```

### Borrows of static data

**Decided: a borrow of static data borrows nothing.** Static data is the text of a string literal
(and of a byte-string literal) and the value of a `const` item, which comptime copies into
read-only memory ([`comptime.md`: Evaluation model](comptime.md#evaluation-model)). A literal is
in effect a borrow of an anonymous `const`. A borrow's rules protect its source, which must not be
mutated, moved or dropped while the borrow lives; static data is immutable, never moves and lives
as long as the program, so there is nothing to protect.

- **It is an ordinary `&T`.** `"hi"` is a `&str` ([`text.md`: String types](text.md#string-types)),
  accepted wherever a `&str` is, but it freezes no variable and is never ended by the checker.
- **A function with no borrowed parameter may return a borrow**, which must then be of static
  data:

  ```rust
  fn greeting() -> &str { "hi" }                              // static: fine
  fn table() -> &[u32; 256] { &CRC_TABLE }                    // a const: fine
  fn local() -> &int { let x = 1; &x }                        // a local: an error
  ```

  Its callers get a result that borrows nothing, and keep it as long as they like.
- **With borrowed parameters,** a function may also return static data
  (`if found { name } else { "anonymous" }`); its callers still treat the result as borrowing what
  they passed.
- **Only literals and `const`s are static.** `&(1 + 2)` and `&Point { x: 0, y: 0 }` borrow
  temporaries: there is no implicit promotion as in Rust, so what is static stays one rule, and a
  `const` makes anything else static explicitly.
- **Still second-class.** There is no `'static` and no new type, so a `&str` is still not stored in
  owned data: a struct with a `&str` field is a [view type](#view-types). Text a struct owns is a
  `String`, inline when short ([`text.md`: Short strings are stored inline](text.md#short-strings-are-stored-inline)).

## View types

A **view type** is a struct or container whose fields hold borrows, declared with
`view Name<T> { … }` (decided). Each view has **exactly one implicit region**, always inferred. It
is second-class, like a borrow:

- It can be created, kept in locals, passed *down* the call stack, nested in other views, returned
  under the [Returning borrows](#returning-borrows) rules, and lent to structured child tasks ([`concurrency.md`: Tasks and borrowing](concurrency.md#tasks-and-borrowing)).
- It can never be stored in owned data, captured by an escaping closure, or sent to detached work.
- **Viewness is infectious**: a generic container instantiated with a borrow (`Vec<&Node>`,
  `Map<Name, &Node>`) is itself a view type.

The three common patterns:

```rust
// Context objects: sources may come from several frames; the view only flows down.
view AppContext { config: &Config, db: &DbPool, session: &UserSession }

// Custom iterators: the collection is frozen while the iterator is live.
view RingIter<T> { buf: &[T], idx: int }
fn iter(&self) -> RingIter<T>
for x in ring.iter().filter(f).map(g) { … }     // adaptors: view in, view out
ring.push(1);                                   // ✗ while the iterator is live

// Arena-backed graphs: append-only arena allocates through a shared borrow.
fn alloc(&self, n: Node) -> &Node
view SymbolTable { map: Map<Name, &Node> }
```

An alternative to context objects that fits Moddil especially well: things like `Config` and
`DbPool` can be **effects/capabilities** provided by a handler higher up ([Effects and capabilities as a unifying idea](effects-and-capabilities.md)), giving
implicit context with no parameter threading, still visible in signatures.

### Known limitations (accepted)

| Not expressible | Workaround | Rust? |
|---|---|---|
| Structs with ≥2 independent lifetimes (`Foo<'a, 'b>`) | Two views, or own one side | Possible, rare |
| Views inside heap data that outlives the borrowing frame (e.g. a long-lived `Service` holding `&Config`) | Own it or `.to_rc()` | Possible, but `<'a>` infects every user |
| Self-referential structs | Owning views ([Zero-copy I/O](#zero-copy-io)) | Not possible either (needs `yoke`/`ouroboros`) |
| Cyclic arena graphs with in-place link updates | `NodeId` indices, or atomic cells ([rc contents are immutable](#rc-contents-are-immutable)) | Needs `Cell` |

Performance in the supported patterns is the same as Rust: plain pointers, no refcounts, no copies.

## Sharing: `.to_rc()`

### Conversion

```
let node = Node { … }          // unique, zero overhead
let shared = node.to_rc()      // moves node into an rc allocation; returns Rc<Node>
let other = shared.share()     // refcount inc; both point at the same node
let back = shared.weak()       // Weak<Node>: does not keep it alive; upgrade() -> Option<Rc<Node>>
```

- `.to_rc()` **consumes** the owner; the value now lives as long as any `Rc` does. It uses the
  allocator in scope; an `Rc` records its allocator only when it is not the default one, so default
  rcs pay nothing (decided). Arenas stay borrow-based: arena allocations are views tied to the
  arena's scope and cannot become `Rc`s
  ([`freestanding.md`: Memory without an operating system](freestanding.md#memory-without-an-operating-system)).
- RC cannot collect cycles. Back-edges (parent pointers, doubly-linked `prev`) use `Weak`, which is
  **core**, beside `Rc`, because the header layout and biased counting must know about weak counts.
- **There is no garbage collector** (decided). Cycles are avoided by design: `Weak` for back-edges,
  arenas and indices for graphs. Debug builds and the simulator run a cycle detector that reports
  leaked rc cycles with their allocation sites. A collected `Gc<T>` arena could be a library if a
  real need appears.
- Passing an `Rc` as a shared borrow (`&Rc<T>`) does **not** touch the count. Only `.share()` does ([`values.md`: `.share()` for reference counts](values.md#share-for-reference-counts)).
- Compile-time RC elision (Perceus-style) removes inc/dec pairs the compiler can prove redundant.

### rc contents are immutable

Shared data is read-only. The only way to mutate through an rc is a **sync cell**:

| Cell | Use | Cost |
|---|---|---|
| `Atomic<T>` (small `T`) | Counters, flags, indices | Relaxed load/store is a plain `mov` on x86 |
| `AtomicRc<T>` | Swap a pointer (e.g. a link in a shared graph, a config snapshot) | One atomic op + RC |
| `Mutex<T>`, `RwLock<T>` | Arbitrary mutation | Lock acquire/release |

Atomics are core intrinsics; the cell types live in `moddil/sync`. There is deliberately **no
`RefCell` and no copy-on-write `make_mut`**: every shared mutation is visibly synchronised.

Consequence: a mutable graph used by one thread pays (cheap, uncontended) atomic costs on its link
fields. The idiomatic answers are arenas + indices, or a `moddil/collections` type that implements
the structure internally with `unsafe` and exposes a safe API.

### Biased reference counting

Every `Rc` is safe to send between threads, but cross-thread traffic is the rare case. Moddil uses
**biased reference counting** (Choi, Shull, Torrellas, PACT 2018; also used by CPython 3.13's
free-threaded build):

- The header holds an **owner thread id**, a **non-atomic owner count** and an **atomic shared
  count** (~16 bytes vs ~8 for a plain atomic count).
- On the owner thread: compare thread id, then plain inc/dec (~1 cycle, predictable branch).
- On other threads: atomic inc/dec of the shared count.
- When the owner count reaches zero the counts merge. If another thread drops the last shared
  reference while the owner still holds some, the object is queued to the owner, which frees it
  the next time its runtime worker polls its queue.

Notes:
- The paper reports roughly a 20% speed-up over always-atomic RC on Swift benchmarks.
- Biased RC does **not** help many threads cloning the same rc concurrently (cache-line contention
  is unchanged). Moddil mitigates this differently: child tasks *borrow* an rc without touching its
  count ([`concurrency.md`: Tasks and borrowing](concurrency.md#tasks-and-borrowing)), removing most of the clones Rust needs for
  `'static` spawns.
- The merge-queue poll is part of the runtime-handler contract ([R2](requirements.md#firm-requirements-decided),
  [`runtime.md`: Runtime contract](runtime.md#runtime-contract)). On single-threaded WASM every count is the owner count.

**Decided: bias to the owner task, not the owner thread.** Tasks migrate between worker threads
([`runtime.md`: Tasks, threads and executors](runtime.md#tasks-threads-and-executors)), and with a thread id in the header a migrated task would
lose the fast path. Instead:

1. **The header stores the owner *task* id.** Only the owning task touches the non-atomic owner
   count. A task runs on one thread at a time, and every migration passes through the scheduler's
   queues (release/acquire), so the owner's successive accesses stay ordered even across threads.
   Non-atomic is still correct, and migration costs nothing. The check compares against the current
   task id, which the runtime keeps in a register or thread-local slot: as cheap as a thread-id
   check. Non-owners only ever touch the atomic shared count, so the merge protocol is unchanged.
2. **Merge queue per task.** An object whose last shared reference is dropped by a non-owner while
   the owner still holds some is queued to the owner *task*. The runtime drains the queue when it
   resumes the task, and always at task exit (when the owner count must reach zero anyway).
3. **Rebias on unique transfer.** When a uniquely held rc (owner count 1, shared count 0) moves to
   another task (a channel send, a move into a child), ownership passes to the receiver for free, so
   pipelines handing objects from stage to stage stay on the fast path.
4. **Fewer count operations in the first place:** children borrow rcs without touching counts;
   compile-time RC elision (Perceus-style) removes inc/dec pairs; rc contents are immutable, so the
   slow path is only an atomic increment or decrement, never a lock.
5. **Measured:** the runtime counts slow-path rc operations per task and call site, and the profiler
   reports them.

The remaining slow case is an rc used heavily by many tasks at once (cache-line contention), which
biasing never addressed; borrowing does.
- An rc's destructor runs on **whichever thread drops the last reference**. Destructors must not
  rely on thread identity. This is one reason linear values cannot be rc'd ([Resources: affine and linear](resources.md#resources-affine-and-linear)).

## Zero-copy I/O

Goal: data arrives from a NIC (or disk, or GPU) and is consumed without ever being copied.

1. **The buffer is an owned resource.** The NIC DMA-writes into pool memory (io_uring provided
   buffers, AF_XDP UMEM, DPDK mbufs). The runtime/net mod hands out a `Packet`: a unique value whose
   type carries its memory space (e.g. `Buffer<@dma(nic0)>`, [Concurrency across GPUs and accelerators](accelerators.md)). Dropping it returns
   the slot to the ring. Moving it moves a pointer and a length.
2. **Validate once, access in place.** `moddil/serde` uses compile-time reflection to generate
   accessor views over the bytes (FlatBuffers / Cap'n Proto / SBE style). Validation happens once at
   the boundary and produces a validated type, which also implements the untrusted-data rule of
   [Distributed and multi-tier programs](distribution.md).
3. **Choose a carrier:**

| Case | Carrier | Cost |
|---|---|---|
| Consumed in the same call chain | **View type** over the packet ([View types](#view-types)) | Zero |
| Handed to another task / thread / queue | **Owning view** `Parsed<T>` = the `Packet` + an offset table; accessors project borrows on demand. Moved, never copied. Safe because it stores offsets, not pointers into itself. | One add per field access |
| Fan-out to several consumers | `.to_rc()` the packet; each consumer gets an **rc slice** (rc + range). Immutable, so race-free. | One biased-RC inc/dec per consumer |

```rust
fn on_rx(pkt: Packet) with Net, fail(DecodeError) {
    let order = wire::Order::validate(pkt)?; // Parsed<Order>: owns pkt, zero copy
    if order.symbol() == "ESZ6" {            // projection: borrow derived from order
        strategy_queue.push(order);          // move: the buffer travels, the payload is not copied
    }
}                                            // otherwise order drops → slot back to the NIC ring
```

**Ring pressure:** holding a `Packet` for a long time ties up a ring slot. This is resource
pressure, not unsafety. The runtime mod should expose ring occupancy, and offer an explicit
`.detach()` that copies the payload out for long-term storage.

## Cost table

The full cost model, including calls, effects and checks, is in
[`performance.md`: A published cost model](performance.md#a-published-cost-model).

| Construct | Allocation | Per-use cost | Thread-safe to share |
|---|---|---|---|
| Owned value | Wherever its owner lives | None | Move only |
| `&` / `&mut` borrow | None | None (may be passed by value) | Lent to child tasks |
| View type | None | None | Lent to child tasks |
| Owning view `Parsed<T>` | The underlying buffer | One add per field access | Move only |
| `Rc<T>` | Header + value | Biased inc/dec on share/drop; free when borrowed | Yes |
| `Weak<T>` | (shares rc allocation) | Upgrade check | Yes |
| Sync cell | Inline | Atomic op or lock | Yes |

## Fit check against the reference examples

| Example | Where rc appears | Linear resources | Zero-copy views | Gaps |
|---|---|---|---|---|
| [Multiplayer game](examples/multiplayer-game.md) | Immutable world snapshots fanned out to N player connections (rc slices); chat room membership | — | Input packets from WebTransport, decoded in place on `hub` | Entity graphs: use ECS arenas + indices, not rc graphs |
| [HFT](examples/hft-platform.md) | Market data fan-out to multiple strategies (rc slices) | Order lifecycle (sent → acked/rejected) is a natural linear candidate | **Core case**: NIC → decode → book → strategy with owning views; zero copy end to end | Order book is single-threaded mutable: owned, not rc |
| [Web store](examples/web-store.md) | Config / DB pool shared across request tasks (or as effects) | DB transactions must commit or roll back | HTTP bodies parsed into views | None found; `AppContext` view or effects cover handler context |
| [Hashiverse](examples/hashiverse.md) | Shared routing table snapshots (`AtomicRc` swap); key material handles | Key-locker unlock sessions? (to evaluate) | RPC packets validated once (PoW-stamped) and passed as owning views | Kademlia routing tables: indices or `AtomicRc` snapshots, not mutable rc graphs |

No case required first-class references with explicit lifetimes.

## The allocator in scope

**Decided: the allocator in scope is an ambient scoped handler** (decided November 2026).
`.to_rc()` and growing collections ([Conversion](#conversion)) use the allocator that the innermost
`with Alloc = a { … }` installs, as Jai's `push_allocator` and context allocator do, and the
profile's default allocator where none is installed.

```rust
let frame = Arena::new(1[MiB]);
with Alloc = &frame {
    let path = find_path(&map, from, to);   // its `Vec`s grow in `frame`
    steer(&path);
}                                           // `frame` is reset or dropped after the block
```

- **Ambient, not an effect in signatures.** `Alloc` is never written after `with` in a signature,
  and never inferred into one, like the ambient write-only effects
  ([`effects-and-capabilities.md`: The effect catalogue](effects-and-capabilities.md#the-effect-catalogue)).
  So allocation does not spread into every signature: a library that grows a `Vec` uses whatever
  allocator its caller installed.
- **Installed like any handler.** `with Alloc = … { … }` takes a value or a borrow of an allocator,
  and the innermost one wins. Explicit `_in(alloc)` constructors (`Vec::new_in(&tmp)`) remain for a
  value that must use a particular allocator whatever is in scope.
- **Lifetimes are checked by the borrow rules.** A handler borrowed from an arena lives as long as
  the `with` block, and what is allocated in the arena is a view tied to it, so it cannot leave the
  block ([View types](#view-types)); as before, arena allocations cannot become `Rc`s.
- **`#[no_alloc]` is unchanged.** It forbids any call into an allocator, whichever one is in scope
  ([`performance.md`: Hot-path guarantees](performance.md#hot-path-guarantees)); installing an
  arena does not make allocation allowed there.
- Scratch arenas install themselves the same way
  ([`stdlib.md`: Scratch arenas](stdlib.md#scratch-arenas)). It is implemented with `Vec` in slice
  S7 ([`roadmap.md`](roadmap.md#m0-the-v0-language-in-slices)).

## Open questions

None at present.

## Background

The problem and the state of the art that led to this design.

**Problem.** Every approach trades something away:

| Approach                                         | Gives you                        | Costs you                                                                      |
| ------------------------------------------------ | -------------------------------- | ------------------------------------------------------------------------------ |
| Manual (C)                                       | Total control                    | Use-after-free, leaks, double frees: most CVEs                                 |
| Tracing GC (Java, Go, C#)                        | Simplicity, safety               | Pauses, memory overhead, non-deterministic destruction                         |
| RAII (C++)                                       | Deterministic, zero-cost cleanup | Only if followed consistently. Nothing enforces it; dangling references remain |
| Reference counting (Swift, Python, `shared_ptr`) | Deterministic, incremental       | Atomic RC is slow under contention; cycles leak; extra traffic on every copy   |
| Ownership/borrowing (Rust)                       | Safe, zero-cost, deterministic   | Learning curve; some data structures (graphs, doubly-linked lists) are awkward |

**State of the art.**
- **Rust**: the reference point for safe, GC-free memory management.
- **Modern GCs** (ZGC, Shenandoah, Go's concurrent GC) have made pauses sub-millisecond for many
  workloads. "GC = random delays" is less true than it was, but throughput and memory overhead remain.
- **Perceus** (Koka, Lean 4): compile-time reference counting with reuse analysis. Most RC operations
  are removed, and functional code can update in place.
- **Lobster**: compile-time RC elision with ownership inference.
- **Vale**: generational references, so memory-safety checks happen at runtime but cheaply, without a
  borrow checker.
- **Zig/Odin**: explicit allocators passed around, arenas everywhere, with very simple semantics.
- **Pony ORCA**: per-actor GC with no global pauses.

**What still hurts.**
- RAII is brilliant but *opt-in by convention*; nothing stops a raw `new` in C++.
- Most languages only manage **memory**. File handles, sockets, locks, GPU buffers and transactions
  are managed with `defer`, `using`, `with` or finalisers, which are easy to forget.
- Atomic RC makes multithreaded code slower even when objects are never shared.
- Rust lifetimes leak into APIs and make refactoring expensive.

**Directions.**
- **Allocators as a capability**: the allocator is visible (Zig-style), and per-scope arenas are cheap
  and idiomatic.
- Machine-applicable diagnostics for the borrow check (e.g. "insert `.to_rc()` here"), aimed at
  agents ([LLMs as first-class programmers](agent-interface.md)).

## Prior art

Rust (ownership, borrowing, `Arc`/`Weak`, `thread::scope`); Hylo/Val (mutable value semantics,
second-class references, parameter conventions, subscripts); Mojo (argument conventions, origins);
Swift (ARC, non-escaping closures, borrow/consume parameters); Koka/Lean 4 Perceus (RC elision and
reuse); Choi, Shull, Torrellas, *Biased Reference Counting* (PACT 2018) and CPython PEP 703; Austral
and Linear Haskell (linear types); Rust `bytes::Bytes` (rc slices) and `yoke` (owning views);
FlatBuffers, Cap'n Proto, SBE (zero-copy formats); io_uring, AF_XDP, DPDK (zero-copy I/O); Zig
explicit allocators; Odin and Jai context allocators and Jai temporary storage.

# Memory and Resources

> **Status:** decided direction, details open. Expands [`rationale.md`](rationale.md) §2.2 and is the
> foundation for CPU concurrency in [`concurrency.md`](concurrency.md). Snippets are **illustration
> only**; no syntax is committed.

## 1. Summary

| Rule | One line |
|---|---|
| **Unique ownership by default** | Every value has one owner; destruction is deterministic (RAII) and cannot be bypassed. |
| **Move or borrow** | Values are moved, or lent as a shared (`let`) or exclusive (`mut`) borrow. A shared borrow freezes the source. |
| **Borrows are second-class** | Borrows live in parameters, locals and *view types*. They are never stored in owned data. **No lifetime parameters exist.** |
| **`.to_rc()` to share** | The only way to hold more than one pointer to a value is to convert it to a reference-counted handle. The cost is visible at that point. |
| **rc is immutable inside** | Shared mutation only through thread-safe *sync cells* (`Mutex`, `Atomic<T>`, …). |
| **Biased RC** | rc is always thread-safe, and nearly free when it stays on one thread. |
| **Affine + linear resources** | Ordinary values auto-drop; *linear* values must be explicitly consumed. |

Together these give memory safety and data-race freedom without a GC, without `Send`/`Sync`
traits and without lifetime annotations ([`concurrency.md`](concurrency.md) §2).

## 2. Ownership and RAII

- Every value has exactly one owner. When the owner goes out of scope the value is dropped,
  in reverse declaration order. Drops also run on early return, error propagation and task
  cancellation.
- This applies to **every resource**: memory, file handles, sockets, locks, GPU buffers, NIC ring
  slots. There is no raw `new`/`free` outside `unsafe`.
- Moving transfers ownership; the source can no longer be used (checked at compile time).
- **Drop must not suspend and must not fail.** Finalisation that can fail or needs a decision is a
  linear type (§7).
- Unique ownership covers more than people expect: singly linked lists, trees, ASTs with owned
  children and builder patterns need no rc at all.

## 3. Borrowing

### 3.1 Parameter modes

| Mode | Meaning | Caller's value during the call |
|---|---|---|
| `let` | Shared, read-only borrow | Readable, frozen |
| `mut` | Exclusive borrow | Inaccessible |
| `sink` | Move (ownership transferred) | Gone |

Because a `let` borrow's address cannot be observed or stored, the compiler may pass it **by value or
by pointer**, whichever is cheaper (as Swift and Hylo do). Small types travel in registers.

### 3.2 Rules

- Any number of shared borrows, or exactly one `mut` borrow, at any time (aliasing XOR mutation).
- A source cannot be mutated, moved or dropped while a borrow of it is live.
- Checking is **intraprocedural** and flow-sensitive (NLL-style): a borrow ends at its last use.
  Function signatures carry everything a caller needs, so checking stays local and incremental
  (rationale §2.14).
- **Non-escaping closures** may capture borrows (Swift's model). Escaping closures capture by move
  or rc.

### 3.3 Returning borrows

A function may return a borrow or a view (§4) derived from its borrowed parameters:

- **Methods**: the result derives from `self`.
- **Free functions**: the result derives from *all* borrowed parameters (conservative default).
- **Optional** precision: `-> T from a` names a *parameter*, never an abstract region.

```
fn get(self: let Map<K, V>, key: let K) -> Option<&V>      // derives from self
fn longer(a: let str, b: let str) -> &str                   // derives from a and b
fn first_word(text: let str, sep: let str) -> &str from text
```

## 4. View types

A **view type** is a struct or container whose fields hold borrows. Each view has **exactly one
implicit region**, always inferred. It is second-class, like a borrow:

- It can be created, kept in locals, passed *down* the call stack, nested in other views, returned
  under the §3.3 rules, and lent to structured child tasks ([`concurrency.md`](concurrency.md) §3).
- It can never be stored in owned data, captured by an escaping closure, or sent to detached work.
- **Viewness is infectious**: a generic container instantiated with a borrow (`Vec<&Node>`,
  `Map<Name, &Node>`) is itself a view type.

The three common patterns:

```
// Context objects: sources may come from several frames; the view only flows down.
view AppContext { config: &Config, db: &DbPool, session: &UserSession }

// Custom iterators: the collection is frozen while the iterator is live.
view RingIter<T> { buf: &[T], idx: usize }
fn iter(self: let RingBuffer<T>) -> RingIter<T>
for x in ring.iter().filter(f).map(g) { … }     // adaptors: view in, view out
ring.push(1)                                    // ✗ while the iterator is live

// Arena-backed graphs: append-only arena allocates through a shared borrow.
fn alloc(self: let Arena, n: Node) -> &Node
view SymbolTable { map: Map<Name, &Node> }
```

An alternative to context objects that fits Moddil especially well: things like `Config` and
`DbPool` can be **effects/capabilities** provided by a handler higher up (rationale §2.1), giving
implicit context with no parameter threading, still visible in signatures.

### 4.1 Known limitations (accepted)

| Not expressible | Workaround | Rust? |
|---|---|---|
| Structs with ≥2 independent lifetimes (`Foo<'a, 'b>`) | Two views, or own one side | Possible, rare |
| Views inside heap data that outlives the borrowing frame (e.g. a long-lived `Service` holding `&Config`) | Own it or `.to_rc()` | Possible, but `<'a>` infects every user |
| Self-referential structs | Owning views (§6) | Not possible either (needs `yoke`/`ouroboros`) |
| Cyclic arena graphs with in-place link updates | `NodeId` indices, or atomic cells (§5.2) | Needs `Cell` |

Performance in the supported patterns is the same as Rust: plain pointers, no refcounts, no copies.

## 5. Sharing: `.to_rc()`

### 5.1 Conversion

```
let node = Node { … }          // unique, zero overhead
let shared = node.to_rc()      // moves node into an rc allocation; returns Rc<Node>
let other = shared.clone()     // refcount inc; both point at the same node
let back = shared.weak()       // Weak<Node>: does not keep it alive; upgrade() -> Option<Rc<Node>>
```

- `.to_rc()` **consumes** the owner; the value now lives as long as any `Rc` does.
- RC cannot collect cycles. Back-edges (parent pointers, doubly-linked `prev`) use `Weak`.
- Passing an `Rc` as a `let` borrow does **not** touch the count. Only storing a copy does.
- Compile-time RC elision (Perceus-style) removes inc/dec pairs the compiler can prove redundant.

### 5.2 rc contents are immutable

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

### 5.3 Biased reference counting

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
  count ([`concurrency.md`](concurrency.md) §3), removing most of the clones Rust needs for
  `'static` spawns.
- The merge-queue poll is part of the runtime-handler contract (rationale R2,
  [`concurrency.md`](concurrency.md) §6). On single-threaded WASM every count is the owner count.
- An rc's destructor runs on **whichever thread drops the last reference**. Destructors must not
  rely on thread identity. This is one reason linear values cannot be rc'd (§7).

## 6. Zero-copy I/O

Goal: data arrives from a NIC (or disk, or GPU) and is consumed without ever being copied.

1. **The buffer is an owned resource.** The NIC DMA-writes into pool memory (io_uring provided
   buffers, AF_XDP UMEM, DPDK mbufs). The runtime/net mod hands out a `Packet`: a unique value whose
   type carries its memory space (e.g. `Buffer<@dma(nic0)>`, rationale §2.9). Dropping it returns
   the slot to the ring. Moving it moves a pointer and a length.
2. **Validate once, access in place.** `moddil/serde` uses compile-time reflection to generate
   accessor views over the bytes (FlatBuffers / Cap'n Proto / SBE style). Validation happens once at
   the boundary and produces a validated type, which also implements the untrusted-data rule of
   rationale §2.10.
3. **Choose a carrier:**

| Case | Carrier | Cost |
|---|---|---|
| Consumed in the same call chain | **View type** over the packet (§4) | Zero |
| Handed to another task / thread / queue | **Owning view** `Parsed<T>` = the `Packet` + an offset table; accessors project borrows on demand. Moved, never copied. Safe because it stores offsets, not pointers into itself. | One add per field access |
| Fan-out to several consumers | `.to_rc()` the packet; each consumer gets an **rc slice** (rc + range). Immutable, so race-free. | One biased-RC inc/dec per consumer |

```
fn on_rx(pkt: sink Packet) ! {net} {
    let order = wire::Order.validate(pkt)?   // Parsed<Order>: owns pkt, zero copy
    if order.symbol() == "ESZ6" {            // projection: borrow derived from order
        strategy_queue.push(order)           // move: the buffer travels, the payload is not copied
    }
}                                            // otherwise order drops → slot back to the NIC ring
```

**Ring pressure:** holding a `Packet` for a long time ties up a ring slot. This is resource
pressure, not unsafety. The runtime mod should expose ring occupancy, and offer an explicit
`.detach()` that copies the payload out for long-term storage.

## 7. Resources: affine and linear

- **Affine (default):** a value is used at most once and dropped automatically. Files, sockets,
  buffers, lock guards.
- **Linear (opt-in per type):** a value **must** be consumed by one of its designated operations,
  or the program does not compile. For resources whose finalisation can fail or needs a decision.

```
linear type Txn { … }
fn commit(self: sink Txn) -> Result<(), DbError>
fn rollback(self: sink Txn)

fn checkout(db: let Db) ! {db, fail(DbError)} {
    let txn = db.begin()
    txn.insert(order)?        // ✗ unless the error path also consumes txn
    txn.commit()?
}
```

Rules:
- Linear values **cannot be `.to_rc()`'d**: the last-drop point of an rc is not known statically,
  and it may be on any thread.
- Every control-flow path, including error propagation, must consume a linear value. Error
  propagation (`?`) on a path holding a linear value requires an explicit handler for it
  (exact syntax open; see rationale §2.3).
- Behaviour on **task cancellation** is open ([`concurrency.md`](concurrency.md) §7).

## 8. Cost table

| Construct | Allocation | Per-use cost | Thread-safe to share |
|---|---|---|---|
| Owned value | Wherever its owner lives | None | Move only |
| `let` / `mut` borrow | None | None (may be passed by value) | Lent to child tasks |
| View type | None | None | Lent to child tasks |
| Owning view `Parsed<T>` | The underlying buffer | One add per field access | Move only |
| `Rc<T>` | Header + value | Biased inc/dec on clone/drop; free when borrowed | Yes |
| `Weak<T>` | (shares rc allocation) | Upgrade check | Yes |
| Sync cell | Inline | Atomic op or lock | Yes |

## 9. Fit check against the reference examples

| Example | Where rc appears | Linear resources | Zero-copy views | Gaps |
|---|---|---|---|---|
| [Multiplayer game](examples/01-multiplayer-game.md) | Immutable world snapshots fanned out to N player connections (rc slices); chat room membership | — | Input packets from WebTransport, decoded in place on `@hub` | Entity graphs: use ECS arenas + indices, not rc graphs |
| [HFT](examples/02-hft-platform.md) | Market data fan-out to multiple strategies (rc slices) | Order lifecycle (sent → acked/rejected) is a natural linear candidate | **Core case**: NIC → decode → book → strategy with owning views; zero copy end to end | Order book is single-threaded mutable: owned, not rc |
| [Web store](examples/03-web-store.md) | Config / DB pool shared across request tasks (or as effects) | DB transactions must commit or roll back | HTTP bodies parsed into views | None found; `AppContext` view or effects cover handler context |
| [Hashiverse](examples/04-hashiverse.md) | Shared routing table snapshots (`AtomicRc` swap); key material handles | Key-locker unlock sessions? (to evaluate) | RPC packets validated once (PoW-stamped) and passed as owning views | Kademlia routing tables: indices or `AtomicRc` snapshots, not mutable rc graphs |

No case required first-class references with explicit lifetimes.

## 10. Open questions

- Exact syntax for parameter modes, views and `from`.
- Allocators: how does the allocator capability (rationale §2.2, Zig-style) interact with `.to_rc()`
  and arenas? Does an `Rc` remember its allocator?
- Should `Weak` be core or a mod? (It needs header support, so probably core.)
- Linear values on error paths: what is the least noisy explicit handling?
- Can the borrow check give diagnostics good enough for LLM agents (rationale §2.18) — e.g.
  machine-applicable "insert `.to_rc()` here" fixes?

## Prior art

Rust (ownership, borrowing, `Arc`/`Weak`, `thread::scope`); Hylo/Val (mutable value semantics,
second-class references, parameter conventions, subscripts); Mojo (argument conventions, origins);
Swift (ARC, non-escaping closures, borrow/consume parameters); Koka/Lean 4 Perceus (RC elision and
reuse); Choi, Shull, Torrellas, *Biased Reference Counting* (PACT 2018) and CPython PEP 703; Austral
and Linear Haskell (linear types); Rust `bytes::Bytes` (rc slices) and `yoke` (owning views);
FlatBuffers, Cap'n Proto, SBE (zero-copy formats); io_uring, AF_XDP, DPDK (zero-copy I/O).

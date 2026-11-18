# Memory and Resources

> **Status:** decided direction, details open. Indexed in [`README.md`](README.md) and is the
> foundation for CPU concurrency in [`concurrency.md`](concurrency.md). Snippets are **illustration
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
| **Affine + linear resources** | Ordinary values auto-drop; *linear* values must be explicitly consumed. |
| **No special constructors** | A value exists only once a literal has supplied every field. "Constructors" are ordinary functions, which may suspend or fail. |
| **Destructors are rare and infallible** | Drop glue is generated; a hand-written `fn drop(self)` lives with its type and cannot fail or suspend. `defer` follows the same rules; `errdefer` may fail, and its error is suppressed. |

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
    `Conn { socket, session }`. This follows from definite initialisation ([Null and invalid states](null-safety.md)).
  - The literal is pure: it cannot suspend or fail.
  - With private fields, only the type's own module can write the literal, so its creator
    functions enforce the invariants.
  - Creator functions (`connect`, `open`, `new`) are ordinary functions. They may suspend, fail and
    acquire resources.
  - There is no `this` before the literal, so a partially constructed object can never be observed.
  - If creation fails or is cancelled midway, the resources acquired so far are owned locals and
    are dropped. Linear ones must be consumed on that path ([Resources: affine and linear](#resources-affine-and-linear)).
  - The asymmetry is deliberate: creation may suspend, drop may not. Async teardown is a linear
    type's explicit `close` or `commit` ([Resources: affine and linear](#resources-affine-and-linear), [`concurrency.md`: Construction may suspend; drop may not](concurrency.md#construction-may-suspend-drop-may-not)).
- **Drop must not suspend and must not fail.** The compiler checks this: a drop implementation has
  an empty effect set. Finalisation that can fail or needs a decision is a linear type ([Resources: affine and linear](#resources-affine-and-linear), [Cleanup that can fail (why drop cannot)](#cleanup-that-can-fail-why-drop-cannot)).
- Unique ownership covers more than people expect: singly linked lists, trees, ASTs with owned
  children and builder patterns need no rc at all.

### Destructors

**Most types never write one.** The compiler generates *drop glue*: dropping a value drops each of
its fields. A struct of `String`, `Vec` and `File` needs no code at all.

A **hand-written destructor** is only for a type that owns a raw resource: a C handle, raw memory, a
NIC ring slot. In practice that is mostly low-level mods.

- **It lives with the type.** It implements the core `Drop` trait, and **only the type's own module
  may implement it**, so nobody else can change what dropping your type does.
- **Signature: `fn drop(self)`.** The destructor consumes the value and may
  **destructure** it to move fields out. Destructuring `self` inside `drop` does not call `drop`
  again; fields that are not moved out are dropped by glue afterwards. (Rust's `drop(&mut self)`
  cannot move fields out, which forces `Option::take` and `ManuallyDrop` workarounds.)
- **Effect set: empty**, checked by the compiler (no `fail`, `suspend` or I/O). Two things are still
  allowed: freeing memory, and the write-only observability effects `log`, `span` and `metric`
  ([`observability.md`: Observability effects](observability.md#observability-effects)), so a destructor can report a problem.
- **A destructor is checked like an entry point** ([`error-handling.md`: Entry points must handle everything (R3)](error-handling.md#entry-points-must-handle-everything-r3)):
  its failure set must be empty. A fallible call inside it must be handled there ([If a destructor "errors"](#if-a-destructor-errors)).

```rust
impl Drop for Socket {
    fn drop(self) {
        let Socket { fd, buf } = self;            // move fields out; no recursive drop of Socket
        let r = c::close(fd);                     // C close can report an error…
        if let Err(e) = r { log.warn(fmt"close failed: {e}") }   // …handled here: logged
    }                                             // buf is dropped by glue here
}
```

### When drops run

- At the end of a scope, in **reverse declaration order**.
- When a variable is reassigned: the old value is dropped.
- On every exit path: early `return`, `?` propagation, and cancellation
  ([`concurrency.md`: Cancellation and shutdown](concurrency.md#cancellation-and-shutdown)).
- When the last `Rc` is released, on whichever thread or task releases it ([Biased reference counting](#biased-reference-counting)).
- **A moved-from value is never dropped.** If a value is moved on only some paths, a hidden drop flag
  decides at runtime; the compiler removes it wherever the path is known.
- **Temporaries are dropped at the end of their statement.** There are no temporary-lifetime-extension
  rules (a known source of surprise in Rust); bind a value with `let` to keep it.
- `drop(x)` is not special: it is an ordinary function taking its argument by value, so it ends
  `x` early.
- **Fields are dropped in reverse declaration order**, mirroring the literal that built them (as
  C++ does; Rust drops fields in declaration order).

### `defer` and `errdefer`

RAII covers resources that have a type. `defer` covers one-off cleanup that doesn't deserve one:
restoring a flag, decrementing a gauge, releasing a lock obtained through FFI.

```
defer { metrics.in_flight.dec() }        // runs on every exit: success, error, cancellation
errdefer fs.remove(tmp)                  // runs only on error and cancellation exits
```

- `defer` blocks, `errdefer` blocks and drops share one **last-in, first-out order** by declaration
  point, so cleanup always unwinds in the reverse of setup.
- **`defer` follows drop's rules**: an empty effect set, so it cannot fail or suspend. `defer f.close()`
  on a fallible `close` is rejected, which steers you to an explicit `close()?` or a linear type. This
  closes Go's `defer f.Close()` hole at compile time.
- **`errdefer` may fail.** It only runs on paths that already carry a primary error or cancellation,
  so its own failure is **suppressed** onto that outcome ([`error-handling.md`](error-handling.md)
  D10). On cancellation paths it may also suspend, inside a shielded, deadline-bounded scope
  ([`concurrency.md`: Cancellation and shutdown](concurrency.md#cancellation-and-shutdown)). It is where fallible cleanup such as `txn.rollback()`
  belongs.
- A `defer` or `errdefer` body **borrows what it mentions until the scope ends**, so the borrow
  checker rejects moving `x` away and then cleaning up `x`.

### If a destructor "errors"

There are three cases, each with a precise answer:

1. **A fallible operation inside `drop`** (e.g. C's `close()` returns an error code). It does not
   compile unless it is dealt with inside the destructor, because a destructor's failure set must be
   empty. The author can:
   - handle it locally, for example log it and continue (as in [Destructors](#destructors));
   - discard it explicitly, with a reason that shows up in review;
   - or make the type **linear** with an explicit `close() with fail(E)`, so callers must handle it.

   Silently swallowing the error (C++, Rust's `BufWriter`) and `std::terminate` are both impossible.
2. **Cleanup that fails while an error or cancellation is already in progress.** Only `errdefer`
   bodies and explicit cleanup on those paths can fail; their errors are **suppressed** onto the
   primary outcome: never lost, never fatal ([Cleanup that can fail (why drop cannot)](#cleanup-that-can-fail-why-drop-cannot)).
3. **A trap inside `drop`** (out-of-bounds, division by zero, out of memory, an FFI fault). This is a
   class-3 crash ([`error-handling.md`: Three outcome classes](error-handling.md#three-outcome-classes)): the process terminates with a full
   trace and flight-recorder dump; **remaining drops do not run**, because there is no unwinding; a
   trap while already handling a trap aborts immediately.

## Borrowing

### Parameter modes

Parameter modes use Rust's spelling ([`syntax.md`](syntax.md)); borrows are second-class, so no
lifetimes appear.

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
- **Non-escaping closures** may capture borrows (Swift's model). Escaping closures capture by move
  or rc. Details in [`closures.md`](closures.md).

### Returning borrows

A function may return a borrow or a view ([View types](#view-types)) derived from its borrowed parameters:

- **Methods**: the result derives from `self`.
- **Free functions**: the result derives from *all* borrowed parameters (conservative default).
- **Optional** precision: `-> T from a` names a *parameter*, never an abstract region.

```rust
fn get(&self, key: &K) -> Option<&V>                        // derives from self
fn longer(a: &str, b: &str) -> &str                         // derives from a and b
fn first_word(text: &str, sep: &str) -> &str from text
```

## View types

A **view type** is a struct or container whose fields hold borrows. Each view has **exactly one
implicit region**, always inferred. It is second-class, like a borrow:

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
view RingIter<T> { buf: &[T], idx: usize }
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

- `.to_rc()` **consumes** the owner; the value now lives as long as any `Rc` does.
- RC cannot collect cycles. Back-edges (parent pointers, doubly-linked `prev`) use `Weak`.
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
  [`concurrency.md`: Runtime contract](concurrency.md#runtime-contract)). On single-threaded WASM every count is the owner count.

**Proposed: bias to the owner task, not the owner thread.** Tasks migrate between worker threads
([`concurrency.md`: Tasks, threads and executors](concurrency.md#tasks-threads-and-executors)), and with a thread id in the header a migrated task would
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
  rely on thread identity. This is one reason linear values cannot be rc'd ([Resources: affine and linear](#resources-affine-and-linear)).

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
fn on_rx(pkt: Packet) with net {
    let order = wire::Order::validate(pkt)?; // Parsed<Order>: owns pkt, zero copy
    if order.symbol() == "ESZ6" {            // projection: borrow derived from order
        strategy_queue.push(order);          // move: the buffer travels, the payload is not copied
    }
}                                            // otherwise order drops → slot back to the NIC ring
```

**Ring pressure:** holding a `Packet` for a long time ties up a ring slot. This is resource
pressure, not unsafety. The runtime mod should expose ring occupancy, and offer an explicit
`.detach()` that copies the payload out for long-term storage.

## Resources: affine and linear

- **Affine (default):** a value is used at most once and dropped automatically. Files, sockets,
  buffers, lock guards.
- **Linear (opt-in per type):** a value **must** be consumed by one of its designated operations,
  or the program does not compile. For resources whose finalisation can fail or needs a decision.

```rust
linear struct Txn { … }
impl Txn {
    fn commit(self) -> Result<(), DbError>;
    fn rollback(self);
}

fn checkout(db: &Db) with db, fail(DbError) {
    let txn = db.begin();
    txn.insert(order)?;       // ✗ unless the error path also consumes txn
    txn.commit()?;
}
```

Rules:
- Linear values **cannot be `.to_rc()`'d**: the last-drop point of an rc is not known statically,
  and it may be on any thread.
- Every control-flow path, including error propagation, must consume a linear value. Error
  propagation (`?`) on a path holding a linear value requires an explicit handler for it
  (exact syntax open; see [Error handling](error-handling.md)).
- **Cancellation paths are checked like error paths**: a task observes cancellation only at a
  suspension point, and must consume its linear values on that path too, typically with
  `errdefer txn.rollback()` ([`concurrency.md`: Cancellation and shutdown](concurrency.md#cancellation-and-shutdown)).

### Cleanup that can fail (why drop cannot)

**The problem in C++.**
- Destructors run during stack unwinding. If one throws while another exception is in flight, C++
  cannot carry two exceptions and calls `std::terminate`.
- Since C++11, destructors are implicitly `noexcept`, so a throw terminates even when nothing else
  is in flight. The rule is enforced at runtime, by killing the process.
- Cleanup can genuinely fail: `fclose` flushing to a full disk, a network filesystem reporting a
  write error on close, a commit. Because destructors can't report it, libraries swallow the error,
  and the program believes its data was written.
- A throwing member destructor can leave the remaining members and bases uncleaned.

**Elsewhere.**
- Rust: `Drop` cannot return an error, a panic in drop while already panicking aborts, and
  `BufWriter` silently discards flush errors on drop.
- Go: `defer f.Close()` routinely ignores the error.
- Java: try-with-resources is the best mainstream answer. It records a failing `close` as a
  *suppressed* exception on the primary one.

**Moddil's rules.**
1. **Drop has an empty effect set, checked by the compiler.** A drop implementation cannot fail,
   suspend, or call anything that does. C++'s `noexcept` becomes a compile error, not a
   `std::terminate`.
2. **Fallible cleanup is a linear type.** Written files, buffered writers, transactions and TLS
   streams must be explicitly consumed (`close`, `commit`, `rollback`, `abort`). These are ordinary
   functions that may fail and suspend. Since every path must consume the value, the cleanup error
   is always reported.
3. **Errors do not unwind.** `fail` is compiled to the return channel
   ([`error-handling.md`: Performance: why C++ exceptions are slow, and why Moddil's errors aren't](error-handling.md#performance-why-c-exceptions-are-slow-and-why-moddils-errors-arent), [`error-handling.md`: Strawman (not yet decided)](error-handling.md#strawman-not-yet-decided)), so no error is "in flight" while drops run.
   The double-fault mechanism does not exist.
4. **Two errors at once are explicit, and nothing is lost.** On an error path, a linear value is
   consumed by an `errdefer` (or a handler). If that cleanup fails too, its error is attached to the
   primary error as **suppressed** rather than replacing it or terminating
   ([`error-handling.md`: Cleanup](error-handling.md#cleanup)).
5. **Traps inside drop are crashes** (class 3: out of memory, ÷ 0, out of bounds). They terminate
   with a full trace, as in Rust. They are honest, never silently swallowed.
6. **No flush-on-drop trap.** A drop cannot call a fallible `flush`, so a type like Rust's
   `BufWriter` must be linear, or its author must discard the error explicitly and visibly.

```
let out = fs.create(path)?           // linear: must be closed or aborted
errdefer out.abort()                 // consumes out on error paths; if abort fails, its error is suppressed
out.write(data)?
out.close()?                         // a flush/close failure surfaces here, not in a destructor
```

## Cost table

| Construct | Allocation | Per-use cost | Thread-safe to share |
|---|---|---|---|
| Owned value | Wherever its owner lives | None | Move only |
| `&` / `&mut` borrow | None | None (may be passed by value) | Lent to child tasks |
| View type | None | None | Lent to child tasks |
| Owning view `Parsed<T>` | The underlying buffer | One add per field access | Move only |
| `Rc<T>` | Header + value | Biased inc/dec on clone/drop; free when borrowed | Yes |
| `Weak<T>` | (shares rc allocation) | Upgrade check | Yes |
| Sync cell | Inline | Atomic op or lock | Yes |

## Fit check against the reference examples

| Example | Where rc appears | Linear resources | Zero-copy views | Gaps |
|---|---|---|---|---|
| [Multiplayer game](examples/01-multiplayer-game.md) | Immutable world snapshots fanned out to N player connections (rc slices); chat room membership | — | Input packets from WebTransport, decoded in place on `hub` | Entity graphs: use ECS arenas + indices, not rc graphs |
| [HFT](examples/02-hft-platform.md) | Market data fan-out to multiple strategies (rc slices) | Order lifecycle (sent → acked/rejected) is a natural linear candidate | **Core case**: NIC → decode → book → strategy with owning views; zero copy end to end | Order book is single-threaded mutable: owned, not rc |
| [Web store](examples/03-web-store.md) | Config / DB pool shared across request tasks (or as effects) | DB transactions must commit or roll back | HTTP bodies parsed into views | None found; `AppContext` view or effects cover handler context |
| [Hashiverse](examples/04-hashiverse.md) | Shared routing table snapshots (`AtomicRc` swap); key material handles | Key-locker unlock sessions? (to evaluate) | RPC packets validated once (PoW-stamped) and passed as owning views | Kademlia routing tables: indices or `AtomicRc` snapshots, not mutable rc graphs |

No case required first-class references with explicit lifetimes.

## Open questions

- Exact syntax for parameter modes, views and `from`.
- Allocators: how does the allocator capability (Zig-style) interact with `.to_rc()`
  and arenas? Does an `Rc` remember its allocator? (Proposed answer in
  [`freestanding.md`: Memory without an operating system](freestanding.md#memory-without-an-operating-system).)
- Should `Weak` be core or a mod? (It needs header support, so probably core.)
- Linear values on error paths: what is the least noisy explicit handling?
- Can the borrow check give diagnostics good enough for LLM agents ([LLMs as first-class programmers](agent-interface.md)) — e.g.
  machine-applicable "insert `.to_rc()` here" fixes?
- Is there still a place for an optional per-task/per-actor collector, or does this model make it
  unnecessary?

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
FlatBuffers, Cap'n Proto, SBE (zero-copy formats); io_uring, AF_XDP, DPDK (zero-copy I/O).

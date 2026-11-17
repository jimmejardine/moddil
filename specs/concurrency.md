# Concurrency across CPUs

> **Status:** decided direction, details open. Expands [`rationale.md`](rationale.md) §2.8 and builds on
> [`memory-and-resources.md`](memory-and-resources.md). Snippets are **illustration only**.

## 1. Summary

Moddil gets **data-race freedom by construction** from three ingredients:

1. The memory model ([`memory-and-resources.md`](memory-and-resources.md)): unique ownership,
   second-class borrows and views, immutable thread-safe `Rc`, shared mutation only through sync
   cells.
2. **Structured concurrency as the only way to start a task**: every task has a parent scope and
   finishes before that scope ends.
3. **Suspension is an effect**, not a function colour (rationale §2.8, §2.1).

## 2. No `Send` / `Sync`

Rust needs `Send` and `Sync` because it has non-thread-safe sharing (`Rc`, `Cell`, `RefCell`). Moddil
has none:

| Way to reach data | Why it is race-free across tasks |
|---|---|
| Owned value | Moved into the task; nobody else has it |
| `let` borrow / view | Read-only, and the source is frozen until all borrows end |
| `mut` borrow | Exclusive; lent to exactly one task |
| `Rc<T>` | Biased RC is thread-safe; contents are immutable |
| Sync cell | Thread-safe by definition |

So **every type is sendable**, and there are no auto traits to learn, infer, or get wrong in
generic bounds. The one exception is `unsafe` code, which must uphold these rules itself (and is
visible in capabilities, rationale §2.16).

## 3. Tasks and borrowing

Because children always finish before their parent scope ends, children can **borrow from the
parent's frame**. This is Rust's `thread::scope`, but it is the only mode, so the `'static` +
`Arc::clone` dance disappears.

```
fn total(orders: let [Order]) -> Money ! {spawn} {
    let (left, right) = orders.split_at(orders.len() / 2)
    scope { s ->
        let a = s.spawn { sum(left) }     // shared borrows: many children may read
        let b = s.spawn { sum(right) }
        a.join() + b.join()
    }                                     // children are done here; borrows end
}

fn normalise(prices: mut [Price]) ! {spawn} {
    let (lo, hi) = prices.split_at_mut(prices.len() / 2)
    scope { s ->
        s.spawn { scale(lo) }             // each mut borrow goes to exactly one child
        s.spawn { scale(hi) }
    }
}
```

Rules:
- A child task closure is a **non-escaping closure** relative to its scope, so it may capture
  borrows and views from enclosing frames.
- A shared borrow may be captured by any number of children; a `mut` borrow by exactly one.
- The parent cannot touch a source lent `mut` to a child until the scope ends.
- Passing an `Rc` into children by borrow costs nothing. A child only clones it if it stores the
  `Rc` somewhere that outlives the borrow (e.g. in a value it sends on a channel).
- Long-lived tasks (e.g. a server's per-connection tasks, children of an accept-loop scope near the
  top of `main`) still borrow from that top-level frame, or receive owned values and `Rc`s.
- Cancellation and errors propagate through scopes as defined by the language (rationale §2.8).

## 4. Shared mutable state

All shared mutation goes through sync cells ([`memory-and-resources.md`](memory-and-resources.md)
§5.2):

- **Core intrinsics:** atomic loads/stores/RMW with explicit memory orderings, fences.
- **`moddil/sync`:** `Atomic<T>`, `AtomicRc<T>`, `Mutex<T>`, `RwLock<T>`, channels, lock-free
  SPSC/MPMC queues.
- Locks expose their contents through **scoped access**, not storable guards:

```
config.with { c -> c.max_conns = 100 }     // lock held for the closure only
```

A guard is a view (second-class), so it cannot leak out of the scope or be held across task
boundaries by accident. Holding a lock across a suspension point is allowed but visible
(the closure has the `suspend` effect); a lint can flag it.

## 5. Patterns the model makes easy

| Pattern | Shape |
|---|---|
| Fork/join data parallelism | Split borrows; children in a scope (§3) |
| Read-mostly shared config | `AtomicRc<Config>`; readers take a snapshot `Rc`, writers swap |
| Pipelines (e.g. NIC → decode → strategy) | Owned values / owning views moved through SPSC queues; zero copy ([`memory-and-resources.md`](memory-and-resources.md) §6) |
| Fan-out of immutable data | rc slices |
| Actors | A task owning its state, plus a channel; a library pattern, not a language feature |
| Thread-per-core / sharded state | Each shard owns its data; cross-shard work is messages |

## 6. Runtime contract

Scheduling is a core **effect**; runtimes are mods that provide handlers (rationale R2,
[`mods.md`](mods.md) §3). The memory model adds obligations for runtime handlers:

- **Biased-RC merge queue:** each worker thread must poll its queue of objects whose last shared
  reference was dropped elsewhere. The runtime chooses when (between tasks, on idle, on a timer).
- **Task migration:** a task moved by work stealing just becomes a "non-owner" for rcs created on its
  old thread; correctness is unaffected, only the fast path is lost.
- **Single-threaded targets** (browser WASM without cross-origin isolation, rationale R1): every rc
  operation is an owner operation; scopes still work, with children interleaved on one thread.
- **Deterministic simulation** (`moddil/runtime-sim`, rationale §2.11): the scheduler is a handler, so
  interleavings are seed-controlled and replayable.

## 7. Open questions

- **Preemption:** guaranteed (BEAM-style, via compiler-inserted yield points) or cooperative?
  Affects tail latency (HFT, game tick) and fairness.
- **Cancellation and linear values:** a cancelled task may hold a linear `Txn`. Options: cancellation
  is only observed at suspension points and the type system requires linear values to be consumed
  before any suspension that can be cancelled; or linear types declare a mandatory
  `on_cancel` consumer (e.g. rollback); or cancellation is deferred while linear values are live.
- **Deadlock:** this model prevents data races, not deadlocks. Lock ordering by type? A lint? Lock
  levels (as in some kernels)? Or steer toward message passing?
- **Async drop:** drop may not suspend (§8). Is that enough for resources whose clean shutdown needs
  I/O (e.g. flushing a TLS stream)? Probably: they become linear with an explicit `close`.
- **Priority inversion** under the scheduler effect: does the core effect interface carry priority?
- **Long-lived tasks:** what is the idiomatic shape for a server's top-level scope (accept loop with
  per-connection children) so that it reads naturally, not as a workaround for "no fire-and-forget"?

## 8. Construction may suspend; drop may not

**The problem elsewhere.** C#, JavaScript, Python, Java and C++ all forbid async constructors:
1. `new T()` must produce a `T` synchronously. An async constructor would make it produce a
   `Task<T>` or a promise instead.
2. The object is allocated before the constructor runs, which only initialises `this`. An `async`
   body is rewritten into a separate state machine, and that cannot initialise `this` or its
   `readonly` fields.
3. A constructor suspended at an `await` would expose a half-built `this` to anything that already
   holds it (event handlers, virtual calls, stored references).
4. Even if it were allowed, async colouring would spread to every `new` and every caller.

The usual workarounds are a static `CreateAsync()` factory, a two-phase `InitAsync()` that leaves a
usable-but-invalid object in between, and `Lazy<Task<T>>`. Each is awkward or leaves invalid states
reachable.

**Moddil's answer.** Two decisions remove the problem rather than working around it:
- **No special constructors** ([`memory-and-resources.md`](memory-and-resources.md) §2). A value
  exists only once an all-fields literal has been evaluated. Creators are ordinary functions.
- **Suspension is an effect, not a colour** (§1).

So async construction is simply a function that suspends:

```
fn connect(addr: let Addr) -> Conn ! {net, suspend, fail(NetError)} {
    let socket  = tcp.connect(addr)     // may suspend
    let session = handshake(socket)     // may suspend or fail; on failure, socket is dropped
    Conn { socket, session }            // the Conn exists only from here, fully formed
}
```

- Callers do not change colour; their signatures gain the effects, which are inferred inside a
  module.
- There is never a partial object. Before the literal there are only owned locals, and failure or
  cancellation drops them (or requires linear ones to be consumed).
- **Long-lived services** (the dependency-injection case) are built by suspending functions at the
  top of a structured scope, or provided as effect handlers. Nothing like `Lazy<Task<T>>` is needed.

**Drop never suspends.** Destruction runs on every exit path, including cancellation and traps, so
it must be immediate and infallible. A resource whose teardown needs I/O is a **linear type** with an
explicit `close()` or `commit()`, which may suspend and fail like any other function
([`memory-and-resources.md`](memory-and-resources.md) §7).

## Prior art

Rust (`Send`/`Sync`, `thread::scope`, rayon); Trio, Swift `TaskGroup`, Kotlin scopes, Java
`StructuredTaskScope`; Pony reference capabilities; Erlang/OTP; Go; Koka and OCaml 5 effect
handlers; FoundationDB and TigerBeetle deterministic simulation; Choi et al., biased reference
counting (PACT 2018).

# Concurrency across CPUs

> **Status:** complete: decided, with no open questions. Indexed in [`README.md`](README.md) and builds on
> [`memory.md`](memory.md). GPUs and other accelerators, which reuse these
> rules for kernel launches, are in [`accelerators.md`](accelerators.md). Snippets are
> **illustration only**.

## Summary

Moddil gets **data-race freedom by construction** from three ingredients:

1. The memory model ([`memory.md`](memory.md)): unique ownership,
   second-class borrows and views, immutable thread-safe `Rc`, shared mutation only through sync
   cells.
2. **Structured concurrency as the only way to start a task**: every task has a parent scope and
   finishes before that scope ends.
3. **Suspension is an effect**, not a function colour ([Effects and capabilities as a unifying idea](effects-and-capabilities.md)).

## No `Send` / `Sync`

Rust needs `Send` and `Sync` because it has non-thread-safe sharing (`Rc`, `Cell`, `RefCell`). Moddil
has none:

| Way to reach data | Why it is race-free across tasks |
|---|---|
| Owned value | Moved into the task; nobody else has it |
| `&` borrow / view | Read-only, and the source is frozen until all borrows end |
| `&mut` borrow | Exclusive; lent to exactly one task |
| `Rc<T>` | Biased RC is thread-safe; contents are immutable |
| Sync cell | Thread-safe by definition |

So **every type is sendable**, and there are no auto traits to learn, infer, or get wrong in
generic bounds. The one exception is `unsafe` code, which must uphold these rules itself (and is
visible in capabilities, [Package management and supply-chain security](packages-and-supply-chain.md)).

## Tasks and borrowing

Because children always finish before their parent scope ends, children can **borrow from the
parent's frame**. This is Rust's `thread::scope`, but it is the only mode, so the `'static` +
`Arc::clone` dance disappears.

```rust
fn total(orders: &[Order]) -> Money with spawn {
    let (left, right) = orders.split_at(orders.len() / 2);
    scope(|s| {
        let a = s.spawn(|| sum(left));    // shared borrows: many children may read
        let b = s.spawn(|| sum(right));
        a.join() + b.join()
    })                                    // children are done here; borrows end
}

fn normalise(prices: &mut [Price]) with spawn {
    let (lo, hi) = prices.split_at_mut(prices.len() / 2);
    scope(|s| {
        s.spawn(|| scale(lo));            // each &mut borrow goes to exactly one child
        s.spawn(|| scale(hi));
    })
}
```

Rules:
- A child task closure is a **non-escaping closure** relative to its scope, so it may capture
  borrows and views from enclosing frames. See [`closures.md`](closures.md) for closures in general.
- A shared borrow may be captured by any number of children; a `&mut` borrow by exactly one.
- The parent cannot touch a source lent `&mut` to a child until the scope ends.
- Passing an `Rc` into children by borrow costs nothing. A child only clones it if it stores the
  `Rc` somewhere that outlives the borrow (e.g. in a value it sends on a channel).
- Long-lived tasks (e.g. a server's per-connection tasks, children of an accept-loop scope near the
  top of `main`) still borrow from that top-level frame, or receive owned values and `Rc`s.
  **Libraries package the shape** (decided): `http::serve(listener, app)` is an accept loop with an
  isolated scope per connection ([`stdlib.md`: HTTP](stdlib.md#http)), and
  `supervise(|s| …, restart: …)` in `moddil/sync` covers pollers and workers with restart policies.
  The hand-written loop in the [tour](tour.md) is the documented pattern underneath.
- Cancellation and errors propagate through scopes
  ([`error-handling.md`: Tasks and concurrency](error-handling.md#tasks-and-concurrency)).
  Cancellation is cooperative and observed only at suspension points, and the cancellation path
  must consume linear values like the error path
  ([`lifecycle.md`: Cancellation and shutdown](lifecycle.md#cancellation-and-shutdown)). There is no
  async drop ([`stdlib.md`: I/O](stdlib.md#io)).

## Shared mutable state

All shared mutation goes through sync cells ([`memory.md`: rc contents are immutable](memory.md#rc-contents-are-immutable)):

- **Core intrinsics:** atomic loads/stores/RMW with explicit memory orderings, fences.
- **`moddil/sync`:** `Atomic<T>`, `AtomicRc<T>`, `Mutex<T>`, `RwLock<T>`, channels, lock-free
  SPSC/MPMC queues.
- Locks expose their contents through **scoped access**, not storable guards:

```rust
config.with(|c| c.max_conns = 100);         // lock held for the closure only
```

A guard is a view (second-class), so it cannot leak out of the scope or be held across task
boundaries by accident. Holding a lock across a suspension point is allowed but visible
(the closure has the `suspend` effect); a lint can flag it.

**Deadlocks** (decided). The model prevents data races, not deadlocks, so:
- a mutex may carry a **lock level** (`Mutex<T, level: 3>`). Taking a lock whose level is equal to
  or lower than one already held is a compile error where it can be seen statically, and a trap
  otherwise, as in kernel lock-ordering schemes; unlevelled mutexes are allowed;
- `runtime-sim` detects actual deadlocks (every task blocked) and reports the cycle with the seed
  that replays it;
- documentation steers towards channels and actors.

**Software transactional memory is a library pattern, not a language feature** (decided). Nothing in
the core is needed: `atomically(|tx| …)` can take a closure whose effects are restricted to an
`Stm` trait ([`effects-and-capabilities.md`: Restricted code is one mechanism](effects-and-capabilities.md#restricted-code-is-one-mechanism)),
so a retried body can do no I/O, `spawn` or `suspend`, and immutable `Rc` contents make snapshots
cheap. It stays out of the standard mods: logging every read and write allocates (so it is
unusable under `#[no_alloc]`), contention causes retry storms, and it is unavailable in
freestanding builds and on GPUs. Community mods may provide it.

## Patterns the model makes easy

| Pattern | Shape |
|---|---|
| Fork/join data parallelism | Split borrows; children in a scope ([Tasks and borrowing](#tasks-and-borrowing)); see [Data parallelism: `parallel_map`](#data-parallelism-parallel_map) for `parallel_map` |
| Read-mostly shared config | `AtomicRc<Config>`; readers take a snapshot `Rc`, writers swap |
| Pipelines (e.g. NIC → decode → strategy) | Owned values / owning views moved through SPSC queues; zero copy ([`memory.md`: Zero-copy I/O](memory.md#zero-copy-io)) |
| Fan-out of immutable data | rc slices |
| Actors | A task owning its state, plus a channel; a library pattern, not a language feature |
| Thread-per-core / sharded state | Each shard owns its data; cross-shard work is messages; `pinned` executors ([Tasks, threads and executors](runtime.md#tasks-threads-and-executors)) |
| GPU offload | A kernel launch is a child task borrowing its buffers; the scope end is the fence ([`accelerators.md`: Launches are structured tasks](accelerators.md#launches-are-structured-tasks)) |

### Data parallelism: `parallel_map`

A worked example. It needs no new language features: structured scopes ([Tasks and borrowing](#tasks-and-borrowing)), shared borrows
([`memory.md`: Borrowing](memory.md#borrowing)), closure parameter modes
([`closures.md`: Closure parameters: `Fn`, `FnMut`, `FnOnce`](closures.md#closure-parameters-fn-fnmut-fnonce)) and effect polymorphism
([`error-handling.md`: Results and effects](error-handling.md#results-and-effects)) are enough.

```rust
let thumbs = photos.parallel_map(|p| render_thumbnail(p));            // CPU-bound
let pages  = urls.parallel_map(|u| http.get(u)?, limit: 32);          // I/O-bound: suspends, can fail
```

Same shape as `map`; output order matches input order; the lambda's effects flow through.

**Signature**

```rust
fn parallel_map<T, U, E>(items: &[T], f: impl Fn(&T) -> U with E) -> Vec<U> with spawn, E
```

- `items: &[T]` is a read-only borrow, so the collection is **frozen for the whole call**. That
  the collection is read-only is enforced by the compiler, not presumed.
- `f: impl Fn…` means the closure may be called concurrently, so its captures are read-only.
- Elements arrive as `&T` borrows; to own part of an element, the lambda copies it or takes an rc.

**Implementation sketch** (`moddil/collections` or a `moddil/par` mod)

- **Recursive splitting with work stealing** (Rayon's `join` model): the input is halved into two
  children in a scope until a minimum grain size, which balances uneven per-element cost better than
  fixed chunks.
- **Output:** pre-allocated as uninitialised slots that must each be written exactly once (a linear
  type, [`resources.md`: Resources: affine and linear](resources.md#resources-affine-and-linear)). It is split in step with the input,
  so each child writes its own disjoint range: no locks, no copying, and the type system checks that
  every slot is filled. The one piece of `unsafe` lives in an audited place in the standard library,
  never in user code.
- When the scope ends, every child has finished, so the borrows end and the output is complete.

**Errors, cancellation and limits**

- If `f` fails, the first error cancels the sibling tasks and propagates; errors from siblings that
  fail before stopping are attached to it as suppressed ([`error-handling.md`: Tasks and concurrency](error-handling.md#tasks-and-concurrency), D15).
- `parallel_map_all` returns `Vec<Result<U, E>>` instead: each item's failure is captured as a value,
  so nothing is cancelled and every item runs to completion, even when some fail.
- `limit:` caps concurrency for I/O-bound work; CPU-bound work defaults to the runtime's worker count.

**What the compiler rejects**

```rust
let mut count = 0;
items.parallel_map(|x| { count += 1; f(x) });   // error: `count` is mutated inside an `Fn` closure
```

This is a genuine data race. The error suggests `parallel_reduce`, an `Atomic` counter, or `.len()`
on the result.

**Related operations**

- `parallel_for_each_mut(items: &mut [T], f: impl Fn(&mut T) with E)`: in-place updates; each
  child receives its own disjoint `&mut` sub-slice (as in [Tasks and borrowing](#tasks-and-borrowing)).
- `parallel_reduce(items, identity, f, combine)`: `combine` must be associative. For floating point,
  a **fixed split tree** gives the same result regardless of thread count
  ([`determinism.md`: Sources of nondeterminism](determinism.md#sources-of-nondeterminism)).
- Hash maps and trees take part through a `Split` trait in `moddil/collections` (Rayon's producers).
  Linked lists split badly and should be collected first.

**Runtime behaviour**

- Single-threaded targets (browser WASM without cross-origin isolation): children interleave on one
  thread. Same result, just not faster.
- Under `runtime-sim`: interleavings are explored deterministically.
- With a deterministic `f`, the output is identical across runs and thread counts.

**Compared with Rayon:** `items.par_iter().map(f).collect()` needs `F: Fn + Sync + Send`,
`T: Sync` and `U: Send`, cannot take async closures, and needs `collect::<Result<Vec<_>, _>>()` to
use `?`. None of that applies here.

**Later:** a lambda that is safe to run as a GPU kernel could let the same call dispatch to a GPU
([`accelerators.md`](accelerators.md)).

## Open questions

None at present.

## Background

The problem and the state of the art that led to this design.

**Problem.** Shared-memory parallelism is still the main source of the hardest bugs: data races,
deadlocks, priority inversion, and memory-ordering mistakes. Async programming has split many
ecosystems into "sync" and "async" halves.

**State of the art.**
- **Rust**: `Send`/`Sync` traits plus ownership give compile-time freedom from data races. This is a
  real breakthrough.
- **Erlang/BEAM**: share-nothing processes, message passing, supervisors, and preemptive scheduling of
  millions of lightweight processes. It has the best story for fault tolerance.
- **Go**: goroutines and channels make concurrency cheap and ergonomic, and there are no coloured
  functions.
- **Pony**: reference capabilities (`iso`, `val`, `ref`, `tag`, …) give actors data-race freedom by
  construction, plus a per-actor GC (ORCA) with no stop-the-world pauses.
- **Structured concurrency**: Trio (Python), Swift's `TaskGroup`, Kotlin coroutine scopes, and Java's
  virtual threads plus `StructuredTaskScope`. A task's lifetime is bounded by its lexical scope, so
  cancellation and errors propagate properly.

**What still hurts.**
- **Function colouring** (Bob Nystrom's "What color is your function?"). In Rust, C#, JS and Python,
  `async` spreads through the codebase, and libraries split into sync and async variants. Rust async adds
  `Pin`, `Send` bounds across `.await`, and executor lock-in (Tokio vs others).
- Go has no compile-time protection against races. The race detector is dynamic and optional.
- Erlang's isolation costs copying, and its raw compute performance is weak.
- Rust's guarantees come with a learning curve that is still the most common complaint about the
  language.
- Cancellation is an afterthought almost everywhere. Blocking reads (stdin, `accept`) ignore
  shutdown, forcing workarounds such as pressing Enter after Ctrl-C or connecting to your own socket.
- **No async constructors** in C#, JavaScript, Python, Java or C++. The workarounds (`CreateAsync`
  factories, two-phase `InitAsync`, `Lazy<Task<T>>`) give awkward APIs or leave invalid objects
  reachable.

**Directions.**
- Actors, channels and thread-per-core sharding as library patterns built on these primitives, not
  as the only model.
- Deterministic scheduling mode for tests (see [Determinism and reproducibility](determinism.md)).
- Lints for holding a lock across a suspension point.

## Prior art

Rust (`Send`/`Sync`, `thread::scope`, rayon); Trio, Swift `TaskGroup`, Kotlin scopes, Java
`StructuredTaskScope`; Pony reference capabilities; Erlang/OTP; Go; Koka and OCaml 5 effect
handlers; FoundationDB and TigerBeetle deterministic simulation; Choi et al., biased reference
counting (PACT 2018).

# Concurrency across CPUs

> **Status:** decided direction, details open. Indexed in [`README.md`](README.md) and builds on
> [`memory-and-resources.md`](memory-and-resources.md). GPUs and other accelerators, which reuse these
> rules for kernel launches, are in [`accelerators.md`](accelerators.md). Snippets are
> **illustration only**.

## Summary

Moddil gets **data-race freedom by construction** from three ingredients:

1. The memory model ([`memory-and-resources.md`](memory-and-resources.md)): unique ownership,
   second-class borrows and views, immutable thread-safe `Rc`, shared mutation only through sync
   cells.
2. **Structured concurrency as the only way to start a task**: every task has a parent scope and
   finishes before that scope ends.
3. **Suspension is an effect**, not a function colour (Concurrency across CPUs, [Effects and capabilities as a unifying idea](effects-and-capabilities.md)).

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
- Cancellation and errors propagate through scopes as defined by the language (Concurrency across CPUs).

## Shared mutable state

All shared mutation goes through sync cells ([`memory-and-resources.md`: rc contents are immutable](memory-and-resources.md#rc-contents-are-immutable)):

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

## Patterns the model makes easy

| Pattern | Shape |
|---|---|
| Fork/join data parallelism | Split borrows; children in a scope ([Tasks and borrowing](#tasks-and-borrowing)); see [Data parallelism: `parallel_map`](#data-parallelism-parallel_map) for `parallel_map` |
| Read-mostly shared config | `AtomicRc<Config>`; readers take a snapshot `Rc`, writers swap |
| Pipelines (e.g. NIC → decode → strategy) | Owned values / owning views moved through SPSC queues; zero copy ([`memory-and-resources.md`: Zero-copy I/O](memory-and-resources.md#zero-copy-io)) |
| Fan-out of immutable data | rc slices |
| Actors | A task owning its state, plus a channel; a library pattern, not a language feature |
| Thread-per-core / sharded state | Each shard owns its data; cross-shard work is messages; `pinned` executors ([Tasks, threads and executors](#tasks-threads-and-executors)) |
| GPU offload | A kernel launch is a child task borrowing its buffers; the scope end is the fence ([`accelerators.md`: Launches are structured tasks](accelerators.md#launches-are-structured-tasks)) |

### Data parallelism: `parallel_map`

A worked example. It needs no new language features: structured scopes ([Tasks and borrowing](#tasks-and-borrowing)), shared borrows
([`memory-and-resources.md`: Borrowing](memory-and-resources.md#borrowing)), closure parameter modes
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
  type, [`memory-and-resources.md`: Resources: affine and linear](memory-and-resources.md#resources-affine-and-linear)). It is split in step with the input,
  so each child writes its own disjoint range: no locks, no copying, and the type system checks that
  every slot is filled. The one piece of `unsafe` lives in an audited place in the standard library,
  never in user code.
- When the scope ends, every child has finished, so the borrows end and the output is complete.

**Errors, cancellation and limits**

- If `f` fails, the first error cancels the sibling tasks and propagates ([Tasks and borrowing](#tasks-and-borrowing)).
- Collecting *all* errors instead is open ([`error-handling.md`: Tasks and concurrency](error-handling.md#tasks-and-concurrency)); a
  `parallel_map_all` returning `Vec<Result<U, E>>` would cover it.
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

## Runtime contract

Scheduling is a core **effect**; runtimes are mods that provide handlers ([R2](requirements.md#firm-requirements-decided),
[`mods.md`: The core / mod boundary](mods.md#the-core--mod-boundary)). The memory model adds obligations for runtime handlers:

- **Biased-RC merge queue:** objects whose last shared reference was dropped elsewhere are queued
  to their owner. *Proposed* ([`memory-and-resources.md`: Biased reference counting](memory-and-resources.md#biased-reference-counting)): the owner
  is a **task**, and the runtime drains a task's queue when it resumes the task and at task exit.
- **Task migration:** with task-biased RC (proposed), a task moved by work stealing keeps its fast
  path; migration costs nothing. With thread-biased RC it would become a non-owner on its new thread
  (correct, but slower).
- **Single-threaded targets** (browser WASM without cross-origin isolation, [R1](requirements.md#firm-requirements-decided)): every rc
  operation is an owner operation; scopes still work, with children interleaved on one thread.
- **Deterministic simulation** (`moddil/runtime-sim`, [Determinism and reproducibility](determinism.md)): the scheduler is a handler, so
  interleavings are seed-controlled and replayable.
  See [`determinism.md`](determinism.md) for the simulation runtime, virtual time and record/replay.
- **Observability:** runtimes expose the task tree and each task's blocked-on effect operation to
  spans and to the `introspect` capability ([`observability.md`: Task-aware stacks and the task tree](observability.md#task-aware-stacks-and-the-task-tree), [`observability.md`: Live introspection](observability.md#live-introspection)).
- **Cancellable I/O:** every pending I/O operation must be cancellable promptly at the OS level
  (io_uring cancel, epoll/kqueue deregistration, `CancelIoEx`), using per-platform strategies for
  terminals and pipes; results that complete in a race with cancellation are retained, so standard
  I/O stays cancel-safe ([Cancelling I/O: no "press Enter", no self-connect](#cancelling-io-no-press-enter-no-self-connect)).

### Tasks, threads and executors

**Tasks are the only programming model; threads are an execution resource.**

- Everything concurrent is a **task**: a stackful coroutine, implemented with stack switching
  ([`bootstrap.md`: WASM concurrency lowering](bootstrap.md#wasm-concurrency-lowering)). There is **no thread API in user code**, as with Go
  goroutines, Java virtual threads and Erlang processes.
- Scheduling is M:N: any task may run on any worker thread of its executor.
- The same code runs single-threaded (browser WASM), on a work-stealing pool, or pinned to a core.
  Nothing is marked async or sync (suspension is an effect, [Construction may suspend; drop may not](#construction-may-suspend-drop-may-not)).

**One runtime per process, many executors.** The runtime is the scheduler handler installed in
`main` (or `runtime-sim` in tests). Inside it, **executors** are pools of OS threads with a policy:

| Executor | Policy | Typical use |
|---|---|---|
| `default` | Work-stealing, one thread per core | Normal CPU and I/O tasks |
| `blocking` | Grows on demand | Blocking FFI and system calls (tokio's `spawn_blocking`; Go hands off its processor during syscalls) |
| `pinned(cores)` | One thread per core, no stealing, optional busy-polling and real-time priority | HFT hot path; shard-per-core designs (Seastar, Glommio) |
| `affine(main)` | One dedicated thread | Thread-affine resources: GUI main thread, GL contexts, C libraries with thread-local state |

- Executors are configured in code or in the deployment manifest ([Distributed and multi-tier programs](distribution.md)).
- **Placement:** a scope or spawn chooses its executor; children inherit their parent's executor
  unless they override it. This is the same placement idea as GPU devices and tiers, one level down
  (thread → core → GPU → machine).

  ```rust
  scope(|s| { … }, on: pinned(2..4));
  s.spawn(|| legacy_c_lib.compress(buf), on: blocking);
  ```

- **One I/O reactor** (io_uring, epoll, kqueue or IOCP) is shared by all executors. I/O is an effect,
  so there are no runtime-specific I/O types (as tokio's `TcpStream` is), no "cannot start a runtime
  from within a runtime", and no nested `block_on`.

**Where coroutines and threads genuinely differ:**

| Issue | Moddil's answer |
|---|---|
| A task blocks its OS thread | All I/O goes through effects, so safe code cannot block. FFI functions are declared `blocking` and run on the `blocking` executor automatically. |
| A long CPU loop never yields | **Recommended:** preemption at compiler-inserted safe points in loops and function entries (a cheap flag check), as in Go 1.14+ and Erlang. `pinned` executors can disable it. See [Open questions](#open-questions). |
| Thread identity, thread-local storage | Tasks migrate, so safe code has no thread-local storage. Task-local context comes from effects and handlers. Thread-affine resources require the `affine` executor (a capability). |
| Stack memory | Native: large virtual reservations, committed lazily, with guard pages. Go-style stack copying is impossible because borrows point into frames. *Open:* very many tasks on wasm32 are limited by address space. |
| Biased RC after migration | See the task-biased proposal in [`memory-and-resources.md`: Biased reference counting](memory-and-resources.md#biased-reference-counting): migration costs nothing. `pinned` executors never migrate anyway. |
| Switch cost | Task switch: tens of nanoseconds. OS thread switch: microseconds. |
| Priorities, real time | OS thread priorities per executor. Task priorities within an executor are open ([Open questions](#open-questions)). |

**Targets:** on browser WASM without cross-origin isolation, all executors collapse onto one thread;
with it, Web Workers add threads. GPU devices act as executors for kernel tasks
([`accelerators.md`](accelerators.md)). Under `runtime-sim` the whole runtime is one deterministic
thread; executor placement is recorded and simulated ([`determinism.md`](determinism.md)).

## Open questions

- **Preemption:** guaranteed (BEAM-style, via compiler-inserted yield points) or cooperative?
  Affects tail latency (HFT, game tick) and fairness. *Recommended* ([Tasks, threads and executors](#tasks-threads-and-executors)): compiler-inserted safe
  points in loops and function entries, which `pinned` executors can disable.
- ~~**Cancellation and linear values.**~~ Resolved: cancellation is cooperative and observed only at
  suspension points, and the cancellation path must consume linear values like the error path ([Cancellation and shutdown](#cancellation-and-shutdown)).
- **Deadlock:** this model prevents data races, not deadlocks. Lock ordering by type? A lint? Lock
  levels (as in some kernels)? Or steer toward message passing?
- **Async drop:** drop may not suspend ([Construction may suspend; drop may not](#construction-may-suspend-drop-may-not)). Is that enough for resources whose clean shutdown needs
  I/O (e.g. flushing a TLS stream)? Probably: they become linear with an explicit `close`.
- **Priority inversion** under the scheduler effect: does the core effect interface carry priority?
- **Long-lived tasks:** what is the idiomatic shape for a server's top-level scope (accept loop with
  per-connection children) so that it reads naturally, not as a workaround for "no fire-and-forget"?

## Construction may suspend; drop may not

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
- **No special constructors** ([`memory-and-resources.md`: Ownership and RAII](memory-and-resources.md#ownership-and-raii)). A value
  exists only once an all-fields literal has been evaluated. Creators are ordinary functions.
- **Suspension is an effect, not a colour** ([Summary](#summary)).

So async construction is simply a function that suspends:

```rust
fn connect(addr: &Addr) -> Conn with net, suspend, fail(NetError) {
    let socket  = tcp.connect(addr);    // may suspend
    let session = handshake(socket)?;   // may suspend or fail; on failure, socket is dropped
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
([`memory-and-resources.md`: Resources: affine and linear](memory-and-resources.md#resources-affine-and-linear)).

## Cancellation and shutdown

### Why cancellation has to exist

In servers and interactive programs, work regularly becomes unwanted before it finishes:

| Cause | Example |
|---|---|
| Sibling failure | A request fans out to three services; one fails, so the other results are useless |
| Deadlines | The caller's 200 ms budget has expired |
| The requester left | The client disconnected, or the user navigated away |
| Racing | Hedged requests: ask two replicas, take the first answer, stop the other |
| Shutdown | Ctrl-C, or SIGTERM during a rolling deploy |

Without cancellation that work keeps running and holding resources (Go's goroutine leaks before
`context.Context`).

**Two designs Moddil rejects:**
- **Explicit tokens** (C#'s `CancellationToken`, Go's `context.Context`) must be threaded through
  every call. People forget, libraries ignore them, and signatures get noisier.
- **Asynchronous killing** (.NET's `Thread.Abort`, Java's `Thread.stop`) was removed from both
  platforms: stopping code at an arbitrary instruction breaks invariants halfway through an update.

### The model: implicit and cooperative

- **A task's scope is its cancellation token.** It is carried by structured concurrency (as in Trio,
  Kotlin and Swift); nothing is passed through signatures.
- **Cancellation is observed only at suspension points.** Suspension is an effect, so every place a
  task can pause is known: I/O, joining children, locks, timers, channels. When a task is cancelled,
  its next suspension point returns a **`Cancelled` outcome** instead of a value. Code between
  suspension points is never interrupted, so invariants are never torn.
- **The cancellation path is an ordinary, checked exit path:**
  - drops, `defer` and `errdefer` run ([`memory-and-resources.md`: When drops run](memory-and-resources.md#when-drops-run)–[`memory-and-resources.md`: `defer` and `errdefer`](memory-and-resources.md#defer-and-errdefer));
  - **linear values must be consumed on it**, exactly as on error paths, typically with
    `errdefer txn.rollback()`;
  - cleanup on this path **may suspend** (a rollback needs the network) inside a **shielded,
    deadline-bounded scope**: it is not itself cancelled midway, and it cannot hang for ever.
- `Cancelled` is a distinct outcome, not an error ([`error-handling.md`: Tasks and concurrency](error-handling.md#tasks-and-concurrency));
  it is accounted for at the join.
- **Long pure-compute loops** have no suspension points. They call a cheap `checkpoint()` (a
  suspension point that observes cancellation); a lint flags long loops in cancellable tasks without
  one. Preemption safe points ([Tasks, threads and executors](#tasks-threads-and-executors)) only yield the thread; they never deliver cancellation, because
  that would interrupt code between effect operations.
- **Blocking FFI calls** cannot be interrupted. They run on the `blocking` executor ([Tasks, threads and executors](#tasks-threads-and-executors)); the scope
  waits for them, or detaches the OS thread if the function's overlay allows it
  ([`interop.md`: Annotation overlays](interop.md#annotation-overlays)).

### Two levels: stop, then cancel

| | **Stop** (graceful) | **Cancel** (hard, still cooperative) |
|---|---|---|
| Trigger | Ctrl-C / SIGTERM, deploy drain, a parent asks | Grace deadline expired, sibling failed, request deadline, racing |
| Observed | Only where code asks, e.g. `stop_requested()` in a `select` | At every suspension point (`Cancelled`) |
| Typical reaction | Stop taking new work, finish current work, say goodbye to peers, persist state | Abort current work, run cleanup |
| Like | Go's `http.Server.Shutdown`; Hashiverse's shutdown token | Trio / Kotlin / Swift cancellation |

- Signals are not asynchronous handlers: the runtime mod turns SIGINT and SIGTERM into a **stop**
  event on `main`'s root scope.
- If the scope has not finished when its **grace deadline** expires, stop escalates to cancel.
- A **second signal** exits the process immediately; the OS reclaims everything.

### Layered shutdown: innermost first

A naive shared token tells everyone to stop at once, so a service can shut down while code that
depends on it is still draining. Moddil orders shutdown by the scope tree instead:

- **Services provided as effect handlers cannot disappear early.** A handler (database pool, logger,
  DHT client) is installed by an outer scope, and a scope cannot end until everything inside it has
  finished, so no child outlives a handler it uses.
- **Scopes are shutdown layers.** Stop (and later cancel) is delivered to the **innermost layer
  first**. Outer layers keep **operating completely normally** (serving requests, accepting writes)
  while inner layers drain. Only when an inner layer has finished does the next layer out receive
  stop.
- **Tasks within one layer are peers** and stop together: thousands of connection handlers drain
  concurrently.
- Dependencies are expressed by **nesting**: services in an outer scope, the code that uses them in
  an inner one. This is the same rule as drop order (reverse of construction) and as Erlang/OTP
  supervisors, which terminate children in reverse start order.

```rust
fn main(sys: System) {
    scope(|services| {                                     // layer 1 (outer)
        services.spawn(|| db_writer());
        services.spawn(|| dht.maintain());

        scope(|app| {                                      // layer 2 (inner)
            app.spawn(|| server.serve());
        });
        // layer 2 has fully finished; only now do db_writer and dht see stop_requested()
    }, stop_on: [SIGINT, SIGTERM], grace: 10<s>);
}

fn serve() with net, suspend {
    scope(|s| {
        loop {
            select {
                conn = listener.accept() => s.spawn(|| handle(conn)),
                _ = stop_requested()     => break,     // stop accepting; in-flight handlers finish
            }
        }
    })                                                   // waits for in-flight handlers
}
```

On Ctrl-C:
1. `serve` stops accepting; in-flight handlers finish, still using `db_writer` and `dht`, which are
   running normally.
2. Layer 2 ends.
3. `db_writer` flushes and `dht` persists its routing table, then they end.
4. `main` returns an exit status.

If the grace deadline expires first, cancellation proceeds in the same inner-to-outer order, so each
layer's shielded cleanup still finds the outer services alive.

### Cancelling I/O: no "press Enter", no self-connect

**The problem elsewhere.** In Rust, a task reading stdin, or a thread in a blocking `accept`, sits
inside a blocking system call. A shutdown signal cannot reach it until the call returns, so users
press Enter after Ctrl-C, and programs connect to their own listening socket just to wake it up.
- Even `tokio::io::stdin` reads on a background blocking thread: stdin can't portably be made
  non-blocking (setting `O_NONBLOCK` on a shared terminal affects other processes; Windows console
  and anonymous-pipe handles don't support overlapped I/O).
- `select!` adds a second trap: each operation must be checked for *cancel-safety*, because some
  (such as `read_line`) lose data already read when they lose the race.

**Moddil's rules:**
1. **Every I/O operation is a suspension point and a cancellation point.** `accept`, `read`,
   `read_line`, `recv` and timers are effect operations handled by the runtime's reactor ([Tasks, threads and executors](#tasks-threads-and-executors)),
   never blocking system calls on a thread. A stop that wins a `select`, or a hard cancel, removes
   the pending operation and the task continues **immediately**.
2. **The runtime cancels at the OS level** ([Runtime contract](#runtime-contract)): io_uring cancel requests, removing interest from
   epoll or kqueue, `CancelIoEx` on Windows. An operation that completes in a race with its
   cancellation keeps its result (rule 4).
3. **Hard sources are handled once, in the runtime, per platform:**

   | Source | Strategy |
   |---|---|
   | Unix terminals and pipes | Wait for readiness, then read only what is available, so a read never blocks; never set `O_NONBLOCK` on a shared descriptor. Cancelling means no longer waiting. |
   | Linux with io_uring | A read on stdin can be cancelled (io_uring interrupts its worker), or use the readiness path. |
   | Windows console | Wait on the console input handle together with a cancel event; read only when input is signalled. |
   | Windows anonymous pipes | A thread on the `blocking` executor, interrupted with `CancelSynchronousIo`. |
   | Devices and FFI calls the OS cannot interrupt | The `blocking` executor; the scope waits, or detaches the thread if the function's overlay allows it ([`interop.md`: Annotation overlays](interop.md#annotation-overlays)). This is the **only** documented exception. |

4. **Standard I/O is always cancel-safe.** Buffers belong to the reader or stream object, not to the
   in-flight operation, so a cancelled `read_line` loses nothing: partial data stays in the reader for
   the next call. A `select` cancels its losing branches, and with cancel-safe operations that is
   always correct. There is no per-method "is this cancel-safe?" to look up.
5. **Ctrl-C itself.** In normal (cooked) terminal mode, SIGINT becomes a stop event on the root scope
   ([Two levels: stop, then cancel](#two-levels-stop-then-cancel)). In raw mode (full-screen terminal apps) it arrives as a key event from the terminal API.
6. **Testable.** Stdin, sockets and timers are effects, so the simulation runtime can feed input,
   stop and cancel deterministically ([`determinism.md`](determinism.md)).

```rust
fn repl() with stdin, stdout, suspend {
    let input = stdin.lines();
    loop {
        select {
            line = input.next()  => eval(line?),
            _ = stop_requested() => break,      // Ctrl-C: the pending read is cancelled; no Enter needed
        }
    }
}

fn serve(listener: &Listener) with net, suspend {
    scope(|s| {
        loop {
            select {
                conn = listener.accept() => s.spawn(|| handle(conn)),
                _ = stop_requested()     => break,     // the pending accept is cancelled; no self-connect
            }
        }
    })
}
```

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

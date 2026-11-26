# Runtime: scheduling, executors and I/O

The concurrency *model* (structured tasks, sharing, cancellation) is in
[`concurrency.md`](concurrency.md) and [`lifecycle.md`](lifecycle.md); this document covers what
runs it: runtimes, executors, the I/O reactor, and how they differ by target.

## Summary

| Rule                                  | One line                                                                                                                                                                                                              |
| ------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Runtimes are handlers**             | Scheduling, I/O and time are core effects; runtimes are mods that handle them, so every library works on every runtime ([Pluggable runtimes without ecosystem splits](#pluggable-runtimes-without-ecosystem-splits)). |
| **Tasks, not threads**                | Everything concurrent is a stackful task; threads are an execution resource with no user-facing API ([Tasks, threads and executors](#tasks-threads-and-executors)).                                                   |
| **Executors**                         | One runtime per process with several executors: `default`, `blocking`, `pinned(cores)`, `affine(main)`.                                                                                                               |
| **A reactor per worker**              | Each worker thread has its own I/O reactor (io_uring, epoll, kqueue, IOCP); every operation is cancellable at the OS level.                                                                                           |
| **Obligations from the memory model** | Runtimes drain biased-RC merge queues, expose the task tree, and keep I/O cancel-safe ([Runtime contract](#runtime-contract)).                                                                                        |

## Problem

Rust put `async` in the language but left the executor out, with no standard interface for spawning,
timers or I/O. Libraries tied themselves to tokio, async-std or smol, and the ecosystem split. At the
same time, real systems need very different runtimes: a work-stealing pool for services, a
thread-per-core design for trading, a single thread in the browser, a deterministic simulator in tests,
and a static scheduler on a microcontroller.

## Pluggable runtimes without ecosystem splits

**Lesson from Rust.** `async` is in the language, but the executor is not, and there is no standard
interface for spawning, timers or I/O. The result: libraries tied to tokio, async-std or smol, and an
ecosystem that partly fragmented.

**Moddil's approach.** The *interfaces* are core effects: `suspend`, `spawn` (scoped), `Time`, `Net`,
`Fs`, `Rand`, `log`. *Implementations* are mods that provide **handlers** for those effects:

- `moddil/runtime`: the standard multi-threaded work-stealing runtime.
- `moddil/runtime-wasm`: browser and WASI runtime; JSPI in browsers and Node first, WASM stack
  switching on WASI hosts later (R1).
- `moddil/runtime-sim`: deterministic simulation runtime ([Determinism and reproducibility](determinism.md)).
- A community `fastrt` (the tokio-equivalent): a different scheduler, io_uring, thread-per-core, etc.

Because libraries only use the *effects*, **every library works on every runtime**. Swapping the
runtime is a choice in `main` (or in the deployment manifest, [Distributed and multi-tier programs](distribution.md)), not a choice made by the whole
dependency tree.

**How a runtime is chosen (decided November 2026):**
- **The effect is how a runtime is chosen, not how one is written.** A runtime's scheduler (stack
  switching, moving tasks between threads, work stealing, atomics) is runtime code linked into
  the program; the handler its mod provides is how `main`, or the manifest, picks it.
- **One runtime per process,** installed once, beneath `suspend` and `spawn`, which stay as they
  are in signatures. Libraries never install one.
- **Under tests the default is `runtime-sim`:** the deterministic scheduler every implementation
  shares (the two interpreters, native code and WASM), so a test's interleaving is the same on
  all of them. The parallel runtime is `main`'s default, and is stress-tested on its own.

The same mechanism generalises the pluggable traits that real projects build by hand. Hashiverse,
for example, has `TransportFactory`, `TimeProvider`, `ClientStorage` and `KeyLockerManager` traits plus
an in-memory transport for tests (see [`examples/hashiverse.md`](examples/hashiverse.md)). In
Moddil these are effects, and the test versions are handlers.

### Starting a program

**Decided (November 2026):** a runtime is installed with `with Runtime = …`, as any handler is;
executors are values built from the `Cpus` root capability; tasks within an executor are
scheduled fairly, and priority belongs to executors.

**On the deterministic runtime.** A test needs nothing: each runs under `runtime-sim` with its own
seed, printed when it fails, so a failure replays exactly. A whole program can be run under it
from `main`:

```rust
test "orders settle" {
    scope(|s| {
        s.spawn(|| book.submit(buy));
        s.spawn(|| book.submit(sell));
    });
    book.trades().len()                 // => 1, in the same order on every run
}

fn main(args: Args, stdio: Stdio) -> ExitCode {
    with Runtime = Sim::seeded(42) {    // one task at a time, seed-ordered, virtual time
        app::run(&stdio)
    }
}
```

**On the parallel runtime.** Making OS threads, pinning them and setting their priority are OS
privileges, so `main` takes a `Cpus` root capability, builds its executors as values from it, and
installs the runtime as any handler is installed:

```rust
fn main(args: Args, stdio: Stdio, net: NetRoot, cpus: Cpus) -> ExitCode {
    let hot   = pinned(&cpus, cores: 0..8, priority: Priority::RealTime, busy_poll: true);
    let batch = pool(&cpus, cores: 8..120, priority: Priority::Low);
    let io    = blocking(&cpus, max_threads: 64);
    // `default` is a work-stealing pool over the cores left (120..128).
    with Runtime = runtime::parallel(&cpus), Net = &net {
        scope(|s| {
            s.spawn(|| matcher.run(), on: hot);             // latency-critical; never migrates
            for shard in shards {
                s.spawn(|| analytics(shard), on: batch);    // thousands of tasks on 112 cores
            }
            s.spawn(|| api.serve());                        // inherits `default`
            s.spawn(|| zlib.compress(buf), on: io);         // a blocking C call
        })
    }
}
```

The same `matcher`, `analytics` and `api` run unchanged under `Sim::seeded` in tests: only `main`
differs. Under `runtime-sim`, executor placement is recorded and simulated; under
`runtime::parallel` it is real. So one program can keep 128 cores busy with varied work at
varied priorities: priority is per executor, and tasks within one are scheduled fairly
([Tasks, threads and executors](#tasks-threads-and-executors)).

**Decided: a program means the same under every runtime, and is compiled the same way.** Rc
counts, locks and safe points always use the threaded lowering, so the tests under `runtime-sim`
run the code that ships. The exception is a *one-thread build*, set when the program is built (a
manifest key and `--threads one`), and always on WASM. It lowers Rc counts to plain counts,
leaves out the safe points and makes locks without atomics; `runtime::parallel` refuses it. The
cost of the threaded lowering on one thread is a few percent at most ([`memory.md`: Biased
reference counting](memory.md#biased-reference-counting)).

## Runtime contract

Scheduling is a core **effect**; runtimes are mods that provide handlers ([R2](requirements.md#firm-requirements-decided),
[`stdlib.md`: The core / mod boundary](stdlib.md#the-core--mod-boundary)). The memory model adds obligations for runtime handlers:

- **Biased-RC merge queue:** objects whose last shared reference was dropped elsewhere are queued
  to their owner. Decided ([`memory.md`: Biased reference counting](memory.md#biased-reference-counting)): the owner
  is a **task**, and the runtime drains a task's queue when it resumes the task and at task exit.
- **Task migration:** with task-biased RC, a task moved by work stealing keeps its fast
  path; migration costs nothing. With thread-biased RC it would become a non-owner on its new thread
  (correct, but slower).
- **Single-threaded targets** (browser WASM without cross-origin isolation, [R1](requirements.md#firm-requirements-decided)): every rc
  operation is an owner operation; scopes still work, with children interleaved on one thread.
- **Deterministic simulation** (`moddil/runtime-sim`, [Determinism and reproducibility](determinism.md)): the scheduler is a handler, so
  interleavings are seed-controlled and replayable.
  See [`determinism.md`](determinism.md) for the simulation runtime, virtual time and record/replay.
- **Observability:** runtimes expose the task tree and each task's blocked-on effect operation to
  spans and to the `Introspect` capability ([`observability.md`: Task-aware stacks and the task tree](observability.md#task-aware-stacks-and-the-task-tree), [`observability.md`: Live introspection](observability.md#live-introspection)).
- **Cancellable I/O:** every pending I/O operation must be cancellable promptly at the OS level
  (io_uring cancel, epoll/kqueue deregistration, `CancelIoEx`), using per-platform strategies for
  terminals and pipes; results that complete in a race with cancellation are retained, so standard
  I/O stays cancel-safe ([Cancelling I/O: no "press Enter", no self-connect](lifecycle.md#cancelling-io-no-press-enter-no-self-connect)).

### Tasks, threads and executors

**Tasks are the only programming model; threads are an execution resource.**

- Everything concurrent is a **task**: a stackful coroutine, implemented with native stack switching,
  JSPI in browsers and Node, and WASM stack switching on WASI hosts later
  ([`compiler.md`: WASM concurrency lowering](compiler.md#wasm-concurrency-lowering)). There is **no thread API in user code**, as with Go
  goroutines, Java virtual threads and Erlang processes.
- Scheduling is M:N: any task may run on any worker thread of its executor.
- The same code runs single-threaded (browser WASM), on a work-stealing pool, or pinned to a core.
  Nothing is marked async or sync (suspension is an effect, [Construction may suspend; drop may not](resources.md#construction-may-suspend-drop-may-not)).

**One runtime per process, many executors.** The runtime is the scheduler handler installed in
`main` (or `runtime-sim` in tests). Inside it, **executors** are pools of OS threads with a policy:

| Executor | Policy | Typical use |
|---|---|---|
| `default` | Work-stealing, one thread per core | Normal CPU and I/O tasks |
| `blocking` | Grows on demand | Blocking FFI and system calls (tokio's `spawn_blocking`; Go hands off its processor during syscalls) |
| `pinned(cores)` | One thread per core, no stealing, optional busy-polling and real-time priority | HFT hot path; shard-per-core designs (Seastar, Glommio) |
| `affine(main)` | One dedicated thread | Thread-affine resources: GUI main thread, GL contexts, C libraries with thread-local state |

- **Executors are values (decided November 2026),** built in `main` from the `Cpus` root
  capability, since making OS threads, pinning them and setting their priority are OS
  privileges ([Starting a program](#starting-a-program)). The deployment manifest may retune
  their core counts per deployment ([Distributed and multi-tier programs](distribution.md)).
- **NUMA placement (decided November 2026):** an executor can be placed on a NUMA node
  (`pool(&cpus, cores: node(1), …)`), and what its tasks allocate comes from that node's memory.
- **Idle executors lend their threads (decided November 2026):** a worker of an executor with no
  work may run another executor's ready tasks, at its own executor's priority, so no core sits
  idle while work waits. `pinned` executors neither lend nor borrow, keeping their latency and
  their caches.
- **Placement:** a scope or spawn chooses its executor; children inherit their parent's executor
  unless they override it. This is the same placement idea as GPU devices and tiers, one level down
  (thread → core → GPU → machine).

  ```rust
  scope(|s| { … }, on: pinned(2..4));
  s.spawn(|| legacy_c_lib.compress(buf), on: blocking);
  ```

- **A reactor per worker (decided November 2026):** each worker thread has its own io_uring ring or
  poller (epoll, kqueue, IOCP), so I/O does not contend on one shared reactor at many cores; a
  task's operations complete on its worker's reactor, and one moved by stealing re-registers. I/O
  is an effect, so there are no runtime-specific I/O types (as tokio's `TcpStream` is), no
  "cannot start a runtime from within a runtime", and no nested `block_on`.

**Where coroutines and threads genuinely differ:**

| Issue | Moddil's answer |
|---|---|
| A task blocks its OS thread | All I/O goes through effects, so safe code cannot block. FFI functions are declared `blocking` and run on the `blocking` executor automatically. |
| A long CPU loop never yields | **Decided:** preemption at compiler-inserted safe points at loop back-edges and function entries (a cheap flag check), as in Go 1.14+ and Erlang, guaranteed on `default` executors. `pinned` executors and `#[no_lock]`/`#[bounded]` hot paths can turn the checks off. The same safe points serve live update and cancellation. They are compiled in under every runtime and never fire under `runtime-sim`; a one-thread build and WASM leave them out ([Starting a program](#starting-a-program)). |
| Thread identity, thread-local storage | Tasks migrate, so safe code has no thread-local storage. Task-local context comes from effects and handlers. Thread-affine resources require the `affine` executor (a capability). |
| Stack memory | Native: large virtual reservations, committed lazily, with guard pages. Go-style stack copying is impossible because borrows point into frames. **wasm32 (decided):** small initial stacks (around 16 KiB) that grow in segments, never by copying; the task limit is documented, and programs needing very many tasks target wasm64 (memory64). Revisited once growable stacks are standardised in stack switching. |
| Biased RC after migration | See the task-biased proposal in [`memory.md`: Biased reference counting](memory.md#biased-reference-counting): migration costs nothing. `pinned` executors never migrate anyway. |
| Switch cost | Task switch: tens of nanoseconds. OS thread switch: microseconds. |
| Priorities, real time | **Decided:** priority lives on executors only (OS thread priority per pool); tasks within an executor are scheduled fairly, so latency-critical work gets its own executor. Moddil's `Mutex` uses priority inheritance across executors where the OS supports it. Guarantees of `pinned`: [`performance.md`: Real-time guarantees](performance.md#real-time-guarantees). |

**As built (November 2026):** the runtime is the single deterministic scheduler: one task runs at
a time, in the interpreters' order. Natively each task is an OS thread that waits for the baton
(`crates/rt`), and on WASM a JSPI call. The stackful coroutines on M:N workers described above
come with the parallel runtime (P38 in [`roadmap.md`: On the horizon](roadmap.md#on-the-horizon)).

**Targets:** on browser WASM without cross-origin isolation, all executors collapse onto one thread;
with it, Web Workers add threads. GPU devices act as executors for kernel tasks
([`accelerators.md`](accelerators.md)). Under `runtime-sim` the whole runtime is one deterministic
thread; executor placement is recorded and simulated ([`determinism.md`](determinism.md)).

## Targets

| Target | Runtime |
|---|---|
| Native (Linux, macOS, Windows) | `moddil/runtime`: work-stealing executors, a reactor per worker |
| WASM with stack switching (Wasmtime) | `moddil/runtime-wasm`; tasks are continuations ([`compiler.md`: WASM concurrency lowering](compiler.md#wasm-concurrency-lowering)) |
| Browsers (JSPI) | `moddil/runtime-wasm`; each task is a promising entry; one thread unless cross-origin isolated |
| Deterministic simulation | `moddil/runtime-sim`: one deterministic thread, virtual time ([`determinism.md`](determinism.md)) |
| Freestanding (embedded) | `moddil/rt-embedded`: a static scheduler with fixed task slots ([`freestanding.md`: Concurrency without a runtime](freestanding.md#concurrency-without-a-runtime)) |

## Prior art

Go (goroutines, M:N scheduling, asynchronous preemption since 1.14); Erlang/BEAM (reduction-counted
preemption); Java virtual threads; tokio, async-std, smol and the Rust executor split; Seastar and
Glommio (thread-per-core); io_uring; Wasmtime stack switching; JSPI.

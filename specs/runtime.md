# Runtime: scheduling, executors and I/O

> **Status:** complete: decided, with no open questions. Listed in the [index](README.md). The concurrency
> *model* (structured tasks, sharing, cancellation) is in [`concurrency.md`](concurrency.md) and
> [`lifecycle.md`](lifecycle.md); this document covers what runs it: runtimes, executors, the I/O reactor,
> and how they differ by target. Snippets are **illustration only**.

## Summary

| Rule | One line |
|---|---|
| **Runtimes are handlers** | Scheduling, I/O and time are core effects; runtimes are mods that handle them, so every library works on every runtime ([Pluggable runtimes without ecosystem splits](#pluggable-runtimes-without-ecosystem-splits)). |
| **Tasks, not threads** | Everything concurrent is a stackful task; threads are an execution resource with no user-facing API ([Tasks, threads and executors](#tasks-threads-and-executors)). |
| **Executors** | One runtime per process with several executors: `default`, `blocking`, `pinned(cores)`, `affine(main)`. |
| **One reactor** | A single I/O reactor (io_uring, epoll, kqueue, IOCP) shared by all executors; every operation is cancellable at the OS level. |
| **Obligations from the memory model** | Runtimes drain biased-RC merge queues, expose the task tree, and keep I/O cancel-safe ([Runtime contract](#runtime-contract)). |

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

The same mechanism generalises the pluggable traits that real projects build by hand. Hashiverse,
for example, has `TransportFactory`, `TimeProvider`, `ClientStorage` and `KeyLockerManager` traits plus
an in-memory transport for tests (see [`examples/04-hashiverse.md`](examples/04-hashiverse.md)). In
Moddil these are effects, and the test versions are handlers.

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
| A long CPU loop never yields | **Decided:** preemption at compiler-inserted safe points at loop back-edges and function entries (a cheap flag check), as in Go 1.14+ and Erlang, guaranteed on `default` executors. `pinned` executors and `#[no_lock]`/`#[bounded]` hot paths can turn the checks off. The same safe points serve live update and cancellation. |
| Thread identity, thread-local storage | Tasks migrate, so safe code has no thread-local storage. Task-local context comes from effects and handlers. Thread-affine resources require the `affine` executor (a capability). |
| Stack memory | Native: large virtual reservations, committed lazily, with guard pages. Go-style stack copying is impossible because borrows point into frames. **wasm32 (decided):** small initial stacks (around 16 KiB) that grow in segments, never by copying; the task limit is documented, and programs needing very many tasks target wasm64 (memory64). Revisited once growable stacks are standardised in stack switching. |
| Biased RC after migration | See the task-biased proposal in [`memory.md`: Biased reference counting](memory.md#biased-reference-counting): migration costs nothing. `pinned` executors never migrate anyway. |
| Switch cost | Task switch: tens of nanoseconds. OS thread switch: microseconds. |
| Priorities, real time | **Decided:** priority lives on executors only (OS thread priority per pool); tasks within an executor are scheduled fairly, so latency-critical work gets its own executor. Moddil's `Mutex` uses priority inheritance across executors where the OS supports it. Guarantees of `pinned`: [`performance.md`: Real-time guarantees](performance.md#real-time-guarantees). |

**Targets:** on browser WASM without cross-origin isolation, all executors collapse onto one thread;
with it, Web Workers add threads. GPU devices act as executors for kernel tasks
([`accelerators.md`](accelerators.md)). Under `runtime-sim` the whole runtime is one deterministic
thread; executor placement is recorded and simulated ([`determinism.md`](determinism.md)).

## Targets

| Target | Runtime |
|---|---|
| Native (Linux, macOS, Windows) | `moddil/runtime`: work-stealing executors, one reactor per process |
| WASM with stack switching (Wasmtime) | `moddil/runtime-wasm`; tasks are continuations ([`compiler.md`: WASM concurrency lowering](compiler.md#wasm-concurrency-lowering)) |
| Browsers (JSPI) | `moddil/runtime-wasm`; each task is a promising entry; one thread unless cross-origin isolated |
| Deterministic simulation | `moddil/runtime-sim`: one deterministic thread, virtual time ([`determinism.md`](determinism.md)) |
| Freestanding (embedded) | `moddil/rt-embedded`: a static scheduler with fixed task slots ([`freestanding.md`: Concurrency without a runtime](freestanding.md#concurrency-without-a-runtime)) |

## Open questions

None at present.


## Prior art

Go (goroutines, M:N scheduling, asynchronous preemption since 1.14); Erlang/BEAM (reduction-counted
preemption); Java virtual threads; tokio, async-std, smol and the Rust executor split; Seastar and
Glommio (thread-per-core); io_uring; Wasmtime stack switching; JSPI.

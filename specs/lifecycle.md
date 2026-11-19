# Program Lifecycle

> **Status:** complete: decided, with no open questions. Listed in the [index](README.md). Snippets are
> **illustration only**.

## Summary

| Rule | One line |
|---|---|
| **No life before `main`** | No global constructors; static data is evaluated at compile time. |
| **`main` declares its root capabilities** | `fn main(args: Args, net: NetRoot, fs: FsRoot)`: exactly what the program uses, and the runtime drops the rest at startup. |
| **Conventional exits** | `main` returns `()` or `ExitCode`; traps exit with 70, a second signal with 128+n; no ambient `exit()`. |
| **Signals are values** | SIGINT and SIGTERM become a graceful stop; other signals arrive as a typed stream. |
| **Typed, layered configuration** | One `#[derive(Config)]` struct filled from defaults, a file, the environment and the command line, validated at startup. |
| **Traps end the process, unless a scope is isolated** | An isolated scope unwinds on a trap and reports `Trapped` to its parent. |
| **No mutable globals** | Process-wide state lives in values created in `main`, or in context effects. |

## Problem

- **Ambient authority from the first instruction**: in C, Rust and Go, `main` can open any file or
  socket, so nothing tells a reader or the OS what a program actually needs.
- **Life before `main`**: C++ global constructors run in an unspecified order (the "static
  initialisation order fiasco"); Rust's lazy statics hide initialisation and shared state.
- **`exit()` skips cleanup**: Go's `os.Exit` and C's `exit` bypass deferred work and destructors.
- **Configuration is scattered** across arguments, environment variables and files, parsed by hand
  and validated late, if at all.
- **What a crash ends is unclear**: some runtimes kill the process, some the thread, some let
  `recover` swallow bugs.

## State of the art

| Topic | Approaches |
|---|---|
| **Entry and authority** | **C, Rust, Go**: `main` has full ambient authority. **WASI**: directories and sockets are pre-opened and handed in. **OpenBSD `pledge`/`unveil`**, **Linux Landlock and seccomp**: a process gives up privileges it doesn't need. **Deno**: `--allow-*` flags. |
| **Exit** | **Rust**: `main` returns `Result` or `ExitCode`; a panic exits with 101. **Go**: `os.Exit` skips defers. Conventions: 128+n for death by signal n; `sysexits.h` (64 usage, 70 internal software error, 78 configuration). |
| **Configuration** | **Rust `clap` derive**: the type is the command line. **figment**, **viper**: layered sources. **12-factor**: configuration from the environment. |
| **Crash scope** | **Rust**: panics unwind (or abort), `catch_unwind`, `Mutex` poisoning. **Erlang**: a crashed process is isolated and a supervisor restarts it. **Midori**: abandonment of the failed process. **Go**: a panic kills the process unless recovered. **Swift**: traps always kill the process. |
| **Global state** | **C++**: static initialisation order. **Rust**: `static`, `LazyLock`, `OnceLock`. **Zig**: comptime-known globals. **Erlang**, **Pony**: no mutable globals. |

## Decisions

### No life before `main`

- There are **no global constructors**. Static data is evaluated at compile time
  ([`comptime.md`](comptime.md)), so nothing runs before `main`.
- In freestanding profiles, `#[entry]` replaces `main`
  ([`freestanding.md`](freestanding.md)).

### `main` declares its root capabilities

`main`'s parameters list **exactly the root capabilities the program uses**; the runtime supplies
them.

```rust
fn main(args: Args, env: Env, stdio: Stdio, files: FsRoot, net: NetRoot, clock: Clock) -> ExitCode {
    with Stderr = stdio.err {
        let cfg = try { ServerConfig::load(&args, &env, &files)? } catch {
            e => { eprintln(fmt"{e}"); return ExitCode::Config }
        };
        with Net = net, Fs = files.read_only(cfg.data_dir), Time = clock {
            server::run(cfg)
        }
    }
    ExitCode::Ok
}
```

| Root capability | Grants |
|---|---|
| `Args`, `Env` | Command-line arguments; environment variables |
| `Stdio` | Standard input, output and error (handlers for `Stdin`, `Stdout` and `Stderr`) |
| `FsRoot` | The file system, narrowed with `read_only`, `only(path)` |
| `NetRoot` | Network access, narrowed by address or port |
| `Clock`, `Entropy` | Real time; OS randomness (handlers for `Time` and `Rand`) |
| `ProcessRoot` | Spawning processes; `exit_now` |
| `Signals` | Signals beyond the stop signals |
| `Devices` | GPUs and accelerators ([`accelerators.md`](accelerators.md)) |
| `Introspect` | Live introspection ([`observability.md`](observability.md)) |

- Nothing writes to the terminal ambiently: `print` and `println` perform `Stdout`, `eprintln`
  performs `Stderr`, and both are handled from `Stdio`. (The write-only `log` effect is separate.)
- Root capabilities are handlers, or build them
  ([`effects-and-capabilities.md`: Authority comes only from root capabilities](effects-and-capabilities.md#authority-comes-only-from-root-capabilities)).
- **Startup sandboxing.** The executable's root set is known at compile time, so the runtime
  **drops every OS privilege that wasn't requested** before `main` runs: Landlock and seccomp on
  Linux, `pledge`/`unveil` on OpenBSD, pre-opens on WASI, and the best available mechanism elsewhere.
  What could and couldn't be enforced is reported.
- `moddil audit` shows each executable's root set, and a change to it appears in the semantic diff.
- Tests and simulations call the same code with **simulated** roots
  ([`determinism.md`](determinism.md)).

### Conventional exits

| Exit | Code |
|---|---|
| `main` returns `()` or `ExitCode::Ok` | 0 |
| `main` returns another `ExitCode` | That code. Every `sysexits.h` code is named: `Usage` 64, `DataErr` 65, `NoInput` 66, `NoUser` 67, `NoHost` 68, `Unavailable` 69, `Software` 70, `OsErr` 71, `OsFile` 72, `CantCreate` 73, `IoErr` 74, `TempFail` 75, `Protocol` 76, `NoPerm` 77, `Config` 78; `ExitCode::new(n)` for anything else |
| A trap | 70 (`EX_SOFTWARE`), after printing the trace |
| A second stop signal | 128 + n |

- R3 makes every error at the entry point a compile-time error to leave unhandled
  ([`requirements.md`](requirements.md)), so `main` decides how each one becomes an exit code.
- There is **no ambient `exit()`**. `exit_now(code)` exists only on `ProcessRoot`; it skips cleanup,
  so it shows up in the root set and in review.

### Signals are values

- SIGINT and SIGTERM (and Windows Ctrl-C and close events) become a **stop** on `main`'s root scope,
  escalating to cancel after the grace deadline; a second signal exits immediately
  ([Cancellation and shutdown](#cancellation-and-shutdown)).
- **Other signals** (SIGHUP, SIGUSR1, SIGUSR2) arrive as a typed stream from `Signals`, which a task
  can `select` on. There are never asynchronous signal handlers.

### Typed, layered configuration

```rust
#[derive(Config)]
struct ServerConfig {
    /// Port to listen on.
    port: Port = 8080,
    data_dir: Path,
    /// Database password; from the environment or a file only.
    db_password: Secret<String>,
    max_conns: u32 where self >= 1 = 1000,
}
```

- **Layers**, later ones winning: field defaults → a configuration file → environment variables →
  command-line arguments.
- Values are **validated into refined types** at startup (parse, don't validate:
  [`data.md`](data.md)). **Unknown keys are rejected**, since a configuration file is an untyped edge
  ([`data.md`: Unknown fields depend on the boundary](data.md#unknown-fields-depend-on-the-boundary)).
- `--help`, environment variable names (`SERVER_PORT`) and the documentation come from the struct and
  its doc comments.
- **`Secret<T>` fields** are read only from the environment or files, never from arguments (which
  other processes can see), and never print
  ([`observability.md`: Secrets never reach observability](observability.md#secrets-never-reach-observability)).
- Bad arguments exit with `Usage`; bad configuration with `Config`.
- The configuration is passed down as a value, or provided as context data
  (`with ServerConfig = cfg { … }`, [`effects-and-capabilities.md`: Context data](effects-and-capabilities.md#context-data)).

### Traps: the process by default, isolated scopes on request

- A trap (a bug: overflow, out-of-bounds, a failed contract) **terminates the process by default**,
  with a trace; the orchestrator restarts it ([`error-handling.md`](error-handling.md)).
- A scope can opt into **isolation**, typically for servers handling independent requests:

```rust
scope(|s| {
    for conn in listener.incoming() {
        s.spawn(|| handle(conn))            // a bug in one request doesn't kill the server
    }
}, isolate: true)?                                         // fail(Trapped) if a task inside trapped
```

- When a task in an isolated scope traps:
  - the scope's tasks are cancelled and **unwound**, running their drops (which are infallible
    and never suspend, [`resources.md`: Destructors](resources.md#destructors));
  - **sync cells** held at the time are **poisoned**; later access fails with `Poisoned`;
  - **linear values** are discarded with their `abandon` operation
    ([`resources.md`: Resources: affine and linear](resources.md#resources-affine-and-linear));
  - the parent receives **`fail(Trapped)`** with the trace.
- A trap inside `drop` or `abandon` still terminates the process. Shared rc data is immutable, so a
  trap cannot leave it half-updated.

### No mutable globals

- Globals are **comptime constants only**.
- Process-wide state (configuration, caches, connection pools, metric registries) lives in values
  created in `main`, or in context effects.
- So there are no initialisation-order problems, no hidden shared state, and simulations stay
  deterministic. Sync cells exist only inside values ([`concurrency.md`](concurrency.md)).

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
  - drops, `defer` and `errdefer` run ([`resources.md`: When drops run](resources.md#when-drops-run)–[`resources.md`: `defer` and `errdefer`](resources.md#defer-and-errdefer));
  - **linear values must be consumed on it**, exactly as on error paths, typically with
    `errdefer txn.rollback()`;
  - cleanup on this path **may suspend** (a rollback needs the network) inside a **shielded,
    deadline-bounded scope**: it is not itself cancelled midway, and it cannot hang for ever.
- `Cancelled` is a distinct outcome, not an error ([`error-handling.md`: Tasks and concurrency](error-handling.md#tasks-and-concurrency));
  it is accounted for at the join.
- **Long pure-compute loops** have no suspension points. They call a cheap `checkpoint()` (a
  suspension point that observes cancellation); a lint flags long loops in cancellable tasks without
  one. Preemption safe points ([Tasks, threads and executors](runtime.md#tasks-threads-and-executors)) only yield the thread; they never deliver cancellation, because
  that would interrupt code between effect operations.
- **Blocking FFI calls** cannot be interrupted. They run on the `blocking` executor ([Tasks, threads and executors](runtime.md#tasks-threads-and-executors)); the scope
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
fn main(net: NetRoot) {                                    // declares only the roots it uses
    scope(|services| {                                     // layer 1 (outer)
        services.spawn(|| db_writer());
        services.spawn(|| dht.maintain());

        scope(|app| {                                      // layer 2 (inner)
            app.spawn(|| server.serve());
        });
        // layer 2 has fully finished; only now do db_writer and dht see stop_requested()
    }, stop_on: [SIGINT, SIGTERM], grace: 10[s]);
}

fn serve() with Net, suspend, spawn {
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
   `read_line`, `recv` and timers are effect operations handled by the runtime's reactor ([Tasks, threads and executors](runtime.md#tasks-threads-and-executors)),
   never blocking system calls on a thread. A stop that wins a `select`, or a hard cancel, removes
   the pending operation and the task continues **immediately**.
2. **The runtime cancels at the OS level** ([Runtime contract](runtime.md#runtime-contract)): io_uring cancel requests, removing interest from
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
fn repl() with stdin: Stdin, Stdout, suspend {
    let input = stdin.lines();
    loop {
        select {
            line = input.next()  => eval(line?),
            _ = stop_requested() => break,      // Ctrl-C: the pending read is cancelled; no Enter needed
        }
    }
}

fn serve(listener: &Listener) with Net, suspend, spawn {
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

## Fit check against the reference examples

| Example | Uses |
|---|---|
| [Multiplayer game](examples/01-multiplayer-game.md) | The server declares `NetRoot` and `Clock` only; the browser client gets its roots from the WASM host |
| [HFT](examples/02-hft-platform.md) | Typed configuration with risk limits as refined types; traps kill the process (no partial trading state) |
| [Web store](examples/03-web-store.md) | Isolated scopes per request; secrets from the environment; layered configuration per environment |
| [Hashiverse](examples/04-hashiverse.md) | Volunteer binaries sandboxed to the roots they declare; layered shutdown on Ctrl-C |

## Resolved questions

Decided in the open-question review:

1. **Sandboxing follows narrowing.** Narrowing a root capability after start
   (`files.read_only(cfg.data_dir)`) also tightens the OS sandbox (adding Landlock rules, calling
   `unveil`) once the wider root is no longer held anywhere. This happens wherever the OS allows
   tightening after start, and the final enforced state is reported.
2. **Supervision is a library:** `supervise(|s| …, restart: Backoff { … }, max: 5 per 1[min])` in
   `moddil/sync`, built on isolated scopes ([`concurrency.md`](concurrency.md#open-questions)).
3. **Configuration is a snapshot by default.** A program that wants reloading holds `Live<Config>`,
   refreshed by a task that `select`s on SIGHUP from `Signals` and re-validates; readers take a
   consistent snapshot per request, and a failed reload keeps the old value and logs.
4. **Windows** uses the same exit codes as other platforms (Ctrl-C gives 0xC000013A, as Windows
   expects). A trap prints the same trace to stderr and exits 70; Windows Error Reporting dialogs are
   suppressed so services never hang on one, and crash dumps are opt-in.
5. **Arguments:** the main path is `#[derive(Args)] struct Cli { port: Port = 8080, verbose: bool }`,
   parsed with `Cli::parse(&args)?`. It shares its machinery with `#[derive(Config)]`, giving
   `--help`, subcommands from enums and refined-type validation. Small tools can read one typed
   flag: `args.value::<Port>("port", default: 8080)?`.
6. **Exit codes:** the whole `sysexits.h` set is named ([Conventional exits](#conventional-exits)).

## Open questions

None at present.

## Prior art

WASI pre-opens; OpenBSD `pledge` and `unveil`; Linux Landlock and seccomp; Deno permissions; Rust
`ExitCode`, panics, unwinding and `Mutex` poisoning; `sysexits.h`; clap, figment and viper; the
twelve-factor app; Erlang/OTP supervisors; Midori (Joe Duffy, *The Error Model*); Swift traps.

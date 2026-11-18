# Program Lifecycle

> **Status:** decided direction, details open. Listed in the [index](README.md). Snippets are
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
fn main(args: Args, env: Env, files: FsRoot, net: NetRoot, clock: Clock) -> ExitCode {
    let cfg = match ServerConfig::load(&args, &env, &files) {
        Ok(cfg) => cfg,
        Err(e)  => { eprintln(e); return ExitCode::Config }
    }
    with net = net, fs = files.read_only(cfg.data_dir), time = clock {
        server::run(cfg)
    }
    ExitCode::Ok
}
```

| Root capability | Grants |
|---|---|
| `Args`, `Env` | Command-line arguments; environment variables |
| `Stdio` | Standard input, output and error |
| `FsRoot` | The file system, narrowed with `read_only`, `only(path)` |
| `NetRoot` | Network access, narrowed by address or port |
| `Clock`, `Entropy` | Real time; OS randomness (handlers for `time` and `rand`) |
| `ProcessRoot` | Spawning processes; `exit_now` |
| `Signals` | Signals beyond the stop signals |
| `Devices` | GPUs and accelerators ([`accelerators.md`](accelerators.md)) |
| `Introspect` | Live introspection ([`observability.md`](observability.md)) |

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
| `main` returns another `ExitCode` | That code; named `Usage` (64) and `Config` (78) follow `sysexits.h` |
| A trap | 70 (`EX_SOFTWARE`), after printing the trace |
| A second stop signal | 128 + n |

- R3 makes every error at the entry point a compile-time error to leave unhandled
  ([`requirements.md`](requirements.md)), so `main` decides how each one becomes an exit code.
- There is **no ambient `exit()`**. `exit_now(code)` exists only on `ProcessRoot`; it skips cleanup,
  so it shows up in the root set and in review.

### Signals are values

- SIGINT and SIGTERM (and Windows Ctrl-C and close events) become a **stop** on `main`'s root scope,
  escalating to cancel after the grace deadline; a second signal exits immediately
  ([`concurrency.md`: Cancellation and shutdown](concurrency.md#cancellation-and-shutdown)).
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
- The configuration is passed down as a value, or provided as a context effect
  ([`effects-and-capabilities.md`](effects-and-capabilities.md)).

### Traps: the process by default, isolated scopes on request

- A trap (a bug: overflow, out-of-bounds, a failed contract) **terminates the process by default**,
  with a trace; the orchestrator restarts it ([`error-handling.md`](error-handling.md)).
- A scope can opt into **isolation**, typically for servers handling independent requests:

```rust
scope(isolate: true, |s| {
    for conn in listener.incoming() {
        s.spawn(|| handle(conn))            // a bug in one request doesn't kill the server
    }
})?                                         // fail(Trapped) if a task inside trapped
```

- When a task in an isolated scope traps:
  - the scope's tasks are cancelled and **unwound**, running their drops (which are infallible
    and never suspend, [`memory-and-resources.md`: Destructors](memory-and-resources.md#destructors));
  - **sync cells** held at the time are **poisoned**; later access fails with `Poisoned`;
  - **linear values** are discarded with their `abandon` operation
    ([`memory-and-resources.md`: Resources: affine and linear](memory-and-resources.md#resources-affine-and-linear));
  - the parent receives **`fail(Trapped)`** with the trace.
- A trap inside `drop` or `abandon` still terminates the process. Shared rc data is immutable, so a
  trap cannot leave it half-updated.

### No mutable globals

- Globals are **comptime constants only**.
- Process-wide state (configuration, caches, connection pools, metric registries) lives in values
  created in `main`, or in context effects.
- So there are no initialisation-order problems, no hidden shared state, and simulations stay
  deterministic. Sync cells exist only inside values ([`concurrency.md`](concurrency.md)).

## Fit check against the reference examples

| Example | Uses |
|---|---|
| [Multiplayer game](examples/01-multiplayer-game.md) | The server declares `NetRoot` and `Clock` only; the browser client gets its roots from the WASM host |
| [HFT](examples/02-hft-platform.md) | Typed configuration with risk limits as refined types; traps kill the process (no partial trading state) |
| [Web store](examples/03-web-store.md) | Isolated scopes per request; secrets from the environment; layered configuration per environment |
| [Hashiverse](examples/04-hashiverse.md) | Volunteer binaries sandboxed to the roots they declare; layered shutdown on Ctrl-C |

## Open questions

- How fine-grained can startup sandboxing be when paths come from configuration at runtime (sandbox
  again after loading configuration)?
- Should isolated scopes offer restart policies (supervision), or leave that to a library?
- Is configuration a snapshot, or a live value reloaded on SIGHUP (`AtomicRc<Config>`)?
- Exit codes and trap reporting on Windows.

## Prior art

WASI pre-opens; OpenBSD `pledge` and `unveil`; Linux Landlock and seccomp; Deno permissions; Rust
`ExitCode`, panics, unwinding and `Mutex` poisoning; `sysexits.h`; clap, figment and viper; the
twelve-factor app; Erlang/OTP supervisors; Midori (Joe Duffy, *The Error Model*); Swift traps.

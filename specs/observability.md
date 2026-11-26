# Observability and Debuggability

Builds on effects and handlers ([Effects and capabilities as a unifying idea](effects-and-capabilities.md), R2), structured concurrency ([`concurrency.md`](concurrency.md)), error
traces ([`error-handling.md`: Traces](error-handling.md#traces)), record and replay
([`determinism.md`: Record and replay](determinism.md#record-and-replay)) and comptime literals ([`comptime.md`: Tagged string literals (embedded DSLs)](comptime.md#tagged-string-literals-embedded-dsls)).

## Summary

| Rule | One line |
|---|---|
| **Write-only ambient effects** | `log`, `span` and `metric` are always available and never appear in signatures. Handlers are pluggable mods and cannot feed values back, so observing never changes results. |
| **Compile-time interned events** | Format strings are checked and interned at build time; the hot path writes an event ID plus typed arguments. Text is rendered later. |
| **Spans follow the task tree** | Every scope and task is a span. Trace context crosses `Remote` calls automatically. |
| **Secrets never reach observability** | A `Secret<T>` value cannot flow into any event: compile error. |
| **Task-aware stacks** | Traces show the logical chain of frames and spawn sites; a live view shows what each task is blocked on. |
| **Flight recorder + replay** | A bounded production recording of effect results is dumped on failure and replayed deterministically, including stepping backwards ([`determinism.md`: Record and replay](determinism.md#record-and-replay)). |
| **Live introspection** | Attach to a running process, inspect tasks and enable tracing dynamically, behind an `Introspect` capability. |
| **Release builds stay debuggable** | Full debug info is always produced, stored separately and keyed by build hash. |

## Observability effects

`log`, `span` and `metric` are effects with **handlers provided by mods** ([R2](requirements.md#firm-requirements)):
stdout/pretty-printing, OTLP export, Prometheus, a ring buffer, test capture, the simulation runtime.
The handler is chosen in `main` or the deployment manifest. There is no global logger, and libraries
never choose where their events go.

**Write-only, so ambient.** Unlike other effects, these three do not appear in signatures:
- their operations return nothing, and a handler **cannot resume with a value**, so a handler cannot
  change what the program computes;
- therefore emitting an event does not affect results, purity or determinism, and showing it in
  every signature would be noise;
- they are still effects underneath, so handlers can be swapped, scoped and tested.

Where they run:
- **Comptime** ([`comptime.md`](comptime.md)): events become compiler diagnostics or build logs.
- **Kernels** ([`accelerators.md`](accelerators.md)): handler-specific, e.g. a device-side buffer
  read back at the join; may be compiled out.
- **Levels and categories** are comptime configuration: disabled events are compiled out entirely.

## Compile-time interned events

```
log.info(fmt"order {order.id} filled {qty} @ {price}")
metric.count("orders.filled", 1, venue: order.venue)
```

- **Events have stable names:** the module path plus an optional `name:`, with typed
  fields. Adding a field is free. Renaming or removing a field, or changing its type, is reported by
  the semantic diff ([`semantic-vcs.md`](semantic-vcs.md)): a warning, or a breaking change for events
  marked `#[stable]`, which dashboards and alerts depend on.
- The `fmt"…"` literal is checked at build time (argument types, field names) and **interned**: the
  build records a table of event IDs → format strings, keyed by the content-addressed build hash
  ([Build times](build-times.md)).
- On the hot path an event writes only its ID and typed arguments into a per-task or per-core
  buffer, as NanoLog and defmt do. No string formatting happens in the program.
- Text is rendered later by the handler or by offline tools using the string table.
- Every event is **structured**: fields can be queried without parsing text.

## Spans and context propagation

- **Every scope and task is a span** automatically. The structured-concurrency tree is the span tree,
  so parent/child relationships need no manual context passing.
- Users add spans for logical operations (`span("checkout") { … }`).
- **Span fields carry request-scoped data** such as a request ID: `span("request", id: rid) { … }`.
  Every log line and trace inside carries it, and it never appears in a signature
  ([`effects-and-capabilities.md`: Context data](effects-and-capabilities.md#context-data)).
- **GPU launches are spans** ([`accelerators.md`: Launches are structured tasks](accelerators.md#launches-are-structured-tasks)).
- **Calls through the `Remote` effect** ([Distributed and multi-tier programs](distribution.md)) carry W3C Trace Context automatically, so a
  trace follows a request from browser to API to worker.
- Metrics: explicit counters, gauges and histograms, plus rate/error/duration metrics derivable from
  spans.

## Secrets never reach observability

- `Secret<T>` is the information-flow marking from [Distributed and multi-tier programs](distribution.md) (secrets cannot flow to
  `client`).
- A `Secret<T>` value **cannot flow into the arguments of any observability event**. This is a compile
  error, including when the secret is nested inside a struct or an error payload being logged.
- Explicit projections are allowed and visible: `secret.redacted()`, `secret.hash_for_correlation()`.
- **`Personal<T>`** (personal data) follows the same rule ([`security.md`: Information-flow labels](security.md#information-flow-labels)).
- Reviewers no longer need to check every log line for leaked credentials or personal data.

## Task-aware stacks and the task tree

- **Logical stacks**: a trace shows frames *and* the chain of spawn sites back to the root scope,
  joined with error return traces ([`error-handling.md`: Traces](error-handling.md#traces)).
- **The live task tree**: every scope, its child tasks, and **which effect operation each task is
  blocked on** (network, a lock, a channel, a GPU join, a timer), with how long it has waited.
- **Deadlock reports** come from the same data: a cycle of tasks blocked on each other's locks or
  channels.
- Runtime handler mods expose this data ([`runtime.md`: Runtime contract](runtime.md#runtime-contract)).

## Flight recorder and replay

Record and replay itself is defined in [`determinism.md`: Record and replay](determinism.md#record-and-replay): every source of
nondeterminism is an effect, so recording effect results is enough to replay a run exactly. This
section covers how observability uses it.

- The **flight recorder** is a recorder handler running in production with a **bounded ring
  buffer**, opt-in per scope or task and optionally sampled.
- On an error or trap, it dumps its buffer alongside the error return trace and the task tree
  ([Task-aware stacks and the task tree](#task-aware-stacks-and-the-task-tree)). That dump is a replayable recording of the moments before the failure.
- The **simulation runtime** replays the dump; debuggers can step forwards and backwards through it
  (time-travel debugging).
- Recordings link to traces and logs by span ID, so an engineer or agent can jump from a failing
  span to its replay.

Limits:
- Recording I/O can be large; volume and retention are configurable.
- Code outside the effect boundary (FFI, `unsafe`, code performing `nondet`) is not
  replayable unless wrapped by effects; the recorder flags it.
- **Secrets in recordings:** effect results typed `Secret<T>` or `Personal<T>` are stored
  **encrypted** with a per-deployment key, so the rest of a recording can be shared freely. Replaying
  them needs the **`ReplayKey`** capability; without it, replay substitutes placeholders and reports
  where the run diverged. Retention is configuration
  ([`determinism.md`: Record and replay](determinism.md#record-and-replay)).
- **Sampling:** a deterministic head decision from a hash of the trace ID, propagated with
  the trace context, so a trace is kept or dropped as a whole. Every task also buffers its spans in
  its ring buffer, and a trace that errors, traps or exceeds a latency threshold is kept in full
  anyway (tail rescue). Recordings follow the same decision.
- **Fleet replay:** each process replays on its own from its own recording; other
  processes' replies are recorded `Net` and `Remote` results. Recordings are linked by trace, span
  and `Remote` call IDs, so tools follow a request across processes and replay the chain in causal
  order. Moddil peers can also be loaded together into one simulation with their recorded external
  inputs ([`determinism.md`: Deterministic simulation testing](determinism.md#deterministic-simulation-testing)).

## Live introspection

Behind an **`Introspect` capability**, granted by `main` or the manifest and never ambient:
- attach to a running process, locally or remotely;
- view the task tree ([Task-aware stacks and the task tree](#task-aware-stacks-and-the-task-tree)) and per-task flight-recorder buffers;
- enable or disable spans and log levels for a module **without restarting**, as BEAM allows;
- take heap and task snapshots;
- use probe points (USDT-style) that cost nothing while disabled.

In the browser this maps onto DevTools.

**Continuous profiling** is a handler mod enabled through `Introspect`. It combines CPU
sampling with the runtime's record of which effect operation each task is blocked on, so on-CPU and
off-CPU time is attributed per task, per span and per effect (`Net`, `Db`, locks, GPU). It exports
pprof and OTLP profiles, and costs nothing while disabled.

## Overhead budgets

These are budgets, tracked in CI like the others
([`performance.md`: Benchmarks and budgets](performance.md#benchmarks-and-budgets)):

| Level | Cost on the hot path |
|---|---|
| Compiled out (comptime level) | 0 |
| Disabled at run time | One predictable branch |
| Enabled interned event | ≤ 10 ns |
| Span | ≤ 50 ns |
| Flight recorder | ≤ 5% throughput on the benchmark suite |

## Release builds stay debuggable

- Full debug info is always produced, including for optimised builds, stored separately and served
  from a **symbol store keyed by build hash**.
- Debug and release builds share the same semantics ([Build times](build-times.md)), so behaviour cannot differ
  between them, only performance and the quality of variable locations.
- Optimised code keeps best-effort variable locations and inlined-frame information.
- WASM builds carry DWARF and source maps.

## For agents

The task tree, recordings, replay and debugger are exposed as a **structured API**, part of the
compiler-as-a-service ([LLMs as first-class programmers](agent-interface.md)). An agent can ask: "replay incident 4711 and show the inputs
of the task that failed", or "which tasks have been blocked on the database for over a second?".

## Fit check against the reference examples

| Example | Uses |
|---|---|
| [Multiplayer game](examples/multiplayer-game.md) | Replay of a desynchronised match from the server's recording; spans across client, hub and chat |
| [HFT](examples/hft-platform.md) | Interned logs on the tick-to-trade path; bit-for-bit replay of a trading day; latency histograms |
| [Web store](examples/web-store.md) | OTLP traces across client, API and database; payment keys and personal data blocked from logs by type |
| [Hashiverse](examples/hashiverse.md) | Opt-in live introspection of a volunteer node; replay of P2P incidents in simulation |

## Background

The problems this design answers.


- **Release builds can't be debugged**: variables are "optimised out", inlined frames disappear, and
  debug and release builds sometimes behave differently.
- **Async stacks are unreadable**: you see executor frames, not who awaited whom.
- **Tracing is bolted on**: trace IDs are threaded by hand, or kept in thread-locals that break
  across suspension points; instrumentation relies on agents or monkey-patching.
- **Logging is slow and stringly typed**: formatting on the hot path, levels checked at run time, a
  global logger that libraries fight over.
- **Secrets end up in logs**, caught only by luck in review.
- **Production bugs can't be reproduced**: logs are the only evidence.

## Prior art

OpenTelemetry; W3C Trace Context; Rust `tracing`; Go `log/slog`, pprof and the execution tracer;
Java Flight Recorder; .NET EventPipe; BEAM observer, `dbg` and recon; defmt; Yang et al., *NanoLog*
(USENIX ATC 2018); rr; Pernosco; WinDbg Time Travel Debugging; tokio-console; Swift async
backtraces; Zig error return traces; eBPF, USDT and DTrace; Parca and Pyroscope; Sentry.

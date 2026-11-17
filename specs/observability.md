# Observability and Debuggability

> **Status:** decided direction, details open. Expands [`rationale.md`](rationale.md) §2.12. Builds on
> effects and handlers (§2.1, R2), structured concurrency ([`concurrency.md`](concurrency.md)), error
> traces ([`error-handling.md`](error-handling.md) §6), record and replay
> ([`determinism.md`](determinism.md) §5) and comptime literals ([`comptime.md`](comptime.md) §8).
> Snippets are **illustration only**.

## 1. Summary

| Rule | One line |
|---|---|
| **Write-only ambient effects** | `log`, `span` and `metric` are always available and never appear in signatures. Handlers are pluggable mods and cannot feed values back, so observing never changes results. |
| **Compile-time interned events** | Format strings are checked and interned at build time; the hot path writes an event ID plus typed arguments. Text is rendered later. |
| **Spans follow the task tree** | Every scope and task is a span. Trace context crosses `remote` calls automatically. |
| **Secrets never reach observability** | A `Secret<T>` value cannot flow into any event: compile error. |
| **Task-aware stacks** | Traces show the logical chain of frames and spawn sites; a live view shows what each task is blocked on. |
| **Flight recorder + replay** | A bounded production recording of effect results is dumped on failure and replayed deterministically, including stepping backwards ([`determinism.md`](determinism.md) §5). |
| **Live introspection** | Attach to a running process, inspect tasks and enable tracing dynamically, behind an `introspect` capability. |
| **Release builds stay debuggable** | Full debug info is always produced, stored separately and keyed by build hash. |

## 2. Problems

- **Release builds can't be debugged**: variables are "optimised out", inlined frames disappear, and
  debug and release builds sometimes behave differently.
- **Async stacks are unreadable**: you see executor frames, not who awaited whom.
- **Tracing is bolted on**: trace IDs are threaded by hand, or kept in thread-locals that break
  across suspension points; instrumentation relies on agents or monkey-patching.
- **Logging is slow and stringly typed**: formatting on the hot path, levels checked at run time, a
  global logger that libraries fight over.
- **Secrets end up in logs**, caught only by luck in review.
- **Production bugs can't be reproduced**: logs are the only evidence.

## 3. Observability effects

`log`, `span` and `metric` are effects with **handlers provided by mods** (rationale R2):
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

## 4. Compile-time interned events

```
log.info(fmt"order {order.id} filled {qty} @ {price}")
metric.count("orders.filled", 1, venue: order.venue)
```

- The `fmt"…"` literal is checked at build time (argument types, field names) and **interned**: the
  build records a table of event IDs → format strings, keyed by the content-addressed build hash
  (rationale §2.14).
- On the hot path an event writes only its ID and typed arguments into a per-task or per-core
  buffer, as NanoLog and defmt do. No string formatting happens in the program.
- Text is rendered later by the handler or by offline tools using the string table.
- Every event is **structured**: fields can be queried without parsing text.

## 5. Spans and context propagation

- **Every scope and task is a span** automatically. The structured-concurrency tree is the span tree,
  so parent/child relationships need no manual context passing.
- Users add spans for logical operations (`span("checkout") { … }`).
- **GPU launches are spans** ([`accelerators.md`](accelerators.md) §6).
- **Calls through the `remote` effect** (rationale §2.10) carry W3C Trace Context automatically, so a
  trace follows a request from browser to API to worker.
- Metrics: explicit counters, gauges and histograms, plus rate/error/duration metrics derivable from
  spans.

## 6. Secrets never reach observability

- `Secret<T>` is the information-flow marking from rationale §2.10 (secrets cannot flow to
  `@client`).
- A `Secret<T>` value **cannot flow into the arguments of any observability event**. This is a compile
  error, including when the secret is nested inside a struct or an error payload being logged.
- Explicit projections are allowed and visible: `secret.redacted()`, `secret.hash_for_correlation()`.
- Reviewers no longer need to check every log line for leaked credentials or personal data.

## 7. Task-aware stacks and the task tree

- **Logical stacks**: a trace shows frames *and* the chain of spawn sites back to the root scope,
  joined with error return traces ([`error-handling.md`](error-handling.md) §6).
- **The live task tree**: every scope, its child tasks, and **which effect operation each task is
  blocked on** (network, a lock, a channel, a GPU join, a timer), with how long it has waited.
- **Deadlock reports** come from the same data: a cycle of tasks blocked on each other's locks or
  channels.
- Runtime handler mods expose this data ([`concurrency.md`](concurrency.md) §6).

## 8. Flight recorder and replay

Record and replay itself is defined in [`determinism.md`](determinism.md) §5: every source of
nondeterminism is an effect, so recording effect results is enough to replay a run exactly. This
section covers how observability uses it.

- The **flight recorder** is a recorder handler running in production with a **bounded ring
  buffer**, opt-in per scope or task and optionally sampled.
- On an error or trap, it dumps its buffer alongside the error return trace and the task tree
  (§7). That dump is a replayable recording of the moments before the failure.
- The **simulation runtime** replays the dump; debuggers can step forwards and backwards through it
  (time-travel debugging).
- Recordings link to traces and logs by span ID, so an engineer or agent can jump from a failing
  span to its replay.

Limits:
- Recording I/O can be large; volume and retention are configurable.
- Code outside the effect boundary (FFI, `unsafe`, explicitly nondeterministic maths) is not
  replayable unless wrapped by effects; the recorder flags it.
- Recordings may contain sensitive data; how secrets are excluded or encrypted is open (§13).

## 9. Live introspection

Behind an **`introspect` capability**, granted by `main` or the manifest and never ambient:
- attach to a running process, locally or remotely;
- view the task tree (§7) and per-task flight-recorder buffers;
- enable or disable spans and log levels for a module **without restarting**, as BEAM allows;
- take heap and task snapshots;
- use probe points (USDT-style) that cost nothing while disabled.

In the browser this maps onto DevTools.

## 10. Release builds stay debuggable

- Full debug info is always produced, including for optimised builds, stored separately and served
  from a **symbol store keyed by build hash**.
- Debug and release builds share the same semantics (rationale §2.14), so behaviour cannot differ
  between them, only performance and the quality of variable locations.
- Optimised code keeps best-effort variable locations and inlined-frame information.
- WASM builds carry DWARF and source maps.

## 11. For agents

The task tree, recordings, replay and debugger are exposed as a **structured API**, part of the
compiler-as-a-service (rationale §2.18). An agent can ask: "replay incident 4711 and show the inputs
of the task that failed", or "which tasks have been blocked on the database for over a second?".

## 12. Fit check against the reference examples

| Example | Uses |
|---|---|
| [Multiplayer game](examples/01-multiplayer-game.md) | Replay of a desynchronised match from the server's recording; spans across client, hub and chat |
| [HFT](examples/02-hft-platform.md) | Interned logs on the tick-to-trade path; bit-for-bit replay of a trading day; latency histograms |
| [Web store](examples/03-web-store.md) | OTLP traces across client, API and database; payment keys and personal data blocked from logs by type |
| [Hashiverse](examples/04-hashiverse.md) | Opt-in live introspection of a volunteer node; replay of P2P incidents in simulation |

## 13. Open questions

- **Continuous profiling** as a handler, with time attributed by effect (network, locks, GPU)?
- Sampling policy for spans and recordings.
- Recording volume and retention; encrypting recordings, or excluding secret-typed effect results.
- Replaying a whole fleet: combining recordings from several processes.
- Schema evolution of structured events across versions.
- Overhead targets per level (e.g. nanoseconds per interned event on the hot path).

## Prior art

OpenTelemetry; W3C Trace Context; Rust `tracing`; Go `log/slog`, pprof and the execution tracer;
Java Flight Recorder; .NET EventPipe; BEAM observer, `dbg` and recon; defmt; Yang et al., *NanoLog*
(USENIX ATC 2018); rr; Pernosco; WinDbg Time Travel Debugging; tokio-console; Swift async
backtraces; Zig error return traces; eBPF, USDT and DTrace; Parca and Pyroscope; Sentry.

# Error Handling

> **Status:** survey + partial decisions. Indexed in [`README.md`](README.md) and defines
> requirement **R3** ([Firm requirements (decided)](requirements.md#firm-requirements-decided)). Decisions made so far are marked **Decided**; everything else is open.

## Decisions so far

| # | Decision |
|---|---|
| D1 | **Rust-style `?`** for propagation. Chained `?` propagates at every step. |
| D2 | **Try blocks**: `try { … }` is an expression that scopes `?`. |
| D3 | **R3: no crash from an unhandled, language-known error.** An unhandled error at any entry point or call site is a **compile-time error**. |
| D4 | **No fire-and-forget tasks.** Every spawned task is awaited by its enclosing scope (structured concurrency, [Concurrency across CPUs](concurrency.md)). |
| D5 | **Integer overflow traps, in every build** (as in Swift). Wrapping is explicit: `+%`, `-%`, `*%` or `Wrapping<T>` ([`numerics.md`: Overflow traps](numerics.md#overflow-traps)). Floats follow IEEE 754 (NaN, ±∞). |
| D6 | **Integer division traps, as in Rust**: `x / 0`, `x % 0`, `MIN / -1`. Same crash class as out-of-memory. |
| D7 | **Indexing traps on out-of-bounds**; `get(i)` returns `Option`. Bounds checks are elided where the compiler proves the index is in range. |
| D8 | **Resource exhaustion and faults are crashes**, not R3 violations: out of memory, stack overflow, `unreachable`, native FFI faults. |
| D9 | **Drop cannot fail or suspend**: a drop implementation has an empty effect set, checked by the compiler. Cleanup that can fail is a linear type with an explicit `close`/`commit` ([`resources.md`: Cleanup that can fail](resources.md#cleanup-that-can-fail)). |
| D10 | **Cleanup failures during error handling are suppressed, not lost**: they are attached to the primary error (as in Java's try-with-resources), never replace it and never terminate ([`resources.md`: Cleanup that can fail](resources.md#cleanup-that-can-fail)). |
| D11 | **`defer` is infallible; `errdefer` may fail.** `defer` follows drop's rules (no fail, no suspend); `errdefer` runs on error and cancellation exits, and its failure is suppressed ([`resources.md`: Cleanup that can fail](resources.md#cleanup-that-can-fail)). |
| D12 | **A destructor is checked like an entry point**: its failure set must be empty, so a fallible call inside `drop` must be handled there (logged, explicitly discarded, or the type made linear) ([`resources.md`: Cleanup that can fail](resources.md#cleanup-that-can-fail)). |

## Three outcome classes

| Class | Examples | Handling | Checked by |
|---|---|---|---|
| **1. Errors** | I/O failure, parse error, not found, network timeout, validation | Must be handled (R3) | Compiler: effect/type checking |
| **2. Defined arithmetic** | Float NaN/∞; explicit wrapping (`+%`, `Wrapping<T>`) | No handling; the result is defined | Nothing (by design: fast) |
| **3. Traps (crashes)** | OOM, stack overflow, integer overflow, integer ÷ 0, out-of-bounds index, `unreachable`, FFI fault | Terminates the process with a full trace, or only an isolated scope ([`lifecycle.md`](lifecycle.md#traps-the-process-by-default-isolated-scopes-on-request)) | Runtime |

R3 is about class 1 only: **a Moddil program never crashes because an error the language knows about
was not handled.**

Notes:
- **Integer division (D6).** Like Rust, the compiler inserts the check rather than relying on the
  hardware. The hardware differs: x86 `idiv` and WASM `i32.div_s` trap, while AArch64 `sdiv` returns 0.
  The check makes behaviour, messages and traces identical on every target. It costs a compare and
  branch next to a 20–90-cycle divide, and is elided for known non-zero divisors (constants, a
  `NonZero` type). `checked_div` returns `Option`.
- **Overflow (D5)** traps in every build, so debug and release behave the same and wrong values are
  never silent. Checks cost typically 1–5% and are elided where range analysis or refinements prove
  safety. Wrapping (`+%`, `Wrapping<T>`), `checked_*` and `saturating_*` are explicit
  ([`numerics.md`: Overflow traps](numerics.md#overflow-traps)).
- **Open:** do failed contract assertions ([Correctness tooling: contracts and verification](correctness.md)) trap (class 3), or raise an error (class 1)?
  [`correctness.md`: Decisions to lock early](correctness.md#decisions-to-lock-early) recommends trapping, with untrusted input handled by
  validated newtypes that return errors.

## Results and effects

### The difference

**`Result` (value).** The error is part of the return type: `fn parse(s) -> Result<Config, ParseError>`.
- Good: it is data. You can store it, collect it, or send it between tasks.
- Bad: making a function fallible changes its return type, and that spreads to every caller.
- Bad: `?` inside a closure returns from the closure, not from the function around it ([Why `Result`-only languages duplicate higher-order APIs](#why-result-only-languages-duplicate-higher-order-apis)).

**Effect.** The return type stays `Config`, and failure is in the effect row:
`fn parse(s) -> Config with fail(ParseError)`. Raising performs the `fail` effect; a handler further up
the stack decides what happens. Because functions can be **generic over effects**, one higher-order
function works for fallible and infallible callers alike ([Why `Result`-only languages duplicate higher-order APIs](#why-result-only-languages-duplicate-higher-order-apis)). It is the same mechanism as Moddil's
other effects ([Effects and capabilities as a unifying idea](effects-and-capabilities.md)).

**Moddil has both, linked:**
```
let r: Result<Config, ParseError> = try { parse(s)? }   // effect → value (reify)
let c: Config = r?                                       // value → effect (reflect)
```

**Performance is identical.** A `fail` handler that cannot resume compiles to an ordinary return
through the return channel (as with Koka's evidence passing and Swift's typed throws). Resumable
recovery ("retry", "use a default") would be a *separate* effect, so ordinary errors never pay for
it.

### Why `Result`-only languages duplicate higher-order APIs

In Rust, `?` inside a closure exits the closure:

```rust
fn load_all(paths: &[Path]) -> Result<Vec<Config>, Error> {
    paths.iter().map(|p| parse(p)?)   // `?` exits the closure, not load_all
    ...
}
```

So you need a separate mechanism to stop at the first error:
`paths.iter().map(parse).collect::<Result<Vec<_>, _>>()?`. Across the standard library this has
produced parallel APIs:

| Infallible | Fallible version Rust added |
|---|---|
| `fold` | `try_fold` |
| `for_each` | `try_for_each` |
| `Option::map(f)` where `f` returns `Result` | gives `Option<Result<..>>`, so `.transpose()` |
| `collect()` | `collect::<Result<..>>()` |
| `From` / `Into` | `TryFrom` / `TryInto` |
| `iter().map(async ..)` | a separate stream library |

Each new "colour" (fallible, async, const) adds another copy. Rust's *keyword generics* (effect
generics) initiative exists to fix exactly this.

With effects, one `map` serves every case:

```rust
fn map<A, B, E>(xs: List<A>, f: impl FnMut(A) -> B with E) -> List<B> with E

fn load_all(paths: List<Path>) -> List<Config> {
    paths.map(|p| parse(p)?)      // `?` propagates through map into load_all's errors
}
```

The cost: `map` is compiled per combination of effects (monomorphised, as Rust does with generics) or
receives a hidden handler argument (Koka). The happy path stays at Rust speed.

## Propagation, matching and try blocks

### Chained `?`

```
fn load(path) -> Config {                     // error set inferred
    let text = fs.read(path)?;                // + Io
    let raw  = toml.parse(text)?;             // + Toml{line}
    raw.validate()?                           // + Invalid{field}
}
```

The error set of `load` is inferred as the **union** of what each `?` can forward:
`Io | Toml{line} | Invalid{field}`. Chains like `a()?.b()?.c()?` work the same way.

### Matching removes what it handles

```
let config = try { load(p)? } catch {
    Io(NotFound) => Config::default(),
    // Toml and Invalid are not listed, so they keep propagating
};
```

- A `catch` handles only the variants it lists. **The remaining variants propagate automatically.**
- The inferred error set after the catch is the original set **minus** the handled cases.
- `match` on a reified `Result` works too, with exhaustiveness checking.

### Try blocks

- `try { … }` evaluates to `Result<T, E>`, where `E` is inferred from the `?`s inside it.
- `try { … } catch { … }` is shorthand for matching that result.
- Open details: how `return`/`break` inside a try block interact with it, and whether the error type
  of a try block can always be inferred (a known problem for Rust's unstable `try_blocks`; open
  unions avoid most of it).

### Entry points must handle everything (R3)

Every **entry point** must have an *empty* failure set:
- `main` (it returns an explicit exit status, never an escaped error);
- task bodies at spawn;
- exported and FFI functions and callbacks;
- tier endpoints (handlers of `server` components etc., [Distributed and multi-tier programs](distribution.md));
- effect handlers.

If anything remains, compilation fails, and the error lists each unhandled variant with the chain of
`?`s it would travel through.

**Ignored results:** dropping a `Result` silently is a compile error (not a warning, as `must_use` is
in Rust). *Open:* is an explicit discard allowed at all, and should it require a reason string, such
as `discard(r, "best-effort cache warm")`, so it is visible in reviews and semantic diffs ([Diffs, code review and version control](semantic-vcs.md))?

### Tasks and concurrency

- No fire-and-forget (D4). Long-running background work (servers, pollers) is a child of a long-lived
  scope, typically the service's root scope, whose handler supervises and restarts it.
- A task's errors are delivered at its **join point** and must be handled there.
- **Several tasks can fail at once.** The scope aggregates their errors (Python's `ExceptionGroup` /
  `except*`, Trio, Swift `ThrowingTaskGroup`). *Open:* first error plus cancel siblings, or collect all?
- **Cancellation** is a distinct outcome, not an error, but it must still be accounted for at the join.

## Error types

**Inferred open unions with payloads** (Roc, OCaml polymorphic variants), combined with Zig-style
inference:
- No error enums to write for internal code; the compiler computes the set.
- Unlike Zig, variants carry **payloads** (`Toml{line}`, `Io(kind, path)`).
- Unlike Rust, no `From` conversions are needed to combine errors from different sources; the sets
  simply union.
- **Explicit at mod boundaries.** Public functions of a mod declare their error set (consistent with
  [Effects and capabilities as a unifying idea](effects-and-capabilities.md) and [LLMs as first-class programmers](agent-interface.md)). Inference stays local, which helps incremental builds ([Build times](build-times.md)) and makes
  semantic diffs meaningful ("`parse()` can now fail with `Io`", [Diffs, code review and version control](semantic-vcs.md)).
- Named error types are still possible for documentation and reuse.

*Open:* how to attach context (`.context("loading config")`, as in `anyhow`) without losing the
structured variant?

## Traces

- **Error return traces** (Zig): each `?` that forwards an error records its location, giving the
  exact path from origin to handler, which is more useful than a classic stack trace for propagated
  errors.
  - Debug builds: on by default (cheap per-task buffer).
  - Release builds: configurable (off, bounded, or sampled).
- **Origin capture** (Rust's `Backtrace`): opt-in per error type or in debug builds, since a full stack
  walk is expensive.
- **Async-aware:** traces include the chain of task spawn sites; across tiers they link through trace
  IDs ([Distributed and multi-tier programs](distribution.md), [Observability and debuggability](observability.md)).
- **Traps (class 3)** always capture a full trace.
- Traces join the task-aware stacks and flight-recorder dumps described in
  [`observability.md`: Task-aware stacks and the task tree](observability.md#task-aware-stacks-and-the-task-tree)–[`observability.md`: Flight recorder and replay](observability.md#flight-recorder-and-replay).

## Language server support

The compiler must compute every function's error set to enforce R3, so the language server only has
to query it:
- hover on a function: its full error set;
- hover on a `?`: which errors pass through that point;
- "go to origin" for each variant;
- diagnostics at entry points listing unhandled variants.

Hard cases:
- **Recursion:** the set is computed as a fixpoint (it is finite). Zig requires explicit sets here;
  Moddil can infer.
- **Function values, trait objects, FFI:** the error set must be declared in the interface type.
- **Generics:** expressed with effect variables ("whatever errors `f` has").

## Performance: why C++ exceptions are slow, and why Moddil's errors aren't

**C++ ("zero-cost" table-based exceptions, Itanium ABI):**

*The throw path* often costs microseconds:
- heap allocation of the exception object;
- RTTI type matching to find a handler;
- two-phase unwinding (search, then cleanup) that interprets DWARF unwind tables frame by frame;
- historically, a global lock in the unwinder, so concurrent throws scaled badly (improved in recent
  GCC/glibc).

*The happy path* is not really zero-cost:
- unwind tables and landing pads make binaries larger;
- any call not marked `noexcept` might throw, which restricts optimisation;
- control flow is invisible.

Older models (setjmp/longjmp, 32-bit Windows SEH) also charged every `try`. The result is
unpredictable latency, so games, HFT, embedded and kernel code use `-fno-exceptions`, and C++ splits
into dialects. This is the motivation for Sutter's P0709. On WASM, exceptions need a dedicated
proposal, and the older JavaScript-based emulation was very slow.

**Moddil:** errors are values returned through the normal return channel (as in Rust, Swift, Zig,
and P0709):
- no allocation, no RTTI (sets are static, so matching is a tag switch), no unwind tables, no lock;
- the error path costs about the same as a normal return, with predictable timing (example 2, HFT);
- functions that cannot fail are known from their types, so they are effectively `noexcept` and the
  optimiser is unconstrained;
- every exit point is visible as `?`;
- ordinary errors do not depend on the WASM exceptions proposal (R1).

**Costs we accept:**
- one predictable branch per `?` (can be marked cold);
- larger return values when payloads are big. Mitigations: small or boxed payloads (Rust's
  `result_large_err` lint), packing the tag into spare bits, or returning the error flag in a
  register (Swift) or CPU flag (P0709);
- in very deep chains where errors almost never happen, table-based exceptions can benchmark faster.

**Traps (class 3)** are where cleanup is hard. Decided: a trap terminates the process by default; a
scope that opts into isolation is unwound on this rare path instead, running drops, poisoning held
sync cells and abandoning linear values
([`lifecycle.md`: Traps: the process by default, isolated scopes on request](lifecycle.md#traps-the-process-by-default-isolated-scopes-on-request)).

## State of the art: what to take, what to avoid

| Source | Take | Avoid / lesson |
|---|---|---|
| **Rust** | `Result` + `?`; `must_use`; typed errors in libraries | Enum boilerplate; `From` plumbing; `try_*` API duplication; try blocks unstable for years over type inference; `thiserror`/`anyhow` split |
| **Zig** | Inferred error sets; `try`/`catch`; **`errdefer`**; **error return traces** | No payloads on errors |
| **Swift** | Visible `try` at call sites; typed throws (Swift 6, SE-0413); return-channel implementation; `rethrows` | Untyped `throws` by default (fixed late) |
| **Midori** (Joe Duffy, *The Error Model*) | Errors vs bugs split; abandonment; marked call sites; checked exceptions done right | — (the key reading) |
| **C++** | `std::expected` (C++23); P0709 static exceptions | Table-based exceptions ([Performance: why C++ exceptions are slow, and why Moddil's errors aren't](#performance-why-c-exceptions-are-slow-and-why-moddils-errors-arent)); throwing destructors call `std::terminate`, so cleanup errors get swallowed ([`resources.md`: Cleanup that can fail](resources.md#cleanup-that-can-fail)) |
| **Java try-with-resources** | **Suppressed exceptions**: a failing `close` is attached to the primary error ([`resources.md`: Cleanup that can fail](resources.md#cleanup-that-can-fail)) | — |
| **Roc / OCaml polymorphic variants** | **Open structural unions with payloads, inferred** | — |
| **Koka / Effekt / OCaml 5** | Errors as effects; handlers; effect polymorphism | Resumable handlers for ordinary errors (cost) |
| **Common Lisp conditions** | Restarts: recovery decided by the handler without unwinding | Complexity |
| **Scala ZIO / Effect-TS** | Typed error channel that composes with concurrency | Library-level, not language-level |
| **Go** | Errors are values; wrapping (`errors.Is/As`) | Verbosity; in 2025 the Go team stopped pursuing new error syntax, which shows how hard retrofitting is |
| **Java checked exceptions** | The *intent* | No inference, no polymorphism, so `catch (Exception e) {}` everywhere |
| **Erlang / Elixir** | Supervision; `{:ok, v}` / `{:error, r}` plus `with` | "Let it crash" (contradicts R3 for class 1) |
| **Python 3.11 / Trio / Swift task groups** | Aggregating multiple concurrent errors | — |
| **Kotlin "rich errors"** (announced 2025, in design) | Evidence that mainstream languages are converging on union error types | Status uncertain |
| **SPARK Ada, Pony, Elm, Koka** | Compile-time "every error handled" (prior art for R3) | Pony's `x / 0 = 0` (Moddil traps instead, D6) |

## Strawman (not yet decided)

- `?` and `try {}` (decided), with `catch` that handles listed variants and propagates the rest.
- `fail(E)` as an effect underneath; `Result` as its value form.
- Inferred open error unions with payloads; explicit sets at mod boundaries.
- Non-resumable `fail`, compiled to the return channel; no unwinding for errors.
- Error return traces (debug on, release configurable).
- `errdefer`.
- Aggregated errors in task scopes; cancellation as a separate outcome.
- Traps (class 3) terminate the process by default.

## Open questions

1. Are explicit discards of errors allowed, and do they need a reason ([Entry points must handle everything (R3)](#entry-points-must-handle-everything-r3))?
2. Do failed contract assertions trap or raise an error ([Three outcome classes](#three-outcome-classes))? (Recommendation: trap; see
   [`correctness.md`: Decisions to lock early](correctness.md#decisions-to-lock-early).)
3. Task scopes: first error and cancel siblings, or collect all errors ([Tasks and concurrency](#tasks-and-concurrency))?
4. Should a resumable recovery effect ("retry"/"default") exist as a separate effect?
5. How is context attached to structured errors ([Error types](#error-types))?
6. Is `Option` part of the same machinery (`?` on `Option`, as in Rust)?
7. Should errors from the `remote` effect ([Distributed and multi-tier programs](distribution.md)) have a standard shape (timeout, unreachable,
   incompatible version), and how are errors serialised across tiers?
8. Checked-exception fatigue (Java) vs inference: how much of an error set should be inferred?

## Background

The problem and the state of the art that led to this design.

- **Problem.** Exceptions hide control flow; error codes get ignored; `Result` types are explicit but
  noisy; panics/abort are coarse. Composing error types across libraries is painful (Rust's
  `thiserror`/`anyhow` split shows this).
- **State of the art.** Rust `Result` + `?`; Swift typed `throws`; Zig error unions with inferred error
  sets and error return traces; Go explicit `error` values; Erlang "let it crash" plus supervisors.
- **Directions.** Errors as an *effect* (`fail(E)`) so they are visible in signatures but cheap to
  propagate; inferred error sets (Zig) to reduce boilerplate; mandatory handling; error return traces
  for free; a clear split between *recoverable errors* and *bugs* (contract violations → abort the
  task, not the process, in structured concurrency). Error propagation must also respect linear
  values ([Memory: ownership, borrowing and sharing](memory.md)): a path that propagates an error while holding an open transaction has to consume it.
  Absence is *not* an error: "not found" is an `Option` ([`types.md`: Optional values and invalid states](types.md#optional-values-and-invalid-states)), and `or_fail` converts explicitly.

## References

- Joe Duffy, *The Error Model* (2016), on the Midori project.
- Herb Sutter, P0709 *Zero-overhead deterministic exceptions: Throwing values*.
- Swift Evolution SE-0413, *Typed throws*.
- Rust `try_blocks` tracking issue; the Rust keyword-generics initiative.
- Zig language reference: errors, `errdefer`, error return traces.
- Roc: tag unions and error handling.
- Koka and Effekt: effect handlers.
- PEP 654, *Exception Groups and except\**.
- Go blog, "[ On | No ] syntactic support for error handling" (2025).
- Common Lisp condition system (Kent Pitman, *Condition Handling in the Lisp Language Family*).

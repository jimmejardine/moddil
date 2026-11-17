# Error Handling

> **Status:** survey + partial decisions. Expands [`rationale.md`](rationale.md) §2.3 and defines
> requirement **R3** (§5.1). Decisions made so far are marked **Decided**; everything else is open.

## 1. Decisions so far

| # | Decision |
|---|---|
| D1 | **Rust-style `?`** for propagation. Chained `?` propagates at every step. |
| D2 | **Try blocks**: `try { … }` is an expression that scopes `?`. |
| D3 | **R3: no crash from an unhandled, language-known error.** An unhandled error at any entry point or call site is a **compile-time error**. |
| D4 | **No fire-and-forget tasks.** Every spawned task is awaited by its enclosing scope (structured concurrency, §2.8). |
| D5 | **Arithmetic is unchecked.** Integers wrap (two's complement, no UB); floats follow IEEE 754 (NaN, ±∞). |
| D6 | **Integer division traps, as in Rust**: `x / 0`, `x % 0`, `MIN / -1`. Same crash class as out-of-memory. |
| D7 | **Indexing traps on out-of-bounds**; `get(i)` returns `Option`. Bounds checks are elided where the compiler proves the index is in range. |
| D8 | **Resource exhaustion and faults are crashes**, not R3 violations: out of memory, stack overflow, `unreachable`, native FFI faults. |
| D9 | **Drop cannot fail or suspend**: a drop implementation has an empty effect set, checked by the compiler. Cleanup that can fail is a linear type with an explicit `close`/`commit` (§9). |
| D10 | **Cleanup failures during error handling are suppressed, not lost**: they are attached to the primary error (as in Java's try-with-resources), never replace it and never terminate (§9). |

## 2. Three outcome classes

| Class | Examples | Handling | Checked by |
|---|---|---|---|
| **1. Errors** | I/O failure, parse error, not found, network timeout, validation | Must be handled (R3) | Compiler: effect/type checking |
| **2. Defined arithmetic** | Integer overflow (wraps), float NaN/∞ | No handling; the result is defined | Nothing (by design: fast) |
| **3. Traps (crashes)** | OOM, stack overflow, integer ÷ 0, out-of-bounds index, `unreachable`, FFI fault | Terminates the process (option: terminate only the task) with a full trace | Runtime |

R3 is about class 1 only: **a Moddil program never crashes because an error the language knows about
was not handled.**

Notes:
- **Integer division (D6).** Like Rust, the compiler inserts the check rather than relying on the
  hardware. The hardware differs: x86 `idiv` and WASM `i32.div_s` trap, while AArch64 `sdiv` returns 0.
  The check makes behaviour, messages and traces identical on every target. It costs a compare and
  branch next to a 20–90-cycle divide, and is elided for known non-zero divisors (constants, a
  `NonZero` type). `checked_div` returns `Option`.
- **Wrapping arithmetic (D5)** is fast but can silently produce wrong values. `checked_*`,
  `saturating_*` and `wrapping_*` operations live in a mod, and debug sanitisers can be optional tooling.
- **Open:** do failed contract assertions (§2.7) trap (class 3), or raise an error (class 1)?

## 3. Results and effects

### 3.1 The difference

**`Result` (value).** The error is part of the return type: `fn parse(s) -> Result<Config, ParseError>`.
- Good: it is data. You can store it, collect it, or send it between tasks.
- Bad: making a function fallible changes its return type, and that spreads to every caller.
- Bad: `?` inside a closure returns from the closure, not from the function around it (§3.2).

**Effect.** The return type stays `Config`, and failure is in the effect row:
`fn parse(s) -> Config ! fail(ParseError)`. Raising performs the `fail` effect; a handler further up
the stack decides what happens. Because functions can be **generic over effects**, one higher-order
function works for fallible and infallible callers alike (§3.2). It is the same mechanism as Moddil's
other effects (rationale §2.1).

**Moddil has both, linked:**
```
let r: Result<Config, ParseError> = try { parse(s)? }   // effect → value (reify)
let c: Config = r?                                       // value → effect (reflect)
```

**Performance is identical.** A `fail` handler that cannot resume compiles to an ordinary return
through the return channel (as with Koka's evidence passing and Swift's typed throws). Resumable
recovery ("retry", "use a default") would be a *separate* effect, so ordinary errors never pay for
it.

### 3.2 Why `Result`-only languages duplicate higher-order APIs

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

```
fn map<A, B, e>(xs: List<A>, f: A -> B ! e) -> List<B> ! e

fn load_all(paths: List<Path>) -> List<Config> {
    paths.map(|p| parse(p)?)    // `?` propagates through map into load_all's errors
}
```

The cost: `map` is compiled per combination of effects (monomorphised, as Rust does with generics) or
receives a hidden handler argument (Koka). The happy path stays at Rust speed.

## 4. Propagation, matching and try blocks

### 4.1 Chained `?`

```
fn load(path) -> Config {                     // error set inferred
    let text = fs.read(path)?;                // + Io
    let raw  = toml.parse(text)?;             // + Toml{line}
    raw.validate()?                           // + Invalid{field}
}
```

The error set of `load` is inferred as the **union** of what each `?` can forward:
`Io | Toml{line} | Invalid{field}`. Chains like `a()?.b()?.c()?` work the same way.

### 4.2 Matching removes what it handles

```
let config = try { load(p)? } catch {
    Io(NotFound) => Config.default(),
    // Toml and Invalid are not listed, so they keep propagating
};
```

- A `catch` handles only the variants it lists. **The remaining variants propagate automatically.**
- The inferred error set after the catch is the original set **minus** the handled cases.
- `match` on a reified `Result` works too, with exhaustiveness checking.

### 4.3 Try blocks

- `try { … }` evaluates to `Result<T, E>`, where `E` is inferred from the `?`s inside it.
- `try { … } catch { … }` is shorthand for matching that result.
- Open details: how `return`/`break` inside a try block interact with it, and whether the error type
  of a try block can always be inferred (a known problem for Rust's unstable `try_blocks`; open
  unions avoid most of it).

### 4.4 Entry points must handle everything (R3)

Every **entry point** must have an *empty* failure set:
- `main` (it returns an explicit exit status, never an escaped error);
- task bodies at spawn;
- exported and FFI functions and callbacks;
- tier endpoints (`@server` handlers etc., §2.10);
- effect handlers.

If anything remains, compilation fails, and the error lists each unhandled variant with the chain of
`?`s it would travel through.

**Ignored results:** dropping a `Result` silently is a compile error (not a warning, as `must_use` is
in Rust). *Open:* is an explicit discard allowed at all, and should it require a reason string, such
as `discard(r, "best-effort cache warm")`, so it is visible in reviews and semantic diffs (§2.19)?

### 4.5 Tasks and concurrency

- No fire-and-forget (D4). Long-running background work (servers, pollers) is a child of a long-lived
  scope, typically the service's root scope, whose handler supervises and restarts it.
- A task's errors are delivered at its **join point** and must be handled there.
- **Several tasks can fail at once.** The scope aggregates their errors (Python's `ExceptionGroup` /
  `except*`, Trio, Swift `ThrowingTaskGroup`). *Open:* first error plus cancel siblings, or collect all?
- **Cancellation** is a distinct outcome, not an error, but it must still be accounted for at the join.

## 5. Error types

**Inferred open unions with payloads** (Roc, OCaml polymorphic variants), combined with Zig-style
inference:
- No error enums to write for internal code; the compiler computes the set.
- Unlike Zig, variants carry **payloads** (`Toml{line}`, `Io(kind, path)`).
- Unlike Rust, no `From` conversions are needed to combine errors from different sources; the sets
  simply union.
- **Explicit at mod boundaries.** Public functions of a mod declare their error set (consistent with
  rationale §2.1 and §2.18). Inference stays local, which helps incremental builds (§2.14) and makes
  semantic diffs meaningful ("`parse()` can now fail with `Io`", §2.19).
- Named error types are still possible for documentation and reuse.

*Open:* how to attach context (`.context("loading config")`, as in `anyhow`) without losing the
structured variant?

## 6. Traces

- **Error return traces** (Zig): each `?` that forwards an error records its location, giving the
  exact path from origin to handler, which is more useful than a classic stack trace for propagated
  errors.
  - Debug builds: on by default (cheap per-task buffer).
  - Release builds: configurable (off, bounded, or sampled).
- **Origin capture** (Rust's `Backtrace`): opt-in per error type or in debug builds, since a full stack
  walk is expensive.
- **Async-aware:** traces include the chain of task spawn sites; across tiers they link through trace
  IDs (§2.10, §2.12).
- **Traps (class 3)** always capture a full trace.

## 7. Language server support

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

## 8. Performance: why C++ exceptions are slow, and why Moddil's errors aren't

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

**Traps (class 3)** are where cleanup is hard. If a trap terminates only a task rather than the
process, its resources must be freed without unwinding each frame. Options: resources tracked per task
scope or region, so the whole region is released at once (Erlang, Midori), or unwinding used only on
this rare path. *Open.*

## 9. Cleanup

- RAII / linear types (§2.2) clean up on every path.
- **`errdefer`** (Zig): cleanup that runs only when the scope exits with an error, for example rolling
  back a half-built structure.

```
let file = fs.create(tmp)?;
errdefer fs.remove(tmp);       // only if something below fails
write_header(file)?;
write_body(file)?;
fs.rename(tmp, final)?;
```

### 9.1 Cleanup that can fail (D9, D10)

C++ shows what goes wrong otherwise:
- a destructor that throws during unwinding calls `std::terminate`, and since C++11 destructors are
  implicitly `noexcept`;
- so libraries swallow real cleanup errors (a failed flush on `fclose`), and data loss goes
  unreported.

Moddil avoids both:
- **Drop has an empty effect set (D9).** It cannot fail, so there is nothing to throw. Fallible
  cleanup is a linear type, which must be consumed explicitly on every path, so its error is always
  reported ([`memory-and-resources.md`](memory-and-resources.md) §7.1).
- **No unwinding for errors** (§8), so no error is in flight while drops run.
- **Suppressed errors (D10).** When an error path consumes a linear value and that cleanup also
  fails, the cleanup error is attached to the primary error:
  - the primary error stays the one that propagates and is matched on;
  - suppressed errors appear in traces (§6), in the language server (§7) and in logs;
  - they can be read from the error value (exact API open), as Java's `getSuppressed()` allows;
  - a failure inside an `errdefer` body is suppressed in this way automatically.

```
let txn = db.begin()?
errdefer txn.rollback()        // consumes txn on error paths; a rollback failure is suppressed
txn.insert(order)?             // fails: the insert error propagates, with any rollback error attached
txn.commit()?
```

- **Traps inside drop** (class 3) terminate with a full trace (D8).

## 10. State of the art: what to take, what to avoid

| Source | Take | Avoid / lesson |
|---|---|---|
| **Rust** | `Result` + `?`; `must_use`; typed errors in libraries | Enum boilerplate; `From` plumbing; `try_*` API duplication; try blocks unstable for years over type inference; `thiserror`/`anyhow` split |
| **Zig** | Inferred error sets; `try`/`catch`; **`errdefer`**; **error return traces** | No payloads on errors |
| **Swift** | Visible `try` at call sites; typed throws (Swift 6, SE-0413); return-channel implementation; `rethrows` | Untyped `throws` by default (fixed late) |
| **Midori** (Joe Duffy, *The Error Model*) | Errors vs bugs split; abandonment; marked call sites; checked exceptions done right | — (the key reading) |
| **C++** | `std::expected` (C++23); P0709 static exceptions | Table-based exceptions (§8); throwing destructors call `std::terminate`, so cleanup errors get swallowed (§9.1) |
| **Java try-with-resources** | **Suppressed exceptions**: a failing `close` is attached to the primary error (§9.1) | — |
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

## 11. Strawman (not yet decided)

- `?` and `try {}` (decided), with `catch` that handles listed variants and propagates the rest.
- `fail(E)` as an effect underneath; `Result` as its value form.
- Inferred open error unions with payloads; explicit sets at mod boundaries.
- Non-resumable `fail`, compiled to the return channel; no unwinding for errors.
- Error return traces (debug on, release configurable).
- `errdefer`.
- Aggregated errors in task scopes; cancellation as a separate outcome.
- Traps (class 3) terminate the process by default.

## 12. Open questions

1. Are explicit discards of errors allowed, and do they need a reason (§4.4)?
2. Do failed contract assertions trap or raise an error (§2)?
3. Task scopes: first error and cancel siblings, or collect all errors (§4.5)?
4. Can a trap terminate only its task rather than the process, and how are its resources freed (§8)?
5. Should a resumable recovery effect ("retry"/"default") exist as a separate effect?
6. How is context attached to structured errors (§5)?
7. Is `Option` part of the same machinery (`?` on `Option`, as in Rust)?
8. Should errors from the `remote` effect (§2.10) have a standard shape (timeout, unreachable,
   incompatible version), and how are errors serialised across tiers?

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

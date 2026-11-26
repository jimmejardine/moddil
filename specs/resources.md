# Resources: drop, cleanup and linear types

Brings together the rules for destroying values and releasing resources. Ownership, borrowing and
sharing are in [`memory.md`](memory.md).

## Summary

| Rule | One line |
|---|---|
| **Drop glue by default** | Dropping a value drops its fields; hand-written destructors are only for raw resources ([Destructors](#destructors)). |
| **Drop cannot fail or suspend (D9)** | A destructor has an empty effect set, checked by the compiler, and is checked like an entry point (D12). |
| **Specified drop order** | Reverse declaration order for locals and fields; temporaries at the end of their statement ([When drops run](#when-drops-run)). |
| **`defer` and `errdefer` (D11)** | `defer` is infallible and runs on every exit; `errdefer` runs on error and cancellation exits and may fail ([`defer` and `errdefer`](#defer-and-errdefer)). |
| **Construction may suspend; drop may not** | Creators are ordinary functions that can suspend and fail; teardown that needs I/O is a linear type ([Construction may suspend; drop may not](#construction-may-suspend-drop-may-not)). |
| **Linear types for fallible cleanup** | A `linear` value must be consumed explicitly (`close`, `commit`, `rollback`) on every path, including cancellation ([Resources: affine and linear](#resources-affine-and-linear)). |
| **Nothing is lost (D10)** | A cleanup failure on an error path is attached to the primary error as suppressed, never replacing it and never terminating ([Cleanup that can fail](#cleanup-that-can-fail)). |

## Destructors

**Most types never write one.** The compiler generates *drop glue*: dropping a value drops each of
its fields. A struct of `String`, `Vec` and `File` needs no code at all.

A **hand-written destructor** is only for a type that owns a raw resource: a C handle, raw memory, a
NIC ring slot. In practice that is mostly low-level mods.

- **It lives with the type.** It implements the core `Drop` trait, and **only the type's own module
  may implement it**, so nobody else can change what dropping your type does.
- **Signature: `fn drop(self)`.** The destructor consumes the value and may **destructure** it to
  move fields out. Destructuring `self` inside `drop` does not call `drop` again; fields that are not
  moved out are dropped by glue afterwards.
- **Effect set: empty (D9)**, checked by the compiler (no `fail`, `suspend` or I/O). Two things are
  still allowed: freeing memory, and the write-only observability effects `log`, `span` and `metric`
  ([`observability.md`: Observability effects](observability.md#observability-effects)), so a destructor can report a problem.
- **A destructor is checked like an entry point (D12)** ([`error-handling.md`: Entry points must handle everything (R3)](error-handling.md#entry-points-must-handle-everything-r3)):
  its failure set must be empty. A fallible call inside it must be handled there ([If a destructor "errors"](#if-a-destructor-errors)).

```rust
impl Drop for Socket {
    fn drop(self) {
        let Socket { fd, buf } = self;            // move fields out; no recursive drop of Socket
        try { c::close(fd)? } catch {             // C close can report an error…
            e => log.warn(fmt"close failed: {e}"),  // …handled here: logged
        }
    }                                             // buf is dropped by glue here
}
```

## When drops run

- At the end of a scope, in **reverse declaration order**.
- When a variable is reassigned: the old value is dropped.
- On every exit path: early `return`, `?` propagation, and cancellation
  ([`lifecycle.md`: Cancellation and shutdown](lifecycle.md#cancellation-and-shutdown)).
- When the last `Rc` is released, on whichever thread or task releases it ([`memory.md`: Biased reference counting](memory.md#biased-reference-counting)).
- **A moved-from value is never dropped.** If a value is moved on only some paths, a hidden drop flag
  decides at runtime; the compiler removes it wherever the path is known.
- **Temporaries are dropped at the end of their statement.** There are no temporary-lifetime-extension
  rules (a known source of surprise in Rust); bind a value with `let` to keep it.
- `drop(x)` is not special: it is an ordinary function taking its argument by value, so it ends
  `x` early.
- **Fields are dropped in reverse declaration order**, mirroring the literal that built them (as
  C++ does; Rust drops fields in declaration order).

## `defer` and `errdefer`

RAII covers resources that have a type. `defer` covers one-off cleanup that doesn't deserve one:
restoring a flag, decrementing a gauge, releasing a lock obtained through FFI. `errdefer` covers
cleanup that only makes sense if something fails, such as removing a half-written file.

```rust
defer { metrics.in_flight.dec() }        // runs on every exit: success, error, cancellation

let file = fs.create(tmp)?;
errdefer fs.remove(tmp);                 // only if something below fails or the task is cancelled
write_header(file)?;
write_body(file)?;
fs.rename(tmp, final)?;
```

- `defer` blocks, `errdefer` blocks and drops share one **last-in, first-out order** by declaration
  point, so cleanup always unwinds in the reverse of setup.
- **`defer` follows drop's rules (D11)**: an empty effect set, so it cannot fail or suspend.
  `defer f.close()` on a fallible `close` is rejected, which steers you to an explicit `close()?` or a
  linear type. This closes Go's `defer f.Close()` hole at compile time.
- **`errdefer` may fail (D11).** It only runs on paths that already carry a primary error or
  cancellation, so its own failure is **suppressed** onto that outcome (D10). On cancellation paths
  it may also suspend, inside a shielded, deadline-bounded scope
  ([`lifecycle.md`: Cancellation and shutdown](lifecycle.md#cancellation-and-shutdown)). It is where fallible cleanup such as `txn.rollback()`
  belongs.
- A `defer` or `errdefer` body **borrows what it mentions until the scope ends**, so the borrow
  checker rejects moving `x` away and then cleaning up `x`.

## Construction may suspend; drop may not

Two rules remove the async-constructor problem rather than working around it:

- **No special constructors** ([`memory.md`: Ownership and RAII](memory.md#ownership-and-raii)). A value
  exists only once an all-fields literal has been evaluated. Creators are ordinary functions.
- **Suspension is an effect, not a colour** ([`concurrency.md`: Summary](concurrency.md#summary)).

So async construction is simply a function that suspends:

```rust
fn connect(addr: &Addr) -> Conn with net: Net, suspend, fail(NetError) {
    let socket  = net.connect(addr);    // may suspend
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
explicit `close()` or `commit()`, which may suspend and fail like any other function. A resource
whose graceful close only matters for politeness may instead stay affine: `close()` is graceful,
and a drop closes abruptly ([`stdlib.md`: I/O](stdlib.md#io)).

## Resources: affine and linear

- **Affine (default):** a value is used at most once and dropped automatically. Files, sockets,
  buffers, lock guards.
- **Linear (opt-in per type):** a value **must** be consumed by one of its designated operations,
  or the program does not compile. For resources whose finalisation can fail or needs a decision.

```rust
linear struct Txn { … }
impl Txn {
    fn commit(self) with fail(DbError);
    fn rollback(self);
}

fn checkout() with db: Db, suspend, fail(DbError) {
    let txn = db.begin();
    txn.insert(order)?;       // ✗ unless the error path also consumes txn
    txn.commit()?;
}
```

Rules:
- Linear values **cannot be `.to_rc()`'d**: the last-drop point of an rc is not known statically,
  and it may be on any thread.
- Every control-flow path, including error propagation, must consume a linear value. Every `?` or
  suspension point reached while a linear value is held needs an explicit handler for it: usually an
  `errdefer` that consumes it, or explicit handling. The compiler offers the `errdefer` as a quick
  fix, so the cleanup is always visible where the value is used.
- **Cancellation paths are checked like error paths**: a task observes cancellation only at a
  suspension point, and must consume its linear values on that path too, typically with
  `errdefer txn.rollback()` ([`lifecycle.md`: Cancellation and shutdown](lifecycle.md#cancellation-and-shutdown)).
- **Every linear type has an `abandon` operation**: infallible and non-suspending (for a
  transaction, drop the connection so the database rolls back). It runs only when an isolated
  scope traps ([`lifecycle.md`: Traps: the process by default, isolated scopes on request](lifecycle.md#traps-the-process-by-default-isolated-scopes-on-request)).

## Cleanup that can fail

1. **Drop has an empty effect set (D9).** A drop implementation cannot fail, suspend, or call
   anything that does. C++'s `noexcept` becomes a compile error, not a `std::terminate`.
2. **Fallible cleanup is a linear type.** Written files, buffered writers, transactions and TLS
   streams must be explicitly consumed (`close`, `commit`, `rollback`, `abort`). These are ordinary
   functions that may fail and suspend. Since every path must consume the value, the cleanup error
   is always reported.
3. **Errors do not unwind.** `fail` is compiled to the return channel
   ([`error-handling.md`: Performance: why C++ exceptions are slow, and why Moddil's errors aren't](error-handling.md#performance-why-c-exceptions-are-slow-and-why-moddils-errors-arent)), so no error is "in flight" while drops
   run. The double-fault mechanism does not exist.
4. **Two errors at once are explicit, and nothing is lost (D10).** On an error path, a linear value is
   consumed by an `errdefer` (or a handler). If that cleanup fails too, its error is attached to the
   primary error as **suppressed**:
   - the primary error stays the one that propagates and is matched on;
   - suppressed errors appear in traces ([`error-handling.md`: Traces](error-handling.md#traces)), in the language server
     ([`error-handling.md`: Language server support](error-handling.md#language-server-support)) and in logs;
   - they can be read from the error value with **`e.suppressed()`**, a provided method of
     the `Error` trait returning `impl Iterator<Item = &dyn Error>` in the order they occurred, as
     Java's `getSuppressed()` does;
   - a failure inside an `errdefer` body is suppressed in this way automatically.
5. **Traps inside drop are crashes** (class 3, D8: out of memory, ÷ 0, out of bounds). They terminate
   with a full trace, as in Rust. They are honest, never silently swallowed.
6. **No flush-on-drop trap.** A drop cannot call a fallible `flush`, so a type like Rust's
   `BufWriter` must be linear, or its author must discard the error explicitly and visibly.

```rust
let out = fs.create(path)?;          // linear: must be closed or aborted
errdefer out.abort();                // consumes out on error paths; if abort fails, its error is suppressed
out.write(data)?;
out.close()?;                        // a flush/close failure surfaces here, not in a destructor
```

## If a destructor "errors"

There are three cases, each with a precise answer:

1. **A fallible operation inside `drop`** (e.g. C's `close()` returns an error code). It does not
   compile unless it is dealt with inside the destructor, because a destructor's failure set must be
   empty (D12). The author can:
   - handle it locally, for example log it and continue (as in [Destructors](#destructors));
   - discard it explicitly, with a reason that shows up in review;
   - or make the type **linear** with an explicit `close() with fail(E)`, so callers must handle it.

   Silently swallowing the error (C++, Rust's `BufWriter`) and `std::terminate` are both impossible.
2. **Cleanup that fails while an error or cancellation is already in progress.** Only `errdefer`
   bodies and explicit cleanup on those paths can fail; their errors are **suppressed** onto the
   primary outcome (D10): never lost, never fatal.
3. **A trap inside `drop`** (out-of-bounds, division by zero, out of memory, an FFI fault). This is a
   class-3 crash ([`error-handling.md`: Three outcome classes](error-handling.md#three-outcome-classes)): the process terminates with a full
   trace and flight-recorder dump; **remaining drops do not run**, because there is no unwinding; a
   trap while already handling a trap aborts immediately.

## Problem

**C++ shows what goes wrong when cleanup can fail:**

- Destructors run during stack unwinding. If one throws while another exception is in flight, C++
  cannot carry two exceptions and calls `std::terminate`.
- Since C++11, destructors are implicitly `noexcept`, so a throw terminates even when nothing else
  is in flight. The rule is enforced at runtime, by killing the process.
- Cleanup can genuinely fail: `fclose` flushing to a full disk, a network filesystem reporting a
  write error on close, a commit. Because destructors can't report it, libraries swallow the error,
  and the program believes its data was written.
- A throwing member destructor can leave the remaining members and bases uncleaned.

**Construction has the mirror-image problem.** C#, JavaScript, Python, Java and C++ all forbid async
constructors, so code falls back on `CreateAsync()` factories, two-phase `InitAsync()` and
`Lazy<Task<T>>`, each leaving invalid states reachable.

## Alternatives considered

- **Destructors that may fail** (C++ before C++11): a throw during unwinding cannot carry two
  errors and terminates the process.
- **Implicitly `noexcept` destructors enforced at runtime** (C++11): the rule is enforced by
  killing the process; Moddil rejects a fallible destructor at compile time.
- **`drop(&mut self)`** (Rust): fields cannot be moved out, which forces `Option::take` and
  `ManuallyDrop` workarounds; `fn drop(self)` may destructure the value.
- **Flushing or closing on drop and discarding the error** (Rust's `BufWriter`, Go's
  `defer f.Close()`): the program believes its data was written when it was not.
- **Async constructors, `CreateAsync()` factories or two-phase `InitAsync()`:** each leaves
  invalid states reachable; a creator is an ordinary function that may suspend.
- **Temporary lifetime extension** (Rust): a known source of surprise; a value is kept by binding
  it with `let`.

## Prior art

- **Rust:** `Drop` cannot return an error; a panic in drop while already panicking aborts; `BufWriter`
  silently discards flush errors on drop. `drop(&mut self)` cannot move fields out, which forces
  `Option::take` and `ManuallyDrop` workarounds.
- **Go:** `defer f.Close()` routinely ignores the error.
- **Java:** try-with-resources is the best mainstream answer: a failing `close` becomes a *suppressed*
  exception on the primary one.
- **Zig:** `defer` and `errdefer`.
- **Linear types:** Austral, Vale, Linear Haskell, and Rust's long-discussed "must-use" types.

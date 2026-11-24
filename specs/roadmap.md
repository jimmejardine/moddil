# Roadmap and cross-cutting questions

> **Status:** minor questions. The single place for planning: strategic and technical questions that
> span several areas, the milestones, what is still to build (and what built work deferred),
> staging, and the order of the reference examples. What was built is in the git history.

## Cross-cutting questions

Open questions for a single area live in that area's document (see the [index](README.md)). The questions here cut across
areas.

**Strategic questions**
1. **Primary target domain** for v0? **Decided:** backend and infrastructure services, plus fast
   native modules for Python ([`adoption.md`: First audiences](adoption.md#first-audiences)).
2. **Who is the first user?** **Decided:** backend and infrastructure teams, and Python teams that
   need speed ([`adoption.md`: First audiences](adoption.md#first-audiences)).
3. **Adoption path**: embed in existing C/C++/Rust codebases one module at a time, through C export
   and the generated Rust bridge ([Interoperability with C and Rust](interop.md))? **Decided**, with
   Python extension modules and `moddil migrate rust` added ([`adoption.md`: Adopting inside existing codebases](adoption.md#adopting-inside-existing-codebases)).
4. **Is multi-tier deployment ([Distributed and multi-tier programs](distribution.md)) a v0 goal?** **Decided:** the language layer (components,
   placement, `Remote`, contracts, single-process and simulated runs) is v0; images, Compose,
   Kubernetes and skew checks are v1; other adapters later ([Staging](#staging-distributed-programs)).
5. **Mobile and UI**: shared logic under native UI shells, or a Moddil UI mod? **Decided:** native UI
   over a shared Moddil core, always; no standard UI mod ([`ui.md`](ui.md)).

The **reference examples** in [`examples/`](examples/README.md) (multiplayer game, HFT platform, web
store, Hashiverse) are the yardstick for every decision. A feature no example needs is a candidate for
removal.

**Technical questions**
6. **Compiler and back ends — decided** in [`compiler.md`](compiler.md): the compiler is written in
   Rust, with self-hosting deferred until after v1; Cranelift (debug) + LLVM (release) + own WASM
   emitter + naga/LLVM for GPU; JSPI as the first WASM lowering; a separate reference interpreter.
7. **Memory core — decided** in [`memory.md`](memory.md): unique
   ownership, second-class borrows and view types, `.to_rc()` with biased RC, affine + linear
   resources (summary in [Memory: ownership, borrowing and sharing](memory.md)).
8. Effect system design. **Decided:** a hybrid of scoped handlers and second-class capability
   values ([`effects-and-capabilities.md`](effects-and-capabilities.md), [Effects and capabilities as a unifying idea](effects-and-capabilities.md)).
9. **Canonical AST storage — decided:** plain `.moddil` text files in git (not `.mdl`, which other
   tools already use, so editors would misdetect it), always in canonical format (the formatter
   prints the AST). There is no structured store: the language is its canonical text
   ([`definition.md`: The reference](definition.md#the-reference)), and the semantic diff and
   per-definition content hashes give the structured benefits ([`semantic-vcs.md`](semantic-vcs.md)).
10. **Minimum viable language — decided:** M0 implemented the v0 language in vertical slices; what
    was left for later is under [Deferred from built work](#deferred-from-built-work).
11. **How is the language defined?** A prose reference, a formal semantics, or the MIR interpreter as
    the executable reference? How are conformance tests organised? **Decided:** a normative
    reference with formal core rules, the interpreter as its executable form and a mechanised core
    model, plus a shared conformance suite ([`definition.md`](definition.md)).

## Milestones

| Milestone | Contents | Status |
|---|---|---|
| **M0** | The grammar source first ([`definition.md`: One grammar source](definition.md#one-grammar-source)), then the compiler front end in Rust, grown in vertical slices: parser, type, effect and ownership checker, MIR, MIR interpreter. Small programs run. The separate **reference interpreter** starts alongside ([`compiler.md`: Two interpreters](compiler.md#two-interpreters)). **The tour** becomes the `tour/shorty` mod and must parse and check in CI ([`tour.md`: From markdown to source](tour.md#from-markdown-to-source)). | Done |
| **M1** | Cranelift native backend (Linux, macOS, Windows). Own WASM emitter producing core modules with the **JSPI lowering**, run in Node and browsers. | Done |
| **M2** | First standard mods written in Moddil; `moddil/runtime` and `moddil/runtime-sim`; LSP prototype. The tour's tests pass under `moddil/runtime-sim`. **The first GPU path:** kernel lowering through naga to WGSL, the `moddil/gpu` WebGPU handler and CPU simulator, and enough of `moddil/web` for a canvas and input; the **Mandelbulb flight** showcase runs in browsers as its test ([`examples/mandelbulb.md`](examples/mandelbulb.md)). | Done |
| **M3** | **Hashiverse CLI/native client** (phase A). LLVM release backend. **Python bindings** (`moddil bind --lang python`, wheels), the first binding target ([`adoption.md`: Python first](adoption.md#python-first)), shown off by **Talk**, on the CPU pool first and then on WebGPU. | In progress: the C ABI and the first Python bindings are built |
| **M4** | **A backend service**: the web store API with `sql`, HTTP and deterministic simulation; distributed v0 ([Staging: distributed programs](#staging-distributed-programs)). | To do |
| **M5** | **Browser**: the Hashiverse web client, the game client and **fumoca's engine**; Swift, Kotlin and TypeScript bindings ([`ui.md`](ui.md)). WASI components with stack switching once Wasmtime's support leaves Tier 3. | To do |
| **After v1** | Reconsider self-hosting ([`compiler.md`: Staged bootstrap](compiler.md#staged-bootstrap)). | To do |

The order puts the two first audiences, Python teams and backend teams, before the browser
(decided November 2026).

How a milestone is built: in vertical slices, each running end to end (grammar, checker, MIR,
every runner) before the next, and landing with its reference paragraphs and conformance tests
([`definition.md`: Written with the compiler](definition.md#written-with-the-compiler)). What was
built, slice by slice, is in the git history; the reference (`reference/2027/`) and the crates'
READMEs describe what exists.

## Still to build

**M3, the rest:**
- **The Hashiverse CLI/native client** (phase A), through the C ABI: its first pieces next.
- **The LLVM release backend**, beside Cranelift for release builds.
- **Python, beyond the first bindings** (`moddil build --wheel` exists):
  - `async` functions as `asyncio` awaitables, on Moddil's executor;
  - `Vec` results as arrays, a small `name.Array` class with the buffer protocol (decided);
  - keyword arguments, nested modules, methods taking `self` by value;
  - an error's fields, not only its type and `Debug` text;
  - more than one call into Moddil at a time (free-threaded CPython's separate pools);
  - `moddil bind --lang python`, the command the milestone names.
- **Talk**, the showcase ([`examples/talk.md`](examples/talk.md)): on the CPU pool first, then on
  WebGPU.

**M4 and M5** as the milestones above say; nothing of either is built yet.

**Next:** async in the Python bindings and array results; or the Hashiverse client's first pieces
through the C ABI; or the LLVM release backend.

## Deferred from built work

What was left out of something built, until another feature exists or a decision is made. Each
is still open; when one is done, it is removed from this list.

**Decisions to make first**
- **A `for` over an iterator that can fail** with known errors is E0501 (write
  `while let Some(x) = it.next()? { … }`), until it is decided how `for` marks it.
- **How a type-erased `Box<dyn Fn …>` that holds a handler borrow is tracked:** likely its
  borrows flow with the value, as for any view.

**Types and traits**
- Specialisation, generic associated types, default type parameters (so the operator traits take
  `Self` on the right: `Add` has no `Rhs`), and trait aliases.
- Associated consts; `type` items that are generic; a refinement with impls of its own, or whose
  base holds a borrow.
- Const arithmetic in types (`Fixed<{S + T}>`), and consts as array lengths; const parameters are
  `int`s.
- A borrow of a borrow, so `Hash` for `&T` and maps keyed by borrows (`HashMap<&str, V>`).
- An `impl Trait` result in a trait's method, one whose hidden type holds a borrow or a closure,
  and recursion between functions that return one.
- `Rc<dyn Trait>`: coercing to `dyn` is built for `Box` only.
- An anonymous impl implements its trait only, not its supertraits, and its methods take `&self`.
- `#[derive]` on a `view`.
- Closures: a closure bound to a local before `Box::new` is checked as an ordinary one (only one
  written where the box is expected meets its needs at the call); only a closure written as the
  function's value, or after `return`, is taken as the one returned; closures are not `Copy`;
  `escape` is not implemented.
- `IterMut`; `Index` takes an `int`; `[x; n]` needs `Copy` elements, and an array is `Clone` only
  when it is `Copy`.
- A view's method that returns, through a trait whose signature holds no borrow, a borrow of the
  view's own field is not caught.

**Errors, effects and concurrency**
- Inferred error tags (they wait for their syntax), and an error set written as a type.
- A note after `?` is checked but not kept: error return traces, and suppressed errors.
- A handler that holds another handler it captured.
- Cancellation: a cancelled task's pending operation's value is dropped without its destructors;
  a `send` in `select` does not wait for a rendezvous; a `join` of a cancelled task unwinds the
  task that joins.
- Shielded cleanup with deadlines, `checkpoint()`, grace periods, poisoning and `abandon`.
- Only a literal `false` (or leaving `isolate` out) keeps `Trapped` out of a scope's errors.
- A `select` arm on a `#[selectable]` function traps when it runs, until the runtimes wait on it.
- `log` has no handlers; a test cannot be run for listed types or declare capabilities.
- The allocator in scope: `with Alloc = …` and `Vec::new_in`.
- Biased counting, `AtomicRc` and the cycle detector, which wait for a parallel runtime.
- `panic`: only bugs trap (`unreachable`, `todo`, `assert`, `?!`).

**Comptime and contracts**
- `reflect(T)` in full, user-written derives beyond templates, and the typed construction API.
- A `const { }` block is evaluated where it is, each time; `#[fuel(…)]`; a const holds no borrow
  but of text.
- Wrappers: methods (a field to hold `self`); wrapper arguments evaluated at each call;
  `f::Args` cannot be named in code; `FnInfo` has no attributes or effects; `moddil explain` does
  not print the expansion.
- Contracts: `invariant`, `requires always`, contracts on trait methods, the `Contracts`
  handler's policies; `old(e)` needs a `Copy` value.
- Refinements: parenthesised refinements in types (`(u32 where self >= 1)`), niches and proofs;
  a refinement pattern's base is `Copy`; an alias names only the aliases before it.

**Text and numbers**
- Format specs other than those built (`{x:>8}`), and `Display` and `Debug` over a `Write` sink.
- Tagged literals other than `fmt`, such as `re"…"` compiled at compile time with typed captures.
- A position used on other text traps when it runs, rather than failing to compile, and is
  compared by text.
- `char_indices`, `graphemes` and `RcStr` slices.
- `f16`, `bf16`, arbitrary widths (`u3`), `usize` and `isize`, `Wrapping<T>`; exact decimal
  literals (`let p: Fixed<2> = 0.10`).
- The float functions other than `sqrt` are `libm`'s, not yet correctly rounded.
- Exhaustiveness on `i128` and `u128` leaves out their greatest value.
- `String`'s inline size is 23 on every target (11 on 32-bit targets comes with them), and
  strings are byte by byte in Moddil, so slower in the interpreters.

**Modules and tooling**
- A manifest has no registry dependencies, lockfile, targets or profiles; executables have no
  dependencies of their own; `mod name;` with a generating attribute.
- `String`, `Vec` and `core` live in the prelude and the built-in `core` until packaging gives
  them mods (`moddil/core-*`).
- Doc comments are not read: `moddil doc`, doc links and doc tests.
- A trailing comma at the end of a `where` clause before a body (ambiguous to tree-sitter).
- The LSP: no editor extension; going to the prelude's or `core`'s definitions finds nothing;
  names in types resolved by name in the file's module; completion offers the locals bound
  before the cursor; whole-document sync on one thread.

**Standard mods**
- `collections`: `EnumSet`; a sorted persistent map; Perceus-style reuse of a node no one else
  holds; dropping a long `PersistentList` without recursing per cell (needs `Rc::try_unwrap`);
  the persistent map's iterator is O(depth) per entry, as a view cannot hold a stack of borrows.
- `time`: reading a clock (a root capability) and time zones.
- `crypto`: not constant-time.
- The hasher's key is fixed, so map order is the same on every run, until it is keyed from `Rand`.

**GPU**
- `int` is 32 bits on the GPU, and a kernel's traps (overflow, a failed `?!`) are not checked
  there.
- No arrays, slices, closures, `dyn` or text in a kernel.
- `launch` returns once the GPU is done; `scope`'s `s.launch` comes later.
- `Tile` is 8 x 8 `Rgba8`, not generic, and E0221 recognises it by its type's name.
- The browser program is a `main` that awaits frames, not an exported `Flight` the page drives,
  and the page is plain, not the Svelte app.

**C interop**
- Header import through libclang (`#[c_import]`), and annotation overlays.
- Variadic C functions; struct values in callbacks.
- A trap in a Moddil function called through a `fn` value ends the process natively (it runs
  through its C-ABI wrapper), while the MIR interpreter traps as for any call.
- An exported function's trap ends the process; `moddil build --lib` makes no static library.
- On the MIR interpreter: borrows of C's memory are copies, `slice_mut` of it traps, a trap in a
  callback is raised only when the C call returns, and C cannot keep hooks past the call (zlib's
  `zalloc`).
- AArch64's rule that a struct is not split between registers and the stack holds only as far as
  Cranelift assigns registers; the tests stay within them.
- C on WASM.

**Native code and runners**
- A built program's `main` gives `()` or `ExitCode`.
- The reference interpreter: `?` on a call of a function whose signature lists `fail` is taken as
  the call's, so where its errors turn out to be `!` it differs from the compiler; it gives a
  literal its type when it meets one at run time, so it can differ where an unannotated variable
  takes its type from a later use; it types a collection literal by its context only where a
  `let`'s type, a parameter or a result names it.

**Deferred past M0, each for its own milestone:** units; `Simd`; linear types; generators and
coroutines; distribution ([Staging](#staging-distributed-programs)); data contracts.

## Staging: distributed programs

| Stage | Delivers |
|---|---|
| **v0** | Layer A: components, placement, the `Remote` effect, generated contracts, single-process mode and simulation |
| **v1** | OCI images, Compose and Kubernetes adapters, skew checking against deployed contracts |
| **Later** | Nomad and WASI-platform adapters, generated policies, Temporal/Restate workflow handlers, choreographies |

## Order of the reference examples

1. **Hashiverse client.** A real protocol with real users, with existing Rust servers to interoperate
   against. It exercises WASM, interop, crypto and effects-as-pluggable-traits. The CLI/native client
   comes first and the browser client follows once JSPI lowering exists
   ([Milestones](#milestones)).
2. **Multiplayer game.** Multi-tier, browser, real-time, WebGPU.
3. **Web store.** Everyday ergonomics, databases and the mobile question.
4. **HFT platform.** The hardest latency requirements and the flagship "same code in simulation and live"
   demonstration.

The order is open for discussion. HFT has the strongest single demonstration of effects, but the
largest scope.

The **flagship showcases** are not in this order: each is built as soon as its milestone allows,
because they exist to market Moddil to its first audiences.
1. **[Mandelbulb flight](examples/mandelbulb.md), first,** in M2: the simplest program that runs on
   the GPU in a browser.
2. [Talk](examples/talk.md), with the Python bindings in M3.
3. [fumoca](examples/fumoca.md), with the browser in M5.

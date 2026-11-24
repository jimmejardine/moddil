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
- **Python, beyond the first bindings** (`moddil build --wheel` exists; the smaller steps are
  work package P1 below):
  - `async` functions as `asyncio` awaitables, on Moddil's executor;
  - more than one call into Moddil at a time (free-threaded CPython's separate pools);
  - `moddil bind --lang python`, the command the milestone names.
- **Talk**, the showcase ([`examples/talk.md`](examples/talk.md)): on the CPU pool first, then on
  WebGPU.

**M4 and M5** as the milestones above say; nothing of either is built yet.

**Next:** any work package below; or async in the Python bindings; or the Hashiverse client's
first pieces through the C ABI; or the LLVM release backend.

## Deferred from built work

What was left out of something built, until another feature exists or a decision is made, as
**work packages**: each one an agent can finish in one go, in a small set of crates, needing no
decision first, and ending with conformance tests on every runner (M: a few hours; L: a day or
more). Packages are independent unless one says otherwise. When a package is done, remove it;
when part of one is, remove that part.

**P1. Python bindings, the next steps** (`crates/py`, `examples/python`; M)
- An enum error's variant and its fields as the exception's attributes (a struct error's fields
  are).
- Two error types of the same name in different modules (both would be `name.E`).

**P4. Text** (prelude `text`, `crates/sema`; M)
- `char_indices`, and `RcStr` slices.
- A text position used on other text refused when checking (as `text.md` says), not trapping
  when it runs and comparing by text.
- Tagged literals other than `fmt`, first `re"…"` compiled when compiling, with typed captures
  (`mods/regex`, with P11).
- The rest of Rust's format specs: a width or precision from an argument (`{x:w$}`, `{x:.*}`),
  `+`, `#`, and `x`/`b`/`o`/`e` (`{n:#x}`); and `fmt"…"` written straight into a `TextWrite`
  sink, not only as a new `String`.

**P5. Numbers** (`crates/sema`, the prelude, every runner; L)
- `usize` and `isize`, for FFI and memory-level code: a `Num` each, the width of a pointer on
  each target, through sema, MIR and all four runners and the C ABI.
- A `u128` pattern naming a value past `i128`'s greatest (pattern literals are `i128`s); the
  match analysis counts those values already.
- A literal where a `Wrapping<T>` is expected (`let seq: Wrapping<u32> = last + 1`, as
  `numerics.md` writes it), and `+=` on types with operator traits.
- The prelude's `int` `wrapping_add`, `wrapping_sub` and `wrapping_mul` intrinsics, from before
  `+%`: `WrappingInt` now gives them, so the intrinsics and the runners' code for them go.
- `Display` for `Wrapping<T>`, once `Display` writes to a `Write` sink (P3).

**P6. Arrays and iteration** (prelude `arrays` and `iter`, `crates/sema`, `crates/mir`, the
runners; M)
- `[x; n]` for elements that are only `Clone`: a loop in MIR cloning the value into each
  element but the last (a const parameter's length too), and a trap in a clone dropping the
  elements made so far.
- An array's `Clone` for elements that are only `Clone` (it is `Clone` only when `Copy`): the
  same loop, or a `Buf` filled and turned into the array.
- `Index` beyond `int` (ranges as `Index`).

**P7. Shared and anonymous values** (`crates/sema`, prelude `rc` and `derives`; L)
- `Rc<dyn Trait>`: coercing to `dyn` beyond `Box`.
- `#[derive]` on a `view`.
- Anonymous impls that implement the trait's supertraits, and methods taking `&mut self`.

**P8. The trait system** (`crates/sema`, `crates/syntax` and the grammar, the prelude, both
interpreters; L)
- Associated consts.
- Default type parameters, then the operator traits' `Rhs` (`Add<Rhs = Self>`).
- Generic `type` aliases; a refinement with impls of its own, or whose base holds a borrow.
- An `impl Trait` result in a trait's method, one whose hidden type holds a borrow or a closure,
  and recursion between functions that return one.

**P9. Closures** (`crates/sema` `check/closures.rs`, `crates/mir`; L)
- A closure bound to a local before `Box::new` meeting its needs at the call (today only one
  written where the box is expected does).
- A returned closure bound to a local first (today only one written as the function's value,
  or after `return`, is taken as the one returned).
- `Copy` closures; `escape`.

**P10. Borrow checking gaps** (`crates/mir` `ownership.rs`, `crates/sema`; M)
- A borrow of a borrow, so `Hash` for `&T` and maps keyed by borrows (`HashMap<&str, V>`).
- A view's method that returns, through a trait whose signature holds no borrow, a borrow of the
  view's own field: caught.

**P11. Compile-time evaluation** (`crates/sema`, `crates/mir`, the MIR interpreter as evaluator;
L)
- A `const { }` block evaluated once, not where it is each time; `#[fuel(…)]`.
- Consts as array lengths and const arguments; const arithmetic in types (`Fixed<{S + T}>`);
  const parameters of types other than `int`.
- A const holding a borrow of static data beyond text.

**P12. Contracts and refinements** (`crates/sema`, `crates/mir`; M)
- `invariant`; `requires always`; contracts on trait methods; `old(e)` of a value that is not
  `Copy` (by clone).
- Parenthesised refinements in types (`(u32 where self >= 1)`); a refinement pattern whose base
  is not `Copy`; an alias naming an alias after it.

**P13. Wrappers** (`crates/sema` `wrap.rs`, `crates/moddil`; M)
- Wrapping methods (a field to hold `self`).
- Wrapper arguments evaluated once, not at each call; `f::Args` named in code.
- `FnInfo` with attributes and effects; `moddil explain` printing the expansion.

**P14. Cancellation, finished** (`crates/sched`, both runtimes, the reference interpreter; M)
- A cancelled task's pending operation's value dropped with its destructors.
- A `send` in `select` waiting for its rendezvous.
- A `join` of a cancelled task not unwinding the task that joins.
- `isolate:` from any `bool`, not only a literal `false` keeping `Trapped` out of the errors.

**P15. Selectable functions in the runtimes** (`crates/sched`, `crates/interp`, `crates/rt`,
`crates/native`; M)
- A `select` arm on a `#[selectable]` function waits for it, rather than trapping when it runs.

**P16. Errors and effects** (`crates/mir`, both runtimes, the CLI's reports; L)
- Notes after `?` kept as an error return trace, shown with an error that escapes.
- Suppressed errors: a scope's losing tasks' errors kept, not dropped.
- An error set written as a type.
- A handler that holds another handler it captured; `log` with handlers; a test run for listed
  types, or declaring capabilities.

**P17. Built programs' results** (`crates/native`, `crates/rt-main`; M)
- A built program's `main` giving any type, printed as `moddil run` prints it, not only `()` or
  `ExitCode`.

**P18. The reference interpreter's differences** (`crates/refinterp`; M)
- `?` on a call of a function whose signature lists `fail` but whose errors turn out to be `!`
  (it takes the `?` as the call's).
- A literal typed by a later use of an unannotated variable (it types a literal when it meets
  one at run time).
- A collection literal typed from every context, not only a `let`'s type, a parameter or a
  result; and a tuple literal's elements by a tuple pattern's type (`let (x, y): (u8, u8) = (250,
  10)` gives two `int`s there).

**P19. Standard collections** (`mods/collections`, prelude `rc`, `core::collections`; L)
- `EnumSet`; a sorted persistent map.
- `Rc::try_unwrap`, and a long `PersistentList` dropped without recursing per cell.
- The hasher keyed from `Rand`, with tests printing their seed, so map order differs per run.
- Perceus-style reuse of a node no one else holds; the persistent map's iterator in O(1) per
  entry (needs a view that holds a stack of borrows).

**P20. Time** (`mods/time`, `core::os`, both runtimes; M)
- Reading the clock, as a root capability; then time zones (they need the zone data).

**P21. GPU kernels** (`crates/gpu`, `mods/gpu`, `core::gpu`; L)
- Arrays and slices in a kernel (closures, `dyn` and text later).
- A kernel's traps (overflow, a failed `?!`) checked on the GPU; `int` 64 bits there, not 32.
- `Tile` recognised by its type, not its name (E0221), and generic over its pixel and size.
- `scope`'s `s.launch`, so `launch` need not wait for the GPU.

**P22. C interop, the next steps** (`crates/ffi`, `crates/native`, `crates/interp`; L)
- Variadic C functions, in each platform's convention.
- Struct values in callbacks.
- On the MIR interpreter: `slice_mut` over C memory, borrows of it without copies, hooks C keeps
  past the call (zlib's `zalloc`), and a trap in a callback raised where it happens.
- An exported function's trap made a status C can check, beyond `#[export(status)]`.

**P23. Documentation tooling** (`crates/syntax`, `crates/sema`, `crates/moddil`; L)
- Doc comments read: `moddil doc`, doc links, and doc tests run by `moddil test`.

**P24. The language server, next** (`crates/lsp`; M)
- Names in types resolved by their full path, not by name in the file's module.
- Completion offering exactly the locals in scope, not those bound before the cursor.
- Incremental document sync, and analysis off the server's one thread; an editor extension.

**P25. A trailing comma in `where`** (`reference/grammar`, `crates/syntax`, `crates/grammar`; M)
- A trailing comma at the end of a `where` clause before a body, made unambiguous for
  tree-sitter (a type can start with `{`).

**P26. A line-by-line review of the checkers and the native lowering** (`crates/sema` `check/`,
`crates/native` `lower.rs`; L)
- The codebase review of November 2026 scanned these (repeated code, function lengths, markers,
  panics on input) but did not read their bodies, about 30,000 lines. Read them crate by crate
  for duplication, dead paths and limits missing from this list; fix what is small, and add the
  rest here as packages.

**Needs a decision first** (not packaged)
- How a `for` over an iterator that can fail with known errors marks them (today E0501: write
  `while let Some(x) = it.next()? { … }`).
- How a type-erased `Box<dyn Fn …>` that holds a handler borrow is tracked (likely its borrows
  flow with the value, as for any view).
- The syntax of inferred error tags.
- Specialisation, generic associated types and trait aliases: whether, and how far.
- The allocator in scope (`with Alloc = …`, `Vec::new_in`).
- Exact decimal literals (`let p: Fixed<2> = 0.10`), and `f16`, `bf16` and arbitrary widths
  (`u3`).

**Waits on bigger work** (not packaged)
- The rest of M3 and M4 and M5 (above, "Still to build").
- A parallel runtime: biased counting, `AtomicRc`, the cycle detector.
- Packaging: registry dependencies, a lockfile, targets and profiles, executables with
  dependencies of their own, `mod name;` with a generating attribute, and `String`, `Vec` and
  `core` as `moddil/core-*` mods.
- Header import through libclang (`#[c_import]`), with annotation overlays; C on WASM.
- Graphemes (Unicode's data); the browser program as an exported `Flight` the page drives, in the
  Svelte app.
- Each for its own milestone: units, `Simd`, linear types, generators and coroutines,
  distribution ([Staging](#staging-distributed-programs)), data contracts.

**Known limits, with no package yet**
- The float functions other than `sqrt` are `libm`'s, not yet correctly rounded.
- `String`'s inline size is 23 on every target (11 on 32-bit targets comes with them), and
  strings are byte by byte in Moddil, so slower in the interpreters.
- `crypto` is not constant-time.
- A trap in a Moddil function called through a `fn` value ends the process natively (it runs
  through its C-ABI wrapper), while the MIR interpreter traps as for any call.
- AArch64's rule that a struct is not split between registers and the stack holds only as far as
  Cranelift assigns registers; the tests stay within them.
- `panic`: only bugs trap (`unreachable`, `todo`, `assert`, `?!`), by design so far.

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

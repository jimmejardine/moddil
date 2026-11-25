# Roadmap

The single place for planning: the milestones, what is still to build (and what built work
deferred), staging, and the order of the reference examples. Decisions live in each area's
document ([index](README.md)). What was built is in the git history.

## Milestones

M0 to M2 (the front end and both interpreters, the native and WASM backends, the first mods and
the GPU path) are done; other documents still name them.

| Milestone | Contents | Status |
|---|---|---|
| **M3** | **Hashiverse CLI/native client** (phase A). LLVM release backend. **Python bindings** (`moddil bind --lang python`, wheels), the first binding target ([`adoption.md`: Python first](adoption.md#python-first)), shown off by **Talk**, on the CPU pool first and then on WebGPU. | In progress: the C ABI and the first Python bindings are built |
| **M4** | **A backend service**: the web store API with `sql`, HTTP and deterministic simulation; distributed v0 ([Staging: distributed programs](#staging-distributed-programs)). | To do |
| **M5** | **Browser**: the Hashiverse web client, the game client and **fumoca's engine, simulation and GPU code**, replaced together; Swift, Kotlin and TypeScript bindings ([`ui.md`](ui.md)). WASI components with stack switching once Wasmtime's support leaves Tier 3. JSPI's per-switch cost measured on the game client. | To do |
| **After v1** | Reconsider self-hosting ([`compiler.md`: Staged bootstrap](compiler.md#staged-bootstrap)): what would justify it, and whether Moddil-core (the stage 0 subset) is the right size to write a compiler in. | To do |

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
- **The LLVM release backend**, beside Cranelift for release builds, on one pinned LLVM major
  version, upgraded about once a year in one change ([`compiler.md`: Backends](compiler.md#backends)).
- **Python, beyond the first bindings** (`moddil build --wheel` exists; the smaller steps are
  work package P1 below):
  - `async` functions as `asyncio` awaitables, on Moddil's executor;
  - more than one call into Moddil at a time (free-threaded CPython's separate pools);
  - `moddil bind --lang python`, the command the milestone names.
- **Talk**, the showcase ([`examples/talk.md`](examples/talk.md)), published as `moddil-talk`: on
  the CPU pool first, then on WebGPU. It needs:
  - `moddil/tensor` loading safetensors, memory-mapped, with shapes checked at load time
    ([`accelerators.md`: Loading model weights](accelerators.md#loading-model-weights));
  - `moddil/audio`, the `Audio` effect with OS, Web Audio and simulated handlers
    ([`stdlib.md`: Audio](stdlib.md#audio));
  - English G2P: a lexicon, with a small open-weights neural model for unknown words.

**Decided, not yet built** (beyond the milestones):
- **The semantic diff** and per-definition content hashes, which give structured storage's
  benefits over plain text files ([`semantic-vcs.md`](semantic-vcs.md)).
- **The generated Rust bridge** (a Moddil mod as a Rust crate, and Rust crates read from rustdoc's
  JSON) and **`moddil migrate rust`**, the adoption path into existing Rust codebases
  ([`interop.md`: Rust: the generated bridge](interop.md#rust-the-generated-bridge),
  [`adoption.md`: Adopting inside existing codebases](adoption.md#adopting-inside-existing-codebases)).
- **The mechanised core model** in Lean ([`model/`](../model/README.md)), from which the reference's
  core rules are generated and against which the reference interpreter is validated
  ([`definition.md`](definition.md)).

**M4 and M5** as the milestones above say; nothing of either is built yet. Decided for M5:
- fumoca's `engine`, `sim` and `gpu` are replaced together, its workbooks compiled to kernels
  through a `dyn_kernel` shader-builder mod with bindings typed and checked at launch
  ([`accelerators.md`: Kernels built at run time](accelerators.md#kernels-built-at-run-time));
- JSPI's per-switch cost is measured on the game client.

**moddil.com** ([`web.md`: Hosting](web.md#hosting)) goes up with the playground, once `/play` runs
real programs: Cloudflare Pages, with Astro (Starlight) for the site and, once P23 builds it,
`moddil doc`'s API pages and checked code blocks.

**Next:** any work package below; or async in the Python bindings; or the Hashiverse client's
first pieces through the C ABI; or the LLVM release backend.

## Deferred from built work

What was left out of something built, until another feature exists or a decision is made, as
**work packages**: each one an agent can finish in one go, in a small set of crates, needing no
decision first, and ending with conformance tests on every runner (M: a few hours; L: a day or
more). Packages are independent unless one says otherwise. When a package is done, remove it;
when part of one is, remove that part.

**P4. Text** (prelude `text`, `crates/sema`; M)
- A text position used on other text refused when checking (as `text.md` says), not trapping
  when it runs and comparing by text.
- Tagged literals other than `fmt`, first `re"…"` compiled when compiling, with typed captures
  (`mods/regex`, with P11).
- `#?` (`Debug` across lines, indented), and `fmt"…"` written straight into a `TextWrite` sink,
  not only as a new `String`.

**P5. Numbers** (`crates/sema`, the prelude, every runner; L)
- `usize` and `isize`, for FFI and memory-level code: a `Num` each, the width of a pointer on
  each target, through sema, MIR and all four runners and the C ABI.
- A literal where a `Wrapping<T>` is expected (`let seq: Wrapping<u32> = last + 1`, as
  `numerics.md` writes it), and `+=` on types with operator traits.

**P6. Arrays and iteration** (prelude `arrays` and `iter`, `crates/sema`, `crates/mir`, the
runners; M)
- `Index` beyond `int` (ranges as `Index`): `Index<Idx = int>`, with P8's default type
  parameters.

**P7. Shared and anonymous values** (`crates/sema`, the prelude's `pointers`; L)
- `Rc<dyn Trait>`: `Rc` is the prelude's, over `*mut RcBox<T>` with `value: Option<T>`, so a
  `dyn` in it needs a struct whose last field is unsized (its size and alignment from the
  vtable, the pointer fat) through sema's layout, MIR and every runner; then the coercion, as
  `Box`'s.
- Anonymous impls that implement the trait's supertraits, and methods taking `&mut self`.
- `Debug` derived for a view with a borrow field: `&T` implementing `Debug`, as it now implements
  `Hash` and `PartialEq`.

**P8. The trait system** (`crates/sema`, `crates/syntax` and the grammar, the prelude, both
interpreters; L)
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

**P11. Compile-time evaluation** (`crates/sema`, `crates/mir`, the MIR interpreter as evaluator;
L)
- A `const { }` block evaluated once, not where it is each time; `#[fuel(…)]`.
- Consts as array lengths and const arguments; const arithmetic in types (`Fixed<{S + T}>`);
  const parameters of types other than `int`.
- A const holding a borrow of static data beyond text.

**P12. Contracts and refinements** (`crates/sema`, `crates/mir`, `crates/refinterp`; L)
- `invariant` on a struct (`correctness.md`), its field names in scope as in the spec's
  `OrderBook`: checked when a struct literal builds one and when a method of it taking
  `&mut self` returns.
- Contracts on trait methods, binding every impl: the trait's `requires` and `ensures`, written
  in the trait's file and scope, checked on entry to and return from each impl's method.
- A refinement pattern, matched through a borrow, whose base is not `Copy` (it would need the
  value cloned, or `holds` to take a borrow).

**P13. Wrappers** (`crates/sema` `wrap.rs`, `crates/moddil`; M)
- Wrapping methods (a field to hold `self`).
- Wrapper arguments evaluated once, not at each call (with P11's compile-time evaluation).
- `f::Args` made and matched by that name (`f::Args { a, b }`), and for a generic function.
- `moddil explain` printing the expansion (the command itself is not built).

**P15. Selectable trait methods** (`crates/mir` `build/select.rs`, `crates/interp`,
`crates/native`; L)
- A `select` arm on a `#[selectable]` trait method reached through a bound
  (`fn f<S: Source>(s: &S) { select { x = s.take() => … } }`) traps: the impl's two parts
  (`task.select.4`) are known only once `S` is. Free functions and inherent methods, generic ones
  too, already work.

**P16. Errors and effects** (`crates/mir`, both runtimes, the CLI's reports; L)
- Notes after `?` kept as an error return trace, shown with an error that escapes.
- Suppressed errors: a scope's losing tasks' errors kept, not dropped.
- An error set written as a type.
- A handler that holds another handler it captured; `log` with handlers; a test run for listed
  types, or declaring capabilities.

**P19. Standard collections** (`mods/collections`, prelude `rc`, `core::collections`; L)
- A sorted persistent map.
- `EnumSet`'s bits in the smallest unsigned integer that fits its enum, as `stdlib.md` decides
  (for packed structs and wire formats): a `u128` for now, chosen when compiling (P11).
- The hasher keyed from `Rand`, with tests printing their seed, so map order differs per run.
- Perceus-style reuse of a node no one else holds, as a pass on MIR (`mir.md`: Reference
  counting); the persistent map's iterator in O(1) per
  entry (needs a view that holds a stack of borrows).

**P20. Time zones** (`mods/time`; M)
- Time zones, from the zone data (IANA's `tzdata`, compiled into the mod or read through a root
  capability).

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

**P23. Documentation tooling** (`crates/syntax`, `crates/sema`, `crates/moddil`; L)
- `moddil doc`, and doc links (`[[Account::open]]`) checked against the code, as `docs.md` says.
- An example's `// => fail(E)` and snapshot-style expected output (`testing.md`).

**P24. The language server, next** (`crates/lsp`; M)
- Incremental document sync, and analysis off the server's one thread; an editor extension.

**P26. A line-by-line review of the checkers and the native lowering** (`crates/sema` `check/`,
`crates/native` `lower.rs`; L)
- The codebase review of November 2026 scanned these (repeated code, function lengths, markers,
  panics on input) but did not read their bodies, about 30,000 lines. Read them crate by crate
  for duplication, dead paths and limits missing from this list; fix what is small, and add the
  rest here as packages.

**P29. The allocator in scope** (prelude, `crates/sema`, `crates/mir`, `crates/rt`; L)
- `with Alloc = a { … }` (`memory.md`: The allocator in scope): `.to_rc()` and growing
  collections allocate from the innermost installed allocator, the profile's default otherwise;
  no `_in` constructors. `Scratch` in `moddil/alloc` installs itself the same way (`stdlib.md`).

**P30. Exact decimal literals, `f16` and `bf16`** (`crates/sema`, `crates/mir`, the runners; L)
- A decimal literal coerces to `Fixed<S>` exactly, or is an error (`numerics.md`: Exact literals).
- `f16` and `bf16` as built-in floats, with conversions, on every runner.

**P31. Arbitrary integer widths** (`crates/sema`, `crates/mir`, the runners; L)
- `uN` and `iN` (`numerics.md`): stored in the next standard size, packed to their bits only in a
  `#[repr(packed)]` struct, with overflow traps at the width, and a lint for an odd width outside
  a packed struct.

**P33. Runtime type information** (`mods/reflect`, `crates/sema` derives; M)
- `#[derive(Reflect)]` (`stdlib.md`: Runtime type information): a static `TypeInfo` and typed
  field accessors; only fields visible where they are read; read-only unless the type has
  `#[reflect(mut)]`.

**P34. Traps unwind only in isolated scopes** (`crates/native`, `crates/wasm`, `crates/rt`; L)
- Today every internal call returns a status that its caller checks and passes on
  (`crates/native/src/lower.rs`), so every call pays a branch. `mir.md` decides instead:
  - ordinary code has no landing pads, and a trap there ends the process;
  - inside `scope(…, isolate: true)`, a trap unwinds through tables, with exnref on WASM,
    running drops and `abandon`.
- Add a benchmark that guards the cost of isolated scopes.
- Keep traps at the C ABI as they are now (`ffi.export.3`).

**P35. The formatter, next** (`crates/lsp`, `tools/ci.py`, the repository's sources; M)
- Formatting in the language server (`documentFormattingProvider`), so editors format on save.
- `moddil fmt` over the prelude, `mods/`, `tour/`, `examples/` and `conformance/`, with
  `reference/grammar/tests/` left as it is, and `moddil fmt --check` on those in `tools/ci.py`'s
  `fmt` step. Done in one go, as other work edits those files; the conformance suite on every
  runner shows the `//~` markers held.
- Focused snapshot tests of each layout rule in `crates/fmt/tests/`, beside the corpus test.

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
- `moddil bench`, with budgets that fail CI: the HFT hot path's latency is measured that way,
  not checked by the compiler ([`examples/hft-platform.md`](examples/hft-platform.md)).

**Known limits, with no package yet**
- The float functions other than `sqrt` are `libm`'s, not yet correctly rounded.
- `String`'s inline size is 23 on every target (11 on 32-bit targets comes with them), and
  strings are byte by byte in Moddil, so slower in the interpreters.
- `crypto` is not constant-time.
- A trap in a Moddil function called through a `fn` value ends the process natively (it runs
  through its C-ABI wrapper), while the MIR interpreter traps as for any call.
- AArch64's rule that a struct is not split between registers and the stack holds only as far as
  Cranelift assigns registers; the tests stay within them.
- The reference interpreter infers no types (`crates/refinterp/README.md`), so it takes a `?`
  on a call whose signature lists `fail` as the call's even when the errors turn out to be `!`;
  types a literal where it meets it, not by a later use of an unannotated variable; and types a
  collection literal only from a `let`'s type, a parameter or a result. Closing these needs
  inference of its own, which is not decided.
- A `join` of a cancelled task unwinds the task that joins, and cancels its scope
  (`task.cancel.1`). Not unwinding it needs a decision first: how the join is told, since
  `Cancelled` is not an error (`error-handling.md` D15).
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

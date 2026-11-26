# Roadmap

The single place for planning: one numbered list of the work on the horizon. Decisions live in
each area's document ([index](README.md)); what was built is in the git history, the reference
(`reference/2027/`) and the crates' READMEs.

## How work is built

- In vertical slices, each running end to end (grammar, checker, MIR, every runner) before the
  next, and landing with its reference paragraphs and conformance tests
  ([`definition.md`: Written with the compiler](definition.md#written-with-the-compiler)).
- Each item below is numbered; the numbers are names, not an order. Sizes: **M** a few hours,
  **L** a day or more, **XL** several L packages, built in stages. An M or L item is a package an
  agent can finish in one go, in a small set of crates, needing no decision first, and ending
  with conformance tests on every runner. Items are independent unless one says otherwise.
- When an item is done, remove it; when part of one is, remove that part. A part left for later
  goes into its item, or becomes a new one.
- Earlier work was built as milestones: M0 to M2 (the front end and both interpreters, the
  native and WASM backends, the first mods and the GPU path) and M3's first slices (M3a to M3c:
  the C ABI and the first Python bindings). Other documents name those slices.

## On the horizon

Python teams and backend teams come before the browser (decided November 2026,
[`adoption.md`: First audiences](adoption.md#first-audiences)).

**P4. Text** (prelude `text`, `crates/sema`; M)
- Typed captures for `re"…"`: a field for each named group, through a prelude `TagFields` trait
  the tag's value implements, whose fields make a hidden struct (decided). Evaluating the tag
  then happens while the program is collected, so it needs a sub-program of the tag's mod and its
  dependencies, or an evaluator hook from the interpreter into sema.
- Holes in tagged literals other than `fmt` (`sql"… {user} …"`), as typed parameters.

**P5. Numbers** (`crates/sema`, the prelude, every runner; L)
- `usize` and `isize`, for FFI and memory-level code: a `Num` each, the width of a pointer on
  each target, through sema, MIR and all four runners and the C ABI.

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
- A `const { }` block that is not `Copy`, used by value, evaluated once too (each use a clone of
  its static value, or a move out of a value made once); and one in a generic function, for
  each instance.
- Const arithmetic on const parameters in types (`Fixed<{S + T}>`, a normalised type form
  compared by structure), and a length a `const fn` gives (sema folds lengths from the syntax,
  and cannot run the interpreter); const parameters of types other than `int`.
- A generic const (a trait's associated const, a generic impl's) computed when compiling for
  each instance, as other consts are, not when it runs; and a const holding a function pointer
  frozen as data (E1008 now).

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
- Wrapper arguments that are not `Copy` evaluated once, not at each call (a `Copy` one is, as a
  `const { }` block is: P11).
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

**P36. The reference interpreter's own type inference** (`crates/refinterp`; L, in stages)
- Written from the reference's typing rules, not taken from `sema` (the reference interpreter
  stays independent), so that it checks the checker too. One commit per stage:
  1. Literals and collection literals typed locally, from expected types and later uses
     (`let x = 250; let y: u8 = x;` makes `250` a `u8`), from every context, not only a
     `let`'s type, a parameter or a result.
  2. Associated types and error sets resolved from the impls (`I::Fails`), so `?` on a call
     whose errors turn out to be `!` takes apart its `Option`.
  3. Trait selection by type: a trait's function without `self` by the expected type
     (`Zero::zero()`), impls of one trait for different arguments of a generic type told apart,
     and a bound trait's method preferred over an inherent one of the same name in generic code;
     then the conformance suite's avoidance of these ends.

The parallel runtime is four packages, done in this order: P37, then P40 and P41's logic
half, then P38's stages, with P41's benchmarks as each lands (`runtime.md`).

**P37. Choosing the runtime** (`crates/rt`, `crates/sched`, prelude, `mods/runtime`,
`crates/moddil`; M). The apparatus, with only the deterministic runtime behind it.
- The prelude's `Runtime` handler type; the `Cpus` root capability in `mods/runtime`; the
  executor constructors `pool`, `pinned`, `blocking` and `affine` (cores, `node(n)`, priority,
  busy-polling) as values (`runtime.md`: Starting a program).
- `scope(…, on: e)` and `spawn(…, on: e)` in the checker and MIR, children inheriting their
  executor.
- `with Runtime = …` only in `main`, once (an error elsewhere, with its reference paragraphs).
- Today's scheduler named `runtime-sim`, with `Sim::seeded(n)`; placement recorded and
  simulated.
- Each test under `runtime-sim` with its own seed, printed when it fails; `moddil test --seed n`
  replays one.
- `moddil run --runtime sim|parallel` and the manifest's runtime and core counts (`parallel`
  refused with a message until P38); the one-thread build's manifest key and `--threads one`
  (its lowering is P40's).
- Conformance on all four runners, which all run `runtime-sim` and so keep agreeing.

**P38. The parallel runtime** (`crates/rt`, `crates/sched`, `crates/native`; L, after P37 and
P40, with P41's tests). In stages, each a commit with P41's suite green:
1. Several tasks at once: the baton becomes N batons, still a thread per task, so the semantics
   are proven before the mechanics change.
2. User-space stacks: a context switch of its own per target (x86-64 SysV and Win64, AArch64);
   stacks reserved large, committed lazily, with guard pages; tasks become coroutines on worker
   threads (M:N).
3. Workers and stealing: a worker per core with a Chase-Lev deque; stealing; parking and
   unparking with an event count; a spawned task first on its spawner's worker.
4. Preemption: a timer thread sets the safe-point flags (P40); no signals.
5. Executors: `default` and `pool`, work-stealing; `pinned`, with core affinity, real-time
   priority and optional busy-polling, never lending; `blocking`, growing on demand, with
   `blocking` C functions routed to it; `affine`; `on:` placement; idle executors lending their
   threads.
6. A reactor per worker: io_uring on Linux (epoll as fallback), kqueue on macOS, IOCP on
   Windows; a timer wheel per worker; OS-level cancellation; re-registration when a task is
   stolen; the `core::os` calls moved onto it.
7. NUMA: topology discovery (Linux sysfs, Windows `GetLogicalProcessorInformationEx`), executors
   placed by node, node-local allocator arenas.
8. Observability: the task tree and each task's blocked-on operation, for `Introspect` and spans
   (`observability.md`).

WASM stays single-threaded; Web Workers come later.

**P39. Live update, the MVP** (`crates/native`, `crates/moddil`; L)
- The first step of `build-times.md`'s staging: an indirection table in native debug builds,
  `moddil run --watch`, swaps of changed bodies at suspension points. Its later steps follow as
  packages of their own.

**P40. Hardening the language for threads** (prelude `pointers`, `mods/sync`, `crates/rt`,
`crates/native`, `crates/mir`; L, before P38 runs two tasks at once). Each item a commit, the
single-threaded behaviour unchanged. Each commit's message gives single-threaded benchmarks
before and after (rc-heavy code, locks, tight loops): recorded, not gated.
- `Rc`: biased counting (`memory.md`: Biased reference counting). The owner task's count is
  plain, the others' an atomic shared count, and a merge queue per task is drained by the
  runtime when it resumes the task and at its end. `Weak` likewise. Today `RcBox` counts with a
  plain `int`.
- `AtomicRc<T>` for sync cells holding shared values; then the cycle detector.
- `moddil/sync`: `Mutex` and `RwLock` parking through the scheduler's wait queues, not spinning
  with yields as now; condition waits; priority inheritance across executors; the atomics'
  orderings documented against the memory model (`unsafe.md`).
- The runtime's state: `crates/rt`'s one mutex split, the scheduler state per worker; statics
  and `const` values initialised once, atomically; the allocation count without the run's lock
  on each allocation; a thread-safe allocator with per-thread caches.
- Preemption safe points: a flag check at loop back-edges and function entries, in MIR and
  native code, off in `#[bounded]` hot paths and on `pinned` executors (`runtime.md`); inert
  under `runtime-sim`.
- An audit of every `UnsafeCell`, raw pointer and `core::atomic` use in the prelude, `core` and
  the mods: each made safe for threads or confined to one task, with the reason in the
  reference.
- The one-thread lowering, on WASM and in a one-thread build (`runtime.md`: Starting a program):
  plain rc counts, no safe points, locks without atomics; the conformance suite also run in that
  build.

**P41. Testing the parallel runtime** (`crates/rt` tests and benches, `tools/ci.py`,
`conformance`; L, its logic half before P38's first stage).
- Logic: the conformance suite and every mod's tests also run natively under `parallel`
  (`moddil test --runtime parallel`), their results matching `runtime-sim`'s; tests whose result
  depends on interleaving are marked, and checked for a valid result rather than one result. A
  stress suite of random task trees (channels, `select`, cancellation, isolated traps, scoped
  borrows, `Rc`s shared across tasks), thousands of seeds per CI run. ThreadSanitizer builds of
  `moddil-rt` on Linux. The scheduler's Rust core (deques, stealing, parking, merge queues)
  model-checked with `loom` and randomised with `shuttle`.
- Performance: benchmarks in `crates/rt/benches` and Moddil programs under `moddil bench`, each
  with a budget: spawning, a task switch (tens of nanoseconds), channel ping-pong, `Mutex`
  contention, shared and unshared `Rc` operations, reactor throughput, and scaling from 1 to N
  cores on parallel map and tree work. CI runs the small budgets on every change; scaling to 64
  and 128 cores runs on demand on a large machine.

**P42. The Hashiverse CLI/native client**, phase A, through the C ABI
([`examples/hashiverse.md`](examples/hashiverse.md); XL).

**P43. The LLVM release backend**, beside Cranelift for release builds (XL). Its route,
translating the Cranelift IR or lowering from MIR, is open
([`compiler.md`: Open questions](compiler.md#open-questions)); on one pinned LLVM major version,
upgraded about once a year in one change ([`compiler.md`: Backends](compiler.md#backends)).

**P44. Python beyond the first bindings** (`crates/py`, `crates/moddil`; L)
- `async` functions as `asyncio` awaitables, on Moddil's executor.
- More than one call into Moddil at a time (free-threaded CPython's separate pools).
- `moddil bind --lang python`.

**P45. Talk**, the showcase, published as `moddil-talk` ([`examples/talk.md`](examples/talk.md);
XL): on the CPU pool first, then on WebGPU.
- `moddil/tensor` loading safetensors, memory-mapped, with shapes checked at load time
  ([`accelerators.md`: Loading model weights](accelerators.md#loading-model-weights)).
- `moddil/audio`, the `Audio` effect with OS, Web Audio and simulated handlers
  ([`stdlib.md`: Audio](stdlib.md#audio)).
- English G2P: a lexicon, with a small open-weights neural model for unknown words.

**P46. A backend service**: the web store API with `sql`, HTTP and deterministic simulation
([`examples/web-store.md`](examples/web-store.md); XL).

**P47. Distributed programs, v0** ([Staging: distributed programs](#staging-distributed-programs);
XL).

**P48. The browser**: the Hashiverse web client and the game client (XL, after P42).

**P49. fumoca**: its `engine`, `sim` and `gpu` replaced together, its workbooks compiled to
kernels through a `dyn_kernel` shader-builder mod with bindings typed and checked at launch
([`accelerators.md`: Kernels built at run time](accelerators.md#kernels-built-at-run-time); XL).

**P50. Swift, Kotlin and TypeScript bindings** ([`ui.md`](ui.md); XL).

**P51. WASI components with stack switching**, once Wasmtime's support leaves Tier 3; and JSPI's
per-switch cost measured on the game client ([`compiler.md`](compiler.md); L).

**P52. moddil.com** with the playground, once `/play` runs real programs: Cloudflare Pages, with
Astro (Starlight) for the site and, after P23, `moddil doc`'s API pages and checked code blocks
([`web.md`: Hosting](web.md#hosting); L).

**P53. The semantic diff** and per-definition content hashes, which give structured storage's
benefits over plain text files ([`semantic-vcs.md`](semantic-vcs.md); L).

**P54. The generated Rust bridge** (a Moddil mod as a Rust crate, and Rust crates read from
rustdoc's JSON) and **`moddil migrate rust`**
([`interop.md`: Rust: the generated bridge](interop.md#rust-the-generated-bridge),
[`adoption.md`: Adopting inside existing codebases](adoption.md#adopting-inside-existing-codebases); XL).

**P55. The mechanised core model** in Lean ([`model/`](../model/README.md)), from which the
reference's core rules are generated and against which the reference interpreter is validated
([`definition.md`](definition.md); XL).

**P56. Packaging** (XL): registry dependencies, a lockfile, targets and profiles, executables
with dependencies of their own, `mod name;` with a generating attribute, and `String`, `Vec` and
`core` as `moddil/core-*` mods.

**P57. Header import through libclang** (`#[c_import]`), with annotation overlays; C on WASM (L).

**P58. Graphemes**, from Unicode's data (`mods/text`; M).

**P59. The Mandelbulb browser program** as an exported `Flight` the page drives, in the Svelte
app (M).

**P60. `moddil bench`**, with budgets that fail CI; the HFT hot path's latency is measured that
way, not checked by the compiler ([`examples/hft-platform.md`](examples/hft-platform.md); L).

**P61. Units** ([`units.md`](units.md); XL).

**P62. `Simd`** ([`performance.md`](performance.md); XL).

**P63. Linear types** ([`resources.md`](resources.md); XL).

**P64. Generators and coroutines** (XL).

**P65. Data contracts** ([`data.md`](data.md); XL).

**P66. After v1: reconsider self-hosting**
([`compiler.md`: Staged bootstrap](compiler.md#staged-bootstrap)): what would justify it, and
whether Moddil-core (the stage 0 subset) is the right size to write a compiler in.

## Known limits, with no package yet
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
   comes first (P42), and the browser client follows (P48).
2. **Multiplayer game.** Multi-tier, browser, real-time, WebGPU.
3. **Web store.** Everyday ergonomics, databases and the mobile question.
4. **HFT platform.** The hardest latency requirements and the flagship "same code in simulation and live"
   demonstration.

The order is open for discussion. HFT has the strongest single demonstration of effects, but the
largest scope.

The **flagship showcases** are not in this order: each is built as soon as what it needs is,
because they exist to market Moddil to its first audiences.
1. **[Mandelbulb flight](examples/mandelbulb.md), first:** built, the simplest program that runs
   on the GPU in a browser.
2. [Talk](examples/talk.md), with the Python bindings (P45).
3. [fumoca](examples/fumoca.md), with the browser (P49).

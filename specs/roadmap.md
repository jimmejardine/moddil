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
- A tagged literal's fields (`comptime.tag.fields.1`) of types other than `String` and
  `Option<String>` (a `Match`, a number), named by the tag; and a way to name the struct `read`
  gives, so a function can return one.
- Fields for a tag taking `&[&str]`, and for a tag of the literal's own mod (E1012 now): the
  second needs the mod collected without the literals that use it first.

**P5. Numbers** (`crates/sema`, `crates/native`, `crates/wasm`, `crates/rt`; L)
- `usize` and `isize` 32 bits wide on wasm32 (`num.types.2` says a pointer's width; they are 64
  everywhere today): needs 32-bit layouts there, for pointers, borrows and `Buf`, through sema's
  `size_of`, the lowering, the runtime and the shower, with a conformance test's value per target.

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
- `EnumSet`'s bits in the smallest unsigned integer that fits its enum, as `stdlib.md` decides
  (for packed structs and wire formats): a `u128` for now. It waits on generics the compiler
  does not have (I0001 today): bounds on a struct's type parameters, bounds on an associated
  type (`type Bits: BitWord`), and a projection as a field's type (`bits: T::Bits`). Then
  `#[derive(Variant)]` picks `Bits` from the enum's number of variants.
- Perceus-style reuse of a node no one else holds, as a pass on MIR (`mir.md`: Reference
  counting). It needs a MIR pass layer (a query over `mir`, which the runners then call),
  `Rc`'s operations visible to MIR (today `Rc` is prelude code that MIR sees as ordinary
  calls), updates of the persistent collections that take `self` (they borrow it, so the old
  node is never dead), and a count of allocations to test the reuse with.

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

The parallel runtime is three packages, done in this order: what is left of P40 and P41's logic
half, then P38's stages, with P41's benchmarks as each lands (`runtime.md`).

**P38. The parallel runtime** (`crates/rt`, `crates/sched`, `crates/native`; L, after P40, with
P41's tests). In stages, each a commit with P41's suite green:
1. (Tasks at once, as coroutines on a worker for each turn, are built: `crates/rt/src/turns.rs`
   and `baton.rs`.) Under the parallel runtime an `Rc` is counted atomically by every task, as
   no task is its owner. Biased counting comes back with the running task's number from its
   stack (stacks reserved aligned, the task's context at the base), and the merge queue
   (`memory.md`): a non-owner that takes the shared count below 0 queues the allocation to its
   owner task, which the runtime drains when it resumes the task and at its end. The run's own
   task runs on the thread that began the run, not as a coroutine, so it cannot be stopped at a
   safe point of an aborted run.
3. (A queue a worker, `crossbeam-deque`, with stealing, workers asleep woken one at a time, and
   a task given its turn on a worker queued on that worker, are built.) The scheduler's state
   per worker: the turns, the tasks and the channels are still under the run's one lock, which
   every turn's change takes (a ping-pong of tasks, about 0.8 µs a switch).
4. (Preemption is built: a thread sets the safe-point word for the last 200 µs of each 10 ms
   slice, and a task on a worker past its slice gives its turn to a ready one.) `pinned`
   executors' code leaving the checks out. Eight CPU-bound tasks take 1.22 s under `parallel`
   with coroutines, against 1.04 s with a thread each (stage 1): what costs it is still to find.
5. (Each executor's own workers are built, `crates/rt/src/executors.rs`: pinned to its cores,
   with the OS's priority, idle pools lending theirs.) Still: the manifest's core count
   (`[[exe]] cores`, which needs integers in the manifest's TOML subset); `blocking` C functions
   (a `#[blocking]` on an `extern` function) running on the `blocking` executor, which starts
   all its threads now rather than growing on demand; priority inheritance (a task parked at a
   sync cell lending its executor's priority to the task holding it, which the cell would have
   to record).
6. (A reactor a run, on `mio`, is built: `crates/os` waits in epoll, kqueue or IOCP, and under
   the parallel runtime `crates/rt/src/reactor.rs` brings the events in while tasks run.) Still:
   one reactor a worker rather than a run; a timer wheel rather than the scheduler's list; a
   write that waits for the socket in the reactor rather than trying again; io_uring.
7. (NUMA nodes' cores, from Linux's sysfs and Windows' `GetLogicalProcessorInformationEx`, and an
   executor made for a node placed on its cores, are built: `crates/rt/src/numa.rs`; tested on
   one node only.) Still: node-local allocator arenas.
8. (The task tree, `moddil/runtime`'s `Introspect::task_tree`, is built: each task's scope,
   executor, and what it waits for and how long.) Still: spans for scopes and tasks; deadlock
   reports from the tree; the tree as values rather than text.

WASM stays single-threaded; Web Workers come later.

**P39. Live update, the MVP** (`crates/native`, `crates/moddil`; L)
- The first step of `build-times.md`'s staging: an indirection table in native debug builds,
  `moddil run --watch`, swaps of changed bodies at suspension points. Its later steps follow as
  packages of their own.

**P40. Hardening the language for threads** (prelude `pointers`, `mods/sync`, `crates/rt`,
`crates/native`, `crates/mir`; L, before P38 runs two tasks at once). Each item a commit, the
single-threaded behaviour unchanged. Each commit's message gives single-threaded benchmarks
before and after (rc-heavy code, locks, tight loops): recorded, not gated.
- `Rc`'s rebias on a unique transfer (a channel send, a move into a child). The biased counts
  are built (`rc.task.1`); the last owner is found by reading the owner's count, which holds
  while one task runs at a time.
- The runtime's state: `crates/rt`'s one mutex split, the scheduler state per worker; a
  thread-safe allocator with per-thread caches. (The allocation count already takes no lock, and
  a lazy `const`'s value is computed by one task while the others wait.) A `const` whose
  computing traps in an isolated scope leaves the others waiting for it.
- The conformance suite also run in the one-thread build (`--threads one`, built: atomics as
  plain reads and writes, no safe points, the task's number by a call).

**P41. Testing the parallel runtime** (`crates/rt` tests and benches, `tools/ci.py`,
`conformance`; L, its logic half before P38's first stage).
- Logic (the conformance suite, every mod's tests and the stress suite run natively under
  `parallel`, `item.test.value.2` marking what may differ; the workers' sleeping under `loom` and
  `shuttle`, `ci.py loom`; the runtime's and scheduler's tests under ThreadSanitizer, `ci.py
  tsan`; are built): `select` in the stress suite; ThreadSanitizer over tasks (see Known
  limits); `loom` over the deques,
  the turns and the reactor, not only the sleeping.
- Performance (`crates/native/benches/runtime`, `ci.py bench`: spawning, a switch, channels,
  locks, `Rc`s, a loop on every core, under each runtime, are built): budgets that fail CI, and
  scaling curves from 1 to 128 cores on a large machine. A spawn costs about 2.5 µs (`sim`) to
  6.4 µs (`parallel`) and a switch about 0.8 µs: both still the run's one lock.

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

**P67. The rc cycle detector** (`crates/native`, `crates/interp`, `crates/rt`, prelude; L).
Debug builds and `runtime-sim` report leaked `Rc` cycles with their allocation sites
(`memory.md`: no garbage collector). The compiler gives each type a trace of the `Rc`s it holds;
the runtime keeps the live allocations, and at a run's end (or on asking) finds those only
reachable from each other.

**P68. A trap in `main` under `moddil run --wasm`** (`crates/wasm`, `crates/moddil`; M). A run of
a `main` that traps (`assert(x == 2)` failing) stops with "the WASM run stopped (a backend bug):
an unknown command: " instead of reporting the trap with its parts and exiting with 70, as the
native and interpreted runs do. A test that traps under `moddil test --wasm` is reported
properly; the `main` path (`run_main`, and the host's `main` command) is not.

## Known limits, with no package yet
- ThreadSanitizer cannot follow tasks yet: on coroutines' stacks it crashes in
  `__tsan_func_entry`, with or without its fiber calls (`__tsan_switch_to_fiber`, tried and left
  out), so `ci.py tsan` runs only the runtime's and scheduler's own tests, which switch no
  stacks.
- On Linux, the CLI's test of the tour serving natively has its connection reset about one run
  in five (`cli.rs`, `the_tour_serves_natively`), before and after the OS layer moved to `mio`;
  draining or lingering on close did not change it.
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

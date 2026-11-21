# Roadmap and cross-cutting questions

> **Status:** minor questions. The single place for planning: strategic and technical questions that
> span several areas, compiler milestones, staging, and the order of the reference examples.

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
10. **Minimum viable language — decided:** M0 implements the v0 language in ten vertical slices, with
    a list of what waits until later ([M0: the v0 language in slices](#m0-the-v0-language-in-slices)).
11. **How is the language defined?** A prose reference, a formal semantics, or the MIR interpreter as
    the executable reference? How are conformance tests organised? **Decided:** a normative
    reference with formal core rules, the interpreter as its executable form and a mechanised core
    model, plus a shared conformance suite ([`definition.md`](definition.md)).

## Milestones

| Milestone | Contents |
|---|---|
| **M0** | The grammar source first ([`definition.md`: One grammar source](definition.md#one-grammar-source)), then the compiler front end in Rust, grown in vertical slices ([M0: the v0 language in slices](#m0-the-v0-language-in-slices)): parser, type, effect and ownership checker, MIR, MIR interpreter. Small programs run. The separate **reference interpreter** starts alongside ([`compiler.md`: Two interpreters](compiler.md#two-interpreters)). **The tour** becomes the `tour/shorty` mod and must parse and check in CI ([`tour.md`: From markdown to source](tour.md#from-markdown-to-source)). |
| **M1** | Cranelift native backend (Linux, macOS, Windows). Own WASM emitter producing core modules with the **JSPI lowering**, run in Node and browsers. |
| **M2** | First standard mods written in Moddil; `moddil/runtime` and `moddil/runtime-sim`; LSP prototype. The tour's tests pass under `moddil/runtime-sim`. **The first GPU path:** kernel lowering through naga to WGSL, the `moddil/gpu` WebGPU handler and CPU simulator, and enough of `moddil/web` for a canvas and input; the **Mandelbulb flight** showcase runs in browsers as its test ([`examples/mandelbulb.md`](examples/mandelbulb.md)). |
| **M3** | **Hashiverse CLI/native client** (phase A). LLVM release backend. **Python bindings** (`moddil bind --lang python`, wheels), the first binding target ([`adoption.md`: Python first](adoption.md#python-first)), shown off by **Talk**, on the CPU pool first and then on WebGPU. |
| **M4** | **A backend service**: the web store API with `sql`, HTTP and deterministic simulation; distributed v0 ([Staging: distributed programs](#staging-distributed-programs)). |
| **M5** | **Browser**: the Hashiverse web client, the game client and **fumoca's engine**; Swift, Kotlin and TypeScript bindings ([`ui.md`](ui.md)). WASI components with stack switching once Wasmtime's support leaves Tier 3. |
| **After v1** | Reconsider self-hosting ([`compiler.md`: Staged bootstrap](compiler.md#staged-bootstrap)). |

The order puts the two first audiences, Python teams and backend teams, before the browser
(decided November 2026).

## M0: the v0 language in slices

**Decided: M0 grows by vertical slices.** Each slice runs end to end (grammar, parser, checker, MIR,
MIR interpreter) before the next begins, and lands with its reference paragraphs and conformance
tests ([`definition.md`: Written with the compiler](definition.md#written-with-the-compiler)), so
something runs from the first slice.

| Slice | Adds | Why here |
|---|---|---|
| **S0** | `fn`, `let`, `int` and `bool`, arithmetic with overflow traps, `if`, blocks, calls; the conformance runner (`// =>`, `//~ E…`, `//~ trap`) | The smallest program that runs, and the harness for everything after it |
| **S1** | Structs, enums, tuples, exhaustive `match`; loops (`loop`, `while`, `break`, `continue`); `assert` ([`testing.md`: Assertions](testing.md#assertions)) | Data modelling, which every later slice needs; loops, which no other slice brings |
| **S2** | Ownership: moves, `Copy`, second-class `&` and `&mut`, drop and RAII, `defer` | The core of the language, tested before generics complicate it |
| **S3** | Traits, generics (checked once; witness tables in the interpreter), coherence; `Option`; arrays `[T; N]` and slices `[T]` | Needed for collections and effects; `Option` is the first generic type |
| **S4** | `fail(E)`, `?`, `try`/`catch`, R3 entry points, `?!` | Errors, the simplest effect |
| **S5** | User effects: traits after `with`, handlers, inference inside a mod, explicit at `pub` | The distinctive feature, built on traits and `fail` |
| **S6** | Several files and mods: the module tree, visibility, the manifest | The tour has several modules |
| **S7** | Closures, strings, `Vec` and `Map` written in Moddil as the first core mods; collection literals through `FromArrayLiteral` ([`stdlib.md`: Collections](stdlib.md#collections)) | Dogfooding starts |
| **S8** | Refinements (`where`), runtime `requires`, derives (a minimal comptime); `#[around]` wrappers ([`comptime.md`: Function wrappers](comptime.md#function-wrappers)) | The tour's `Code` and `Target` types and its derives |
| **S9** | `scope`, `spawn` and `suspend` under a deterministic interpreter scheduler | The tour's `serve`; **the tour parses and checks**, M0's exit gate |
| **S10** | Numbers: the sized integers, `f32` and `f64`, literals typed by context, the explicit conversions (`to::<T>()` and the others), wrapping operators, `const` items, and the float methods ([`numerics.md`](numerics.md)) | The [Mandelbulb](examples/mandelbulb.md) core checks, and its tests and CPU preview run; its GPU path follows in M2 |

- **S1 is monomorphic (decided):** structs and enums take no type parameters until generics arrive
  in S3, so `Option` comes with them. S1 has no ownership checking yet and copies aggregates; its
  conformance tests never use a value after moving it, so they stay valid when S2 arrives.
- **S2 accepts `#[derive(Copy, Clone)]` early (decided):** structs and enums can be `Copy` from
  S2, checked as [`values.md`](values.md#opt-in-copy-with-a-size-cap) says, although derives in
  general arrive with S8; `Clone` gains its method with traits in S3. S2's drops have no
  hand-written `drop` yet (that needs `impl`, S3), so RAII is observable through `defer`.
- **S3's scope (decided):** type parameters with bounds and `where`; traits with default methods,
  supertraits and associated types; inherent and trait impls, with one impl per trait and type;
  method calls; operators through the prelude's traits; `#[derive(Copy, Clone, PartialEq, Eq,
  PartialOrd, Ord)]`; destructors (`impl Drop`); and the prelude, written in Moddil
  ([`reference/2027/prelude.moddil`](../reference/2027/prelude.moddil)). Specialisation, `dyn`,
  GATs, const generics, default type parameters, `impl Trait`, inferred generic signatures and
  trait aliases come later.
- **S3's interim rules**, until the features they wait for arrive:
  - the operator traits take `Self` on the right (`Add` has no `Rhs` parameter until default type
    parameters arrive), and compound assignment (`+=`) is only for `int` and `bool`;
  - `Ordering`, `PartialEq`, `PartialOrd`, the operator traits and `drop` are in the prelude, until
    operator traits live in a core mod (S6 made the prelude's names shadowable);
  - a type argument cannot be a borrow, since borrows are second-class and a type parameter can be
    stored in a struct;
  - tuples are `Copy` and `Clone` when their elements are `Copy`, and do not implement
    `PartialEq` or `PartialOrd` yet;
  - a trap in the prelude's code is reported where the file's code called it, pending the
    caller-locations question below;
  - a temporary that needs dropping is dropped at the end of its statement
    ([`resources.md`](resources.md)); one that does not still lives to the end of its function,
    as in S2, which nothing can observe.
- **S4's scope and interim rules (decided):** `fail(e)`, `?` (on calls, `Result` and `Option`),
  `?!`, `try` and `catch`, `errdefer`, `discard`, `Result` in the prelude, and R3 for `main`,
  `defer` blocks and destructors; tests fail when an error escapes them. Until the features they
  wait for arrive:
  - errors are values of named types (structs and enums); inferred tags wait for their syntax
    ([`error-handling.md`: Open questions](error-handling.md#open-questions));
  - a function writes its error set (`with fail(A | B)`), until S5 infers a private function's;
  - an error set is a type only as `Result`'s error and in `fail(…)`, and cannot be written
    there yet (it is inferred: `let r = try { … }`);
  - a note after `?` is checked but not kept, since traces and `fmt` strings come later; string
    literals appear only as reasons and notes until `String` arrives in S7.
- **S5's scope and interim rules (decided):** traits and context data after `with` in a
  signature, named or not; handlers installed with `with E = h { … }`, values of named types or
  borrows of them, found by exact effect; a private function's effects and errors inferred inside
  the file, which is one mod until S6, and written out on `pub` functions, trait methods and entry
  points; `&dyn Trait` and `&mut dyn Trait` borrows, with upcasting to a supertrait; handlers passed
  as hidden `&dyn` parameters in MIR. Until the features they wait for arrive:
  - a handler is a value of a named type, or a borrow of one: anonymous impls (`impl Time { … }`
    as an expression), and closures installed as handlers, come with closures in S7;
  - a handler cannot hold another handler it captured when built, since a struct cannot hold a
    borrow until view types arrive;
  - `main` takes no root capabilities, and there are no standard effect traits (`Time`, `Net`)
    yet: a program defines its own;
  - `suspend` and `spawn` wait for S9, and effect polymorphism over closures for S7;
  - `dyn` names one trait, with no associated types fixed, and a trait with associated types
    cannot be used as `dyn`;
  - the reference interpreter finds a generic function's effect (`Store<T>`) by the types it knows
    its type parameters stand for, from the values of its arguments.
- **S6's scope and interim rules (decided):** the module tree from the file tree; private,
  `pub(mod)` and `pub` items; paths from `crate`, `self`, a dependency or a name in scope; `use`
  with lists, `as` and `pub use`; no glob imports; the prelude as shadowable names; `pub` items
  exposing nothing less visible; the manifest, `moddil.toml`
  ([`modules.md`: The manifest](modules.md#the-manifest)), with a library root, executables and
  dependencies by path, which form a DAG; inference within a mod, written out at `pub`. Until the
  features they wait for arrive:
  - a manifest has no versions, registry dependencies, lockfile, capabilities, targets or profiles,
    and its edition is 2027;
  - an executable's main file is a module of its mod's tree, and executables have no dependencies
    of their own;
  - visibility does not yet limit methods or fields;
  - a mod cannot declare a prelude, so there are no glob imports at all, and `mod name;` with a
    generating attribute waits for comptime;
  - `moddil test` runs the tests of the mod being built, and the conformance runner those of every
    mod of a test.
- **The path root becomes `mod` (decided, [`modules.md`: Imports](modules.md#imports)): the first
  step of the slice after S7.** S6 shipped Rust's `crate::`; Moddil has mods, so `use crate::a`
  becomes `use mod::a`, matching `pub(mod)`. `mod::a` and the item `mod name;` differ at their
  second token, so the grammar stays LL(3). A mechanical change, with no other change in behaviour:
  - the grammar: drop `crate` from the keywords; `mod` starts a path in `use_segment`,
    `type_segment`, `pattern_path` and `path_start`; regenerate `kinds.rs` and the tree-sitter
    grammar;
  - `sema` (`modules.rs`, `resolve.rs`, `collect/mod.rs`) and `refinterp` (`program`, `machine`),
    including the diagnostics that print paths;
  - the grammar corpus, `conformance/2027/modules/`, `reference/2027/modules.md` and `names.md`,
    and a conformance case in which `crate::x` is an ordinary unknown name;
  - `examples/mandelbulb/src/`;
  - done when `python tools/ci.py fmt clippy test grammars` passes and `grep -rw crate` finds no
    Moddil source outside that conformance case.
- **Two open questions must be settled before S7**, because S7 writes the first library code in
  Moddil:
  - **caller locations** ([`error-handling.md`: Open questions](error-handling.md#open-questions)),
    so that a trap in `unwrap`, `expect` or `xs[i]` reports the user's line, not one inside the
    prelude or `Vec`. Settle it before the prelude gains a method that traps;
  - **the allocator in scope**, which fixes the constructors of `Vec` and `Map`: settled as an
    ambient scoped handler ([`memory.md`: The allocator in scope](memory.md#the-allocator-in-scope)),
    built on S5's handlers.
- **Deferred past M0:** units; `Fixed` and `Simd`; views and linear types; generators and coroutines;
  full comptime and reflection; `unsafe` and FFI; GPU; distribution; data contracts.
- **The tour only has to check at M0**, so its library dependencies (HTTP, serde, `Url`) are
  signature-only stubs until M2, when its tests must pass.
- **The reference interpreter** starts at S1 and follows each slice, so differential testing starts
  early; **work on the Lean core model** starts after S5, once ownership, traits, errors and effects
  have settled ([`definition.md`: Three artefacts, one meaning](definition.md#three-artefacts-one-meaning)).

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

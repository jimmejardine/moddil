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
| **S11** | `String` rewritten in a core mod over `Buf<u8>`, in the decided small-string form (inline `[u8; 23]` or a heap buffer), with `str` a view of UTF-8 bytes ([`text.md`: String types](text.md#string-types)) | `u8` exists from S10; the built-in `String` is the last core type the interpreters implement natively |

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
  - a trap in the prelude's code is reported where the file's code called it (S7a made this
    `#[track_caller]` on the prelude's functions that can trap);
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
  - visibility does not yet limit methods or fields (done in the M0 debt sweep, `mod.vis.2`);
  - a mod cannot declare a prelude, so there are no glob imports at all, and `mod name;` with a
    generating attribute waits for comptime;
  - `moddil test` runs the tests of the mod being built, and the conformance runner those of every
    mod of a test.
- **S7 is built in three parts (decided November 2026)**, each committed on its own: **S7a**
  closures, the closure traits, captures, anonymous impls and `#[track_caller]`; **S7b** `char`,
  `str`, `String` and their literals; **S7c** arrays, slices, `Vec` and `Map` written in Moddil as
  core mods, `FromArrayLiteral`, `for` and `Iterator`.
- **S7a's scope and interim rules (decided):** closures `|p| body` with inferred captures (shared,
  `&mut` or moved); `impl Fn(…) -> R` parameters and `F: Fn(…) -> R` bounds over the prelude's
  `FnOnce`, `FnMut` and `Fn`; a closure's kind checked against the bound; closures second-class;
  a closure's errors propagating through its callee, whose call is marked, and its effects met
  where it is written; anonymous impls (`impl Trait { … }` expressions) as handlers and values;
  `#[track_caller]`. Until the features they wait for arrive:
  - a closure cannot be stored or returned: `Box<dyn Fn…>` and `impl Fn` results come with `Box`;
  - closures are not `Copy`, and `escape` waits for S9;
  - an anonymous impl's methods take `&self` and cannot name `self`, and it implements its trait
    only, not the trait's supertraits;
  - a `match` on a captured value borrows it, so moving out of it in a pattern is not a capture
    by move;
  - the reference interpreter calls a closure by name (`f(x)`), not through its trait
    (`Fn::call(&f, (x,))`).
- **S7b's scope and interim rules (decided):** `char`, `str` (only behind a borrow) and `String`;
  character, string, raw and dedented block literals; `&String` converted to `&str` where one is
  expected; byte-wise equality and ordering; character and string patterns; the prelude's
  `Display` and checked `fmt"…"` holes; `len`, `push_str`, `push`, `pop`, `trim`, case conversion
  and the like. `String` and `str` are **built in for now**: both interpreters implement them
  natively, through the prelude's `#[intrinsic]` methods. Once S7c's raw buffer and S10's `u8`
  exist, `String` is rewritten as the decided small-string form, inline `[u8; 23]` or a heap
  buffer ([`text.md`: String types](text.md#string-types)), not as a wrap of `Vec`: done in S11,
  in the prelude (a core mod waits for packaging). Until
  the features they wait for arrive:
  - no `StrPos` or slicing (views; both since S13), no `chars()` (`Iterator`, S7c), and no `b"…"` or `b'c'`
    (`u8`, S10);
  - `Display` is `fn fmt(&self, out: &mut String)`, not a `Write` sink; `fmt` holes take no format
    specs (`{x:>8}`) and `Debug` waits for derives of it; tagged literals other than `fmt` wait
    for comptime;
  - `#[intrinsic]` is the prelude's alone (and the built-in `core` mod's).
- **The cost of the prelude in every program: done (November 2026), no redesign needed.**
  - **What was slow.** Most of the test suite's time went into collecting the item tree,
    about 0.85 s per program. Re-checking the prelude's bodies was not the cause. `node_at`
    searched the whole syntax tree once per function. Now it goes down only through the nodes
    that contain the range. Collecting takes 28 ms, and the full `cargo nextest run` takes 11 s
    instead of 66 s.
  - **Not done: leaving the prelude's bodies out of each program's checks.** Measured before
    the fix, it saved only 14%, and then a broken prelude would show only in
    `the_prelude_and_its_derives_check_cleanly`, which does not borrow-check it. Revisit it
    only if the prelude's bodies become a large share of a check again.
- **The conformance suite's shortcomings: swept (November 2026).** A review of all 181 tests
  found:
  - tests that skipped the rule they cited;
  - results that could not tell implementations apart, such as a length or a sum where the order
    was the claim;
  - cases, error codes and citations that the paragraphs called for and no test had.

  All of these are fixed: 19 new test files and 55 changed ones, about 860 lines.
  - **The bugs the stronger tests showed, now fixed:**
    - a trait's method called on an error set ran nothing;
    - errors that a bound on `F::Fails` takes could leave the function (now E0502);
    - a borrow passed for a type parameter came back borrowing nothing (`id(&x)`);
    - a closure's call could name its arguments;
    - E0607 was reported on every inferred function;
    - the lexer rejected `'\''`, `_` in `\u{…}` and line continuations;
    - a block literal's errors were reported where nothing could expect them.
  - **The runner and the reference:**
    - three files' 22 values never ran, because the files expected diagnostics too; the runner
      now fails such a file;
    - the reference now says what `set_capacity` does below the length, which code a block's
      indentation errors are, and that a result of a type parameter can hold a borrow.
  - **Left open:**
    - a `send` in a `select` that doesn't wait for its value to be received;
    - a `join` of a cancelled task unwinding the task that joins;
    - the I0001 limits. Conformance doesn't test "not supported yet", which belongs to an
      implementation.
- **Borrows of static data borrow nothing (decided, [`memory.md`: Borrows of static data](memory.md#borrows-of-static-data)):
  done in S7c (November 2026).** A string literal (and, once S10 brings `const`, a
  `const`'s value) is static data:
  - drop E0310 for a function with no borrowed parameter; at each return of such a function, the
    borrow must be of static data, or it is E0308;
  - in the ownership check, a borrow of static data freezes nothing and never ends;
  - update **item.fn.ret-borrow.1** and `reference/2027/borrows.md` (a paragraph for static data),
    and the E0310 test in `crates/sema/src/tests.rs`;
  - conformance cases: `fn greeting() -> &str { "hi" }` runs; `fn f() -> &int { &1 }` is E0308;
    a static result stays usable after everything else is moved.
- **S7c's scope and interim rules (decided November 2026: one slice, maps included):** arrays
  `[T; N]` and slices `[T]` with indexing that traps out of bounds; const parameters
  (`const N: int`); views (`view Name { … }`), with borrows allowed as type arguments and in
  tuples, which makes them views; `for` over `IntoIterator` and `Iterator`, and ranges; `Buf<T>`,
  built-in storage of fixed capacity, and `Vec<T>` written over it in Moddil; `Index` and
  `IndexMut`; collection literals typed by context through `FromArrayLiteral`; `Hash`, `Hasher`
  and `#[derive(Hash)]`; and a built-in mod `core`, every mod's dependency, whose
  `core::collections` holds SipHash-1-3, `HashMap`, `HashSet` and the `Map` and `Set` traits,
  written in Moddil. Until the features they wait for arrive:
  - `Buf` uses the default allocator: `with Alloc = …` waits for an allocator to install
    (`Arena`), and `Vec::new_in` with it;
  - const parameters are `int`s; a length is a literal or a const parameter, with no arithmetic
    (comptime, S8); `[x; n]` as an array needs `Copy` elements, and an array is `Clone` only when
    it is `Copy`;
  - no slice patterns, no slicing by a range (`xs[a..b]`) (both since S13), no `IterMut` and no iterator adaptors
    (`map`, `filter`), which need closures stored in views; `Index` takes an `int`;
  - a borrow still cannot point to a borrow, so a map's keys cannot be borrows
    (`HashMap<&str, V>`): `PartialEq` and `Hash` for `&T` need it; use `String`;
  - the hasher's key is fixed, so map order is the same on every run; keying it from `Rand`, and
    tests printing a fresh seed, come with randomness; `OrderedMap` and `TreeMap` are for
    `moddil/collections`;
  - a `for` over a value of a type parameter needs the parameter bounded by `Iterator` (an
    `IntoIterator` bound cannot yet say its iterator's type is one);
  - a method of a view that returns, through a trait whose signature holds no borrow, a borrow of
    one of the view's own fields (not of what it borrows) is not caught;
  - the reference interpreter types a collection literal by its context only where a `let`'s type,
    a called function's parameter or a function's result names the type;
  - `core` is built into the implementations; the `moddil/core-*` mods and their names come with
    packaging; `String` was rewritten over `Buf<u8>` in S11, in the prelude.
- **S8 is built in three parts (decided November 2026)**, each committed on its own: **S8a**
  compile-time evaluation (`const` items, `const fn`, `const { }`) and the `Debug` and `Default`
  derives; **S8b** refinements (`type X = T where p`: a distinct type, `X::new` failing with
  `RefinementError`, conversion to the base type, refined literals checked when compiling, `p: X`
  patterns) and contracts (`requires`, `ensures` and `old`, pure, trapping when broken, every one
  checked for now); **S8c** `#[around]` wrappers. Comptime stays minimal: derives remain written
  by the compiler, and `reflect(T)`, user-written derives and the typed construction API wait.
- **S8a's scope and interim rules (decided):** const items, kept as functions of no parameters
  whose value is computed when compiling, by the MIR interpreter, with fuel (10 million steps);
  `const fn`; compile-time code calls only `const fn`s and the prelude's functions, with no effect
  and no error; a borrow of a const is static data; `Debug` (with `{x:?}`) and `Default`, derived
  and for the built-in types; attributes may be named by keywords (`#[default]`). Until the
  features they wait for arrive:
  - array lengths and const arguments are still literals or const parameters, not consts, since
    types are checked before consts are evaluated; there are no associated consts;
  - a const holds no borrow but of text; a `const { }` block is checked as compile-time code but
    evaluated where it is, each time; `#[fuel(…)]` waits;
  - `Debug` is `fn fmt(&self, out: &mut String)`, as `Display` is, and other format specs
    (`{x:>8}`) wait; `Debug` is not implemented for borrows (`&T`), which needs a borrow of a
    borrow;
  - the reference interpreter finds a derived `Default`'s fields' defaults by their types as
    written, and a `Default::default()` call's type by its context as it does a collection
    literal's.
- **S8b's scope and interim rules (decided November 2026):** refinements, `type X = T where p`, a
  distinct type used as its base (it converts to it, and has its methods, operators and traits),
  made by `X::new(v)` (failing with `RefinementError { ty, clause, value }`, `value` the rejected
  value's `Debug`), by a literal checked when compiling, or by a `q: X` pattern; plain aliases,
  `type X = T;`; contracts, `requires` and `ensures` with `result` and `old(e)`, pure, every one
  checked, a broken precondition reported at the call; and `Iterator::all` and `any`. Until the
  features they wait for arrive:
  - `type` items are not generic, and a refinement has no impls of its own; its base holds no
    borrow;
  - a refinement pattern's base is `Copy`; a refined literal is recognised where the type is
    written (a `let`, a parameter) or is the function's result; parenthesised refinements in
    types (`(u32 where self >= 1)`), niches and proofs wait;
  - `old(e)` needs a `Copy` value; `invariant`, `requires always`, contracts on trait methods and
    the `Contracts` handler's policies wait;
  - an alias may name only the aliases before it.
- **S8c's scope and interim rules (decided November 2026):** function wrappers, `#[around(w)]`,
  expanded as [`comptime.md`: Function wrappers](comptime.md#function-wrappers) says: `f::Args` a
  generated struct with a field per parameter (a view when one is a borrow, with `Debug`, `Clone`
  and `PartialEq` when every parameter has them, its `Debug` showing `f::Args { … }`); `&INFO` a
  const `FnInfo { path, params: Vec<ParamInfo { name, ty }> }`, the types as written; wrapper
  arguments, `#[around(w(c, …))]`, compile-time code passed before `&INFO`; several wrappers in
  written order, the first outermost; contracts checked around the wrappers. Until the features
  they wait for arrive:
  - a wrapped function is free, not `const`, and its parameters are names; methods wait for a
    field to hold `self`;
  - wrapper arguments are evaluated at each call, as a `const { }` block is, and `f::Args` cannot
    be named in code;
  - `FnInfo` has no attributes or effects (they wait for `reflect`), `moddil explain` does not
    print the expansion yet, and `Debug` of a borrow still waits, so `A: Debug` does not hold for
    a borrowed parameter;
  - a wrapper's errors are reported where `?` would report them: E0501 on a `pub` function that
    does not list them.
- **S9 is built in four parts (decided November 2026)**, each committed on its own: **S9a**
  `scope`, `spawn`, `join`, the control effects `suspend` and `spawn`, and a deterministic
  scheduler; **S9b** cancellation and the unwinding it needs (with `isolate: true` and `Trapped`,
  which need it too), `select`, channels and `stop_requested`; **S9c** the language features the
  tour still needs; **S9d** the tour as `tour/shorty/`, with its libraries as signature-only stub
  mods, checked in CI.
- **S9a's scope and interim rules (decided November 2026):** the prelude's `scope(|s| …)`,
  `s.spawn(f)` and `t.join()`, and `yield_now()`; one task runs at a time, switching only where
  it suspends, first in, first out; a task's error goes to its `join`, or, if it is never joined,
  fails the scope (the first to end wins); `s.spawn(f)` is marked as any call passing a closure
  that can fail is, so the scope's errors are its tasks' too; the scope keeps what its tasks
  borrow until it ends. Until the features they wait for arrive:
  - no cancellation: a scope waits for every task, however many have failed, and a failed body
    waits for them too (S9b);
  - the errors of tasks that lose are dropped, not kept as suppressed;
  - entry points have `suspend` and `spawn` from the runtime; a trap in any task ends the run;
  - the reference interpreter runs each task on a thread of its own, parsing the program again,
    and hands the turn on at suspension points.
- **S9b's scope and interim rules (decided November 2026):** one slice, with all of it. Named and
  default arguments (`syntax.md`: Named and default arguments); unwinding through cleanup edges
  in the MIR (each scope item's cleanup block, chained, ending in `Resume`), used by cancellation
  and isolated traps; a scope's body runs as its first task; a failing body, or a task whose
  `Task` has been dropped, fails its scope and cancels it, and a cancelled task unwinds at its
  next suspension point; `scope(…, isolate: true)` turns a trap inside into `Trapped { class, at
  }`; bounded channels (`recv` gives `None` once closed, `send` fails with `Closed`, capacity 0 a
  rendezvous); `select` on `recv`, `send`, `join` and `stop_requested()`, the first ready arm in
  written order; `s.stop()`, innermost scopes first. Until the features they wait for arrive:
  - name-only `*` parameters wait; a default is compile-time code whose type names no type
    parameter;
  - only a literal `false` (or leaving `isolate` out) keeps `Trapped` out of a scope's errors;
  - a cancelled task's pending operation's value is dropped without its destructors (values
    left in a channel run theirs since the M0 debt sweep); a `send` in `select` does not wait for
    a rendezvous;
  - shielded cleanup with deadlines, timers, `checkpoint()`, signals, grace periods, suppressed
    errors, poisoning and `abandon` wait;
  - `Trapped`'s `at` is `line:column`, without the file's name (`file:line:column` since the
    M0 debt sweep).
- **S9c's scope and interim rules (decided November 2026):** what checking the tour found missing.
  A function named as a value is a closure calling it; `-> impl Trait` is an opaque type whose
  hidden type the function's body fixes, revealed when the MIR is built; `map`, `filter` and
  `collect` (with `FromIterator`); `test "…" for subject`; the ambient `log`; `main` taking the
  root capabilities and returning `ExitCode`; `#[selectable]` on a function a `select` may wait
  on. Until the features they wait for arrive:
  - a type's function (`Vec::new`) and a variant cannot be used as values (done in the M0 debt
    sweep);
  - an `impl Trait` result is the whole result type, not in a trait's method, and its hidden type
    holds no borrow or closure; functions that return one cannot call each other recursively;
  - `map` and `filter` run their closure over every item when called (lazy since S12, with
    `Iterator::Fails`);
  - a test cannot be run for listed types or declare capabilities;
  - `log` prints to standard error, with no handlers;
  - the root capabilities have no operations (a program can no longer make one since the M0
    debt sweep made fields private); `main` may still return any type, which is printed;
  - a `select` arm on a `#[selectable]` function traps when it runs (`not implemented yet`).
- **S9d's scope and interim rules (decided November 2026): M0's gate is met.** The tour is the
  `tour/shorty/` mod, and `python tools/ci.py tour` checks it, against `stubs/moddil/`, a **stub
  mod** (`stub = true`) holding the signatures of the standard mods it uses until M2. Built for
  it: stub mods, whose functions may have no body (calling one traps) and whose traits' derives
  declare empty impls; the root capabilities moved from the prelude to the stubs, marked
  `#[root_capability]`; doc comments (`///`, `//!`), which the compiler skips; a `Pattern` for
  `contains`, `starts_with` and `ends_with` (`&str` or `char`); a bound on `F::Fails` taking a
  closure parameter's errors, which an error set meets when each member does; a borrow of a
  refined value as a borrow of its base; `Option::map`; `true` and `false` in a manifest. The
  gaps the tour exposed are in [`tour.md`](tour.md#gaps-the-tour-exposed), each an open question
  in its area. Until the features they wait for arrive:
  - the tour's tests trap, and its doc example does not run, until the standard mods (M2) and
    doc tests;
  - doc comments are not read: `moddil doc` and doc links wait;
  - a `where` clause before a function's body cannot end with a comma.
- **S10's scope and interim rules (decided November 2026):** one slice. The integer types `i8` to
  `i128` and `u8` to `u128` (`i64` is `int`), and `f32` and `f64`; literals exact until their
  type is known, `int` and `f64` by default, checked against their type (E0204); the operators at
  each type's width, trapping as `int`'s do, and IEEE 754 on the floats; the wrapping operators
  `+%`, `-%`, `*%`; implicit widening where every value fits; `to`, `checked_to`, `truncate`,
  `wrap` and `saturate`; the float methods, through the `libm` crate in both interpreters, so
  every platform gives the same bits. The [Mandelbulb](examples/mandelbulb.md) core checks, its
  tests pass, and its CPU preview runs, all in CI (`python tools/ci.py examples`). The rewrite of
  `String` over `Buf<u8>` became slice S11. Until the features they wait for arrive:
  - `f16`, `bf16`, arbitrary widths (`u3`), `usize`/`isize`, `Wrapping<T>`, decimals, big numbers
    wait (`round::<I>(mode)` and the sized integers' `Hash` came with the M0 debt sweep);
  - a literal gets a type, rather than constant arithmetic on literals being exact (exact since
    the M0 debt sweep, but for a quotient of fractional literals);
  - the float functions other than `sqrt` are `libm`'s, not yet correctly rounded;
  - the reference interpreter gives a literal its type when it meets one at run time (a typed
    `let`, parameter, field or result, or another number in an operator), so it differs from the
    compiler where an unannotated variable takes its type from a later use, and where literals
    meet only each other; the conformance tests avoid both;
  - exhaustiveness on `i128` and `u128` leaves out their greatest value.
- **S11's scope and interim rules (decided November 2026):** `String` is the prelude's struct,
  written in Moddil over `u8` bytes in the decided small-string form: an enum of 23 inline bytes
  with their length, or a heap `Buf<u8>`, so `capacity()` is never below 23. It stays in the
  prelude, next to `Vec`, since the prelude's own `Display` and `fmt"…"` need it; moving both to
  a core mod waits for packaging. `str` stays built in: `String::as_str` views its bytes through
  `str::view`, and `str::byte_at` reads them. In Moddil now: `push` (UTF-8 encoding), `push_str`,
  `pop`, `clear`, `len`, `clone`, comparisons, hashing, `contains`, `starts_with`, `ends_with` and
  `Pattern`. Still native: case conversion, `trim`, the `char` classes, `quoted`, and each
  number's, `bool`'s and `char`'s text for `Display`. Until the features they wait for arrive:
  - the inline size is 23 on every target (11 on 32-bit targets comes with them);
  - `String::from_utf8`, `StrPos`, slicing and `find` wait (done in S13);
  - strings are slower in the interpreters, byte by byte in Moddil.
- **The M0 debt sweep (decided November 2026): one pass over M0's interim limits before M1.**
  - **Paid:**
    - `Hash` for the sized integers, and `f.round::<I>(mode)` with `Round` (`num.round.1`);
    - exact arithmetic on literals: an operator on literals folds exactly before the result gets
      a type, so `200 + 100` as a `u8` is E0204 and `0.1 * 3.0` as an `f32` is `0.3`
      (`num.lit.1`);
    - `Trapped.at` is `file:line:column`, and values left in a channel run their destructors;
    - fields and the functions of inherent impls have visibility, private to their module unless
      marked (`mod.vis.2`), so root capabilities cannot be made outside `moddil::root`;
    - a type's function and a tuple variant are values (`Vec::new`, `Point::y`, `Some`);
    - an impl's bounds see the bounds of the parameters before them (`F: FnMut(I::Item) -> B`),
      and a parameter fixed by a bound on a fixed one is not E0415.
  - **Deferred, each for its own step:**
    - lazy adapters, `Box<T>`, `Box<dyn Fn …>` and `impl Fn` results: a closure kept beyond the
      call needs its bound to name its effects (`Fn(A) -> R with E`, `closures.md`: Effects flow
      through), which the grammar's `fn_sugar` does not have yet, and owned captures (all but
      `impl Fn` results done in S12);
    - slicing and text (`xs[a..b]`, slice patterns, `StrPos`, `find`, `from_utf8`): sub-slice
      borrows need a new kind of pointer in both interpreters (done in S13);
    - the 8 open questions the tour exposed ([`tour.md`](tour.md#gaps-the-tour-exposed)), which
      are decisions (decided after S13);
    - a trailing comma in a `where` clause: a type can start with `{`, so the comma before a
      body is ambiguous to tree-sitter;
    - suppressed errors (they need error return traces), and the destructors of a cancelled
      task's pending `send`.
- **Slice S12 (decided November 2026): closure effects, `Box` and lazy adapters, in one slice.**
  - **Effects in closure bounds** (`expr.closure.effects.2`):
    - `F: Fn(A) -> R with E` in a parameter's or a `where` clause's bound, and
      `impl Fn() with E`. The grammar's `pred_bound` takes the effects; a bare one has no `&T`,
      whose type would take the next `+`.
    - An effect variable `E` stands for the closure's errors. The function's `with E` passes them
      on, and naming `E` without listing it keeps them in.
    - `with fail(X)` fixes them (E0502 for more). A handler's effect in a closure parameter's
      bound is E0609.
  - **`Box<T>`** (`type.box.1`): a prelude struct over `Buf`, with `Deref` and `DerefMut` for
    `*b`. It is the indirection a recursive type needs.
  - **`Box<dyn Trait>`** (`type.box.dyn.1`):
    - `Box<T>` coerces to it. `Ty::Dyn` keeps the associated types a `dyn Fn` fixes.
    - The MIR interpreter keeps the value's type with it (`Value::Owned`); the reference
      interpreter needs nothing new.
  - **Stored closures** (`expr.closure.stored.1`):
    - A `Box<dyn Fn(A) -> R with …>` holds a closure that owns its captures (E0802 for a borrow).
    - Its type fixes its errors and control effects.
    - The handlers' effects it lists are met where it is called (`closures.md`), passed after its
      arguments.
  - **Lazy adapters** (`iter.adapt.1`, `iter.fails.1`, `item.trait.assoc.default.1`):
    - `Iterator` has `type Fails = !` (associated types may have defaults), and `next`,
      `count`, `all`, `any` and `collect` fail with it.
    - `map` and `filter` give `Map` and `Filter`, whose `Fails` is `I::Fails | F::Fails` (an
      associated type may be an error set).
    - A call whose errors are a type parameter's `Fails` passes them on unmarked, as a closure
      parameter's call does.
  - **Interim limits:**
    - no `Box<dyn FnOnce>`, and no `impl Fn` result (both done in S12b);
    - only `Box::new(|…| …)` written where the box is expected meets its needs at the call (a
      closure bound to a local first is checked as an ordinary one);
    - a `Box<dyn Fn>` is called through its box (`f(x)`, `(**f)(x)`), not as a `&dyn Fn`
      parameter;
    - field access and method calls do not look through a `Box` (`(*b).x`; method calls do
      since the tour's questions were decided, field access still does not);
    - a `for` loop over an iterator that can fail with known errors is E0501 (use
      `while let Some(x) = it.next()? { … }`), since how `for` marks one is not decided;
    - the reference interpreter takes `?` on a call of a function whose signature lists `fail`
      as the call's, so `?` on such a call whose errors turn out to be `!`, giving an `Option`,
      differs between the interpreters;
    - `escape` is not implemented, so its rule for stored closures waits.
- **Slice S12b (decided November 2026): `impl Fn` results and `Box<dyn FnOnce>`.**
  - **`impl Fn` results** (`expr.closure.result.1`):
    - A closure a function returns as `impl Fn…` is a stored one. It owns its captures, and its
      result type fixes its errors, control effects and the needs its caller meets:
      `-> (impl Fn() with Clock)`, parenthesised as `syntax.md` says.
    - The checker reads the result type as the `dyn Fn…` it describes (`FnSig::opaque_fn`), for
      the closure's types and for each call of the result. The MIR sees the closure itself.
  - **`Box<dyn FnOnce…>`:** a call moves the box, and the MIR interpreter takes the closure out
    of it.
  - **Captures:** a call of a closure parameter inside a closure is now a read, a write or a move
    as its strongest bound says (`Fn`, `FnMut`, `FnOnce`), not always a write.
  - **Interim limit:** only a closure written as the function's value, or after `return`, is
    taken as the one returned (a closure bound to a local first is not).
- **Slice S13 (decided November 2026): slicing and text.**
  - **Range slicing** (`expr.slice.1`):
    - `xs[a..b]`, with `..=`, `a..`, `..b` and `..`, lowers from the syntax to a HIR `Slice`, so
      no generic `Range<T>` is needed.
    - It works on arrays, slices and a `Vec` (through `as_slice` and `as_mut_slice`).
    - The MIR has `Projection::Subslice`, checked by `Rvalue::SpanOk`.
    - In both interpreters a `Pointer` carries a span over the array it points into.
    - The prelude has `split_at` and `split_at_mut`, the second through a native that splits one
      `&mut` borrow in two.
  - **Slice patterns** (`pat.slice.1`):
    - `[a, .., z]` and `[x, rest @ ..]`, with `rest` a sub-slice borrow.
    - Exhaustiveness is by length: the lengths the rows name, and one longer than all.
  - **Text positions** (`text.pos.1`, `text.find.1`):
    - `StrPos` is a prelude view of `(text, offset)`.
    - `find`, `rfind`, `split_once`, `start`, `end`, `after`, `offset`, `pos_at` and
      `slice_bytes`, on `str` and `String`.
    - `&s[a..b]` lowers to the prelude's `between`.
  - **`from_utf8`** (`text.utf8.1`): on `str` and `String`, validating in Moddil.
  - **Interim limits:**
    - a position used on other text traps when it runs, rather than failing to compile as
      `text.md` says, and it is compared by text, so a position of equal text passes;
    - `#[derive]` does not apply to a `view` (`StrPos`'s `Clone` and `Copy` are written out);
    - integer literals in a collection literal for `Vec<u8>` take `int`
      (`let v: Vec<u8> = [1, 2]` is E0402), so build one from a typed array;
    - no `char_indices`, `bytes`, `graphemes` or `RcStr` slices.
- **The tour's 8 questions (decided November 2026), the last of M0's deferrals.**
  - **Closed without code:**
    - `default` stays reserved (#1);
    - both marks on a failing spawn (#5);
    - a handler installed from a borrow (#3), which existed and which the tour now uses;
    - floats for the lossy network's `rate` (#8).
  - **Built:**
    - a type's function and method sharing a name, told apart by `self`
      (`item.impl.same-name.1`);
    - E0610, for a task spawned into a scope opened outside a `with` whose handler it uses
      (`task.with.1`);
    - `self` in an import group (`mod.use.self.1`). A module's child module is now found
      before its imports, so `use mod::m::{self}` in the root resolves;
    - method calls through `Deref` (`expr.method.deref.1`), so `Mutex::lock()` gives a
      `Guard<T>` in the stubs.
  - **Interim limit:** field access does not look through `Deref`.
- **The path root becomes `mod` (decided, [`modules.md`: Imports](modules.md#imports)): done
  after S7a (November 2026).** In types, `mod` only starts a path (`mod::T`), so `pub(mod)` and a
  parenthesised type differ at their third token. S6 shipped Rust's `crate::`; Moddil has mods, so `use crate::a`
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
  - **caller locations**, so that a trap in `unwrap`, `expect` or `xs[i]` reports the user's
    line, not one inside the prelude or `Vec`: settled as `#[track_caller]`
    ([`error-handling.md`: Caller locations](error-handling.md#caller-locations)), built in S7a;
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

## M1: native code in slices

M1 grows the native backend as M0 grew the front end: vertical slices C0, C1, …, each compiling
more of the MIR with Cranelift, run in process by `cranelift-jit` (writing executables comes
later), and each checked by the conformance suite, which runs every file the backend can compile
and needs its results to equal the interpreters' (decided November 2026). The runtime native code
calls is a Rust crate, `moddil-rt`, for now.

- **Slice C0 (decided November 2026): scalars and calls.**
  - **Values:** `int`, `bool`, `char`, the sized integers and the floats.
  - **Operations:** every operator, with the MIR's overflow, division and shift checks.
  - **Control flow and calls:** blocks, branches and loops, and direct calls of non-generic
    functions, compiled lazily from what a test or `main` reaches.
  - **The ABI:** a status return. Every function returns a status beside its result; a failed
    check calls `moddil-rt` to record its site and returns "trapped", which callers pass on. So a
    trap in a test leaves the compiler's process alone. A 128-bit result comes back through a
    pointer the caller gives.
  - **Recursion:** calls are counted (`moddil-rt`'s `enter` and `leave`), so recursion without
    end traps at the interpreters' depth, 10,000 frames, rather than overflowing the native
    stack. Runs have a 512 MiB stack.
  - **The command:** `moddil run --native` and `moddil test --native`, which report what they
    do not compile yet. The interpreter stays the default.
  - **Checking:** a third conformance runner, `Native`, which skips files that reach anything
    else. `cargo test -p moddil-conformance -- --nocapture` prints how many files each area ran
    natively (22 files at C0).
  - **The toolchain:** Cranelift 0.138, with Rust moved to the latest stable, 1.100.0, for it
    (November 2026).
- **Slice C1 (decided November 2026): values on the stack, and borrows.**
  - **Values:** tuples, structs, enums and arrays live in stack slots. Places are addresses:
    fields, variants, elements (bounds-checked) and dereferences. `match` on enums reads the
    discriminant.
  - **Borrows:** `&` and `&mut` of those places are pointers. A scalar something borrows lives
    in memory; others stay in registers.
  - **Layout:**
    - fields go largest alignment first, so there is as little padding as can be;
    - an enum uses a niche when it can: `Option<bool>` and `Option<Option<bool>>` are one byte,
      and `Option<&T>` and `Option<char>` need no tag;
    - otherwise a tag comes first, then the largest payload.
  - **The ABI:** internal functions use Cranelift's `tail` convention, so a status and up to two
    words come back in registers. An aggregate of up to 16 bytes, splitting into at most two
    pieces, crosses a call in registers; larger ones pass as a pointer to the caller's copy and
    return through a pointer the caller gives. `i128` and `u128` return in registers too now.
  - **Observing values:** a `// =>` value's address goes to `moddil-rt`, which hands it to a
    renderer the host installs for the run; it reads the bytes while they are alive and writes
    them as the interpreters do.
  - **Checking:** 45 conformance files run natively (from 22).
  - **Interim limits:**
    - no slices, `str` or `dyn`, which need wide pointers;
    - no closures, generics or trait methods;
    - no destructors;
    - no errors through the status channel;
    - `#[repr(C)]` is not honoured yet.
- **Slice C2 (decided November 2026): generics and wide pointers.**
  - **Generics:** monomorphised. Each function is compiled once for each set of type arguments
    it is called with, the prelude's and derived code included.
  - **Trait methods:** called statically, through the impl's method or the trait's default. The
    built-in `Clone` of a borrow or tuple is a copy.
  - **`#[track_caller]`:** `moddil-rt` keeps the stack of call sites. A trap in tracked code (in
    `#[track_caller]`, the prelude's or derived code) is reported at the innermost call that
    untracked code makes, as the interpreters report it.
  - **Wide pointers:** `&str`, `&[T]` and `&dyn Trait` are two words, an address then a length
    or a vtable. They cross calls in registers, and the address is a niche.
    - Text constants are read-only data.
    - `&[T; N]` becomes `&[T]`; indexing, `Len` and subslices of slices are checked against
      the length.
  - **Vtables:** one per type and trait. Each starts with the type's number, so a value seen
    through `&dyn` can be shown, then each method's address. A trait's first supertrait's methods
    come first, so a `&dyn Sub` is a `&dyn Super` as it is. Converting to any other supertrait is
    not compiled yet.
  - **Built-in functions:**
    - text: `len`, `byte_at`, `char_at_byte`, `bytes_between`, and comparing;
    - a slice's `len`;
    - the float functions, through `libm` as the interpreters compute them;
    - `char`'s classes, `code` and `len_utf8`;
    - `int`'s wrapping operations;
    - `swap` and `replace`.
  - **Checking:** 64 conformance files run natively (from 45). `MODDIL_NATIVE_WHY=1` says what
    keeps each other file from running natively.
  - **Interim limits:**
    - no heap: `Buf`, so `Vec`, `String` and `Box`;
    - no closures, destructors, or errors through the status channel;
    - no number conversions (`checked_to` and the others), and no `const` items.
- **Slice C3 (decided November 2026): the heap, drops and closures.**
  - **The heap:** `moddil-rt` allocates over Rust's global allocator and counts the live
    allocations.
    - A `Buf<T>` is an address, a length and a capacity. Its built-ins are compiled inline, so
      `Vec`, `String` and `Box`, written in Moddil over it, run natively.
    - `Box<T>` as `Box<dyn Trait>` gives its `Buf` the vtable.
  - **Drops:**
    - each type that needs dropping has drop glue: its `Drop` impl, else its fields last to
      first, a `Buf`'s elements first to last and then its memory;
    - drop flags, per local and per part moved out of, keep a moved value from being dropped
      where it was.
    - The MIR now drops a `Buf` (so whatever owns one) even without a `Drop` impl inside, so
      native code can free it; for the interpreters such a drop does nothing.
  - **Closures:**
    - a closure is its captures;
    - `call`, `call_mut` and `call_once` run its body, which takes the captures by pointer, so a
      closure called through a borrow changes in place;
    - `&dyn Fn…`, `Box<dyn Fn…>` and `Box<dyn FnOnce…>` call through vtables, whose header gives
      the type's drop glue, size and alignment;
    - a call's error edge whose error is `!` is left out.
  - **A fix in sema:** an `impl Fn…` result's hidden type is the closure as closure analysis
    completes it, with its captures. Before, it had none.
  - **Leak checks:** a native run that returns must end with nothing allocated, after its
    result is dropped. The conformance runner reports a leak as a disagreement.
  - **Checking:** 79 conformance files run natively (from 64), with no leaks.
  - **Interim limits:**
    - no errors through the status channel (`fail`, `?`, `try`);
    - no number conversions, `text`, the log, or `const` items;
    - no concurrency;
    - no anonymous impls;
    - converting a `dyn` borrow to a supertrait that is not its first.
- **Slice C4 (decided November 2026): errors, conversions, text, `const` items and the log.**
  - **The error ABI:** every function takes an error pointer last (null where a call cannot
    fail). Status 2 means failed: the error is at the pointer.
    - The error channel always holds a tagged error: the error type's id (its struct's or
      enum's index in the program) in a word, then the value. So a smaller error set passes
      into a larger one by copying.
    - An error set (`A | B`) is laid out the same way. `ErrorIs` compares the id.
    - `fail` records its site in `moddil-rt`. An error that leaves a test is reported there, as
      the interpreters report it.
  - **A MIR fix:** a `catch` arm matches the caught error as its member, `(err as Member)`. Before,
    a field read in the arm did not say which member it was of; the interpreters ignore the
    projection.
  - **A second MIR fix:** a stored closure revealed as its own type (an `impl Fn… with …`
    result) was given its handlers in a tuple typed `&{error}`; now each is typed by the
    handler's place.
  - **Conversions:** `checked_to`, `truncate`, `wrap` and `saturate` moved from the interpreter
    to `moddil_mir::convert`, on bits. Native code calls the same function through the host.
  - **Text:** `text` and `quoted_text` are made in the host and live until the run ends, as the
    `&str` they give may.
  - **`const` items:** each has a slot. Its value is computed the first time it is used, and
    what that allocates is left out of the leak check.
  - **The log:** written to standard error as the interpreters write it.
  - **Checking:** 102 conformance files run natively (from 79).
  - **Interim limits:**
    - no concurrency (`spawn`, channels, scopes);
    - no anonymous impls;
    - not `upper_text`, `lower_text`, `trim` or `split_at_mut_unchecked`;
    - converting a `dyn` borrow to a supertrait that is not its first.
- **Slice C5 (decided November 2026): concurrency, and the last built-ins.**
  - **Tasks are threads with a baton.** Each task is an OS thread, but only the one holding the
    baton runs. The scheduler is `moddil-interp`'s, ported to `moddil-rt` with values as bytes,
    so tasks run in the interpreters' order.
    - What a run records (the trap, the live allocations, the observer, the texts) is shared by
      its threads.
    - A trap outside an isolated scope aborts the run. Every waiting task is woken to return,
      running no more code, and the host joins their threads before it frees the code.
  - **Unwinding is status 3.** Cleanup blocks are compiled. A cancelled task, or a trap in an
    isolated scope, unwinds through them, so destructors and `defer` run.
  - **The operations:**
    - tasks and scopes: `spawn`, `__scope_run`, `join`, `stop`, `stop_requested` and `yield_now`;
    - channels: `channel`, `send` (a rendezvous waits), `recv`, `Sender::clone`, and the drops
      of `Sender`, `Receiver` and `Task`;
    - `select`, which waits in the runtime and does the chosen arm's operation inline;
    - `Trapped`, built from the trap's class and `file:line:column`.
  - **The last built-ins:**
    - text: `upper_text`, `lower_text` and `trim`;
    - `split_at_mut_unchecked` and `char::from_code`;
    - refined types, which are their base;
    - anonymous impls (closures, with thunks for their vtables);
    - methods called on an error set (by its id);
    - a stub mod's functions, which trap as unimplemented.
  - **Errors:** an error set's value is copied only as far as the member it holds, since a
    caller's view of the errors can be a smaller set. And an error is converted when the MIR
    moves it between a set and its members (`Err(e)` in a `try` block).
  - **Checking:** all 119 conformance files the interpreters run also run natively (from 102),
    with no disagreements and no leaks.
  - **Interim limit:** converting a `dyn` borrow to a supertrait that is not its first.
- **Slice C6 (decided November 2026): native by default, and `moddil build`.**
  - **Native by default:** `moddil run` and `moddil test` compile natively, and `--interp` uses
    the MIR interpreter.
    - Code native compilation does not support runs on the interpreter instead, after a note.
    - Each native test runs in a process of its own, so a crash fails that test.
    - A failed `assert` natively shows its parts' values, as the interpreter's does.
  - **The runtime owns what the host did:**
    - the number conversions and number text (`moddil_mir` calls them, so there is one source
      of truth);
    - `text` and the log;
    - where a tracked trap is reported.
    - Everything native code calls is exported by name (`moddil_rt_*`).
  - **`moddil build`:** the lowering is generic over Cranelift's `Module`, so the same code
    writes an object file.
    - The object exports the entry, `main`'s exit code, and the tables a trap is reported from.
    - `moddil-rt-main`, a new static library, gives the process its `main`: it exits with
      `main`'s `ExitCode`, or prints a trap's class and place and exits 70 (`lifecycle.md`).
    - Linking uses MSVC's `link.exe` on Windows and `cc` elsewhere.
  - **Interim limit:** a built program's `main` gives `()` or `ExitCode`.
- **Slice C7 (decided November 2026): the native backend finished, on Windows and Linux.**
  - **Root capabilities:** `main` may take its root capabilities (`main.root.1`), natively and
    in a built program, each made with its handle zero, as the interpreter makes them.
  - **Exit codes:**
    - `moddil run` exits 70 on a trap, as a built program does (`lifecycle.md`); it was 101;
    - a native `main`'s `ExitCode` is `moddil run`'s exit code, as with `--interp`.
  - **Linux:** the CI's steps pass in WSL (Ubuntu 20.04), the conformance suite natively
    included, and `moddil build` links with `cc` into an ELF executable. One fix:
    `moddil-rt-main` is empty when built for tests, since `cargo test --all-targets` builds it
    and its `main` would clash with the test harness's.
  - **Not verified:** macOS (no Mac to run it on). The WSL run's `grammars` step fails because
    WSL resolves `tree-sitter` to the Windows binary; that is an environment problem, not the
    code's.
- **Slice W0 (decided November 2026): the WASM emitter, scalars, calls and linear memory.** M1's
  other half begins: our own emitter, on `wasm-encoder` (`compiler.md`), in a new crate,
  `moddil-wasm`.
  - **What it compiles:** C0's and C1's ground. Scalars with every operator and check, control
    flow, direct calls, and tuples, structs, enums (with niches) and arrays in linear memory,
    with borrows of them.
  - **Control flow:** structured from the dominator tree, after Ramsey's "Beyond Relooper": a
    `loop` for each loop header and a `block` for each merge node. MIR built from structured
    source is reducible; anything else is refused.
  - **Values:**
    - `int`, `i64`, `u64` and `f64` are `i64` and `f64`; the narrower integers, `bool` and
      `char` are `i32`, kept sign- or zero-extended;
    - borrows are `i32` addresses into linear memory;
    - aggregates, and scalars something borrows, live in frames on a shadow stack.
    - `i128` and `u128` are not compiled yet.
  - **The ABI:** the status return, with multi-value results. Aggregates pass by the address
    of a caller's copy, and come back through an out-pointer. Calls are counted against the
    interpreters' depth, and the overflow checks are written out.
  - **Shared with the native backend:** its layouts (now given a pointer type, so 32-bit
    pointers lay out right) and its value renderer (now reading through a memory abstraction).
  - **Running:** a small JavaScript host runs a module's entries in Node, and snapshots the
    shadow stack at each `// =>` for the renderer. `moddil run --wasm` and `moddil test --wasm`
    use it.
  - **Checking:** a fourth conformance runner, `Wasm`. 45 files run as WASM, as many as C1 ran
    natively, all agreeing with the interpreters. Tests validate each module with `wasmparser`.
- **Slice W1 (decided November 2026): WASM at parity with native, but for tasks, by
  translating Cranelift IR.** Rather than a second lowering, the WASM backend reuses the native
  one: it builds through a generic `cranelift_module::Module`, so `moddil-wasm`'s `WasmModule`
  takes each finished Cranelift IR function and translates it to WASM. One lowering serves
  both backends, so they stay at parity. W0's direct emitter is replaced; its structuring stays.
  - **The translation:**
    - each Cranelift value is a wasm local: `i128` two `i64`s, and `i8` and `i16` zero-extended
      in an `i32`;
    - pointers are `i64`s, as the shared 64-bit layouts have them, wrapped to `i32` addresses at
      each access;
    - stack slots are a frame on a shadow stack;
    - blocks are structured from the dominator tree, with each branch's arguments stored on
      its edge.
  - **The runtime is `moddil-rt` itself**, built for `wasm32-unknown-unknown` by `moddil-wasm`'s
    build script as a new crate, `moddil-rt-wasm`.
    - It imports the program's memory, so the program calls it with its own addresses.
    - Its allocator, with the live count the leak check uses, is the heap.
    - Its `i128` arithmetic covers what WASM has no instructions for.
  - **Memory:** the runtime's static data, then the program's scratch, its data at fixed
    addresses and a 16 MiB shadow stack; the heap grows above.
  - **Running:** the host runs the module in a worker thread with a deep stack, driven over a
    pipe. Values are rendered lazily, the renderer reading memory a page at a time. Results,
    errors leaving a run, failed `assert`s' parts, leaks and tracked traps are reported as the
    JIT runner reports them.
  - **One native fix:** a call through a `dyn Fn…` passed the needs its method is given after
    the arguments' tuple, which the closure's body does not take (it takes them in the tuple).
    Natively that went unseen; WASM's typed indirect calls refused it.
  - **Checking:** 110 conformance files run as WASM, every file native code runs but the nine
    with tasks, all agreeing with the interpreters.
- **Next:** tasks on WASM with the JSPI lowering (`compiler.md`: WASM concurrency lowering).
  Release builds (`lld` or `mold`, optimisation and debug info) after.

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

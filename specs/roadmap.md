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
9. **Canonical AST storage — decided:** plain `.mdl` text files in git, always in canonical format
   (the formatter prints the AST). There is no structured store: the language is its canonical text
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
| **M2** | First standard mods written in Moddil; `moddil/runtime` and `moddil/runtime-sim`; LSP prototype. The tour's tests pass under `moddil/runtime-sim`. |
| **M3** | **Hashiverse CLI/native client** (example 4, phase A). LLVM release backend. **Python bindings** (`moddil bind --lang python`, wheels), the first binding target ([`adoption.md`: Python first](adoption.md#python-first)). |
| **M4** | **A backend service**: the web store API (example 3) with `sql`, HTTP and deterministic simulation; distributed v0 ([Staging: distributed programs](#staging-distributed-programs)). |
| **M5** | **Browser**: the Hashiverse web client and the game client; Swift, Kotlin and TypeScript bindings ([`ui.md`](ui.md)). WASI components with stack switching once Wasmtime's support leaves Tier 3. |
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
| **S3** | Traits, generics (checked once; witness tables in the interpreter), coherence; `Option` | Needed for collections and effects; `Option` is the first generic type |
| **S4** | `fail(E)`, `?`, `try`/`catch`, R3 entry points, `?!` | Errors, the simplest effect |
| **S5** | User effects: traits after `with`, handlers, inference inside a mod, explicit at `pub` | The distinctive feature, built on traits and `fail` |
| **S6** | Several files and mods: the module tree, visibility, the manifest | The tour has several modules |
| **S7** | Closures, strings, `Vec` and `Map` written in Moddil as the first core mods | Dogfooding starts |
| **S8** | Refinements (`where`), runtime `requires`, derives (a minimal comptime) | The tour's `Code` and `Target` types and its derives |
| **S9** | `scope`, `spawn` and `suspend` under a deterministic interpreter scheduler | The tour's `serve`; **the tour parses and checks**, M0's exit gate |

- **S1 is monomorphic (decided):** structs and enums take no type parameters until generics arrive
  in S3, so `Option` comes with them. S1 has no ownership checking yet and copies aggregates; its
  conformance tests never use a value after moving it, so they stay valid when S2 arrives.
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

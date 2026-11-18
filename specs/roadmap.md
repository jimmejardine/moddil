# Roadmap and cross-cutting questions

> **Status:** major questions. The single place for planning: strategic and technical questions that
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
   placement, `remote`, contracts, single-process and simulated runs) is v0; images, Compose,
   Kubernetes and skew checks are v1; other adapters later ([Staging](#staging-distributed-programs)).
5. **Mobile and UI**: shared logic under native UI shells, or a Moddil UI mod? **Decided:** native UI
   over a shared Moddil core, always; no standard UI mod ([`ui.md`](ui.md)).

The **reference examples** in [`examples/`](examples/README.md) (multiplayer game, HFT platform, web
store, Hashiverse) are the yardstick for every decision. A feature no example needs is a candidate for
removal.

**Technical questions**
6. **Bootstrap compiler and back ends — proposed** in [`compiler.md`](compiler.md): stage 0 in
   Rust, then self-host; Cranelift (debug) + LLVM (release) + own WASM emitter + naga/LLVM for GPU;
   Wasmtime as the reference WASM runtime first.
7. **Memory core — decided** in [`memory.md`](memory.md): unique
   ownership, second-class borrows and view types, `.to_rc()` with biased RC, affine + linear
   resources (summary in [Memory: ownership, borrowing and sharing](memory.md)).
8. Effect system design. **Decided:** a hybrid of scoped handlers and second-class capability
   values ([`effects-and-capabilities.md`](effects-and-capabilities.md), [Effects and capabilities as a unifying idea](effects-and-capabilities.md)).
9. Canonical AST storage: text with canonical printing, or a structured store (Unison-style) with text
   as a view?
10. **Minimum viable language**: which language features are in v0 and which come later? (Separate
    from the compiler milestones in [Milestones](#milestones).)
11. **How is the language defined?** A prose reference, a formal semantics, or the MIR interpreter as
    the executable reference? How are conformance tests organised? **Decided:** a normative
    reference with formal core rules, the interpreter as its executable form and a mechanised core
    model, plus a shared conformance suite ([`definition.md`](definition.md)).

## Milestones

| Milestone | Contents |
|---|---|
| **M0** | Stage 0: parser, type, effect and ownership checker, MIR, interpreter. Small programs run. |
| **M1** | Cranelift native backend. Own WASM backend producing **WASI components for Wasmtime**, with stack switching (Linux x86_64). |
| **M2** | First standard mods written in Moddil; `moddil/runtime` and `moddil/runtime-sim`; LSP prototype. |
| **M3** | **Hashiverse CLI/native client** (example 4, phase A). LLVM release backend. **JSPI lowering for browsers and Node.** |
| **M4** | Hashiverse **browser** client. Self-hosted stage 1; fixed-point CI; checked-in WASM bootstrap; double-compiling check. |

**Python bindings** (`moddil bind --lang python`, wheels) follow M3, as the first binding target and
one of the two first audiences ([`adoption.md`: Python first](adoption.md#python-first)). Recommended placement: between M3 and M4.

## Staging: distributed programs

| Stage | Delivers |
|---|---|
| **v0** | Layer A: components, placement, the `remote` effect, generated contracts, single-process mode and simulation |
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

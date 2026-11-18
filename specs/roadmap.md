# Roadmap and cross-cutting questions

> **Status:** major questions. Strategic and technical questions that span several areas, and the
> reference examples that every decision is tested against.

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
   Kubernetes and skew checks are v1; other adapters later ([`distribution.md`: Staging](distribution.md#staging)).
5. **Mobile and UI**: shared logic under native UI shells, or a Moddil UI mod? **Decided:** native UI
   over a shared Moddil core, always; no standard UI mod ([`ui.md`](ui.md)).

The **reference examples** in [`examples/`](examples/README.md) (multiplayer game, HFT platform, web
store, Hashiverse) are the yardstick for every decision. A feature no example needs is a candidate for
removal.

**Technical questions**
6. **Bootstrap compiler and back ends — proposed** in [`bootstrap.md`](bootstrap.md): stage 0 in
   Rust, then self-host; Cranelift (debug) + LLVM (release) + own WASM emitter + naga/LLVM for GPU;
   Wasmtime as the reference WASM runtime first.
7. **Memory core — decided** in [`memory-and-resources.md`](memory-and-resources.md): unique
   ownership, second-class borrows and view types, `.to_rc()` with biased RC, affine + linear
   resources (summary in [Memory and resource management](memory-and-resources.md)).
8. Effect system design. **Decided:** a hybrid of scoped handlers and second-class capability
   values ([`effects-and-capabilities.md`](effects-and-capabilities.md), [Effects and capabilities as a unifying idea](effects-and-capabilities.md)).
9. Canonical AST storage: text with canonical printing, or a structured store (Unison-style) with text
   as a view?
10. **Minimum viable language**: which language features are in v0 and which come later? (Separate
    from the compiler milestones in [`bootstrap.md`: Milestones](bootstrap.md#milestones).)
11. **How is the language defined?** A prose reference, a formal semantics, or the MIR interpreter as
    the executable reference? How are conformance tests organised?

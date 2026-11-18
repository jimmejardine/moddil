# Effects and Capabilities

> **Status:** brainstorm. Expands [`rationale.md`](rationale.md) §2.1. Nothing here is decided beyond what other specs already rely on
> (suspension, failure, scheduling and observability as effects; capabilities as values).

## 1. Problem

Signatures say what a function takes and returns, but not what it *does*: I/O, allocation,
suspension, failure, nondeterminism, GPU work, unsafe code.

## 2. State of the art

Koka, Effekt, Eff (algebraic effects and handlers); OCaml 5 effect handlers; Unison abilities;
Haskell monads/effect libraries; object-capability languages (E, Austral, Monte), WASI capabilities.

## 3. Why it matters here

One mechanism could solve several of the rationale's problem areas:

- concurrency without colouring (suspension is an effect, rationale §2.8)
- GPU-safe subsets (kernels are code with a restricted effect set, rationale §2.9)
- resource safety (effects plus linear types, rationale §2.2)
- supply-chain security (capabilities, rationale §2.16)
- reviewability (effects in signatures, rationale §2.18, §2.19)
- testability (handlers swap real I/O for fakes, rationale §2.11)
- implicit context (config, database pools) provided by handlers instead of threading context
  objects through every call ([`memory-and-resources.md`](memory-and-resources.md) §4)

## 4. Directions

- Effect *inference* inside a module, explicit effects at module/package boundaries.
- Capabilities as values.
- Handlers compiled to zero-cost code where possible (Koka's evidence passing).

## 5. Open questions

- Can effect systems be made ergonomic for mainstream programmers? No mainstream language has done
  it yet; this is both the biggest risk and the biggest opportunity.
- Effect system design: row-polymorphic (Koka) vs capability-passing vs hybrid.
  [`determinism.md`](determinism.md) §4 recommends a hybrid: scoped handlers plus second-class
  capability values.

## Prior art

Leijen, *Koka* papers; Effekt language; OCaml 5 effect handlers; Unison abilities; Miller, *Robust
Composition* (object capabilities, 2006); Austral; WASI.

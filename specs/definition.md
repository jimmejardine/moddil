# Language definition and conformance

> **Status:** not started. Nothing is decided yet; this page records the problem and the
> questions to settle. Listed in the [index](README.md).

## Problem

Moddil has no definition yet beyond these design documents. Before there are several tools (the
compiler, the language server, the formatter, agents' constrained decoding) and possibly several
compilers, the language needs an authoritative definition, a grammar that tools and models can rely
on, and tests that show an implementation conforms.

## Touched on in

[`roadmap.md`](roadmap.md) (questions 9 and 11), [`syntax.md`](syntax.md), [`bootstrap.md`](bootstrap.md) (the MIR interpreter, fixed-point builds),
[`agent-interface.md`](agent-interface.md) (canonical AST, constrained decoding), [`toolchain.md`](toolchain.md) (formatter),
[`evolution.md`](evolution.md) (editions).

## Questions to settle

- **What defines the language**: a prose reference, a formal semantics, or an executable reference
  (the MIR interpreter), or a combination?
- **Lexical structure**: identifiers and Unicode in source, comments, keywords and reserved words,
  literals.
- **The grammar**: unambiguous and easy for tools and LLMs to parse (LL(k)?), published in a
  machine-readable form for constrained decoding.
- **Conformance tests**: a suite organised by feature and edition, run by every implementation.
- **Testing the compiler**: fuzzing, checking that debug and release builds behave identically,
  translation validation for optimisations.
- **Other implementations**: who may write one, and what it must pass to call itself Moddil.

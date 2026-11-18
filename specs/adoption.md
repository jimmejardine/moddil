# Adoption

> **Status:** not started. Nothing is decided yet; this page records the problem and the
> questions to settle. Listed in the [index](README.md).

## Problem

A language succeeds only if programmers choose it. Moddil's guarantees matter only if people
write code in it, and a new language starts with no users, no libraries, no Stack Overflow answers and
almost no training data for the models that now write much of the code. Adoption has to be designed,
not hoped for.

## Touched on in

[`vision.md`](vision.md), [`roadmap.md`](roadmap.md) (questions 1 to 3), [`agent-interface.md`](agent-interface.md) (the cold start for LLMs),
[`toolchain.md`: Playground: learning without a REPL](toolchain.md#playground-learning-without-a-repl), [`interop.md`](interop.md),
[`ui.md`: Generated bindings](ui.md#generated-bindings), [`examples/README.md`](examples/README.md).

## Questions to settle

- **The itch it scratches**: which pain, felt today, does Moddil remove? Candidates: Rust's lifetimes
  and async, coloured functions, slow builds, supply-chain fear, untestable time and I/O.
- **Simpler than Rust**: what a newcomer does not have to learn, and how to measure it (concepts
  needed for a first program, compile-error rate for newcomers and for agents).
- **Transpiling existing projects**: Rust and TypeScript to Moddil, so teams can start from code
  they already have. How complete, how much is agent-assisted, and how the result is verified (the
  original tests, run against both).
- **Consumable from Python**: `moddil bind --lang python` producing a native extension module and
  wheels (as PyO3 and maturin do for Rust), so Python teams can move one hot module at a time. A
  potentially decisive reason to adopt.
- **Incremental adoption** inside C, C++ and Rust codebases, one module at a time
  ([`interop.md`](interop.md)).
- **The LLM cold start**: a compact reference written for agents, an MCP server, example corpora,
  and the transpilers above as a source of training data.
- **Learning materials**: a book, the in-browser playground, the reference examples as tutorials.
- **Flagship software**: which libraries or applications make people look (the reference examples,
  Hashiverse).
- **The first audience**: who Moddil is for first ([`roadmap.md`](roadmap.md), questions 1 and 2).

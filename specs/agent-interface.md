# LLMs as First-Class Programmers: the Agent Interface

> **Status:** brainstorm. Expands [`rationale.md`](rationale.md) §2.18. Nothing here is decided, except `note` blocks
> ([`docs-and-tests.md`](docs-and-tests.md) §9).

## 1. Problem

Languages assume the person who writes the code also understands it. In an LLM-heavy
workflow, the writer is a model and the human is the *reviewer, architect and approver*. Current
languages optimise for the writer and against the reviewer.

## 2. What LLMs struggle with today

- **Non-local semantics**: implicit conversions, overload resolution, operator overloading, Scala
  implicits, C++ ADL, monkey patching, global state. A model (and a human) has to see far away to
  understand a single line.
- **Many ways to say the same thing**: style variation makes training data noisier and diffs bigger.
- **Opaque macros**: generated code that neither model nor reviewer can see.
- **Weak feedback loops**: compiler diagnostics written for humans, not structured for agents; slow
  builds limiting iterations.
- **Hallucinated APIs**: names that look plausible but don't exist. Fast and precise feedback is the cure.
- **Version drift**: the model learned an old API; nothing tells it the API changed.

## 3. What humans struggle with when reviewing LLM code

- "What does this change *actually* do?" Side effects are invisible in signatures.
- "Is this safe?" Nothing marks which code touches the network, filesystem, or `unsafe`.
- "Why was it written like this?" Intent is lost; only the result remains.
- Big diffs from mechanical changes hide the few lines that matter.

## 4. State of the art

- **Go**: one formatter, a small language, few ways to do things. LLMs write Go well.
- **Rust**: precise, structured compiler errors with suggested fixes; strong types catch hallucinations.
- **TypeScript**: types + LSP give models a tight feedback loop.
- **Dafny / Lean / Verus**: specifications that a model can try to satisfy and a checker can verify.
- **Language servers / tree-sitter**: queryable structure for tooling.

## 5. Directions

- **Canonical form.** Exactly one formatting; ideally the source is a printed view of a canonical
  AST. No style debates, minimal diffs.
- **Local reasoning.** You can understand a line by reading it plus the signatures it calls. No
  implicit conversions, limited overloading, hygienic and *visible* metaprogramming (the expansion can
  always be shown).
- **Effects and capabilities in signatures.** A reviewer can see at a glance that a function does
  I/O, allocates, can fail, can suspend, or runs on a GPU (rationale §2.1). Illustration only:
  ```
  fn fetch_user(id: UserId) -> User ! {net, fail(HttpError)}
  ```
- **Contracts and properties next to code.** Pre/post-conditions, invariants and property-based tests
  are first-class syntax, so a model can be asked to "make these pass" and a human can review the
  *spec* instead of the implementation.
- **Machine-first compiler API.** Diagnostics as structured data with stable codes and machine-applicable
  fixes; a queryable semantic model (types, effects, call graph) exposed as a service; fast incremental
  checking.
- **Intent and provenance.** First-class places for *why* (design notes, linked issues) and *who/what*
  (human, model, model version, prompt reference) that survive refactors, unlike comments, which drift.
  The *why* is decided: `note` blocks attached to declarations
  ([`docs-and-tests.md`](docs-and-tests.md) §9); where provenance lives is still open.
- **Stable, small surface syntax** that changes rarely, so training data stays valid. Editions handle
  evolution (rationale §2.20).
- **Explainability tooling**: `explain` commands that show the expanded, desugared, effect-annotated
  view of any expression.
- **One front end for every tool.** The compiler is a library and a service; every tool
  (formatter, LSP, agent API, semantic diff, docs) uses the same front end (rationale §2.17).

## 6. Open questions

- How verbose can a language be before humans reject it, given they will mostly *read* it? Is
  explicit-but-verbose acceptable if humans rarely type it?
- Should provenance live in the source, in version control metadata, or in a sidecar?
- Can the language be designed so that type checking and effect checking are fast enough to run after
  every token an agent generates (constrained decoding)?

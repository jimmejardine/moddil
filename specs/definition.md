# Language Definition and Conformance

> **Status:** complete: decided, with no open questions. Listed in the [index](README.md). Snippets and IDs
> are **illustration only**.

## Summary

| Rule | One line |
|---|---|
| **Three artefacts, one meaning** | A normative prose reference with formal core rules, an executable reference interpreter, and a mechanised core model; any disagreement is a bug. |
| **Stable paragraph IDs** | Every rule in the reference has a permanent ID that tests cite. |
| **Unicode identifiers, safely** | UAX #31 identifiers, NFC-normalised, with confusable and mixed-script lints; bidi controls rejected. |
| **One grammar source** | An unambiguous LL(k) grammar that generates the parser tests, the tree-sitter grammar and the grammars for constrained decoding. |
| **A shared conformance suite** | Test262-style: Moddil files with inline expectations, organised by feature and edition. |
| **Written with the compiler** | A feature is done only when its reference paragraphs and conformance tests exist. |
| **Open implementations, protected name** | Anyone may implement Moddil; calling it "Moddil" needs the edition's conformance suite to pass. |

## Problem

- Moddil has no definition yet beyond these design documents.
- Several tools will need to agree exactly on what a program means: the compiler, the language
  server, the formatter, agents' constrained decoding, the verifier, and perhaps other compilers.
- **"The compiler is the spec"** (Rust until 2025) turns compiler bugs into semantics, and makes
  other implementations copy them.
- **Large prose standards** (C, C++) accumulate undefined and implementation-defined behaviour that
  nobody can test.

## State of the art

| Language | Definition | Conformance |
|---|---|---|
| **Go** | A short, normative prose spec; several implementations (gc, gccgo, TinyGo) | Each implementation's own tests |
| **Rust** | The compiler, until the Ferrocene Language Specification (FLS) was adopted in 2025, with stable paragraph IDs that tests trace to | rustc UI tests (`//~ ERROR`) |
| **WebAssembly** | Prose and formal rules in one document; an OCaml reference interpreter; mechanised proofs (WasmCert in Coq and Isabelle) | A shared `.wast` suite that every engine runs |
| **C, C++** | ISO prose standards with much UB | Commercial suites (Plum Hall, SuperTest) |
| **JavaScript** | ECMA-262, written as pseudo-code algorithms | **Test262**: a large shared suite tagged by feature |
| **Java** | The Java Language Specification | The JCK, a certification suite tied to the name |

Compiler testing: Csmith and YARPGen (random programs for differential testing), grammar-based
fuzzers, and Alive2 (translation validation for LLVM optimisations).

## Decisions

### Three artefacts, one meaning

| Artefact | Role |
|---|---|
| **The reference** | **Normative.** Prose, with formal rules for the core (types, effects, borrows and evaluation). |
| **The reference interpreter** | The executable form of the reference: a **separate, small interpreter over the canonical AST**, independent of the compiler's MIR interpreter (which runs comptime and the unsafe checker), so that each can check the other; it shares only the parser and AST with the compiler ([`compiler.md`: Two interpreters](compiler.md#two-interpreters)). Tested against the reference. |
| **The core model** | A mechanised model of the core **in Lean** that proves the core rules sound and validates the interpreter ([`correctness.md`: A formal semantics of the core](correctness.md#a-formal-semantics-of-the-core)). |

- **Any disagreement between them is a bug.** The reference decides the intended meaning (changing it
  if it was wrong), and the fix lands in all three.
- This is WebAssembly's arrangement, which has kept several engines in close agreement.
- **Lean is the source of truth for the core** (decided). The reference shows the core rules as
  inference rules generated from the Lean definitions, so the two cannot drift.
- **The formal core covers everything from the start** (decided): dynamic semantics, type and
  effect soundness, and borrows, in the style of RustBelt.
- **When each starts (decided):** the reference and the conformance suite grow from the first M0
  slice; the reference interpreter from slice S1; work on the Lean model after slice S5, once the core
  has settled ([`roadmap.md`: M0: the v0 language in slices](roadmap.md#m0-the-v0-language-in-slices)).

### The reference

- It lives in its own directory (for example `reference/`), separate from these design documents,
  and is **versioned by edition** ([`evolution.md`](evolution.md)).
- Every rule has a **stable paragraph ID**, such as `expr.call.effects.3`. An ID never changes
  meaning; a rule that changes gets a new ID, and the old one is marked with the edition that
  retired it.
- Conformance tests cite the IDs they cover, so coverage of the reference is measurable.
- **The language is defined by its canonical text form** (decided): the grammar and lexical rules
  below. However code is stored ([`roadmap.md`](roadmap.md), question 9), a structured store would
  be a lossless encoding of that text, so the reference does not change.

### Lexical structure

- Source files are **UTF-8**.
- **Identifiers follow Unicode UAX #31** and are **NFC-normalised**, so two spellings of the same
  name are the same identifier.
- **Confusable and mixed-script identifiers are linted** (Unicode UTS #39): `раssword` with a
  Cyrillic `р` next to `password` is flagged.
- **Keywords are ASCII and all reserved** (decided): there are no contextual keywords
  ([`syntax.md`: Grammar rules](syntax.md#grammar-rules)). A keyword may still name a **member** (a
  field or method, where declared and after `.` or `::`) or be a named-argument label
  (`pub fn view(&self)`, `Type::from(x)`, `default: 8080`), as in Swift and Kotlin, because those
  positions are unambiguous.
- **Bidirectional control characters are rejected** in source, except as escapes in string literals,
  which closes the "Trojan Source" attack.
- Text handling inside programs is covered in [`text.md`](text.md).

### One grammar source

- The grammar is **LL(k) with a small k** and **unambiguous**: for example, there is no Rust-style
  ambiguity between a struct literal and the block of an `if` condition, because a struct literal
  in a condition head must be parenthesised, as in Rust. There are no contextual keywords.
- One machine-readable source generates:
  - the parser's grammar tests;
  - the official **tree-sitter** grammar ([`toolchain.md`](toolchain.md));
  - **GBNF and Lark grammars** for constrained decoding by models
    ([`agent-interface.md`](agent-interface.md)).
- The formatter's canonical output must parse back to the same AST, and the grammar tests check it.
- **The source (decided):** `reference/grammar/moddil.ebnf`, in a small EBNF dialect of our own. A
  Rust tool checks that it is LL(k) and unambiguous and generates the outputs above.
- **Notation (decided):** W3C-style EBNF (`rule ::= a b? c* (d | e)+`), with quoted terminals and
  UPPERCASE token rules, in one file with a Tokens section and a Syntax section. GBNF and Lark use
  the same operators, so generating them is nearly direct.
- **Writing it (decided):** an inventory of every decided construct comes first
  (`reference/grammar/inventory.md`). Ambiguities and conflicts found along the way become open
  questions in [`syntax.md`](syntax.md#open-questions), so the grammar only ever holds decided
  syntax. It covers **all
  decided syntax from the start**, not only what M0 implements, so ambiguities surface before any
  parser code exists; the compiler's hand-written parser implements it slice by slice.

### A shared conformance suite

```rust
// conformance/2027/effects/handler-scope.mdl
// covers: effect.handler.scope.1

trait Counter { fn tick(&self) -> int; }

fn twice() -> int with counter: Counter { counter.tick() + counter.tick() }

test "a handler is in scope inside its `with` block" {
    with Counter = Counting::new() {
        twice()                   // => 3
    }
}
```

```rust
// conformance/2027/effects/handler-leak.mdl
// covers: effect.handler.scope.2

fn leak() -> int { counter.tick() }   //~ E0601
```

- Organised **by feature and edition**, like Test262.
- Tests are ordinary Moddil files with **inline expectations**: `// =>` for values (as in
  [`docs.md`](docs.md)), `//~ E0601` for diagnostics, `//~ trap` for traps.
- **How expectations bind (decided):** `// =>` follows an expression statement or a block's final
  expression inside a `test` item, and every such line must run and produce exactly that value;
  `//~ E…` names a diagnostic starting on its line; `//~ trap` marks the line where a trap
  happens. A file with expected diagnostics is only checked, not run, so error tests live in their
  own files. A file has only items at the top level, so values are always observed in tests.
- **Diagnostic codes are part of conformance; message wording is not**, so implementations may
  word errors their own way.
- **Codes come in blocks by area (decided):** E00xx syntax, E01xx names, E02xx types, E03xx
  ownership and mutability, E04xx traits and generics, E05xx errors and `fail`, E06xx effects, and
  further blocks as areas arrive. A code never changes meaning. The table is in the reference
  ([`reference/2027/diagnostics.md`](../reference/2027/diagnostics.md)).
- Determinism ([`determinism.md`](determinism.md)) makes expected outputs exact, including for
  concurrent tests run under the simulation scheduler.
- **The spec team owns the suite**, and any implementation may contribute tests. A disputed test
  (implementations disagree, or one thinks the test is wrong) is resolved by clarifying the
  reference through the RFC and governance process ([`governance.md`](governance.md)); until then
  it is marked *disputed* and does not count towards conformance.

### Written with the compiler

- The design documents stay as they are; the reference grows as the stage-0 compiler does
  ([`compiler.md`](compiler.md)).
- **A feature is done only when it has reference paragraphs and conformance tests**, as well as an
  implementation.
- The reference is complete for an edition when that edition ships.

### Testing the compiler

- **Differential testing**: generated programs run through the interpreter and every backend
  ([`correctness.md`: Differential testing of the compiler](correctness.md#differential-testing-of-the-compiler)).
- **Grammar-based fuzzing** of the parser and the type checker.
- **Debug and release builds** must have the same observable behaviour; the test runner compares
  them.
- **Translation validation** (Alive2-style) for the optimisations Moddil writes itself.
- The core model validates the interpreter, which in turn validates the backends.

### Open implementations, protected name

- The reference and the conformance suite are open, under the same MIT/Apache-2.0 licence as the
  rest of the project. **Anyone may build an implementation.**
- An implementation may call itself **"Moddil <edition>"** only if it passes that edition's
  conformance suite. The trademark policy lives in [`governance.md`](governance.md).

### Generated views

- The **context-sized reference** for models ([`agent-interface.md`](agent-interface.md)) and the
  **error-code explanations** are generated from the reference, so they can't drift from it.

## Fit check against the reference examples

The examples need a definition only indirectly: every feature they use is covered by reference
paragraphs and conformance tests, and determinism lets the HFT and game examples rely on identical
behaviour across the interpreter, native backends and WASM.

## Open questions

None at present.

## Prior art

The Go specification; the Ferrocene Language Specification and rustc UI tests; the WebAssembly
specification, its reference interpreter and WasmCert; ECMA-262 and Test262; the Java Language
Specification and JCK; Unicode UAX #31 and UTS #39; "Trojan Source" (Boucher and Anderson, 2021);
Csmith, YARPGen and Alive2; tree-sitter; GBNF and Lark.

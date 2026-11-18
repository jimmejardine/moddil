# LLMs as First-Class Programmers: the Agent Interface

> **Status:** decided direction, open questions remain. Expands [`rationale.md`](rationale.md) §2.18.
> Snippets and message formats are **illustration only**.

## Summary

| Rule | One line |
|---|---|
| **Verbosity is a view** | The source is explicit only at mod boundaries; editors and review tools show inferred effects, types and error sets without putting them in the file. |
| **An agent server** | The compiler is a service over LSP and MCP: agents query the semantic model and act through fixes, refactor intents, affected tests and replays. |
| **Diagnostics built for agents** | Stable codes, one root cause, machine-applicable fixes with a confidence level, relevant signatures inline. |
| **"Did you mean?" everywhere** | Suggestions drawn from scope *and* the installed API, ranked by type, aware of other languages' habits, imports and version drift. |
| **Solving the cold start** | Familiar syntax, a reference small enough for a model's context, and a corpus of checked examples. |
| **Agent-written code runs safely** | Pure comptime, simulated tests and declared capabilities. |
| **A review surface for humans** | Semantic diffs, untested changes, mutation scores and checked summaries. |

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
- **A new language starts cold**: models write well in languages they have seen millions of examples
  of. Moddil starts with none (§5.5).

## 3. What humans struggle with when reviewing LLM code

- "What does this change *actually* do?" Side effects are invisible in signatures.
- "Is this safe?" Nothing marks which code touches the network, filesystem, or `unsafe`.
- "Why was it written like this?" Intent is lost; only the result remains.
- Big diffs from mechanical changes hide the few lines that matter.

## 4. State of the art

- **Go**: one formatter, a small language, few ways to do things. LLMs write Go well.
- **Rust and Elm**: precise, structured compiler errors with suggested fixes; strong types catch
  hallucinations.
- **TypeScript**: types + LSP give models a tight feedback loop.
- **Dafny / Lean / Verus**: specifications that a model can try to satisfy and a checker can verify.
- **Language servers / tree-sitter / rust-analyzer**: queryable structure for tooling.
- **MCP** (Model Context Protocol): a common protocol for giving agents tools and context.
- **Constrained decoding**: grammar-constrained generation (Outlines, llama.cpp grammars) and research
  on type-constrained decoding.

## 5. Decisions

### 5.1 Verbosity is a view

The source is a printed view of a canonical AST, so how much is shown is a choice of view, not a
property of the source.
- **Explicit in the source** only where readers need it: at **mod boundaries**, public signatures
  state effects, error sets, contracts and capabilities (rationale §2.1, §2.3, §2.7).
- **Inferred inside** a module: local effects, error sets and types.
- **Two views of the same code**:
  - a **compact view** for writing;
  - a **reviewer view**, used by editors and review tools, that shows inferred effects, types and
    error sets inline without putting them in the file, along with the groups each private item
    inherits ([`docs-and-tests.md`](docs-and-tests.md) §4.1).
- Humans mostly *read* the reviewer view and *type* the compact one.

```
fn load(path: Path) -> Config ! {fs, fail(LoadError)}     // explicit: public signature
    let text = fs.read(path)?          // reviewer view shows:  : String ! {fs, fail(IoError)}
    parse(text)?                       // reviewer view shows:  : Config ! {fail(ParseError)}
```

### 5.2 The agent server

The compiler is a library and a service (rationale §2.17). It is exposed over **LSP** for editors and
over **MCP** for agents, from the same front end.

| Kind | Examples |
|---|---|
| **Queries** | **Overviews first**: mod, then module, then group, then item ([`docs-and-tests.md`](docs-and-tests.md) §4.1); signatures, effects, error sets and contracts of any definition; the call graph; which tests cover a definition; docs and examples of the **installed** version of each mod ([`docs-and-tests.md`](docs-and-tests.md) §11); `explain` (desugared, effect-annotated views; comptime expansions) |
| **Actions** | Apply a fix; submit a **refactor intent** (rename, move, extract) as an operation rather than text edits ([`semantic-vcs.md`](semantic-vcs.md) §4); run the affected tests ([`build-times.md`](build-times.md) §7); replay a recording and inspect the failing task ([`observability.md`](observability.md) §11) |

### 5.3 Diagnostics built for agents

- **Stable codes**, each with an offline explanation.
- **One root cause** per diagnostic, not a cascade of follow-on errors.
- **Machine-applicable fixes**, each with a **confidence** level.
- **Relevant context inline**: the signatures, effects or contracts involved are included, so the
  agent doesn't need another lookup.
- **Streamed early**, as soon as each check finishes ([`build-times.md`](build-times.md) §7).

### 5.4 "Did you mean?"

Cheap to build, because the query-based compiler already indexes every name, and unusually useful
because of earlier decisions:

| Kind of mistake | What the compiler suggests |
|---|---|
| Misspelt or invented name | Fuzzy matches over names in scope **and** the whole installed API (every mod in the lockfile), ranked by the type expected at that position, effects and scope, not just spelling |
| Habit from another language | A curated synonym table (`len` / `length` / `size` / `Count`, `push` / `append`, …) and recognisers for common Rust, TypeScript and Swift constructs, each mapped to the Moddil form |
| Missing import | "`HashMap` exists in `moddil/collections`; add the import?" |
| **Version drift** | Renames are recorded as refactor intents, so the compiler can say exactly "`foo` was renamed to `bar` in 2.0" instead of guessing |
| Type mismatch | Type-directed fixes: add `?`, call `.to_rc()`, pass `mut x`, or an available conversion |

```
error[E0425]: no function `length` on `String`
  --> src/report.mdl:12:18
   |
12 |     if title.length() > 80 {
   |              ^^^^^^ did you mean `len`?  (confidence: high; `length` is common in JS/Java)
   = fix: replace `length` with `len`
```

Every suggestion is a structured fix with a confidence level. The compiler **never applies one
silently**: agents and humans choose.

### 5.5 Solving the cold start

LLMs write well in languages they have seen often; a new language starts with no training data. This
may be Moddil's largest adoption risk. Measures:
- **Familiar syntax**: a Rust/Swift-like surface, so models transfer what they know. Novelty only
  where it carries meaning (effects, placement, contracts).
- **A context-sized reference**, generated from the specification and versioned with the toolchain:
  the whole core language plus the signatures of the standard mods, small enough to load into a
  model's context.
- **A checked example corpus**: every doc example compiles and runs
  ([`docs-and-tests.md`](docs-and-tests.md) §5), and the corpus is published for retrieval and
  training.
- The tight feedback loop (§5.3, §5.4) is the safety net when a model guesses wrong.

### 5.6 Agent-written code runs safely

- Comptime is pure and sandboxed ([`comptime.md`](comptime.md)).
- Tests run under the simulation runtime by default ([`determinism.md`](determinism.md)).
- A program can use only the capabilities it declares (rationale §2.1, §2.16).

So an agent can build and test code without being able to reach the network or the disk.

### 5.7 A review surface for humans

Every change can be reviewed through:
- a **semantic diff**: API, effect, capability and contract changes ([`semantic-vcs.md`](semantic-vcs.md));
- "**this changed, but its tests didn't**" ([`docs-and-tests.md`](docs-and-tests.md) §6);
- **mutation scores** per changed declaration ([`correctness.md`](correctness.md));
- an LLM-written summary that is **checked against the semantic diff**;
- `note` blocks recording *why* ([`docs-and-tests.md`](docs-and-tests.md) §9).

## 6. Directions (not yet decided)

- **Provenance in version control.** *Recommended:* commit trailers record who or what wrote a change
  (model, version, session or prompt reference, approving human), and a per-definition index is
  derived from them, so blame survives moves and renames. Never in the source; prompts by reference
  only, since they may contain secrets.
- **A prefix-checkable grammar.** *Recommended:* an unambiguous grammar suited to grammar-constrained
  decoding, plus a compiler service answering "what may legally come next here?" (names in scope,
  fields and methods of the type at the cursor, valid effect operations). Full checks run per
  statement, not per token, which local inference makes cheap.
- **Translation tools** from Rust and TypeScript, to seed code and port projects.
- **A small, stable surface syntax** that changes rarely, so training data stays valid; editions
  handle evolution (rationale §2.20).

## 7. Open questions

- Should provenance live in version control (recommended, §6), in the source, or in a sidecar?
- How far should constrained decoding go: grammar only, or names and types per statement?
- What size budget should the context-sized reference have, and how is it kept current?
- Who curates the cross-language synonym tables and foreign-syntax recognisers?
- Is MCP the right agent protocol long term, or one adapter among several?

## Prior art

Elm and Rust compiler diagnostics; rust-analyzer; the Language Server Protocol; the Model Context
Protocol; Outlines and llama.cpp grammars (constrained decoding); research on type-constrained code
generation; Go's small language and single formatter; `llms.txt`.

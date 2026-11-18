# Moddil specifications

> **Status:** living index. Each area has its own document, which holds its problem statement, prior
> art, decisions, directions and open questions. Start with [Why Moddil](vision.md) for the thesis.

## Index

Status legend:

| Marker                 | Meaning                                                                                                                    |
| ---------------------- | -------------------------------------------------------------------------------------------------------------------------- |
| ⚪ **not started**     | No content beyond a heading                                                                                                |
| 🔴 **superficial**     | A brainstorm or stub; little is decided                                                                                    |
| 🟠 **major questions** | A direction exists, but open questions could still change the core design, or the document is a proposal not yet confirmed |
| 🟡 **minor questions** | Decided; the remaining questions are details                                                                               |
| 🟢 **complete**        | Decided, with no open questions                                                                                            |

Rule of thumb: 🟠 if an open question could change language semantics or a requirement, or the
document is still a proposal; 🟡 if what is left is detail, tooling or operations.

Areas run from the language core outward: semantics, then execution, then the surrounding system,
then the human and agent workflow. The order reflects dependencies, not priority.

| Area                                                               | Status             | Documents                                                                  |
| ------------------------------------------------------------------ | ------------------ | -------------------------------------------------------------------------- |
| [Why Moddil](vision.md)                                            | —                  | [`vision.md`](vision.md): preamble, thesis, cross-cutting themes, tensions |
| [Effects and capabilities](effects-and-capabilities.md)            | 🟠 major questions | [`effects-and-capabilities.md`](effects-and-capabilities.md)               |
| [Memory and resource management](memory-and-resources.md)          | 🟡 minor questions | [`memory-and-resources.md`](memory-and-resources.md)                       |
| [Error handling](error-handling.md)                                | 🟠 major questions | [`error-handling.md`](error-handling.md)                                   |
| [Null and invalid states](null-safety.md)                          | 🟡 minor questions | [`null-safety.md`](null-safety.md)                                         |
| [Numerics and data](numerics.md)                                   | 🟠 major questions | [`numerics.md`](numerics.md), [`units.md`](units.md)                       |
| [Metaprogramming](comptime.md)                                     | 🟡 minor questions | [`comptime.md`](comptime.md)                                               |
| [Contracts and verification](correctness.md)                       | 🟠 major questions | [`correctness.md`](correctness.md)                                         |
| [Concurrency across CPUs](concurrency.md)                          | 🟡 minor questions | [`concurrency.md`](concurrency.md), [`closures.md`](closures.md)           |
| [GPUs and accelerators](accelerators.md)                           | 🟡 minor questions | [`accelerators.md`](accelerators.md)                                       |
| [Distributed and multi-tier programs](distribution.md)             | 🟡 minor questions | [`distribution.md`](distribution.md)                                       |
| [Determinism and reproducibility](determinism.md)                  | 🟠 major questions | [`determinism.md`](determinism.md)                                         |
| [Observability and debuggability](observability.md)                | 🟡 minor questions | [`observability.md`](observability.md)                                     |
| [Interoperability with C and Rust](interop.md)                     | 🟡 minor questions | [`interop.md`](interop.md)                                                 |
| [Build times](build-times.md)                                      | 🟠 major questions | [`build-times.md`](build-times.md)                                         |
| [Cross-compilation and deployment](targets.md)                     | 🟡 minor questions | [`targets.md`](targets.md), [`freestanding.md`](freestanding.md)           |
| [Packages and supply-chain security](packages-and-supply-chain.md) | 🟡 minor questions | [`packages-and-supply-chain.md`](packages-and-supply-chain.md)             |
| [Tooling uniformity](toolchain.md)                                 | 🟡 minor questions | [`toolchain.md`](toolchain.md), [`docs-and-tests.md`](docs-and-tests.md)   |
| [LLMs as first-class programmers](agent-interface.md)              | 🟡 minor questions | [`agent-interface.md`](agent-interface.md)                                 |
| [Diffs, code review and version control](semantic-vcs.md)          | 🟡 minor questions | [`semantic-vcs.md`](semantic-vcs.md)                                       |
| [Language evolution](evolution.md)                                 | 🟡 minor questions | [`evolution.md`](evolution.md)                                             |
| [Surface syntax and paradigms](syntax.md)                          | 🟡 minor questions | [`syntax.md`](syntax.md), [`paradigms.md`](paradigms.md)                   |
| [Type system core](types.md)                                       | ⚪ not started     | [`types.md`](types.md)                                                     |
| [Modules, visibility and program structure](modules.md)            | ⚪ not started     | [`modules.md`](modules.md)                                                 |
| [`unsafe` and the safety boundary](unsafe.md)                      | ⚪ not started     | [`unsafe.md`](unsafe.md)                                                   |
| [Text and strings](text.md)                                        | ⚪ not started     | [`text.md`](text.md)                                                       |
| [Values: copy, clone and equality](values.md)                      | ⚪ not started     | [`values.md`](values.md)                                                   |
| [Standard library shape](stdlib.md)                                | ⚪ not started     | [`stdlib.md`](stdlib.md)                                                   |
| [Data interchange and persistence](data.md)                        | ⚪ not started     | [`data.md`](data.md)                                                       |
| [UI, mobile and the browser](ui.md)                                | ⚪ not started     | [`ui.md`](ui.md)                                                           |
| [Program lifecycle](lifecycle.md)                                  | ⚪ not started     | [`lifecycle.md`](lifecycle.md)                                             |
| [Requirements and non-goals](requirements.md)                      | 🟡 minor questions | [`requirements.md`](requirements.md): R1–R3 and non-goals                  |
| [Bootstrap compiler and backends](bootstrap.md)                    | 🟠 major questions | [`bootstrap.md`](bootstrap.md) (R1)                                        |
| [Mods and the standard catalogue](mods.md)                         | 🟠 major questions | [`mods.md`](mods.md) (R2)                                                  |
| [Prior art](prior-art.md)                                          | reference          | [`prior-art.md`](prior-art.md)                                             |
| [Roadmap and cross-cutting questions](roadmap.md)                  | 🟠 major questions | [`roadmap.md`](roadmap.md)                                                 |
| [Reference examples](examples/README.md)                           | 🟠 major questions | [`examples/`](examples/README.md): game, HFT, web store, Hashiverse        |

## Working on these specs

For people and LLM agents editing the specs:

- **Link by heading name, not by number.** Sections are not numbered, so they can be reordered and
  rewritten freely. Cross-reference with a heading link: `[Live update (debug builds)](build-times.md#live-update-debug-builds)`
  in another document, or `[Index](#index)` within the same document. Anchors follow
  GitHub's slug rules (lower-case, punctuation removed, spaces become hyphens). Stable identifiers
  that are not positions (R1–R3, D1–D10, example file numbers) stay as they are.
- **Run the link checker after every edit:**

  ```text
  python tools/check_spec_links.py            # check only; exit code 1 on any problem
  python tools/check_spec_links.py --align    # also re-align the index table
  ```

  It checks that every relative link (file and heading anchor) in `specs/` resolves, and that every
  document is listed in the [index](#index). Renaming a heading breaks the links to it; the checker
  lists them.
- **Statuses live only in the index table** above. **Open questions live only in their area's
  document**, under *Open questions*; nothing is copied here.
- **A new document needs an index row**, with a status from the legend.
- **Code blocks are illustrative** and follow [`syntax.md`](syntax.md) (Rust-based syntax).

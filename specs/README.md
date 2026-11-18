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

The table is in reading order for someone new to Moddil: **start here** for the thesis, the
fixed requirements, four concrete programs and the syntax; then **the language** itself; how
**running programs** are placed and coordinated; the **library and ecosystem**; the **tools and
workflow**; and last **the project**. The order is for reading, not priority.

| Area                                                               | Status             | Documents                                                                  |
| ------------------------------------------------------------------ | ------------------ | -------------------------------------------------------------------------- |
| **Start here**                                                     |                    |                                                                            |
| [Why Moddil](vision.md)                                            | —                  | [`vision.md`](vision.md): preamble, thesis, cross-cutting themes, tensions |
| [Requirements and non-goals](requirements.md)                      | 🟡 minor questions | [`requirements.md`](requirements.md): R1–R3 and non-goals                  |
| [Reference examples](examples/README.md)                           | 🟠 major questions | [`examples/`](examples/README.md): game, HFT, web store, Hashiverse        |
| [Surface syntax and paradigms](syntax.md)                          | 🟡 minor questions | [`syntax.md`](syntax.md), [`paradigms.md`](paradigms.md)                   |
| [Language definition and conformance](definition.md)               | ⚪ not started      | [`definition.md`](definition.md)                                           |
| **The language**                                                   |                    |                                                                            |
| [Effects and capabilities](effects-and-capabilities.md)            | 🟠 major questions | [`effects-and-capabilities.md`](effects-and-capabilities.md)               |
| [Memory and resource management](memory-and-resources.md)          | 🟡 minor questions | [`memory-and-resources.md`](memory-and-resources.md)                       |
| [Values: copy, clone and equality](values.md)                      | 🟡 minor questions | [`values.md`](values.md)                                                   |
| [Error handling](error-handling.md)                                | 🟠 major questions | [`error-handling.md`](error-handling.md)                                   |
| [Null and invalid states](null-safety.md)                          | 🟡 minor questions | [`null-safety.md`](null-safety.md)                                         |
| [Type system core](types.md)                                       | 🟠 major questions | [`types.md`](types.md)                                                     |
| [Modules, visibility and program structure](modules.md)            | 🟡 minor questions | [`modules.md`](modules.md)                                                 |
| [`unsafe` and the safety boundary](unsafe.md)                      | 🟡 minor questions | [`unsafe.md`](unsafe.md)                                                   |
| [Security model](security.md)                                      | ⚪ not started      | [`security.md`](security.md)                                               |
| [Numerics and data](numerics.md)                                   | 🟠 major questions | [`numerics.md`](numerics.md), [`units.md`](units.md)                       |
| [Text and strings](text.md)                                        | 🟡 minor questions | [`text.md`](text.md)                                                       |
| [Metaprogramming](comptime.md)                                     | 🟡 minor questions | [`comptime.md`](comptime.md)                                               |
| [Contracts and verification](correctness.md)                       | 🟠 major questions | [`correctness.md`](correctness.md)                                         |
| **Running programs**                                               |                    |                                                                            |
| [Concurrency across CPUs](concurrency.md)                          | 🟡 minor questions | [`concurrency.md`](concurrency.md), [`closures.md`](closures.md)           |
| [Performance model](performance.md)                                | ⚪ not started      | [`performance.md`](performance.md)                                         |
| [GPUs and accelerators](accelerators.md)                           | 🟡 minor questions | [`accelerators.md`](accelerators.md)                                       |
| [Distributed and multi-tier programs](distribution.md)             | 🟡 minor questions | [`distribution.md`](distribution.md)                                       |
| [Determinism and reproducibility](determinism.md)                  | 🟠 major questions | [`determinism.md`](determinism.md)                                         |
| [Program lifecycle](lifecycle.md)                                  | 🟡 minor questions | [`lifecycle.md`](lifecycle.md)                                             |
| [Observability and debuggability](observability.md)                | 🟡 minor questions | [`observability.md`](observability.md)                                     |
| **Library and ecosystem**                                          |                    |                                                                            |
| [Mods and the standard catalogue](mods.md)                         | 🟠 major questions | [`mods.md`](mods.md) (R2)                                                  |
| [Standard library shape](stdlib.md)                                | 🟡 minor questions | [`stdlib.md`](stdlib.md)                                                   |
| [Packages and supply-chain security](packages-and-supply-chain.md) | 🟡 minor questions | [`packages-and-supply-chain.md`](packages-and-supply-chain.md)             |
| [Interoperability with C and Rust](interop.md)                     | 🟡 minor questions | [`interop.md`](interop.md)                                                 |
| [Data interchange and persistence](data.md)                        | 🟡 minor questions | [`data.md`](data.md)                                                       |
| [UI, mobile and the browser](ui.md)                                | 🟡 minor questions | [`ui.md`](ui.md)                                                           |
| **Tools and workflow**                                             |                    |                                                                            |
| [Tooling uniformity](toolchain.md)                                 | 🟡 minor questions | [`toolchain.md`](toolchain.md), [`docs-and-tests.md`](docs-and-tests.md)   |
| [Build times](build-times.md)                                      | 🟠 major questions | [`build-times.md`](build-times.md)                                         |
| [Cross-compilation and deployment](targets.md)                     | 🟡 minor questions | [`targets.md`](targets.md), [`freestanding.md`](freestanding.md)           |
| [LLMs as first-class programmers](agent-interface.md)              | 🟡 minor questions | [`agent-interface.md`](agent-interface.md)                                 |
| [Diffs, code review and version control](semantic-vcs.md)          | 🟡 minor questions | [`semantic-vcs.md`](semantic-vcs.md)                                       |
| [Language evolution](evolution.md)                                 | 🟡 minor questions | [`evolution.md`](evolution.md)                                             |
| [Adoption](adoption.md)                                            | ⚪ not started      | [`adoption.md`](adoption.md)                                               |
| [Governance](governance.md)                                        | ⚪ not started      | [`governance.md`](governance.md)                                           |
| **The project**                                                    |                    |                                                                            |
| [Bootstrap compiler and backends](bootstrap.md)                    | 🟠 major questions | [`bootstrap.md`](bootstrap.md) (R1)                                        |
| [Roadmap and cross-cutting questions](roadmap.md)                  | 🟠 major questions | [`roadmap.md`](roadmap.md)                                                 |
| [Prior art](prior-art.md)                                          | reference          | [`prior-art.md`](prior-art.md)                                             |

## Working on these specs

For people and LLM agents editing the specs:

- **Each area's document is authoritative.** It holds the problem statement, prior art, decisions,
  directions and open questions in full; this index only points to it.
- **Respect what is settled.** Sections marked **Decision** or **Decided**, and the firm
  requirements **R1–R3**, are settled: don't reopen them unless asked. **Directions** and
  **Open questions** are still open.
- **The [reference examples](examples/README.md) are the yardstick:** a feature that none of them
  needs is a candidate for removal.

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
- **House style:** sections run *problem → state of the art → decision → directions → open
  questions*; code blocks are illustration only and follow [`syntax.md`](syntax.md) (Rust-based
  syntax); spelling is British.

<img src="moddil.png" alt="Moddil logo" align="right" width="150">

# Moddil

Moddil is a programming language being designed for three shifts happening at once:

- **LLM agents write more of the code**, and humans mostly review and approve it.
- **The machine is heterogeneous**: many CPU cores, GPUs and other accelerators.
- **A program is a fleet**: browser clients, servers, workers and databases working together.

Its core idea is **semantics that are explicit and checked by machines**. Effects, capabilities,
ownership and resources, and where code and data live all appear in the types and are checked by
the compiler. That makes code easier for agents to write correctly and for humans to review.

## Status

Moddil is at the **design stage**, and there is no compiler yet. The documents in `specs/` range
from brainstorm to decided direction; none of them is a formal specification. Code snippets are
illustration only and do not commit to any syntax.

## Start here

Read [`specs/overview.md`](specs/overview.md) first. Its **index** at the top lists every problem
area with a status (⚪ not started, 🔴 superficial, 🟠 major questions, 🟡 minor questions,
🟢 complete), and links to a short summary and to the subdocument that holds the detail. Then read the Thesis (§1) and the firm requirements (§5).

## Notes for LLMs and agents

- Start with `specs/overview.md`. It holds only an index and a short summary per area. The
  subdocuments linked from its index are authoritative, and hold the problem statement, prior art,
  decisions and directions in full.
- **Keep the overview in sync.** When you change a subdocument:
  - an open question lives in the subdocument's *Open questions* section and is copied into the
    matching overview summary, so change both together;
  - update the area's status in the overview index if it changed;
  - a new subdocument needs an index row.
- Sections marked **Decision** or **Decided**, and the firm requirements **R1–R3**, are settled.
  Don't reopen them unless asked. **Directions** and **Open questions** are still open.
- The four reference systems in [`specs/examples/`](specs/examples/README.md) are the yardstick: a
  feature that none of them needs is a candidate for removal.
- Follow the house style:
  - sections run *problem → state of the art → decision → directions → open questions*;
  - snippets are marked as illustration only;
  - spelling is British.

## Layout

```
specs/
  overview.md      why Moddil exists; index, status and summary of every area; start here
  *.md             subdocuments (memory, concurrency, errors, comptime, mods, bootstrap, …)
  requirements.md  firm requirements R1–R3 and non-goals, in full
  prior-art.md     prior-art matrix across languages, and references
  examples/        four reference systems used to test every decision
```

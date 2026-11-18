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

Read [`specs/README.md`](specs/README.md) first. It is the **index**: every area with a status
(⚪ not started, 🔴 superficial, 🟠 major questions, 🟡 minor questions, 🟢 complete) and a link to
the document that holds its detail. Then read [Why Moddil](specs/vision.md) for the thesis, and the
[firm requirements](specs/requirements.md#firm-requirements-decided).

## Notes for LLMs and agents

- Start with `specs/README.md`. It holds only the index. Each area's document is authoritative and
  holds its problem statement, prior art, decisions, directions and open questions in full.
- **Statuses live only in the index table; open questions live only in the area's document.**
  Nothing is copied between them. A new document needs an index row.
- **Link by heading name, never by section number**, and run `python tools/check_spec_links.py`
  after editing; it fails on any broken link or unindexed document (see
  [Working on these specs](specs/README.md#working-on-these-specs)).
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
  README.md        the index: every area, its status and its documents; start here
  vision.md        why Moddil exists: preamble, thesis, cross-cutting themes, tensions
  roadmap.md       strategic and technical questions that span areas
  requirements.md  firm requirements R1–R3 and non-goals, in full
  *.md             one document per area (memory, concurrency, errors, comptime, mods, …)
  prior-art.md     prior-art matrix across languages, and references
  examples/        four reference systems used to test every decision
tools/
  check_spec_links.py   link and index checker for specs/
```

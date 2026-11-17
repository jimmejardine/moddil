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

Read [`specs/rationale.md`](specs/rationale.md) first. Its status note and **document map** at the
top link to every other document. Then read the Thesis (§1) and the firm requirements (§5).

## Notes for LLMs and agents

- Start with `specs/rationale.md`. For detail, the companion documents linked from its document map
  are authoritative.
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
  rationale.md     why Moddil exists; problems, prior art, decisions; start here
  *.md             companion specs (memory, concurrency, errors, comptime, mods, bootstrap, …)
  examples/        four reference systems used to test every decision
```

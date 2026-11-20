# moddil

The **`moddil` command**: one tool for building, testing, formatting and everything else
([`specs/toolchain.md`](../../specs/toolchain.md)).

**Status:** not started. At M0 it runs files and tests on the MIR interpreter; the full command set
(`build`, `test`, `fmt`, `diff`, `audit`, `bind`, …) arrives over the milestones.

## Rules

- **A closed command set**, the same everywhere; third-party tools are pinned mods run with
  `moddil tool <name>` ([`specs/toolchain.md`](../../specs/toolchain.md)).
- **Diagnostics** in the rustc and Elm style for people, structured for agents
  ([`specs/agent-interface.md`](../../specs/agent-interface.md)).

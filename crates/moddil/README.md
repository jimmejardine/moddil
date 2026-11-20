# moddil

The **`moddil` command**: one tool for building, testing, formatting and everything else
([`specs/toolchain.md`](../../specs/toolchain.md)).

**Status:** slices S0 and S1. At M0 it checks files and runs them and their tests on the MIR
interpreter, and a failed `assert` shows the value of each part of its condition;
the full command set (`build`, `fmt`, `diff`, `audit`, `bind`, …) arrives over the milestones.

```
moddil check file.moddil    # report errors (exit 1 if any)
moddil run file.moddil      # run `fn main()` and print its result (exit 101 on a trap)
moddil test file.moddil     # run the file's `test` items
```

Diagnostics are rendered in the rustc style with `ariadne`; `--format json` comes later.

## Rules

- **A closed command set**, the same everywhere; third-party tools are pinned mods run with
  `moddil tool <name>` ([`specs/toolchain.md`](../../specs/toolchain.md)).
- **Diagnostics** in the rustc and Elm style for people, structured for agents
  ([`specs/agent-interface.md`](../../specs/agent-interface.md)).

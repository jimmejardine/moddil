# moddil

The **`moddil` command**: one tool for building, testing, formatting and everything else
([`specs/toolchain.md`](../../specs/toolchain.md)).

**Status:** slice S0. At M0 it checks files and runs them and their tests on the MIR interpreter;
the full command set (`build`, `fmt`, `diff`, `audit`, `bind`, …) arrives over the milestones.

```
moddil check file.mdl    # report errors (exit 1 if any)
moddil run file.mdl      # run `fn main()` and print its result (exit 101 on a trap)
moddil test file.mdl     # run the file's `test` items
```

Diagnostics are rendered in the rustc style with `ariadne`; `--format json` comes later.

## Rules

- **A closed command set**, the same everywhere; third-party tools are pinned mods run with
  `moddil tool <name>` ([`specs/toolchain.md`](../../specs/toolchain.md)).
- **Diagnostics** in the rustc and Elm style for people, structured for agents
  ([`specs/agent-interface.md`](../../specs/agent-interface.md)).

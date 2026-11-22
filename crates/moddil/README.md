# moddil

The **`moddil` command**: one tool for building, testing, formatting and everything else
([`specs/toolchain.md`](../../specs/toolchain.md)).

**Status:** M1. It checks programs (a file, or a mod's directory with a `moddil.toml`, read with
[`moddil-project`](../project/README.md)), runs them and their tests compiled to native code
([`moddil-native`](../native/README.md)), and builds them into executables. A failed `assert`
shows the value of each part of its condition. The rest of the command set (`fmt`, `diff`,
`audit`, `bind`, …) arrives over the milestones.

```
moddil check file.moddil           # report errors (exit 1 if any)
moddil run file.moddil             # run `fn main()` natively and print its result (70 on a trap)
moddil test file.moddil            # run the file's `test` items natively
moddil run --interp file.moddil    # the same on the MIR interpreter (and `test --interp`)
moddil build file.moddil [-o out]  # an executable: `main`'s exit code, 70 on a trap
```

- **Native by default.** What native code does not compile yet runs on the MIR interpreter
  instead, after a note saying what. Each native test runs in a process of its own, so a
  crash in compiled code fails that test, reported as a bug in the backend.
- **`build`** links the program's object with the runtime library `moddil-rt-main`
  ([`link.rs`](src/link.rs)): with MSVC's `link.exe` on Windows, the system's `cc` elsewhere. The
  library is found beside the `moddil` executable (`cargo build -p moddil-rt-main` puts it
  there), or where `MODDIL_RT_LIB` says.

Diagnostics are rendered in the rustc style with `ariadne`; `--format json` comes later.

## Rules

- **A closed command set**, the same everywhere; third-party tools are pinned mods run with
  `moddil tool <name>` ([`specs/toolchain.md`](../../specs/toolchain.md)).
- **Diagnostics** in the rustc and Elm style for people, structured for agents
  ([`specs/agent-interface.md`](../../specs/agent-interface.md)).

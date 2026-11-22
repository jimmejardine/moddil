# moddil-rt-main

The static library `moddil build` links a program's object file with: the process's `main`,
and through it [`moddil-rt`](../rt/README.md).

**Status:** M1. `main` runs the program's `moddil_entry`, then exits with `moddil_exit_code`'s
code (`main.exit.1`). A trap prints its class and where it was, from the tables the object
holds (`moddil_sites`, `moddil_strings`), and exits 70 (`lifecycle.md`).

## Rules

- **Nothing here is compiled into the compiler.** The library refers to symbols only a
  program's object defines, so it is a `staticlib` alone, with no tests of its own: `moddil
  build`'s tests build and run programs with it.

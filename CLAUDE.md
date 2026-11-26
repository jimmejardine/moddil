# Working in this repository

## Checks

- **While working**, after each change: `python tools/ci.py quick [AREA...]`. It runs what the
  uncommitted changes affect: `cargo fmt`, the layout of the changed `.moddil` files, the tests
  of each crate you changed, and the conformance areas whose files you changed, on all four
  implementations. It cannot see which areas a change to the compiler or the prelude reaches:
  name them (`python tools/ci.py quick borrows comparison`), or a package (`moddil-mir`).
- **Before committing**: `python tools/ci.py fmt clippy links test tour examples`, the steps CI
  runs. `quick` leaves out clippy, the CLI's tests (its commands, the mods, the tour and the
  examples on each runtime, the browser and Python tests), doc tests, `tour` and `examples`.
- If the change touches the CLI (`crates/moddil`), `mods/`, `tour/` or `examples/`, run the
  `test` step while working too: `quick` does not exercise them.
- Run the steps inside WSL (`--wsl`) only when the change has Linux-specific parts.
- The `fmt` step also checks the Moddil sources (the prelude, `mods/`, `tour/`, `examples/`,
  `conformance/`) are in `moddil fmt`'s layout: format a `.moddil` file you write or change with
  `cargo run -p moddil -- fmt <file>`. A conformance marker (`//~ E0201`, `// =>`) goes on the
  line the diagnostic or value is reported at, as the layout breaks it.

## The roadmap

`specs/roadmap.md` is one numbered list of the work on the horizon (the numbers are names, not
an order). It is not a history: git keeps that.
- When you build something the roadmap lists, remove it from the roadmap: a whole item (`P4`,
  `P42`, …) once it is done, or the parts of one you finished.
- When something you build leaves a part for later, add it to the list "On the horizon": to the
  item it belongs to, or as a new item with the next free number (an M or L item is one an agent
  can finish in one go: a small set of crates, no decision needed first, tests on every runner).
- Do not add an entry describing a slice you finished. The commit message, the reference and
  the crates' READMEs describe it.

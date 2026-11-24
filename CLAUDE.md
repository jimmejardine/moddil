# Working in this repository

## Checks

- **While working**, after each change: `python tools/ci.py quick` (about half a minute). It runs
  `fmt` and every crate's tests but the CLI's: the libraries' tests and the whole conformance
  suite on all four implementations.
- **Before committing**: `python tools/ci.py fmt clippy links test tour examples`, the steps CI
  runs. `quick` leaves out clippy, the CLI's tests (its commands, the mods, the tour and the
  examples on each runtime, the browser and Python tests), doc tests, `tour` and `examples`.
- If the change touches the CLI (`crates/moddil`), `mods/`, `tour/` or `examples/`, run the
  `test` step while working too: `quick` does not exercise them.
- Run the steps inside WSL (`--wsl`) only when the change has Linux-specific parts.

## The roadmap

`specs/roadmap.md` lists only what is still to build, and what built work left for later. It is
not a history: git keeps that.
- When you build something the roadmap lists, remove it from the roadmap: a whole work package
  (`P1`, `P2`, …) once it is done, or the parts of one you finished.
- When something you build leaves a part for later, add that part under "Deferred from built
  work": to the package it belongs to, or as a new package an agent can finish in one go (a
  small set of crates, no decision needed first, tests on every runner).
- Do not add an entry describing a slice you finished. The commit message, the reference and
  the crates' READMEs describe it.

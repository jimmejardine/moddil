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

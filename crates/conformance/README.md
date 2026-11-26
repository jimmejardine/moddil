# moddil-conformance

The runner for the [conformance suite](../../conformance/README.md).

**Status:** slices S0 to S11: runs the suite on the MIR interpreter and the reference interpreter,
and checks reference coverage; `cargo test -p moddil-conformance` runs it (`tests/suite.rs`), one
test per area so the test runner spreads it over every core (a new area joins the `areas!` list
there). The reference interpreter does not check programs, so it runs only files that expect no
diagnostics.
The markers and how they bind are in the [suite's README](../../conformance/README.md).

## Does

- Finds `conformance/<edition>/<feature>/*.moddil` files, and directories with a `moddil.toml`
  (read with [`moddil-project`](../project/README.md)), and their inline expectations: `// =>`
  values, `//~ E…` diagnostic codes, `//~ trap`, `//~ error`, and `#~ E…` in a manifest.
- Runs each test on every available implementation (the reference interpreter, the MIR
  interpreter, native code, native code under the parallel runtime, and WASM) and reports any
  disagreement; under the parallel runtime a `// =>` line may also give what its `(in parallel,
  …)` lists (`item.test.value.2`). The native backend runs only the files whose code it compiles
  so far; `MODDIL_NATIVE_WHY=1` prints, for each file it skips, what it does not compile yet.
- `tests/stress.rs`, the runtimes' stress suite: random trees of tasks (nested scopes, spawns,
  joins, channels, `yield_now`, `Rc`s shared across tasks, isolated scopes a task traps in) whose
  value is fixed whatever order the tasks run in, each run under many seeds of the deterministic
  runtime (`task.runtime.1`), on the MIR interpreter and natively: every seed gives the value,
  with no trap and no leak. `MODDIL_STRESS_TREES` and `MODDIL_STRESS_SEEDS` set how many.
- Measures **reference coverage** from the paragraph IDs tests cite (`// covers: …`).
- Skips tests marked *disputed*.

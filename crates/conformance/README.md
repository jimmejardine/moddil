# moddil-conformance

The runner for the [conformance suite](../../conformance/README.md).

**Status:** slices S0 to S7c: runs the suite on the MIR interpreter and the reference interpreter,
and checks reference coverage; `cargo test -p moddil-conformance` runs it (`tests/suite.rs`), one
test per area so the test runner spreads it over every core (a new area joins the `areas!` list
there). The reference interpreter does not check programs, so it runs only files that expect no
diagnostics.
The markers and how they bind are in the [suite's README](../../conformance/README.md).

## Does

- Finds `conformance/<edition>/<feature>/*.moddil` files, and directories with a `moddil.toml`
  (read with [`moddil-project`](../project/README.md)), and their inline expectations: `// =>`
  values, `//~ E…` diagnostic codes, `//~ trap`, `//~ error`, and `#~ E…` in a manifest.
- Runs each test on every available implementation (the reference interpreter, the MIR interpreter,
  later compiled code) and reports any disagreement.
- Measures **reference coverage** from the paragraph IDs tests cite (`// covers: …`).
- Skips tests marked *disputed*.

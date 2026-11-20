# moddil-conformance

The runner for the [conformance suite](../../conformance/README.md).

**Status:** part of slice S0, so every later slice lands with tests.

## Does

- Finds `conformance/<edition>/<feature>/*.mdl` files and their inline expectations:
  `// =>` values, `//~ E…` diagnostic codes, `//~ trap`.
- Runs each test on every available implementation (the reference interpreter, the MIR interpreter,
  later compiled code) and reports any disagreement.
- Measures **reference coverage** from the paragraph IDs tests cite (`// covers: …`).
- Skips tests marked *disputed*.

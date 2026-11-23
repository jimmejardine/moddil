# moddil-wasm

The WASM backend of M1 ([`specs/roadmap.md`: M1](../../specs/roadmap.md)): each function's MIR
is lowered to a core WebAssembly module by our own emitter, on `wasm-encoder`
([`specs/compiler.md`](../../specs/compiler.md)), and the module is run in Node.

**Status:** slice W0: scalars, their operators and checks, control flow, direct calls, and
tuples, structs, enums (with niches) and arrays in linear memory, with borrows of them. Anything
else a program reaches is reported as not compiled to WASM yet.

- **Control flow** is structured from the dominator tree ([`cfg.rs`](src/cfg.rs), after Ramsey's
  "Beyond Relooper"): a `loop` for each loop header, a `block` for each merge node, and each
  branch the code it goes to or a `br` to the label that leads there.
- **Values** ([`emit.rs`](src/emit.rs)):
  - `int`, `i64`, `u64` and `f64` are wasm's `i64` and `f64`;
  - the narrower integers, `bool` and `char` are `i32`, kept sign- or zero-extended as their
    type says;
  - borrows are `i32` addresses.
  - Aggregates, and the scalars something borrows, live in each function's frame on a shadow
    stack in linear memory.
- **The ABI** is the native backend's status return: `(i32 status, result?)`. Aggregates pass
  by the address of a caller's copy, and come back through an out-pointer. The overflow checks
  are written out, since wasm has none.
- **Layouts** are the native backend's ([`moddil_native::layout`](../native/src/layout.rs)),
  with 32-bit pointers.
- **Running:** [`host.cjs`](src/host.cjs) runs a module's entries in Node and reports a JSON
  object a line. At each `// =>` it snapshots the shadow stack in use, and
  [`run.rs`](src/run.rs) renders the value from the snapshot with the native backend's renderer
  ([`moddil_native::show`](../native/src/show.rs)). Node is found on the path, or where
  `MODDIL_NODE` says.

## Rules

- **Checked by the conformance suite:** a fourth runner, `Wasm`, runs every file this backend
  compiles and needs its results to equal the interpreters'; `MODDIL_WASM_WHY=1` says what
  keeps each other file from running. Without Node, the runner and the tests that run modules
  skip, saying so.
- **Every module validates:** the tests check emitted modules with `wasmparser` before they run.

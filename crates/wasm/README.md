# moddil-wasm

The WASM backend of M1 ([`specs/roadmap.md`: M1](../../specs/roadmap.md)). The native lowering
([`moddil_native::backend`](../native/src/backend.rs)) builds Cranelift IR into a `WasmModule`,
which translates each function to core WebAssembly with our own emitter, on `wasm-encoder`
([`specs/compiler.md`](../../specs/compiler.md)). The module runs in Node.

**Status:** slice W1: everything the native backend compiles but tasks, which wait for the JSPI
lowering.

- **One lowering for both backends:** [`module.rs`](src/module.rs) implements
  `cranelift_module::Module`, so the lowering builds into it as it builds into the JIT, and
  each function is translated as it is defined ([`clif.rs`](src/clif.rs)).
- **Values:**
  - each Cranelift value is a wasm local;
  - `i8` and `i16` are kept zero-extended in an `i32`;
  - `i128` is two `i64`s, with what WASM has no instructions for done by the runtime;
  - pointers are `i64`s, as the shared layouts have them, wrapped to `i32` addresses at each
    access.
- **Stack slots** are a frame on a shadow stack, which grows down.
- **Control flow** is structured from the dominator tree ([`cfg.rs`](src/cfg.rs), after Ramsey's
  "Beyond Relooper"): a `loop` for each loop header, a `block` for each merge node, and each
  branch either the code it goes to or a `br` to the label that leads there. A branch's
  arguments are stored on its edge.
- **The runtime** is `moddil-rt` built for wasm32 ([`moddil-rt-wasm`](../rt-wasm/README.md), by
  [`build.rs`](build.rs)). It shares the module's memory, and its allocator is the heap.
  - A call to it is adapted to its wasm32 signature: addresses cross as `i32`s.
  - The host gives `observe` and `log`.
  - A call to a task function is refused for now.
- **Memory** ([`module.rs`](src/module.rs)), from the bottom:
  - the runtime's static data;
  - the program's scratch: each entry's result and error, the run's record and `i128` results;
  - its data objects at fixed addresses;
  - a 16 MiB shadow stack.
  - The heap grows above all that.
- **Running:** [`host.cjs`](src/host.cjs) runs the module in a worker thread with a deep stack,
  as [`run.rs`](src/run.rs) says over a pipe.
  - Before each entry, the data is restored as it was, so one entry's `const` items aren't
    another's.
  - The renderer ([`moddil_native::show`](../native/src/show.rs)) reads values from memory a
    page at a time while they are there.
  - Results, errors, failed `assert`s' parts, leaks and traps are reported as the JIT runner
    reports them.
  - Node is found on the path, or where `MODDIL_NODE` says.

## Rules

- **Checked by the conformance suite:** a fourth runner, `Wasm`, runs every file this backend
  compiles and needs its results to equal the interpreters'. `MODDIL_WASM_WHY=1` says what
  keeps each other file from running. Without Node, the runner and the tests that run modules
  skip, saying so.
- **Every module validates:** the tests check emitted modules with `wasmparser` before they run.

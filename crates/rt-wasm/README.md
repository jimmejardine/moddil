# moddil-rt-wasm

[`moddil-rt`](../rt/README.md) built for `wasm32-unknown-unknown`: the runtime a Moddil WASM
module calls into ([`specs/roadmap.md`: M1](../../specs/roadmap.md)).

- **Built by `moddil-wasm`'s build script** ([`build.rs`](../wasm/build.rs)), into that build's
  own target directory. The target is pinned in `rust-toolchain.toml`. The module imports its
  memory and exports `__heap_base`, where the program's own memory starts.
- **One memory:** the program shares it, so it calls `moddil-rt`'s functions (`moddil_rt_*`, as
  native code links them) with its own addresses. Its allocator takes memory only by growing
  it, above the program's data and stack. It is the heap, and it counts live allocations for
  the leak check.
- **What it adds** ([`src/lib.rs`](src/lib.rs)):
  - for the host: `moddil_wasm_begin` (a new run), `moddil_wasm_live` and `moddil_wasm_record`
    (the trap and the last `fail` a run recorded);
  - for the code: `moddil_wasm_i128`, the `i128` arithmetic WASM has no instructions for.
- **Not used yet:** its task functions, which need the JSPI lowering.

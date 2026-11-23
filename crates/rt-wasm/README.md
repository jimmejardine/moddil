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
  - `moddil_wasm_task`, a task's call, which the host starts as a promising one;
  - `moddil_wasm_sites`, where the program's site tables are (the text of a `Trapped`);
  - for the code: `moddil_wasm_i128`, the `i128` arithmetic WASM has no instructions for.
- **Tasks:** its baton (`moddil-rt`'s `baton.rs`) hands the turn over through the host's
  suspending `moddil_host_switch`. It calls the program's task code and drop glue through the
  shim module (`moddil_call_task`, `moddil_call_drop`), so no JS frame is in between.
- **Its stack pointer** is exported too (`__stack_pointer`): the host gives each task its own.

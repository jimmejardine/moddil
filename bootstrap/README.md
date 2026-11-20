# Bootstrap

Reserved for **after self-hosting**, which is deferred until after v1
([`specs/compiler.md`: Bootstrapping after self-hosting](../specs/compiler.md#bootstrapping-after-self-hosting)).

**Status:** unused. Until then the Rust compiler in [`crates/`](../crates/README.md) is *the*
compiler, built reproducibly with Cargo.

## What will go here

- `moddil1.wasm`: a checked-in WASM build of the compiler (Zig's approach), so building from source
  needs only a WASM runner.
- It must be **reproducible**, and the Rust compiler is kept buildable so the two paths can be compared
  (diverse double-compiling).

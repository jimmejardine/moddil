# moddil-native

The native backend of M1 ([`specs/roadmap.md`: M1](../../specs/roadmap.md)): each function's MIR
is lowered to Cranelift IR and compiled by `cranelift-jit` into executable memory in the
compiler's own process, so `moddil run` and `moddil test` need no linker; or, for `moddil build`,
by `cranelift-object` into an object file, linked with
[`moddil-rt-main`](../rt-main/README.md). The lowering is generic over Cranelift's `Module`,
so both are the same code.

- **What it compiles** grows in slices: C0 covers scalars (`int`, `bool`, `char`, the sized
  integers and the floats), arithmetic with its checks, control flow and direct calls of
  non-generic functions. Anything else a program reaches is reported as not supported yet.
- **Traps** use a status return: every compiled function returns a status beside its result. A
  failed check calls `moddil-rt` to record where, and returns "trapped", which each caller passes
  on, so a trap in a test never takes the process down.
- **`unsafe`** is confined to `jit.rs`, which calls the finished code.
- The conformance suite runs every file it can natively, and its results must equal the
  interpreters'.

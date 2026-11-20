# moddil-mir

Moddil's **mid-level IR** and the lowering from the checked AST into it.

**Status:** not started; its outline is the next design topic.

## Does

- Holds the Moddil-specific lowering: handlers, task scopes, suspension points, drops and
  `defer`/`errdefer`, generics as witness tables (debug) or specialised copies (release).
- Is the single input to every backend: the MIR interpreter now; Cranelift, LLVM, the own WASM
  emitter and GPU backends later
  ([`specs/compiler.md`: Backends](../../specs/compiler.md#backends)).

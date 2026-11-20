# moddil-mir

Moddil's **mid-level IR**, built from the typed HIR, and the passes over it
([`specs/mir.md`](../../specs/mir.md)).

**Status:** slice S0: the `mir(owner)` query builds a body from the typed HIR, for files without
errors. Locals have no projections yet; ownership checking and drops come with slice S2.

- **Checks:** `+`, `-`, `*`, `/`, negation and shifts are preceded by an `Overflows` test and an
  `Assert` with the `overflow` trap class; `/` and `%` by a zero test with `division by zero`.
- **Tests:** `Observe` statements carry the values a test's `// =>` lines expect.
- Blocks control never reaches are removed. The text form (`format!("{body:?}")`) is snapshot
  tested in `src/snapshots/`.

## Shape

- **Places and a control-flow graph**, as Rust's MIR: locals, places, statements and terminators in
  basic blocks; not SSA.
- **Built per item** (function, closure, constant, comptime block), on demand, through the query
  system.
- **Phases:** *built* (from HIR) → *analysed* (ownership checked, drops elaborated) → *runtime* (checks
  explicit, handlers resolved, reference-counting operations, generics resolved) → *lowered* per
  target (suspension, layouts, ABI).

## What it makes explicit

- **Ownership:** moves, drops, linear consumption and second-class borrows are checked here.
- **Suspension:** `Suspend` is an explicit terminator, so every suspension point is visible.
- **Effects:** handlers are hidden parameters (evidence passing); known handlers become direct calls;
  `fail` is a second exit edge on calls.
- **Checks:** overflow, bounds, refinements, `?!` and contracts are `Assert` statements with a trap
  class and a source span.

It is the single input to every backend: the MIR interpreter now; Cranelift, LLVM, the own WASM
emitter and GPU backends later ([`specs/compiler.md`: Backends](../../specs/compiler.md#backends)).

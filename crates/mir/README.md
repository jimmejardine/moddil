# moddil-mir

Moddil's **mid-level IR**, built from the typed HIR, and the passes over it
([`specs/mir.md`](../../specs/mir.md)).

**Status:** slices S0 to S9a: the `mir(owner)` query builds a body from the typed HIR, for files
without errors (a string literal is a constant whose text the body holds, and `fmt"…"` a run of
`String` and `Display` calls; an element is a place with an `Index` projection, checked to be in
bounds first, and a collection literal a call of `FromArrayLiteral`), once for a generic function: calls name their callee and its type arguments, and a
trait's method is resolved when the call runs. Values that need dropping get `Drop` terminators at
the end of their scope, before assignments, and at the end of a temporary's statement. A call of
a function that can fail has an error edge, and `Fail` leaves a function with an error; `?`,
`try`, `catch` and `?!` are branches on these, and `errdefer` blocks run on the error exits of
their scope. A function's effects are hidden parameters after the others, each a `&dyn` borrow of
its handler (evidence passing); a call passes its callee's from the caller's own or from a `with`
around it, which holds its handler for the block; `Unsize` makes a borrow a `dyn` borrow. A
closure is a value built from its captures, and has a body of its own whose two parameters are its
captures and its arguments, as tuples; calls of closures go through the closure traits' methods. Places are a local with field, downcast and deref projections; operands copy or
move; aggregates build tuples, structs and variants; patterns become tests on places and
discriminants, tried arm by arm; loops are back edges; `defer` blocks are emitted on every exit of
their scope, and locals end with `StorageDead`. The `borrowck(owner)` query checks ownership on
it: moves (maybe-moved places), loans (which locals hold which borrows, and which locals are live)
and their conflicts, borrows outliving their locals or temporaries, returned borrows, and moves out
of values with destructors. `diagnostics(file)`
combines it with the front end's.

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

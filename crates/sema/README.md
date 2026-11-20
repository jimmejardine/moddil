# moddil-sema

Semantic analysis: everything Moddil checks before code runs. It produces the **typed HIR**:
desugared and name-resolved, carrying inferred types, effects and error sets. The language server,
`moddil explain` and MIR building all read it
([`specs/mir.md`: Pipeline](../../specs/mir.md#pipeline)). Ownership is checked later, on MIR.

**Status:** slice S0: functions and tests, `let`, `int`, `bool` and `()`, operators, `if`, blocks,
calls and `return`, with the diagnostics of [`reference/2027/diagnostics.md`](../../reference/2027/diagnostics.md).
Valid Moddil beyond S0 is reported as "not supported yet" (`I0001`). It starts as one crate and
splits as it grows (names, types, effects).

## Queries (Salsa)

Every analysis is a query over `SourceFile` inputs, memoised and recomputed only when what it read
changes:

| Query | Result |
|---|---|
| `parse(file)` | The lossless syntax tree |
| `item_tree(file)` | The functions and tests, with their signatures |
| `body(owner)` | A function's or test's body in HIR, with names resolved and sugar removed |
| `infer(owner)` | The types of the body's expressions and locals, and type errors |
| `diagnostics(file)` | Everything wrong with the file, in source order (only syntax errors if there are any) |

An `Owner` is a function or a test. In a `test`, an expression statement or final expression
followed by `// =>` on its line is marked as **observed**, so the test harness sees its value.

## Does

- **Names and modules:** the file tree is the module tree; visibility (`pub`, `pub(mod)`)
  ([`specs/modules.md`](../../specs/modules.md)).
- **Types:** traits, generics checked once at the definition, coherence and bridges, refinements
  ([`specs/types.md`](../../specs/types.md)).
- **Effects and errors:** effect sets inferred inside a mod and explicit at `pub`, handlers, `fail`,
  R3 entry points ([`specs/effects-and-capabilities.md`](../../specs/effects-and-capabilities.md),
  [`specs/error-handling.md`](../../specs/error-handling.md)).
- **Ownership rules** are specified in [`specs/memory.md`](../../specs/memory.md) and
  [`specs/resources.md`](../../specs/resources.md), but checked on MIR (`moddil-mir`), where the
  control-flow graph makes moves and drops precise.
- **Capabilities:** mod ceilings and root capabilities.

Query-based (Salsa), so the language server and agents get check-only results fast
([`specs/compiler.md`: Compiler architecture](../../specs/compiler.md#compiler-architecture)).

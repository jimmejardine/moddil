# moddil-sema

Semantic analysis: everything Moddil checks before code runs.

**Status:** not started. It starts as one crate and splits as it grows (names, types, effects,
ownership).

## Does

- **Names and modules:** the file tree is the module tree; visibility (`pub`, `pub(mod)`)
  ([`specs/modules.md`](../../specs/modules.md)).
- **Types:** traits, generics checked once at the definition, coherence and bridges, refinements
  ([`specs/types.md`](../../specs/types.md)).
- **Effects and errors:** effect sets inferred inside a mod and explicit at `pub`, handlers, `fail`,
  R3 entry points ([`specs/effects-and-capabilities.md`](../../specs/effects-and-capabilities.md),
  [`specs/error-handling.md`](../../specs/error-handling.md)).
- **Ownership:** moves, second-class borrows, views, drop order, linear types
  ([`specs/memory.md`](../../specs/memory.md), [`specs/resources.md`](../../specs/resources.md)).
- **Capabilities:** mod ceilings and root capabilities.

Query-based (Salsa), so the language server and agents get check-only results fast
([`specs/compiler.md`: Compiler architecture](../../specs/compiler.md#compiler-architecture)).

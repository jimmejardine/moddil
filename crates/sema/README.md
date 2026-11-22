# moddil-sema

Semantic analysis: everything Moddil checks before code runs. It produces the **typed HIR**:
desugared and name-resolved, carrying inferred types, effects and error sets. The language server,
`moddil explain` and MIR building all read it
([`specs/mir.md`: Pipeline](../../specs/mir.md#pipeline)). Ownership is checked later, on MIR.

**Status:** slices S0 to S11: functions and tests, `let`, `int`, `bool` and `()`, operators, `if`,
blocks, calls and `return` (S0); tuples, structs, enums, patterns, `match` with exhaustiveness
checking, `if let`, `let … else`, loops and `assert` (S1); borrow types and their mutability
rules, `*`, binding modes, `Copy` and `defer` (S2; moves and borrows are checked on MIR); type
parameters and bounds, traits, impls, methods, operators through traits, derives (expanded to
Moddil source and checked like it), destructors and the prelude, inference by unification, trait
solving and the one-impl-per-trait-and-type check (S3); error sets, `fail`, `?`, `?!`, `try` and
`catch`, `errdefer`, unused `Result`s and entry points (S4); effects in signatures, `with`
handlers, where each call's needs are met, the inference of private functions' effects and errors
(the `effects` query, which checks them until they settle), and `dyn` borrows (S5); programs of
several files and mods (a `Program` input), the module tree, names per module with `use` and
paths, visibility and what `pub` items may expose (S6); closures (checked with their enclosing
body, their captures and kind found at its end), `impl Fn` parameters, closure bounds, anonymous
impls and `#[track_caller]` (S7a); `char`, `str` and `String`, text literals and blocks, `fmt"…"`
with its holes checked against `Display`, and character and string patterns (S7b); arrays,
slices and const parameters, views and borrows as type arguments, `for` (lowered to
`IntoIterator` and `Iterator`), ranges, indexing through `Index`, collection literals typed by
context, `Buf`, `#[derive(Hash)]`, the built-in `core` mod, and borrows of static data (S7c);
const items, `const fn` and `const { }` with the rules of compile-time code, and the `Debug` and
`Default` derives (S8a); refinements (`type X = T where p`, with `X::new`, refined literals and
patterns) and contracts (`requires`, `ensures`, `old`) (S8b); function wrappers (`#[around(w)]`,
expanded to a call of `w` with `&INFO`, `f::Args` and the body as a closure) (S8c); the control
effects `suspend` and `spawn`, and the prelude's `scope`, `spawn` and `join` (S9a); named and default arguments, `select`, and
the prelude's channels, `Trapped` and stop (S9b); functions as values (as closures calling
them), opaque `impl Trait` results, `test … for`, `main`'s root capabilities and `#[selectable]`
(S9c), with the
diagnostics of
[`reference/2027/diagnostics.md`](../../reference/2027/diagnostics.md). Valid Moddil beyond S11 is
reported as "not supported yet" (`I0001`). It starts as one crate and
splits as it grows (names, types, effects).

## Queries (Salsa)

Every analysis is a query over `SourceFile` inputs, memoised and recomputed only when what it read
changes:

| Query | Result |
|---|---|
| `parse(file)` | The lossless syntax tree |
| `item_tree(file)` | The functions, tests, structs and enums, with their signatures |
| `body(owner)` | A function's or test's body in HIR, with names resolved and sugar removed |
| `infer(owner)` | The types of the body's expressions, patterns and locals; type errors; exhaustiveness (Maranget's usefulness algorithm, with integer ranges split at their bounds) |
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

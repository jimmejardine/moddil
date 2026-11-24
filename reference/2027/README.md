# Moddil 2027: the reference

The normative rules of the 2027 edition. It grows with the compiler, slice by slice: a feature is
done only when it has paragraphs here, conformance tests in
[`conformance/2027/`](../../conformance/README.md) and an implementation
([`specs/definition.md`: Written with the compiler](../../specs/definition.md#written-with-the-compiler)).

**Status:** slices S0 to S11: functions, tests, `let`, `int`, `bool` and `()`, arithmetic with
traps, comparisons, logic, `if`, blocks and calls (S0); tuples, structs, enums, patterns, `match`,
`if let`, `let … else`, loops and `assert` (S1); moves, `Copy`, borrows and `defer` (S2); generics,
traits, impls, methods, operators through traits, derives, destructors and the prelude (S3);
errors: `fail`, `?`, `?!`, `try` and `catch`, `errdefer` and `Result` (S4); effects: traits and
context data after `with`, handlers installed with `with`, inference inside a mod, and `dyn`
borrows (S5); modules and mods: the module tree, visibility, `use`, the manifest and path
dependencies (S6); closures, the closure traits, captures and anonymous impls, and
`#[track_caller]` (S7a); text: `char`, `str` and `String`, their literals, `Display` and `fmt"…"`
(S7b); arrays, slices and const parameters, views, `for` and iterators, `Vec` over a built-in
buffer, collection literals, hashing, and the `core` mod's maps and sets, and borrows of static
data (S7c); const items, `const fn` and `const { }`, evaluated when compiling, and the `Debug` and
`Default` derives (S8a); refinements and contracts (S8b); function wrappers, `#[around]` (S8c); tasks and scopes (S9a); named and default arguments, cancellation, isolation, channels and
`select` (S9b); functions as values, `impl Trait` results, `map`, `filter` and `collect`,
`test … for`, `log`, `main`'s root capabilities and `ExitCode`, and `#[selectable]` (S9c); stub
mods, doc comments, `Pattern`, and bounds on a closure's errors (S9d); numbers (S10); `String` in Moddil (S11); derives
written in Moddil (M2a); unsafe code, `Refined`, results that hold handlers, and selectable functions in `select` (M2b); closure bounds that take control effects, and virtual time (M2c).

## Paragraphs

Every rule is a paragraph that starts with its **stable ID** in bold, such as
**expr.arith.overflow.1**. An ID never changes meaning; a rule that changes gets a new ID, and the
old one is marked with the edition that retired it. Conformance tests cite the IDs they cover
(`// covers: expr.arith.overflow.1`), and the conformance runner checks that every ID here is cited
and that every cited ID exists.

Each page lists its diagnostics by code; the codes are in [Diagnostics](diagnostics.md).

## Pages

- [Syntax](syntax.md)
- [Names](names.md), [Modules and mods](modules.md)
- Items: [Functions](items/functions.md), [Tests](items/tests.md), [Structs](items/structs.md),
  [Enums](items/enums.md), [Traits](items/traits.md), [Impls](items/impls.md)
- [Attributes](attributes.md)
- [Types](types.md), [Numbers](numbers.md), [Generics](generics.md), [Text](text.md),
  [Compile-time evaluation](comptime.md), [Refinements](refinements.md)
- [The prelude](prelude.md) ([source](prelude/))
- [Statements](statements.md)
- [Patterns](patterns.md)
- [Ownership](ownership.md), [Borrows](borrows.md), [Views](views.md),
  [Shared values: `Rc` and `Weak`](rc.md)
- [Iteration and collections](collections.md) ([core source](core/collections.moddil))
- [Errors](errors.md), [Effects](effects.md), [Contracts](contracts.md),
  [Concurrency](concurrency.md), [Unsafe code](unsafe.md), [The operating system](os.md),
  [GPU kernels](gpu.md), [C interop](interop.md)
- Expressions: [Evaluation](expressions/evaluation.md), [Literals](expressions/literals.md),
  [Arithmetic](expressions/arithmetic.md), [Bitwise operators](expressions/bitwise.md),
  [Comparisons](expressions/comparison.md), [Logic](expressions/logic.md),
  [Blocks](expressions/blocks.md), [`if`](expressions/if.md), [Calls and `return`](expressions/calls.md),
  [Assignment](expressions/assignment.md), [Tuples, structs and variants](expressions/structs.md),
  [Fields](expressions/fields.md), [Methods](expressions/methods.md),
  [Arrays and indexing](expressions/arrays.md),
  [Closures and anonymous impls](expressions/closures.md), [`match`](expressions/match.md),
  [Loops](expressions/loops.md), [`assert`](expressions/assert.md)
- [Traps](traps.md)
- [Diagnostics](diagnostics.md)

# Conformance suite

The **shared test suite** every Moddil implementation must pass to use the name "Moddil <edition>"
([`specs/definition.md`: A shared conformance suite](../specs/definition.md#a-shared-conformance-suite)).

**Status:** slices S0 to S8a: [`2027/`](2027) tests functions, tests, `let`, `int`, `bool` and `()`,
arithmetic and its traps, comparisons, logic, `if`, blocks and calls (S0); tuples, structs, enums,
patterns, `match`, `if let`, `let … else`, loops and `assert` (S1); moves, `Copy`, borrows and
`defer` (S2); generics, traits, impls, methods, operators through traits, derives, destructors and
the prelude (S3); errors, `try` and `catch` (S4); effects, handlers, inference and `dyn` borrows
(S5); modules, mods, visibility and the manifest (S6); closures and anonymous impls (S7a); text (S7b); arrays, views, iteration and collections (S7c); compile-time evaluation and derives (S8a). It cites every paragraph of the
[2027 reference](../reference/2027/README.md), and every test runs on both the MIR interpreter and
the reference interpreter. The runner is [`crates/conformance`](../crates/conformance/README.md);
`cargo test -p moddil-conformance` runs the suite.

## Layout

`conformance/<edition>/<feature>/*.moddil`, organised by feature and edition, like Test262. A test
of several files is a directory with a `moddil.toml`
([Modules and mods](../reference/2027/modules.md)): a mod, with its `src/` files and the mods it
depends on by path, in directories of their own inside it. Its expectations may be in any of its
files, and a manifest's in `#~ E0705` comments; every test of every mod in it runs.

## Writing a test

Tests are ordinary Moddil files with inline expectations, and cite the reference paragraphs they
cover:

```rust
// conformance/2027/arithmetic/overflow.moddil
// covers: expr.arith.overflow.1, trap.1

fn max() -> int { 9223372036854775807 }

test "in range" {
    max() - 1 + 1             // => 9223372036854775807
}

test "addition" {
    max() + 1;                //~ trap
}
```

```rust
// conformance/2027/calls/errors.moddil
// covers: expr.call.1

fn one(a: int) -> int { a }

fn f() -> int {
    one(1, 2)                 //~ E0202
}
```

| Marker | Expects |
|---|---|
| `// covers: id, …` | The reference paragraphs the file tests; every file has one |
| `// => value` | The expression statement or block's final expression it follows evaluates to `value`, written as Moddil source (`3`, `-1`, `true`, `()`) |
| `//~ E0202` | A diagnostic with that code starting on this line; several codes may follow (wording is not part of conformance). Warnings have `W` codes, such as `//~ W0201` |
| `//~ trap` | A trap on this line: the line of the operation that fails, which may be inside a function the test calls |
| `//~ error` | An error leaves a test on this line (the `?`, `fail` or `try` in the test's body it leaves through), failing the test |
| `// disputed` | The test does not count until the reference is clarified |

- **Values are observed in `test` items.** A file has only items at the top level, so the runner
  runs every `test` in the file, and every `// =>` line must run and produce exactly its value.
  Traps must happen exactly on the `//~ trap` lines, and errors leave tests exactly on the
  `//~ error` lines; a test stops at its trap or error.
- **A file with expected diagnostics is only checked, not run**, and must report exactly the
  diagnostics marked. Error tests therefore live in their own files.
- **Coverage:** the runner fails if a reference paragraph is cited by no test, or a test cites a
  paragraph that does not exist.
- Determinism makes expected outputs exact, including for concurrent tests under the simulation
  scheduler.
- The spec team owns the suite; any implementation may contribute. A disputed test is marked
  *disputed* and does not count until the reference is clarified.
- Every test runs on the reference interpreter, the MIR interpreter and, from M1, compiled code; a
  difference is a bug in one of them. The reference interpreter does not check programs, so it
  runs only files that expect no diagnostics.

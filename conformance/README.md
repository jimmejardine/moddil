# Conformance suite

The **shared test suite** every Moddil implementation must pass to use the name "Moddil <edition>"
([`specs/definition.md`: A shared conformance suite](../specs/definition.md#a-shared-conformance-suite)).

**Status:** empty until slice S0, which brings the runner (`crates/conformance`).

## Layout

`conformance/<edition>/<feature>/*.mdl`, organised by feature and edition, like Test262.

## Writing a test

Tests are ordinary Moddil files with inline expectations, and cite the reference paragraphs they
cover:

```rust
// conformance/2027/effects/handler-scope.mdl
// covers: effect.handler.scope.1, effect.handler.scope.2

with Counter = Counting::new() {
    twice()                       // => 3
}

fn leak() -> int { counter.tick() }   //~ E0601
```

| Marker | Expects |
|---|---|
| `// => value` | The expression evaluates to `value` |
| `//~ E0601` | A diagnostic with that code (wording is not part of conformance) |
| `//~ trap` | A trap |

- Determinism makes expected outputs exact, including for concurrent tests under the simulation
  scheduler.
- The spec team owns the suite; any implementation may contribute. A disputed test is marked
  *disputed* and does not count until the reference is clarified.
- Every test runs on the reference interpreter, the MIR interpreter and compiled code; a difference
  is a bug in one of them.

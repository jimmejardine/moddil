# `unreachable` and `todo`

```rust
fn side(s: &Shape) -> int {
    match s {
        Shape::Square(n) => *n,
        Shape::Circle(_) => unreachable("only squares are measured here"),
    }
}
```

**expr.unreachable.1** `unreachable("reason")` says that the program cannot get there: if it does,
an invariant is broken, and the program traps with the reason ([Traps](../traps.md)). It has type
`!`, so it fits wherever a value is expected. The reason is required, and is a string literal
(E0202). It is for a bug, not for an error the program can handle; that is a value
([Errors](../errors.md)).

**expr.todo.1** `todo("what")` marks code not written yet: it traps with "not implemented yet" and
the reason. It has type `!`, and its reason is a string literal as `unreachable`'s is (E0202).

Both are built-ins in the prelude, as [`assert`](assert.md) is (**expr.assert.name.1**): a local or
an item of the same name comes first, and shadows them. They are called, never used as values
(E0112).

# Loops

```rust
let mut i = 0;
while i < 10 { i += 1; }

let found = 'search: loop {
    loop {
        if done() { break 'search i; }
        i += 1;
    }
};
```

**expr.loop.1** `loop block` runs its block again and again until a `break` leaves it. The block's
type is `()` (E0201). The loop's value is that of the `break` that leaves it; its type is the type
of its `break` values (`()` for `break` without one), which all agree (E0201), or `!` if nothing
leaves it.

**expr.while.1** `while cond block` evaluates `cond`, a `bool` (E0201), and if it is `true` runs
the block, which has type `()`, and starts again; when `cond` is `false` the loop ends. Its type is
`()`. The condition may contain `let` conditions as `if` can ([`if`](if.md)), so
`while let p = e { … }` runs while `e` matches `p`.

**expr.break.1** `break` leaves the innermost enclosing loop, and `continue` goes on to its next
iteration (for `while`, evaluating the condition again). Outside a loop both are errors (E0109).
Both have type `!`.

**expr.break.label.1** A loop may be labelled, `'name: loop { … }`; `break 'name` and
`continue 'name` then act on that loop, from anywhere inside it. A label no enclosing loop has is
an error (E0110).

**expr.break.value.1** `break e` leaves a `loop` with the value of `e`. A `while` loop has no value,
so `break e` out of one is an error (E0212).

# `match`

```rust
fn area(s: Shape) -> int {
    match s {
        Shape::Dot => 0,
        Shape::Circle(r) => 3 * r * r,
        Shape::Rect { w, h } => w * h,
    }
}
```

**expr.match.1** `match e { p₁ => e₁, … }` evaluates `e` once, then tries the arms in order. The
first arm whose pattern matches ([Patterns](../patterns.md)) and whose guard, if it has one
(`p if cond =>`), is `true` runs: its names are bound for the guard and the arm's expression, and
the `match` has that expression's value. Guards are `bool` (E0201) and run only for arms whose
pattern matched.

**expr.match.type.1** The arms have the same type, which is the type of the `match`; an arm of type
`!` fits any other (E0201). A `match` with no arms has type `!`.

**expr.match.exhaustive.1** The arms of a `match` cover every value of the matched type (E0208). An
arm with a guard counts as covering nothing, since its guard may be false.

**expr.match.unreachable.1** An arm whose pattern matches only values that earlier arms without
guards already match can never run; it is a warning (W0201).

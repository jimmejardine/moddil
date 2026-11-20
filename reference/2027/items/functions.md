# Functions

```rust
fn gcd(a: int, b: int) -> int {
    if b == 0 { a } else { gcd(b, a % b) }
}
```

**item.fn.decl.1** `fn name(p₁: T₁, …, pₙ: Tₙ) -> R block` declares a function with parameters of
types `T₁ … Tₙ` and result type `R`. Without `-> R`, the result type is `()`.

**item.fn.params.1** Each parameter binds a name (`x`), a mutable name (`mut x`) or nothing (`_`).
The names of one function's parameters are distinct (E0104). A parameter is a local of the body,
holding the argument's value.

**item.fn.params.2** A parameter may also be any pattern that matches every value of its type
([Patterns](../patterns.md)), such as `(a, b): (int, int)`; its names are locals of the body.

**item.fn.body.1** Calling the function runs its body ([Calls](../expressions/calls.md)). The
body's value is the result unless `return` ends it sooner; its type must fit the result type
(E0201).

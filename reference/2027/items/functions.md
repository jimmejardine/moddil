# Functions

```rust
fn gcd(a: int, b: int) -> int {
    if b == 0 { a } else { gcd(b, a % b) }
}
```

**item.fn.decl.1** `fn name(p₁: T₁, …, pₙ: Tₙ) -> R block` declares a function with parameters of
types `T₁ … Tₙ` and result type `R`. Without `-> R`, the result type is `()`. A function may have
type parameters, `fn name<T>(…)` ([Generics](../generics.md)); a function in an impl or trait may
take `self` ([Impls](impls.md)).

**item.fn.params.1** Each parameter binds a name (`x`), a mutable name (`mut x`) or nothing (`_`).
The names of one function's parameters are distinct (E0104). A parameter is a local of the body,
holding the argument's value.

**item.fn.params.2** A parameter may also be any pattern that matches every value of its type
([Patterns](../patterns.md)), such as `(a, b): (int, int)`; its names are locals of the body.

**item.fn.ret-borrow.1** A function may return a borrow (`-> &T`, `-> &mut T`) or a view. The
borrow must come from a borrowed parameter (or a view): with `from a, b` after the result type,
from one of those parameters, which must be borrowed (E0310); without it, from any borrowed
parameter. A function with no borrowed parameter may return only static data
(**borrow.static.1**). Returning a borrow of the function's own variables or temporaries is an
error (E0308). A caller treats the result as borrowing what it passed for those parameters, for as
long as the result is used ([Borrows](../borrows.md)).

**item.fn.body.1** Calling the function runs its body ([Calls](../expressions/calls.md)). The
body's value is the result unless `return` ends it sooner; its type must fit the result type
(E0201).

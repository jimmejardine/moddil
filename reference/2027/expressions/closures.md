# Closures and anonymous impls

```rust
fn each(f: impl FnMut(int), a: int, b: int) {
    f(a);
    f(b);
}

let mut total = 0;
each(|n| total += n, 4, 5);              // total is 9

let k = 10;
let scaled = |x: int| -> int { x * k };
scaled(3)                                 // 30

with Time = impl Time {                   // a handler written where it is installed
    fn now(&self) -> int { 100 }
} {
    run_once()
}
```

**expr.closure.1** `|p₁, …, pₙ| body` is a **closure**: a value that runs `body` when it is called,
with its parameters bound by the patterns `p₁, …, pₙ` (which cannot fail, E0209). A parameter may be
written with a type, `|x: int|`, and the closure with a result type, `|x| -> int { … }`. Each
closure has a type of its own, which cannot be written. In its body, `return` leaves the closure,
`?` raises an error from it ([Errors](../errors.md)), and no `break` or `continue` leaves it.

**expr.closure.param.1** A parameter of type `impl Fn(A₁, …, Aₙ) -> R` takes a closure: the
function is generic over the closure's type ([Generics](../generics.md)). `F: Fn(A₁, …, Aₙ) -> R`
bounds a type parameter the same way (`-> R` left out means `-> ()`), with `FnMut` and `FnOnce` in
place of `Fn`; only these three traits are written with `(…) -> …` (E0401). A closure passed for
such a parameter takes its parameters' and result's types from the bound. The closure traits are
the prelude's ([The prelude](../prelude.md)): every closure implements `FnOnce`; one that does not
move out what it captures, `FnMut` too; one that does not change it either, `Fn` too. A closure
passed where a kind it does not have is needed is an error (E0801).

**expr.closure.call.1** `f(a₁, …, aₙ)`, for `f` a closure or a value of a type parameter bounded by
a closure trait, calls it with the arguments (E0202 for their number, E0201 for their types): it
borrows `f` for `Fn`, borrows it with `&mut` for `FnMut`, and moves it for `FnOnce`. Any other
value called is an error (E0203). A closure parameter can be called with `&mut` without being
declared `mut`.

**expr.closure.capture.1** A closure **captures** the locals of the enclosing body that its body
names: by a shared borrow if it only reads one, by an `&mut` borrow if it changes it (or calls it
through `&mut`), and by moving it in if it moves it out, which makes the closure `FnOnce`. A closure
is second-class, as a borrow is ([Borrows](../borrows.md)): a value holding one is a view
([Views](../views.md)), a field of a struct or enum that is not a view cannot hold one (E0309), and
the borrows it holds last as long as it does.

**expr.closure.fails.1** A closure's errors are those its body raises. A function with a closure
parameter can fail with that closure's errors (written `F::Fails` when shown); a call of the
parameter that is not marked passes them on, as `?` would, so they are its callers' to handle: a
call that passes a closure that can fail is marked like any call that can fail (E0501),
`apply(|x| check(x)?, 1)?`.

**expr.closure.fails.2** A bound on a closure parameter's errors, `where F::Fails: Trait`,
says the function takes them: they do not pass on to its callers, and a call passing a closure
that can fail is not marked for them. The function deals with them itself (a router answering
each with a response), so a call of `f` in its body must handle them: letting them leave, with
`?` or unmarked, is an error (E0502). An error set meets the bound when each of its members
does, and a closure that cannot fail meets it too.

**expr.closure.effects.1** A closure's effects are met where it is written ([Effects](../effects.md)):
by the `with`s around it there, and the enclosing function's own effects, not by those of the
function that calls it.

**expr.fn-value.1** A function named as a value, not called (`apply(inc, 41)`, `let f = mul;`,
`Shout::shout`), is a closure that takes the function's arguments and calls it: it implements the
closure traits with the function's parameters and result, fails with its errors
(**expr.closure.fails.1**), and the needs of its effects are met where it is named
(**expr.closure.effects.1**). A generic function's type arguments are inferred where it is named,
from what the closure is used as. For now a type's function (`Vec::new`) and a variant
(`Some`) cannot be used as values.

**expr.anon-impl.1** `impl Trait { fn … }` as an expression is a value of a type of its own that
implements the trait, each method a closure over the enclosing body's locals (**expr.closure.capture.1**).
Its methods take `&self` (and cannot name `self` yet); it defines each of the trait's methods that
has no default (E0407), and none the trait does not declare (E0408); a method fails only with the
errors its trait's method lists (E0502). The trait must be one that can be used as `dyn`
([Types](../types.md)). It can be installed as a handler (`with Time = impl Time { … } { … }`) or
used as a value of its trait, as a `dyn` borrow.

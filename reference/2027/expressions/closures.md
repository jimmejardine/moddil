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

**expr.closure.effects.2** A closure bound may name the closure's effects after `with`:
`impl Fn(&T) -> U with E`, `F: FnMut(I::Item) -> B with E`, `F: Fn(int) with fail(Missing)`; a
bare effect has no `&T`, which is parenthesised. Named so, the closure's errors are what the bound
says, and a closure passed for it that can fail with others is an error (E0502); `with suspend` and
`with spawn` are allowed, and the bound with no `fail` takes a closure that cannot fail. An
**effect variable** is a type parameter named after `with`, standing for the errors of the closures
whose bounds name it: the function's `with E` passes them on, and a call of such a closure that is
not marked passes them on as **expr.closure.fails.1** says; a function whose errors are written that
names `E` in a bound but does not list it handles them itself (E0501, E0502). A closure's other
needs are met where it is written, so the bound of a closure parameter lists no handler's effect
(E0609).

**expr.closure.stored.1** A closure is kept beyond the call it is passed to as a
`Box<dyn Fn(A₁, …, Aₙ) -> R with …>` (or `FnMut`, `FnOnce`), a **stored closure**: a
`Box::new(|…| …)` written where one is expected takes its parameters' and result's types from it.
Its type names its effects, as a bound does (**expr.closure.effects.2**): the closure can fail only
with the errors it lists, or not at all if it lists none (E0502), and suspend or spawn only if it
lists `suspend` or `spawn` (E0603). It may run after the function it is written in has returned, so
it owns what it captures: each local it uses is moved in (copied, if it is `Copy`), and one that
holds a borrow is an error (E0802). The handlers' effects its type lists (`dyn Fn() with Clock`)
are met where it is called, by the caller's handlers, not where it is written; one it uses that
its type does not list is an error (E0802), as a handler it would borrow. `f(a₁, …, aₙ)`, for `f`
such a box, calls the closure through it: the call can fail with its errors, and needs what its
type lists; one of a `Box<dyn FnOnce…>` gives up the box, which is moved. For now only a closure
written as `Box::new(|…| …)` where the box is expected meets its needs at the call.

**expr.closure.result.1** A function whose result type is `impl Fn(A₁, …, Aₙ) -> R` (or `FnMut`,
`FnOnce`) returns a closure: one it gives as its value, or returns, takes its parameters' and
result's types from that type, and is a stored closure (**expr.closure.stored.1**), since it may
be called after the function has returned. So it owns what it captures (E0802), and its result
type names its effects, parenthesised after the return arrow:
`-> (impl Fn(int) -> int with fail(Missing))`, `-> (impl Fn() with Clock)`; with none, it cannot
fail (E0502). An effect variable of the function may name them (`with E`), so a closure it is
given and moves into the one it returns passes its errors on. A call of the result is a call of
the closure (**expr.closure.call.1**): it can fail with the errors its type lists, needs the
handlers it lists, met by the caller, and borrows, borrows with `&mut` or moves the value as the
type is `Fn`, `FnMut` or `FnOnce`.

**expr.fn-value.1** A function named as a value, not called (`apply(inc, 41)`, `let f = mul;`,
`Shout::shout`), is a closure that takes the function's arguments and calls it: it implements the
closure traits with the function's parameters and result, fails with its errors
(**expr.closure.fails.1**), and the needs of its effects are met where it is named
(**expr.closure.effects.1**). A generic function's type arguments are inferred where it is named,
from what the closure is used as. A type's function (`Point::at`, `Vec::new`) and a tuple variant
(`Some`) are values too, the variant a closure that builds it.

**expr.anon-impl.1** `impl Trait { fn … }` as an expression is a value of a type of its own that
implements the trait, each method a closure over the enclosing body's locals (**expr.closure.capture.1**).
Its methods take `&self` (and cannot name `self` yet); it defines each of the trait's methods that
has no default (E0407), and none the trait does not declare (E0408); a method fails only with the
errors its trait's method lists (E0502). The trait must be one that can be used as `dyn`
([Types](../types.md)). It can be installed as a handler (`with Time = impl Time { … } { … }`) or
used as a value of its trait, as a `dyn` borrow.

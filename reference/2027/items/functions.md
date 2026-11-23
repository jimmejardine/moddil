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
from one of those parameters, which must be borrowed, a view, or of a type parameter, whose value
may hold borrows (E0310); without it, from any borrowed parameter. A function with no borrowed
parameter may return only static data (**borrow.static.1**). Returning a borrow of the function's
own variables or temporaries is an error (E0308). A caller treats the result as borrowing what it
passed for those parameters, for as long as the result is used ([Borrows](../borrows.md)). A
result of a type parameter may hold a borrow too, for type arguments that do (`fn id<T>(x: T) -> T`
given `&x` keeps `x` borrowed), and may be written with `from`.

**main.root.1** `main`, the program's entry point (a free function of the root module), takes as
its parameters the **root capabilities** the program uses, each at most once, and nothing else
(E0608): structs marked `#[root_capability]`, which only a stub mod or one with the `os`
capability may declare (**mod.stub.1**, **os.cap.1**), and only the runtime makes. The standard
mods' are `Args` and `Env` (the command line and the environment), `Stdio`, `FsRoot`, `NetRoot`,
`Clock` and `Entropy`, in `moddil::root`, written over `core::os` (**os.1**).

**main.exit.1** `main` returns `()`, or an `ExitCode`, which the process exits with: `ExitCode::Ok`
(0) or one of the `sysexits.h` codes by name (`Usage` 64 to `Config` 78), or
`ExitCode::new(n)` for another number, which is the named code if `n` has one; `code()` gives the
number. For now `main` may also return another type, whose value is printed.

**item.fn.body.1** Calling the function runs its body ([Calls](../expressions/calls.md)). The
body's value is the result unless `return` ends it sooner; its type must fit the result type
(E0201).

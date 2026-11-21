# Effects

```rust
trait Time {
    fn now(&self) -> int;
}

struct Clock { t: int }
impl Time for Clock {
    fn now(&self) -> int { self.t }
}

fn deadline(after: int) -> int with time: Time {   // calls it, so names it
    time.now() + after
}

fn schedule() -> int { deadline(5) * 2 }           // needs `Time` too, inferred

pub fn plan() -> int with Time { schedule() }      // `pub`: written out

let clock = Clock { t: 10 };
with Time = clock {                                // installs a handler for the block
    plan()                                         // 30
}
```

An **effect** is something a function needs from whoever calls it, found by its type rather than
passed as an argument: a trait whose handler the caller installs, or **context data**, a value of a
struct or enum of the program.

**eff.1** An effect is listed after `with` in a function's signature, with its errors
([Errors](errors.md)): `with time: Time, Config, fail(E)`. It is a trait, or a struct or enum of the
program without type parameters (E0601). A trait is an effect if it can be used as `dyn`
([Types](types.md)), each of its methods, its supertraits' included, takes `&self`, and it has at
least one (E0601). An effect is listed once (E0606).

**eff.name.1** `with time: Time` names the effect for the body: `time` is a `&dyn Time`, a borrow
of the handler. Context data is read-only, so its name is a shared borrow: `with config: &Config`
(E0601). `with Time`, with no name, only says the function needs it. A name is a parameter's, and
is not declared twice (E0104).

**eff.with.1** `with E = h, … { block }` evaluates each handler `h` in order, then runs the block
with them installed, and has the block's value. A handler for a trait is a value of a type that
implements it, or a borrow of one, or a `dyn` borrow of a subtrait (E0402); for context data, a
value of its type or a borrow of one (E0201). A handler that is a value is dropped when the block
ends ([Ownership](ownership.md)). A `with` gives an effect one handler (E0606); a struct
expression as a handler is written in parentheses, as after `if`.

**eff.need.1** A call needs the effects of the function it calls, with the call's type arguments
(`Store<T>` needs `Store<User>` for `T = User`). Each need is met by the innermost `with` around
the call that installs exactly that effect, or else by the calling function's own: never by a
handler of a subtrait or a supertrait.

**eff.infer.1** A private or `pub(mod)` function's effects are inferred, across the modules of its
mod ([Modules](modules.md)): those it lists, and those its body needs and does not meet. So are its
errors: those it lists, and those that reach the end of its body. A `pub` function's are written
out, and a need it does not list is an error (E0603), as is an error
it does not list (E0502); so are a trait's method's and its impls' methods'. Effects that grow
without end (a generic function calling itself with ever larger type arguments) are an error
(E0607).

**eff.method.1** A trait's method, and a method of an impl of a trait, lists no effect but `fail`
(E0604), so it uses none it does not meet itself (E0603): a handler gets what else it needs when
it is built. A destructor, as a trait's method, uses no effects.

**eff.entry.1** An entry point (`main`, a test) has no effects: each need of its calls is met by a
`with` inside it (E0602).

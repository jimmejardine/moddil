# Calls and `return`

**expr.call.1** `f(a₁, …, aₙ)` calls the function `f` (E0203 if `f` is not a function). It takes
exactly as many arguments as `f` has parameters (E0202), each of its parameter's type (E0201). The
arguments are evaluated left to right, then `f`'s body runs with its parameters bound to them; the
call's value is `f`'s result.

**expr.return.1** `return e` ends the function, with `e` as its result; `e` must have the result
type (E0201). `return` alone means `return ()`. `return` has type `!`.

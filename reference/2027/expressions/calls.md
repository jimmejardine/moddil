# Calls and `return`

**expr.call.1** `f(a₁, …, aₙ)` calls the function `f` (E0203 if `f` is not a function). It passes
an argument for each of `f`'s parameters (E0202), each of its parameter's type (E0201), but for
those it names or leaves out (**expr.call.named.1**, **expr.call.default.1**). The arguments are
evaluated left to right, as written, then `f`'s body runs with its parameters bound to them; the
call's value is `f`'s result.

**expr.call.named.1** An argument may be passed by its parameter's name, `connect(host, port:
8443)`: positional arguments come first, for the first parameters in order, then named ones, in
any order. A name that is not a parameter's, a parameter given twice, or a positional argument
after a named one is an error (E0215). A method call names its arguments as a function call does;
a closure's call names none.

**expr.call.default.1** A parameter may have a default, `fn connect(host: &str, port: int =
443)`: a call that leaves it out passes the default. The default is compile-time code
([Compile-time evaluation](../comptime.md)), naming no parameter, and its type names no type
parameter, for now. A trait's method's defaults are its impls' (`syntax.md`'s rule): an impl's
method writes none of its own (E0216).

**expr.return.1** `return e` ends the function, with `e` as its result; `e` must have the result
type (E0201). `return` alone means `return ()`. `return` has type `!`.

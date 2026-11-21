# Errors

```rust
struct TooBig { n: int }

fn check(n: int) -> int with fail(TooBig) {
    if n > 9 { fail(TooBig { n }); }
    n
}

fn doubled(n: int) -> int with fail(TooBig) {
    check(n)? * 2                     // the error goes on to the caller
}

let r = try { doubled(12)? };        // Err(TooBig { n: 12 })
let n = try { doubled(12)? } catch { TooBig(t) => t.n };   // 12
```

An **error** is a value that a function fails with instead of returning. It is not a trap
([Traps](traps.md)): the program handles it, and every error is handled somewhere, so none can
crash the program.

**err.fail.1** `fail(e)` fails with the error `e`, and has type `!`. An error is a value of a
struct or enum (E0508), or of an error set (**err.set.1**), which `fail(e)` fails with again.

**err.set.1** A function lists the errors it can fail with after its result type:
`-> T with fail(A | B)`, each a struct or enum, at most one instance of a generic one (E0508). A
private function's errors are inferred: those it lists, and those that reach the end of its body
([Effects](effects.md)). A function whose errors are written out (a `pub` function, `main`, a
trait's method or an impl's) fails only with those it lists: an error that reaches the end of it
must be one it lists (E0502), and without `with fail(…)` it cannot fail (E0501). An impl's method
fails with no error its trait's does not list ([Impls](items/impls.md)). Errors of several types together form an **error set**, written
`A | B` when shown; it is a type only as the error of a `Result` and of `fail(…)`.

**err.question.1** A call of a function that can fail is marked: with `?`, `?!` or inside `try`
(E0501). `x?` is the value of the call if it returns; if it fails, the error goes on, to the
innermost enclosing `try` block or out of the function. On a `Result`, `x?` is the `Ok` value, or
fails with the `Err` value; on an `Option`, it is the `Some` value, or else the function returns
`None` (it must return an `Option`) or the enclosing `try` block is `None` (E0506). `?` on
anything else is an error (E0504). A string after `?`, `x? "loading the config"`, is a note for the
error's trace, and does not change it.

**err.bang.1** `x ?! "reason"` is as `x?`, except that an error (or `None`) does not go on: it
traps, with the reason ([Traps](traps.md)). It says the error cannot happen there; the reason is
required.

**err.try.1** `try block` is a `Result` (`prelude.result.1`): `Ok` of the block's value, or `Err`
of the first error its `?`s raise, which leaves the block. Its error is the error set of everything
its `?`s and `fail`s can raise. A `try` block whose `?`s are on `Option`s is an `Option` instead:
`Some` of the value, or `None` (E0506 if it mixes both). `return`, `break` and `continue` leave it
as they leave any block.

**err.catch.1** `try block catch { arms }` handles the block's errors: it is the block's value, or
the value of the first arm that matches its error. An arm `Type(p)` matches an error of that type
that `p` matches; any other pattern is matched against the error as usual, so `Parse::Empty`
matches that variant and `e` any error. An arm for a type the block cannot fail with is an error
(E0505), as is a `catch` on a block that cannot fail (E0505). An error no arm matches goes on, as
from `?`: the errors left are those of the types the arms do not cover. The arms have the block's
type (E0201).

**err.unused.1** A `Result` is not dropped unused: an expression statement, or `let _ =`, of type
`Result` is an error (E0503).

**err.discard.1** `discard(r, "reason")` drops the `Result` `r` on purpose, with the reason why.

**err.entry.1** An error cannot leave an **entry point**: `main` cannot declare errors (E0507),
nor can a `defer` block or a destructor fail (E0501). A test can fail.

**err.test.1** An error that leaves a test fails it, as a trap does, and is reported where it left
the test body.

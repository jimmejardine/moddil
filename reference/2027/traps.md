# Traps

**trap.1** A trap stops the program: nothing after the failing operation runs, and the program
cannot catch or resume it, but in an isolated scope, which a trap fails instead
([Concurrency](concurrency.md)). A test that traps fails. Which operations trap is stated with each
operation; the trap reports where it happened. A failed [`assert`](expressions/assert.md) is a
trap too, and reports the value of each part of its condition that was evaluated and still
holds one (not one moved away, as into a call that takes it); so is an error that `?!` said
cannot happen, which reports its reason ([Errors](errors.md)), and a reached `unreachable` or
`todo`, which report theirs ([`unreachable` and `todo`](expressions/unreachable.md)). A trap is not an error: an error is
a value the program handles. Besides those, indexing out of bounds
([Arrays and indexing](expressions/arrays.md)), filling a `Buf` past its capacity
([Collections](collections.md)), a broken precondition or postcondition
([Contracts](contracts.md)), a deadlock, every task waiting ([Concurrency](concurrency.md)), and
what waits for the runtimes, a `select` on a `#[selectable]` function (**task.select.3**), trap.

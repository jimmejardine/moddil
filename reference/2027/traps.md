# Traps

**trap.1** A trap stops the program: nothing after the failing operation runs, and the program
cannot catch or resume it. A test that traps fails. Which operations trap is stated with each
operation; the trap reports where it happened. A failed [`assert`](expressions/assert.md) is a
trap too, and reports the value of each part of its condition; so is an error that `?!` said
cannot happen, which reports its reason ([Errors](errors.md)). A trap is not an error: an error is
a value the program handles. Besides those, indexing out of bounds
([Arrays and indexing](expressions/arrays.md)) and filling a `Buf` past its capacity
([Collections](collections.md)) trap.

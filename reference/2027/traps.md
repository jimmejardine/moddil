# Traps

**trap.1** A trap stops the program: nothing after the failing operation runs, and the program
cannot catch or resume it. A test that traps fails. Which operations trap is stated with each
operation; the trap reports where it happened. A failed [`assert`](expressions/assert.md) is a
trap too, and reports the value of each part of its condition.

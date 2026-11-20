# Types

**type.int.1** `int` is a signed 64-bit two's-complement integer, from −2⁶³ to 2⁶³ − 1, on every
target.

**type.bool.1** `bool` has two values, `true` and `false`.

**type.unit.1** `()` has one value, also written `()`. It is the type of blocks without a final
expression, of assignments, and the result type of functions that do not declare one.

**type.never.1** `!` is the type of expressions that never produce a value, such as `return`. An
expression of type `!` fits where any type is expected.

**type.match.1** Where an expression of one type is expected, an expression of another type is an
error (E0201); `!` fits everywhere. No conversions are implicit.

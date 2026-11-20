# Arithmetic

**expr.arith.ops.1** `+`, `-` and `*` are addition, subtraction and multiplication of `int`s. `/`
divides, rounding towards zero; `%` is the remainder, with the sign of the left operand, so that
`a == a / b * b + a % b`.

**expr.arith.neg.1** Unary `-` negates an `int`.

**expr.arith.overflow.1** If the exact result of `+`, `-`, `*`, `/` or unary `-` is not an `int`
(as for `int::MIN / -1`), the program traps. There is no build, mode or region in which it wraps
instead ([Traps](../traps.md)).

**expr.arith.div-zero.1** Division or remainder by zero traps. `int::MIN % -1` is `0`, as its exact
result is.

**expr.arith.types.1** Both operands of an arithmetic operator are `int`s; for other operand types
the operator is not defined (E0205), and a right operand of another type is a mismatch (E0201).

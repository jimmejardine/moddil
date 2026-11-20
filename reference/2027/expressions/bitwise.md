# Bitwise operators

**expr.bit.ops.1** On `int`s, `&`, `|` and `^` are bitwise and, or and exclusive or of the
two's-complement representations, and unary `!` is bitwise not. On `bool`s, `&`, `|` and `^` are
logical and, or and exclusive or, evaluating both operands, and `!` is logical not.

**expr.bit.shift.1** `a << n` and `a >> n` shift the `int` `a` by `n` bits; `>>` is arithmetic (it
keeps the sign). If `n` is negative or at least 64, or `<<` loses a bit, so the exact result
`a × 2ⁿ` is not an `int`, the program traps.

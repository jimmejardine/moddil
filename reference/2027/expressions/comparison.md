# Comparisons

**expr.cmp.eq.1** `==` and `!=` compare two values of the same type (E0201) for equality, and have
type `bool`.

**expr.cmp.order.1** `<`, `<=`, `>` and `>=` compare `int`s, and have type `bool`. They are not
defined for other types (E0205).

**expr.cmp.chain.1** A chain `a < b <= c` means `a < b && b <= c`, with `b` evaluated once.
Operands are evaluated left to right, and evaluation stops at the first comparison that is false.

**expr.cmp.chain.2** A chain runs in one direction: every operator is `<` or `<=`, or every one is
`>` or `>=`. `==` and `!=` do not chain (E0002).

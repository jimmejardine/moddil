# Logic

**expr.logic.1** `a && b` and `a || b` take `bool` operands (E0201) and have type `bool`. `b` is
evaluated only if `a` does not decide the result: if `a` is `true` for `&&`, `false` for `||`.

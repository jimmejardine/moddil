# Literals

**expr.lit.int.1** An integer literal is written in decimal (`255`), hexadecimal (`0xff`), octal
(`0o377`) or binary (`0b1111_1111`); underscores between digits are ignored. Its value is exact.

**expr.lit.int.2** An integer literal has type `int`, and its value must be in range (E0204). A `-`
directly before a literal is part of its value, so `-9223372036854775808` is in range.

**expr.lit.bool.1** `true` and `false` are the values of `bool`.

**expr.lit.unit.1** `()` is the value of `()`.

# Assignment

**expr.assign.1** `x = e` evaluates `e` and stores its value in the local `x`, whose type `e` must
have (E0201). The assignment has type `()`. Only a local, or a field of one ([Fields](fields.md)), can be
assigned to (E0206), and only if the local is declared `mut` (E0301).

**expr.assign.compound.1** `x op= e`, for `op` one of `+ - * / % & | ^ << >>`, is `x = x op e`, with
the same types and traps.

**expr.assign.compound.2** For a type with an operator trait (`Add`, `Sub`, `Mul`, `Div`,
`Rem`), `x += e` calls it: `x = Add::add(x, e)`, the result stored back in `x`. Its `Output`
must be `x`'s type (E0201).

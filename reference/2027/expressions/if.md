# `if`

**expr.if.cond.1** The condition of an `if` is a `bool` (E0201). If it is `true`, the first block
runs, otherwise the `else` branch, if any. `else if` is an `if` in the `else` branch.

**expr.if.value.1** With an `else` branch, the `if` has the value of the branch that runs. Both
branches have the same type (E0201), except that a branch of type `!` fits the other.

**expr.if.let.1** A condition may be, or contain joined by `&&`, a `let` condition `let p = e`,
which is `true` if `e` matches `p` ([Patterns](../patterns.md)). Its names are in scope in the
rest of the condition and in the first block, not in the `else` branch:
`if let Shape::Circle(r) = s && r > 1 { r } else { 0 }`.

**expr.if.no-else.1** Without `else`, the `if` has type `()`, and so must its block (E0201).

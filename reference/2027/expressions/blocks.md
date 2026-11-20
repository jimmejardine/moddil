# Blocks

**expr.block.1** A block `{ s₁ … sₙ e }` runs its statements in order, then evaluates `e`; its value
is `e`'s. Without a final expression its value is `()`, and its type is `()`, or `!` if a statement
in it never finishes (such as `return`).

**expr.block.scope.1** A block is a scope: the bindings made in it are not visible after it
([Statements](../statements.md)).

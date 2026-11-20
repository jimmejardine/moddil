# Statements

```rust
let limit: int = 10;
let mut total = 0;
total += limit;
```

**stmt.let.1** `let p = e;` evaluates `e` and binds its value to `p`: a name, `mut name`, or `_`,
which binds nothing. With a type, `let p: T = e;`, the value must have type `T` (E0201); without
one, the binding has `e`'s type.

**stmt.let.scope.1** A binding is in scope from the statement after its `let` to the end of the
enclosing block, not in its own initialiser. A later `let` of the same name shadows it.

**stmt.expr.1** An expression followed by `;` is evaluated and its value discarded. A block-like
expression (such as `if`) may stand as a statement without `;`; then its type must be `()` (E0201).

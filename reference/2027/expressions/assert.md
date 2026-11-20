# `assert`

```rust
test "fees" {
    let amount = 250;
    assert(fee(amount) + amount == 275);
}
```

**expr.assert.1** `assert(cond)` evaluates `cond`, a `bool` (E0201). If it is `false`, the program
traps ([Traps](../traps.md)); otherwise the `assert` has the value `()`. It takes exactly one
argument (E0202), and runs in every build.

**expr.assert.name.1** `assert` is a built-in in the prelude: a local or an item named `assert`
comes first, and shadows it ([Names](../names.md)). It is called, never used as a value (E0112).

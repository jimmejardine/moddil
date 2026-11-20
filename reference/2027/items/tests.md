# Tests

```rust
test "gcd of coprime numbers" {
    gcd(9, 4)        // => 1
}
```

**item.test.decl.1** `test "name" block` declares a test. `moddil test` runs a module's tests; each
runs its block and passes unless it traps.

**item.test.value.1** A test's block, and the statements in it, may have values of any type; they
are not returned anywhere. (Conformance tests and documentation examples state such values with
`// =>`.)

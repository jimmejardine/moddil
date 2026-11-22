# Tests

```rust
test "gcd of coprime numbers" {
    gcd(9, 4)        // => 1
}
```

**item.test.decl.1** `test "name" block` declares a test. `moddil test` runs a module's tests; each
runs its block and passes unless it traps.

**test.for.1** `test "name" for subject block` says what the test is for: `subject` names a
function, a function of a type (`Account::withdraw`) or a trait's method (`Shout::shout`), which
must exist (E0101). It runs as any test does. For now a test cannot be run for listed types
(`with T in […]`) or declare capabilities.

**item.test.value.1** A test's block, and the statements in it, may have values of any type; they
are not returned anywhere. (Conformance tests and documentation examples state such values with
`// =>`.)

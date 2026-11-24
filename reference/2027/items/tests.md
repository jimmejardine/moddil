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

**test.doc.1** An example in a doc comment is a test. In the `///` comment of an item, the lines
after a `# Example` or `# Examples` heading, up to the next heading, are Moddil without their
`///` (and without any ` ``` ` fence lines): `moddil test` runs them as a test named `example of`
the item, in the item's module, so it can use the module's private items. Each line that ends
with `// =>` is a statement of its own, its value checked against what the comment says, and a
problem is reported at its line in the comment.

```rust
/// Doubles `x`.
///
/// # Example
/// twice(2)                        // => 4
/// twice(-3)                       // => -6
fn twice(x: int) -> int { x * 2 }
```

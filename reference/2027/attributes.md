# Attributes

```rust
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Version { major: int, minor: int }
```

**attr.derive.1** `#[derive(T₁, …, Tₙ)]` before a struct or enum implements the traits named, from
its fields. The traits that can be derived are `Copy`, `Clone`, `PartialEq`, `Eq`, `PartialOrd` and
`Ord` ([Prelude](prelude.md)). The impls are **structural**:

- `clone` clones each field (or, for a `Copy` type with no type parameters, copies the value);
- `eq` holds when both values are the same variant and their fields are equal, field by field;
- `partial_cmp` and `cmp` order values by variant, in the order the variants are declared, and
  then by their fields, in declaration order, as the first field that differs does.

Every field must implement the trait derived (E0402), except one whose type is a type parameter:
the derived impl of a generic type bounds each type parameter by the trait
(`impl<T: Clone> Clone for Wrap<T>`). A derive also needs the trait's supertraits (E0402), so
`Eq` comes with `PartialEq` and `Ord` with `PartialOrd` and `Eq`. `Copy` is described in
[Ownership](ownership.md).

# Attributes

```rust
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Version { major: int, minor: int }
```

**attr.derive.1** `#[derive(T₁, …, Tₙ)]` before a struct or enum implements the traits named, from
its fields. The traits that can be derived are `Copy`, `Clone`, `PartialEq`, `Eq`, `PartialOrd`, `Ord`,
`Hash`, `Debug` and `Default` ([Prelude](prelude.md)). The impls are **structural**:

- `clone` clones each field (or, for a `Copy` type with no type parameters, copies the value);
- `eq` holds when both values are the same variant and their fields are equal, field by field;
- `partial_cmp` and `cmp` order values by variant, in the order the variants are declared, and
  then by their fields, in declaration order, as the first field that differs does;
- `hash` hashes an enum's variant's index, then each field in order (**coll.hash.derive.1**);
- `fmt` (of `Debug`) writes the value as Moddil source writes it (**attr.derive.debug.1**), and
  `default` makes one of its fields' defaults (**attr.derive.default.1**).

**attr.derive.debug.1** A derived `Debug` writes `Point { x: 1, y: 2 }` for a struct with named
fields, `Pair(3, true)` for one with positional fields, and `Mode::On(4)` or `Mode::Off` for an
enum's variant, each field by its own `Debug`.

**attr.derive.default.1** A derived `Default` makes a struct of each field's
`Default::default()`. An enum derives it from the one unit variant marked `#[default]`, which it
must have (E0416).

Every field must implement the trait derived (E0402), except one whose type is a type parameter:
the derived impl of a generic type bounds each type parameter by the trait
(`impl<T: Clone> Clone for Wrap<T>`). A derive also needs the trait's supertraits (E0402), so
`Eq` comes with `PartialEq` and `Ord` with `PartialOrd` and `Eq`. `Copy` is described in
[Ownership](ownership.md).

**attr.track-caller.1** `#[track_caller]` before a function makes a trap in it be reported where it
was called ([Traps](traps.md)), and through a tracked caller, where that one was called, up to the
first function that is not tracked. The prelude's functions that can trap are tracked, and so is
derived code. The attribute takes no arguments; other attributes on functions are not supported
yet (I0001).

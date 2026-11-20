# Structs

```rust
struct Point { x: int, y: int }
struct Pair(int, bool);
struct Marker;
```

**item.struct.decl.1** `struct Name { f₁: T₁, …, fₙ: Tₙ }` declares a struct with **named fields**;
`struct Name(T₁, …, Tₙ);` one with **positional fields**, named `0` to `n − 1`; and
`struct Name;` a **unit struct**, with no fields and one value. A struct is a type
([Types](../types.md)), and its value holds one value of each field's type.

**item.struct.fields.1** The fields of a struct have distinct names (E0103).

**item.struct.recursive.1** A struct or enum may not contain itself, directly or through other
structs, enums and tuples, since its values would be infinitely large (E0211).

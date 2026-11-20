# Enums

```rust
enum Shape {
    Dot,
    Circle(int),
    Rect { w: int, h: int },
}
```

**item.enum.decl.1** `enum Name { V₁, …, Vₙ }` declares an enum: a type whose value is exactly one
of its **variants**, together with that variant's fields. Each variant has named fields, positional
fields or none, written as for a struct ([Structs](structs.md)). An enum with no variants has no
values.

**item.enum.variants.1** The variants of an enum have distinct names (E0103), and so do the fields
of each variant. A variant is named through its enum: `Shape::Circle`.

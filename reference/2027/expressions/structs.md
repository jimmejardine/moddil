# Tuples, structs and variants

```rust
let t = (1, true);
let p = Point { x: 1, y: 2 };
let q = Point { y: 5, x };          // `x` is `x: x`
let pair = Pair(1, false);
let s = Shape::Circle(3);
let m = Marker;
```

**expr.tuple.1** `(e₁, …, eₙ)` with `n ≥ 2`, or `(e,)` with one element, is a tuple of type
`(T₁, …, Tₙ)` ([Types](../types.md)). `()` is the unit value and `(e)` is `e`
([Evaluation](evaluation.md)).

**expr.struct.1** `Path { f₁: e₁, …, fₙ: eₙ }` builds a struct with named fields, or an enum
variant with named fields. It gives every field exactly once: a missing field is an error (E0207),
so is a field the struct does not have (E0107) and a field given twice (E0108). Each value must
have its field's type (E0201). `f` alone is short for `f: f`.

**expr.struct.order.1** The field values are evaluated in the order they are written, not the
order the fields are declared in.

**expr.struct.tuple.1** For a struct or variant with positional fields, `Path(e₁, …, eₙ)` builds
it, with one value per field (E0202), each of its field's type (E0201). For one with no fields,
`Path` alone is its value.

**expr.struct.form.1** A struct or variant is built only in the form it is declared with: `Point`
alone, `Pair { … }` or `Marker(…)` are errors (E0111). An enum is not a value; one of its variants
is (E0111).

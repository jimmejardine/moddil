# Patterns

```rust
match shape {
    Shape::Dot => 0,
    Shape::Circle(r @ 1..=9) if r > 2 => r,
    Shape::Circle(_) | Shape::Rect { .. } => -1,
}
```

A pattern is matched against a value. It either **matches**, binding the names in it to parts of
the value, or does not.

**pat.1** `_` matches any value and binds nothing. A literal (`3`, `-1`, `true`) matches the value
equal to it. `()` matches `()`.

**pat.bind.1** A name `x` matches any value and binds `x` to it; `mut x` binds a mutable local.
`x @ p` matches what `p` matches and binds `x` to the whole value. A name that is a unit struct
(`Marker`) is that struct's pattern, not a binding.

**pat.bind.unique.1** A pattern binds each name at most once (E0106).

**pat.range.1** `a..=b` matches the integers from `a` to `b`, `a..b` those from `a` up to but not
including `b`, and `a..` those from `a` up. The bounds are integer literals. A range that matches
nothing, such as `5..=1` or `3..3`, is an error (E0214).

**pat.tuple.1** `(p₁, …, pₙ)` matches a tuple of `n` elements whose elements match `p₁ … pₙ`, and
`(p,)` a tuple of one. `..` among them stands for any number of elements, at most once:
`(first, ..)`, `(.., last)`. The number of patterns must fit the tuple's type (E0213).

**pat.struct.1** `Path { f₁: p₁, …, fₙ: pₙ }` matches a struct, or a value of that variant of an
enum, whose fields `fᵢ` match `pᵢ`; `f` alone is `f: f`. It names every field unless it ends with
`..` (E0207), and only fields the struct has (E0107), each once (E0108).

**pat.struct.tuple.1** `Path(p₁, …, pₙ)` matches a struct or variant with positional fields, field
by field, with `..` as in tuple patterns (E0213). `Path` alone matches a struct or variant with no
fields. A pattern must use the form the struct or variant is declared with (E0111).

**pat.or.1** `p₁ | … | pₙ` matches what any `pᵢ` matches, trying them in order, and binds what the
first that matches binds. Every alternative binds the same names (E0105).

**pat.type.1** A pattern is checked against the type of the value it matches: a literal, range,
tuple, struct or variant pattern for another type is an error (E0201). Each name a pattern binds
gets the type of the part it binds.

**pat.irrefutable.1** The pattern of a `let` without `else`, and of a parameter, must match every
value of its type (E0209): `let (a, b) = pair;` is allowed, `let Shape::Circle(r) = s;` is not.

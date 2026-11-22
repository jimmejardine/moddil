# Impls

```rust
impl Point {
    fn new(x: int, y: int) -> Point { Point { x, y } }
    fn norm(&self) -> int { self.x * self.x + self.y * self.y }
}

impl Shape for Square {
    fn area(&self) -> int { self.side * self.side }
}
```

**item.impl.1** `impl Type { … }` adds functions to a struct or enum declared in the same module
(E0414 for other types): **methods**, whose first parameter is `self`, `&self` or `&mut self`
(taking the value, a shared borrow or an exclusive borrow of it), and functions without `self`. In
the impl, `Self` is `Type`. A type's inherent functions have distinct names (E0103), but for
**item.impl.same-name.1**.

**item.impl.same-name.1** A type's inherent impls may define one function without `self` and one
method of the same name (two of either kind are E0103): `Type::name(…)`, and `Type::name` as a
value, name the function; `e.name(…)` the method ([Methods](../expressions/methods.md)).

**item.impl.trait.1** `impl Trait for Type { … }` implements a trait for a type: it defines each of
the trait's methods without a default, may define the others, and defines each associated type
(`type Item = int;`) (E0407). It defines nothing the trait does not declare (E0408). Each method
has the trait's signature, with `Self` replaced by `Type`: the same way of taking `self`, and the
same parameter and result types (E0409).

**item.impl.coherence.1** A program has at most one impl of a trait for a type (E0406): two impls
of a trait may not apply to the same type, for any type arguments. This includes the prelude's
impls ([Prelude](../prelude.md)). Each type parameter of an impl is used by the type or by the
trait's type arguments (E0415), so that the impl's type arguments are known wherever it applies.

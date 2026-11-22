# Traits

```rust
trait Shape {
    fn area(&self) -> int;
    fn twice(&self) -> int { self.area() * 2 }
}

trait Container {
    type Item;
    fn get(&self) -> Self::Item;
}
```

**item.trait.1** `trait Name { … }` declares a trait: a set of methods (and functions without
`self`) and associated types that a type can implement ([Impls](impls.md)). Within the trait,
`Self` is the type implementing it. `type Name;` declares an associated type, which each impl
defines; the trait's signatures name it as `Self::Name`. A method or associated type is declared
once in a trait (E0103). Traits share one namespace with functions, structs and enums
([Names](../names.md)); a name used as a trait that is not one is an error (E0401).

**item.trait.default.1** A method with a body is a **default**: an impl that does not define the
method gets this body, run with `Self` as the impl's type. A method without a body must be defined
by every impl.

**item.trait.assoc.default.1** `type Name = T;` in a trait gives an associated type a default: an
impl that does not define it takes `T` (`Iterator`'s `type Fails = !;`). An impl's definition of
an associated type sees the impl's bounds (`type Fails = I::Fails | F::Fails;`), and may be an
error set, its members separated by `|` (E0508 for a member that is not an error type).

**item.trait.super.1** `trait Name: A + B` declares **supertraits**: a type implements `Name`
only if it also implements `A` and `B` (E0402, at the impl), so code bounded by `T: Name` may also
use `A`'s and `B`'s methods.

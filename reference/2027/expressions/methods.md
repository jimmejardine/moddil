# Methods and associated functions

```rust
let mut c = Counter::new(1);     // an associated function
c.bump();                        // a method: `Counter::bump(&mut c)`
Shape::area(&square)             // a trait's method, called through the trait
```

**expr.method.1** `e.name(a₁, …, aₙ)` calls the method `name` of `e`'s type, with `e` as its
`self`. The method is looked for with the receiver `e` itself, then a shared borrow of it (`&e`),
then an exclusive borrow (`&mut e`); if `e` is a borrow and none is found, the same follows for
what it points to (`*e`). At each step, a method of an inherent impl ([Impls](../items/impls.md))
comes before a trait's method. A type with no such method is an error (E0403). Borrowing `e` with
`&mut` needs a place that can be changed (E0301, E0302) ([Borrows](../borrows.md)). Then the call
is checked as a call of the method ([Calls](calls.md)).

**expr.method.ambiguous.1** If the methods of more than one trait are found at the same step, the
call is ambiguous (E0410), and is written through a trait instead: `A::hello(&c)`.

**expr.path.assoc.1** `Type::name` names a function of `Type`: one of its inherent impl, or else a
method or function of a trait it implements, for that type. `Trait::name` names a trait's method
or function, for the type the call's arguments and result show. `Self::name` and `T::name`, for a
type parameter `T`, name a function of a trait in scope for that type. A type without the function
is an error (E0403).

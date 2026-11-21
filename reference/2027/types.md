# Types

**type.int.1** `int` is a signed 64-bit two's-complement integer, from −2⁶³ to 2⁶³ − 1, on every
target.

**type.bool.1** `bool` has two values, `true` and `false`.

**type.unit.1** `()` has one value, also written `()`. It is the type of blocks without a final
expression, of assignments, and the result type of functions that do not declare one.

**type.never.1** `!` is the type of expressions that never produce a value, such as `return`. An
expression of type `!` fits where any type is expected.

**type.tuple.1** A tuple type `(T₁, …, Tₙ)` with `n ≥ 2`, or `(T,)` with one element, holds one
value of each element type, in order. `()` is the tuple with no elements, and `(T)` is `T`.

**type.adt.1** Each struct and enum declares a new type ([Structs](items/structs.md),
[Enums](items/enums.md)); a generic one, a type for each list of type arguments
([Generics](generics.md)): `Pair<int, bool>`. Types are **nominal**: two structs with the same
fields are different types.

**type.match.1** Where an expression of one type is expected, an expression of another type is an
error (E0201); `!` fits everywhere. No conversions are implicit but three of borrows: `&mut T` to
`&T` ([Borrows](borrows.md)), a borrow to a `dyn` borrow (**type.dyn.coerce.1**), and `&String`
to `&str` ([Text](text.md)).

**type.dyn.1** `dyn Trait` is the type of a value of some type that implements `Trait`, known only
through it. It has no size of its own, so it is written behind a borrow, `&dyn Trait` or
`&mut dyn Trait`, and names one trait (E0605). The trait must be able to be used as `dyn`: it has
no associated types, and each of its methods, its supertraits' included, takes `&self` or
`&mut self`, has no type parameters of its own, and names `Self` only as `self` (E0605). A method of
the trait, or of a supertrait, called on a `dyn` borrow runs the method of the type of the value it
points to.

**type.dyn.coerce.1** A `&T` can be used where a `&dyn Trait` is expected if `T` implements `Trait`
(E0402), and so can a `&dyn Sub` of a subtrait `Sub` of `Trait`; likewise for `&mut`, which can also
be used where the shared form is expected.

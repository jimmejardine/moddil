# Types

**type.int.1** `int` is a signed 64-bit two's-complement integer, from −2⁶³ to 2⁶³ − 1, on every
target; `i64` names it too. The other integer types and the floats are in [Numbers](numbers.md).

**type.bool.1** `bool` has two values, `true` and `false`.

**type.unit.1** `()` has one value, also written `()`. It is the type of blocks without a final
expression, of assignments, and the result type of functions that do not declare one.

**type.never.1** `!` is the type of expressions that never produce a value, such as `return`. An
expression of type `!` fits where any type is expected.

**type.tuple.1** A tuple type `(T₁, …, Tₙ)` with `n ≥ 2`, or `(T,)` with one element, holds one
value of each element type, in order. `()` is the tuple with no elements, and `(T)` is `T`.

**type.tuple.traits.1** A tuple is `Copy`, and `Clone` by copying, if every element is `Copy`, at
any size. A tuple of 2 to 12 elements is `Clone`, `Debug` and `Hash` if every element is, through
the prelude's impls: each element in order. So `(String, int)` clones, and `.clone()` on a
`&(String, int)` gives a `(String, int)`.

**type.array.1** An array type `[T; N]` holds `N` values of type `T`, in order; `N` is a length
known when compiling, an integer literal or a const parameter ([Generics](generics.md)). An array
is `Copy` if `T` is and it is within the size cap ([Ownership](ownership.md)), and `Clone` then too.

**type.slice.1** A slice type `[T]` is a run of values of type `T` whose length is known only when
the program runs. It has no size of its own, so it is named only behind a borrow (`&[T]`,
`&mut [T]`) or as the type an impl is for (E0901). `xs.len()` is its length.

**type.slice.coerce.1** A `&[T; N]` (or `&mut [T; N]`) is converted to a `&[T]` (or `&mut [T]`) of
its elements where a slice borrow is expected.

**type.adt.1** Each struct and enum declares a new type ([Structs](items/structs.md),
[Enums](items/enums.md)); a generic one, a type for each list of type arguments
([Generics](generics.md)): `Pair<int, bool>`. Types are **nominal**: two structs with the same
fields are different types.

**type.match.1** Where an expression of one type is expected, an expression of another type is an
error (E0201); `!` fits everywhere, and a refined value fits where its base is expected
([Refinements](refinements.md)). No other conversions are implicit but five of borrows: `&mut T` to
`&T` ([Borrows](borrows.md)), a borrow to a `dyn` borrow (**type.dyn.coerce.1**), `&String` to
`&str` ([Text](text.md)), an array borrow to a slice borrow (**type.slice.coerce.1**), and a
borrow to a borrow of what it dereferences to (**type.deref.coerce.1**).

**type.deref.coerce.1** Where a `&B` is expected, a `&A` or a `&mut A` is converted to a `&B` if
`A` implements the prelude's `Deref` with `Output = B`, as `a.deref()` gives it: a `&Box<T>` or a
`&Rc<T>` is a `&T` there. Where a `&mut B` is expected, a `&mut A` is converted through `DerefMut`,
as `a.deref_mut()`, if `A` implements it. It looks through one `Deref`, as a field and a method
do ([Fields](expressions/fields.md), [Methods](expressions/methods.md)): a `&Rc<Box<T>>` is not a
`&T` (E0201). It does not apply where `A` is already what is expected, where `B` is a `dyn` type,
or where the type expected is still being inferred: given `fn f<T>(x: &T)`, `f(&b)` with `b` a
`Box<int>` makes `T` a `Box<int>`.

**type.dyn.1** `dyn Trait` is the type of a value of some type that implements `Trait`, known only
through it. It has no size of its own, so it is written behind a borrow, `&dyn Trait` or
`&mut dyn Trait`, and names one trait (E0605). The trait must be able to be used as `dyn`: it has
no associated types, and each of its methods, its supertraits' included, takes `&self` or
`&mut self`, has no type parameters of its own, and names `Self` only as `self` (E0605). A method of
the trait, or of a supertrait, called on a `dyn` borrow runs the method of the type of the value it
points to.

**type.box.1** `Box<T>`, the prelude's, owns one value of type `T` kept apart from it, on the heap:
`Box::new(x)` makes one, `b.into_inner()` takes the value out again, and the value is dropped
with the box. `*b` is the value, as a place (it reads it, changes it where `b` is `mut`, and
borrows it with `&*b` and `&mut *b`), through the prelude's `Deref` and `DerefMut`: `*x` on a type
other than a borrow is `*x.deref()`, or `*x.deref_mut()` where it is changed. A value cannot be
moved out through `*`, as out of a borrow ([Ownership](ownership.md)). A struct or enum may contain
itself through a `Box` (`enum List { Cons(int, Box<List>), Nil }`), which is not an infinite size
(E0211).

**type.box.dyn.1** `Box<dyn Trait>` owns a value of some type that implements `Trait`, known only
through it: a `Box<T>` can be used where a `Box<dyn Trait>` is expected if `T` implements `Trait`
(E0402), as a borrow is converted to a `dyn` borrow (**type.dyn.coerce.1**). `*b` is the value as a
`dyn Trait` place: its methods run those of its own type, and dropping the box drops the value as
its own type. `dyn` is written without a borrow only as a `Box`'s argument (E0605).

**type.impl.ret.1** `-> impl Trait` (or `impl A + B`, with associated types fixed as in a bound:
`impl Get<Out = T>`) as a function's result type is an **opaque** type: the function's body gives
a value of one type of its choice, its **hidden** type, which must meet the bounds (E0402), and a
caller sees only the bounds: it can call the bounds' methods and pass the value where they are
required, but not use what else the hidden type has (E0403). Each call of a generic function, for
its type arguments, gives one opaque type. For now the result type is `impl Trait` itself (or
`(impl Fn() with E)`, parenthesised), not a type containing one; and a trait's methods cannot
have one. A closure the function returns as `impl Fn…` owns what it captures
(**expr.closure.result.1**); otherwise the hidden type may hold borrows only as
**type.impl.ret.2** says.

**type.impl.ret.2** The hidden type may hold closures and functions named as values that met their
needs where they were written (**expr.closure.effects.1**), so they hold the function's handlers:
`fn app() -> impl Handler with Links { Router::new().get("/{code}", follow) }`. A call's result
then borrows the handlers its caller passed, so it is used while they are installed, inside the
`with` that installs them, and not after (E0308). It may hold no other borrow (E0308, E0310).

**type.dyn.coerce.1** A `&T` can be used where a `&dyn Trait` is expected if `T` implements `Trait`
(E0402), and so can a `&dyn Sub` of a subtrait `Sub` of `Trait`; likewise for `&mut`, which can also
be used where the shared form is expected.

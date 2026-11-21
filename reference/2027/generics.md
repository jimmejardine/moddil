# Generics

```rust
struct Pair<A, B> { first: A, second: B }

fn largest<T: PartialOrd>(a: T, b: T) -> T {
    if a < b { b } else { a }
}

largest(3, 9)                    // => 9
```

**generic.1** Functions, structs, enums, traits and impls may declare **type parameters**:
`fn f<T, U>`, `struct Pair<A, B>`, `impl<T> Wrap<T>`. A type parameter is a type within the item.
A generic struct or enum is a type only with its type arguments: `Pair<int, bool>`. An item takes
exactly as many type arguments as it has type parameters (E0404), and the parameters of one item
have distinct names (E0104).

**generic.infer.1** The type arguments of a call of a generic function, and of a generic struct or
enum built by a struct expression or a constructor, are inferred from how they are used: from the
arguments, and from the type the result is expected to have. They may also be written, after `::`:
`f::<int>(x)`, `Tree::<int>::Leaf`. A type that cannot be inferred must be written (E0405), with a
type annotation on `let` (`let x: Option<int> = None;`) or as type arguments; in a type annotation,
`_` stands for a type to infer.

**generic.bound.1** A **bound** `T: Trait` requires the type argument for `T` to implement the trait
([Traits](items/traits.md)), and is written after the parameter (`<T: Clone + PartialEq>`) or in a
`where` clause after the signature (`where T: Clone`). A generic item is checked once, as written:
its body may use only what its bounds give a type parameter (the traits' methods, and their
operators: E0205, E0403). A use with a type argument that does not implement a bound's trait is an
error (E0402). A bound also gives the trait's supertraits.

**generic.assoc.1** `T::Name`, for a type parameter or `Self`, is the associated type `Name` of a
trait in `T`'s bounds ([Traits](items/traits.md)) (E0413 if none has one). A bound may fix an
associated type: `T: Add<Output = T>`; `T::Output` is then that type. For a type that implements
the trait, the associated type is the one its impl defines.

**generic.borrow.1** Borrows are second-class ([Borrows](borrows.md)), and a type parameter may be
stored in a struct: so a type argument cannot be a borrow or contain one (E0309).
`Some(&x)` is an error; a function that takes a borrow says so in its signature (`x: &T`).

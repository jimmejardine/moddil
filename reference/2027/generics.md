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
`where` clause after the signature (`where T: Clone`, which may end with a comma before the body;
a bound's type there is not an anonymous record, as a `{` after it starts the body). A generic item is checked once, as written:
its body may use only what its bounds give a type parameter (the traits' methods, and their
operators: E0205, E0403). A use with a type argument that does not implement a bound's trait is an
error (E0402). A bound also gives the trait's supertraits.

**generic.default.1** A trait's, a struct's or an enum's type parameter may have a **default**,
`<T, U = T>` or `trait Add<Rhs = Self>`: an argument left out is its default, which may name the
parameters before it and, in a trait, `Self`, the type the bound or impl is for. So `Pair<int>`
is `Pair<int, int>`, and `T: Add` is `T: Add<T>`. The parameters with defaults come last, and a
function's or an impl's has none (E0404).

**generic.assoc.1** `T::Name`, for a type parameter or `Self`, is the associated type `Name` of a
trait in `T`'s bounds ([Traits](items/traits.md)) (E0413 if none has one). A bound may fix an
associated type: `T: Add<Output = T>`; `T::Output` is then that type. For a type that implements
the trait, the associated type is the one its impl defines.

**generic.borrow.1** A type argument may be a borrow, or hold one: `Some(&x)` is an
`Option<&int>`. A type with such an argument is a view ([Views](views.md)), second-class as a
borrow is.

**generic.const.1** A **const parameter**, `const N: int`, stands for a length: it is given by an
array type's length (`fn len<T, const N: int>(a: &[T; N]) -> int`), inferred from the arguments as
a type parameter is, or written as an integer (`f::<3>()`). In the item it is the length of array
types (`[T; N]`), and its value as an `int` where it is named in an expression.

**generic.const.2** A const parameter is a length, not a type, and a type parameter a type, not a
length: naming one where the other is expected is an error (E0102, E0201). For now a const
parameter is an `int`.

**generic.const.3** A length (an array type's, `[x; n]`'s, or a const argument) is an integer
literal, a const parameter, or a const expression: the name of a free `int` const item, or
`+ - * / % << >> & | ^` on literals and such names, in parentheses or a block for a const
argument (`[int; SIZE * 2]`, `Grid<SIZE>`, `f::<{SIZE + 1}>()`). The const items' initialisers
are themselves such expressions, worked out when compiling, before the program is checked. A
length that is not (one calling a `const fn`, overflowing, dividing by zero, or naming a const
parameter in arithmetic) is an error (E0201).

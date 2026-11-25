# Arrays and indexing

**expr.array.1** `[a, b, c]` is an array of the values of its elements, evaluated left to right,
which have one type (E0201); with `n` elements, its type is `[T; n]`. `[]` has length 0, and its
element type is inferred.

**expr.array.repeat.1** `[x; n]` is an array of `n` copies of `x`, which is evaluated once. `n` is
a length known when compiling: an integer literal or a const parameter ([Generics](../generics.md));
`x`'s type is `Clone` (E0402). If it is not `Copy`, the elements but the last are clones of `x`,
made in order, and the last is `x` itself (with `n` of 0, `x` is dropped); a trap in a clone drops
the elements made so far. A length known only when the program runs is an error here (E0201),
but for a type made with `from_repeat` (**expr.array.from.1**).

**expr.array.from.1** Where a type other than an array is expected (by a `let`'s type, a
parameter, a field or a result) and an array literal is given, the literal makes a value of that
type through the prelude's `FromArrayLiteral<T>`: `[a, b]` is `from_array([a, b])`, and `[x; n]`
is `from_repeat(x, n)`, where `T` must be `Clone` and `n` may be any `int`. The type must implement
the trait (E0402). `let v: Vec<int> = [1, 2, 3];` makes a `Vec`, and
`let m: HashMap<int, bool> = [(1, true)];` a map ([Collections](../collections.md)).

**expr.index.1** `a[i]` is the element at index `i`, an `int` (E0201), of an array or a slice `a`,
counting from 0; indexing looks through a borrow, as a field does. It is a place: it can be
assigned to and borrowed, if `a` can be changed ([Borrows](../borrows.md)). An index below 0 or
not below the length traps (an index out of bounds, [Traps](../traps.md)).

**expr.slice.1** `a[i..j]` is the part of an array, a slice or a `Vec` `a` from index `i` up to,
not including, `j`, both `int`s (E0201): a slice place, `[T]`, which has no size of its own, so it
is used behind a borrow, `&a[i..j]` or `&mut a[i..j]` (E0605). `a[i..=j]` includes `j`; `a[i..]`
runs to the end, `a[..j]` from the start, and `a[..]` is all of it. A part borrowed with `&mut`
changes `a`'s elements, and `a` must be one that can be changed, as for an element. Unless
`0 <= i <= j <= len`, it traps (an index out of bounds). A `Vec` is sliced through its
`as_slice`, or `as_mut_slice` where the part is changed; any other type is an error (E0205), but
text, which is sliced by position (**text.pos.1**). On slices and arrays, `xs.split_at(i)` gives the
parts before and from `i`, and `xs.split_at_mut(i)` the same, each borrowed with `&mut`, so the two
can be changed at once.

**expr.index.2** Indexing a type that is not an array or slice and does not implement `Index` is
an error (E0205).

**expr.index.trait.1** On another type, `a[i]` is `*Index::index(&a, i)`, or
`*IndexMut::index_mut(&mut a, i)` where the element is changed (assigned, borrowed with `&mut`, or
the receiver of a method taking `&mut self`); changing it through a type that does not implement
`IndexMut` is an error (E0205). The prelude's `Index` has `type Output` and
`fn index(&self, i: int) -> &Self::Output`, and `IndexMut: Index` has
`fn index_mut(&mut self, i: int) -> &mut Self::Output`.

**expr.index.move.1** An element of an array or slice whose type is not `Copy` cannot be moved
out of by indexing (E0315); it can be swapped out, replaced (`replace(&mut a[i], x)`), or cloned.
Through `Index`, the element is behind a borrow (E0304).

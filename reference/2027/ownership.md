# Ownership

```rust
struct Name { first: Label, last: Label }

let a = Label { id: 1 };
let b = a;                 // `a` moves to `b`
a;                         // error: `a` was moved (E0303)
```

**own.move.1** Every value has one owner: the variable, field or temporary holding it. Using a
value, by assigning it, passing it to a function, returning it, putting it in a struct, enum or
tuple, or binding it in a pattern, **moves** it unless its type is `Copy` (**own.copy.1**). After a
move the source holds nothing: using it again is an error (E0303) until it is assigned a new value.
The check follows every path through the function, so a move on any path to a use is an error.

**own.move.partial.1** Moving a field out of a variable moves only that field: the other fields
can still be used, but the whole value cannot (E0303) until the moved field, or the whole, is
assigned again.

**own.move.discard.1** An expression statement whose expression is a place (a variable, a field,
or `*r`) reads it without moving it: its value is discarded, as `let _ = p;` would.

**own.move.borrow.1** A value reached through a borrow is not owned by it, so it cannot be moved
out (E0304): `*r` and `r.field` of a type that is not `Copy` cannot be used by value.

**own.copy.1** Values of `Copy` types are copied instead, and the source stays usable: the types
that implement the prelude's `Copy` trait ([Prelude](prelude.md)). `int`, `bool`, `()`, `!` and
shared borrows (`&T`) are `Copy`; a tuple is `Copy` if its elements are; a struct or enum is `Copy`
if it is declared so (**own.copy.derive.1**); and a type parameter is `Copy` if it is bounded by
`Copy` ([Generics](generics.md)). `&mut T` is not `Copy`.

**own.copy.derive.1** `#[derive(Copy, Clone)]` before a struct or enum declares it `Copy`, as does
an impl of `Copy` for it. Every field of it must be `Copy` (a field whose type is a type parameter
is, for the type arguments that are), `Copy` must come with `Clone` (E0311), and a type with a
destructor cannot be `Copy` (E0411). It may be at most 64 bytes (E0312), counted as follows: `int`
and borrows take 8 bytes, `bool` 1 and `()` 0; a tuple or struct places its fields in order, each at
the next multiple of its alignment (its size, for these types), and rounds the total up to its
largest alignment; an enum takes an 8-byte tag and then its largest variant, rounded up to 8. A
generic type declared `Copy` is `Copy` only for type arguments that keep it within 64 bytes.

**own.drop.1** A value is **dropped** when its owner goes away without moving it: a variable at the
end of its scope (the variables of a block in the reverse of the order they were declared, with
`defer` blocks among them in the order they were reached), a parameter at the end of its call, the
old value of a place when it is assigned, and a temporary at the end of the statement that made it.
A value moved out is not dropped where it was, and a value partly moved out has only its other
parts dropped. Nothing is dropped after a trap. A temporary that is still borrowed when it is
dropped is an error (E0308): to keep a value, bind it with `let`.

**own.drop.destructor.1** Dropping a value of a struct or enum with a **destructor**, an impl of
the prelude's `Drop` (`fn drop(self)`), calls it with the value; the destructor's `self` then drops
its fields, unless it moves them out. Dropping any other value drops its fields, the last declared
first (the elements of a tuple, last first). A destructor is written for a struct or enum of the
same module, for all its type arguments and with no bounds (`impl<T> Drop for Wrap<T>`) (E0411).
It is run only by dropping: calling it is an error (E0412).

**own.drop.partial.1** A field cannot be moved out of a value whose type has a destructor (E0314),
since the destructor needs the whole value; the destructor itself may move the fields out of its
`self`.

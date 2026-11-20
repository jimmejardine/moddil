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

**own.copy.1** Values of `Copy` types are copied instead, and the source stays usable. `int`,
`bool`, `()`, `!` and shared borrows (`&T`) are `Copy`; a tuple is `Copy` if its elements are; and
a struct or enum is `Copy` if it is declared so (**own.copy.derive.1**). `&mut T` is not `Copy`.

**own.copy.derive.1** `#[derive(Copy, Clone)]` before a struct or enum declares it `Copy`. Every
field of it must be `Copy`, and `Copy` must come with `Clone` (E0311). It may be at most 64 bytes
(E0312), counted as follows: `int` and borrows take 8 bytes, `bool` 1 and `()` 0; a tuple or struct
places its fields in order, each at the next multiple of its alignment (its size, for these
types), and rounds the total up to its largest alignment; an enum takes an 8-byte tag and then its
largest variant, rounded up to 8.

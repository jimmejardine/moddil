# Fields

```rust
let p = Point { x: 1, y: 2 };
p.x                      // => 1
let t = (1, (true, 3));
t.1.0                    // => true
```

**expr.field.1** `e.f` is field `f` of the struct `e`, and `e.0`, `e.1`, … are the positional
fields of a tuple or tuple struct. It has the field's type. A field the type does not have is an
error (E0107); an enum's fields are reached only by matching it ([`match`](match.md)).

**expr.field.deref.1** If `e`'s type (or what a borrow `e` points to) has no field `f` and
implements the prelude's `Deref`, `e.f` is the field of what it dereferences to, as `(*e).f` would
be: a `Box`'s value's, an `Rc`'s. It looks through one `Deref`, as a method call does
([Methods](methods.md)). Changing such a field, or borrowing it with `&mut`, goes through
`DerefMut`, which the type must implement (E0205): a `Box`'s fields can be changed, an `Rc`'s
cannot.

**expr.field.assign.1** A field of a local, or a field of a field of one (`p.x`, `t.1.0`), is a
place: it can be assigned to, and compound-assigned, like a local ([Assignment](assignment.md)),
if the local is declared `mut` (E0301). Assigning changes that field only.

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

**expr.field.assign.1** A field of a local, or a field of a field of one (`p.x`, `t.1.0`), is a
place: it can be assigned to, and compound-assigned, like a local ([Assignment](assignment.md)),
if the local is declared `mut` (E0301). Assigning changes that field only.

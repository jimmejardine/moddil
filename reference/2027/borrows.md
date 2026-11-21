# Borrows

```rust
fn bump(n: &mut int) { *n += 1; }

let mut count = 0;
bump(&mut count);
let r = &count;
*r                         // => 1
```

**borrow.1** `&e` borrows the place `e` (a variable, a field, or `*r`) and `&mut e` borrows it
exclusively. A borrow of any other expression borrows a temporary holding its value. `&e` has type
`&T` and `&mut e` type `&mut T`, where `e` has type `T`.

**borrow.type.1** Borrows are **second-class**: `&T` and `&mut T` are the types of parameters,
variables and results, and of the parts of views ([Views](views.md)): a value holding a borrow is a
view. A field of a struct or enum that is not a view cannot hold one, and a borrow cannot point to
a borrow (E0309).

**borrow.deref.1** `*e`, for a borrow `e`, is the place it points to, of type `T`. A field access
looks through a borrow (`r.x` is `(*r).x`); operators and comparisons do not, so they need `*`: an
arithmetic operator or a comparison applied to borrows is an error (E0205).

**borrow.mut.1** A place can be changed (assigned, compound-assigned or borrowed with `&mut`) if
it is a variable declared `mut` (E0301), or is reached through a `&mut` borrow; through a `&`
borrow it cannot (E0302).

**borrow.coerce.1** A `&mut T` can be used where a `&T` is expected.

**borrow.rules.1** While a borrow is **live**, from where it is made to its last use on any path,
its place may not be borrowed with `&mut` if it is borrowed with `&`, nor borrowed at all if it is
borrowed with `&mut` (E0305); it may not be assigned or moved (E0306); and if it is borrowed with
`&mut`, it may not be used at all except through that borrow (E0307). A borrow ends at its last
use, not at the end of its scope.

**borrow.reborrow.1** Using a `&mut` borrow (passing it, assigning it) lends what it points to
again, for as long as the new borrow is used, and leaves the original usable afterwards.

**borrow.scope.1** A borrow may not be used after the variable it borrows goes out of scope
(E0308).

**borrow.static.1** A borrow of **static data** (a string literal, or a const's value,
`&NAME`, [Compile-time evaluation](comptime.md)) borrows nothing: it freezes no place, never ends,
and can be returned from a function with no borrowed parameter
(`fn greeting() -> &str { "hi" }`). Only literals and consts are static: a borrow of any other
value borrows a temporary.

**borrow.guard.1** While the guards of a `match` run, the matched place is borrowed with `&`, so a
guard cannot change it (E0306).

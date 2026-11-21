# Views

A **view** is a value that holds borrows. Like a borrow, it is second-class
([Borrows](borrows.md)): it lives in parameters, locals and other views, and is passed down and
returned under the same rules, never stored in owned data.

**view.1** `view Name<…> { f: T, … }` declares a struct whose fields may hold borrows and views;
it is otherwise a struct ([Structs](items/structs.md)), built, read and matched as one.

**view.infect.1** A value is a view if its type is a borrow, a closure, a view, or a tuple, array,
struct or enum whose type holds one, including through its type arguments: `Option<&int>`,
`(&str, int)` and `Wrap<&int>` are views. So a type argument may be a borrow.

**view.2** A field of a struct or enum that is not a view cannot hold a borrow or a view (E0309).
A function returning a view follows the rules for returning a borrow
([Functions](items/functions.md)).

**view.borrow.1** A view borrows what its borrows borrow, for as long as it is used: building one
lends it the borrows it is built from, reading a field of one gives a borrow it holds, and a call
that changes a view through `&mut` (`v.push(&x)`) lends it what the call's other arguments borrow.
A call whose result type, as the function declares it, holds no borrow, gives a result borrowing
what its arguments hold, not the borrows passed for them: `it.next()` gives what the iterator
borrows. The rules of [Borrows](borrows.md) apply to all of these (E0305 to E0308).

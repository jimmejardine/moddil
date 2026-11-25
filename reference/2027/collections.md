# Iteration and collections

The prelude's `Iterator`, `IntoIterator`, `Buf` and `Vec`, and the built-in `core` mod's maps and
sets. They are written in Moddil ([`prelude/collections.moddil`](prelude/collections.moddil),
[`core/collections.moddil`](core/collections.moddil)), but for the methods of `Buf`, which are
built in.

## Iteration

**iter.1** An iterator implements `Iterator`, with `type Item` and
`fn next(&mut self) -> Option<Self::Item> with fail(Self::Fails)`: each call gives the next item,
or `None` when there are no more. `it.count()` uses an iterator up and gives how many items it had.

**iter.fails.1** `Iterator`'s `type Fails` is what `next` can fail with: `!` unless an impl says
otherwise (an associated type's default, as `type Fails = !;` declares, is what an impl that does
not define it takes). `count`, `all`, `any` and `collect` fail as `next` does. In a generic
function, a call whose errors are a type parameter's `Fails` (`I::Fails`, through a `for` loop or
`it.next()`) passes them on without a mark, as a call of a closure parameter does
(**expr.closure.fails.1**), so the function's errors must include them; a `for` loop over an
iterator that can fail with known errors marks them, `for x in it? { … }`
([Loops](expressions/loops.md), **expr.for.3**), or they are taken with `collect()?`. An associated type's value
may be an error set, `type Fails = I::Fails | F::Fails;`.

**iter.adapt.1** `it.map(f)` gives the items passed through closure `f`, in order, and
`it.filter(f)` the items for which `f` (given a borrow of each) holds. Both are lazy: they give a
`Map<I, F>` or a `Filter<I, F>` holding the iterator and the closure, and `f` runs as each item is
asked for, in `next`, so they work on an iterator without end, and `f`'s errors are the adapter's
`Fails` (with the iterator's), not those of the call of `map` or `filter`.

**iter.collect.1** `it.collect()` gathers the items left into a collection of the type the context
names (`let v: Vec<int> = it.collect();`), one that implements `FromIterator<Item>`: `Vec<T>` from
`T`s, and `String` from `char`s.

**iter.into.1** `IntoIterator`, with `type Item`, `type IntoIter` and
`fn into_iter(self) -> Self::IntoIter`, is what `for` iterates over
([Loops](expressions/loops.md)). Every iterator implements it, as itself; so do `&[T]` and
`&[T; N]` (borrowing each element, an `&T`), `&mut [T]`, `&mut [T; N]` and `&mut Vec<T>`
(borrowing each with `&mut`, **iter.slice.2**), `[T; N]` and `Vec<T>` (giving up each element),
`&Vec<T>`, `&HashMap<K, V>` (each entry, a `(&K, &V)`) and `&HashSet<T>`.

**iter.slice.1** `xs.iter()`, for a slice, an array or a `Vec`, is a view of type `Iter<T>` that
gives a borrow of each element in order ([Views](views.md)). `s.chars()`, for a `str` or `String`,
gives its characters in order.

**iter.slice.2** `xs.iter_mut()`, for a slice, an array or a `Vec` borrowed with `&mut`, is a view
of type `IterMut<T>` that gives an `&mut` borrow of each element in order. The borrows are of
different elements, so they can be held together; while any lives, `xs` is borrowed (E0305).

## Buffers and vectors

**coll.buf.1** `Buf<T>` is built-in storage with a fixed capacity, of which a prefix holds
elements: `Buf::with_capacity(n)` is empty, `Buf::from_array(a)` holds an array's elements, and
`len`, `capacity`, `set_capacity` (to its length if given less), `push`, `pop`, `insert`, `remove`,
`swap`, `as_slice` and `as_mut_slice` are its methods. Pushing or inserting into a full buffer
traps (capacity exceeded), as do `insert`, `remove` and `swap` at an index out of bounds
([Traps](traps.md)). Its elements are dropped with it, first to last.

**coll.vec.1** `Vec<T>` is a growable array over a `Buf`: `Vec::new()` and
`Vec::with_capacity(n)` are empty, and `push`, `pop`, `insert`, `remove`, `swap`, `truncate`,
`clear`, `len`, `is_empty`, `capacity`, `reserve`, `get`, `get_mut`, `first`, `last`, `as_slice`,
`as_mut_slice`, `iter` and (for `T: PartialEq`) `contains` are its methods. Pushing into a full
`Vec` gives it more room, at least twice as much.

**coll.vec.2** A `Vec` is `Clone` if `T` is, and `PartialEq` and `Eq` element by element.

**coll.vec.index.1** `v[i]` on a `Vec` is its element `i`, through `Index` and `IndexMut`
([Arrays and indexing](expressions/arrays.md)); an index out of bounds traps.

**coll.vec.borrow.1** A borrow of an element of a `Vec` borrows the `Vec`, so it cannot be changed
while the borrow is used (E0305). A `Vec` of borrows is a view, which keeps what it holds borrowed
(E0308).

## Hashing

**coll.hash.1** `Hash`, with `fn hash<H: Hasher>(&self, state: &mut H)`, says what a value hashes;
a `Hasher`, with `fn write_int(&mut self, x: int)` and `fn finish(&self) -> int`, how. `int`,
`bool`, `char`, `str`, `String`, slices, `Vec`, `Option` and pairs implement `Hash`, and a borrow
`&T` hashes as `T` does, so a map can be keyed by borrows (`HashMap<&str, V>`).

**coll.hash.derive.1** `#[derive(Hash)]` hashes an enum's variant's index, then each field in
order ([Attributes](attributes.md)).

**coll.hash.2** The `core` mod's default hasher, `SipHasher`, is SipHash-1-3 over `int`s, with a
fixed key for now, so a map's order is the same on every run and implementation.
`hash_of(&x)` is `x`'s hash under it.

## Maps and sets

**coll.core.1** Every mod depends on the built-in `core` mod, which it names `core` (unless it
names a dependency so): `use core::collections::HashMap;`.

**coll.map.1** `HashMap<K, V>`, for `K: Hash + Eq`, maps keys to values: `new`, `len`, `is_empty`,
`insert` (giving back the value it replaces), `get`, `get_mut`, `contains_key`, `remove` and
`iter` are its methods. It iterates in hash order under its hasher's key. A literal of pairs makes
one, later pairs winning (**expr.array.from.1**).

**coll.map.2** The traits `Map<K, V>` and `Set<T>` let code take any map or set; `HashMap` and
`HashSet` implement them.

**coll.set.1** `HashSet<T>` is a set of values: `insert` (saying whether the value was new),
`contains`, `remove`, `len`, `is_empty` and `iter` are its methods.

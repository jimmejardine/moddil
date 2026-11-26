# Shared values: `Rc` and `Weak`

A value with more than one owner, by counting them ([`specs/memory.md`: Sharing:
`.to_rc()`](../../specs/memory.md#sharing-to_rc), [`specs/values.md`: `.share()` for reference
counts](../../specs/values.md#share-for-reference-counts)). `Rc` and `Weak` are the prelude's,
written in Moddil over a raw pointer ([source](prelude/pointers.moddil)). Added after M3 slice M3a.

**rc.1** `x.to_rc()`, on a value of any type, moves `x` into a new allocation and gives an
`Rc<T>`, its first owner. `rc.share()` gives another owner of the same value, adding one to the
count, and copies nothing. Borrowing an `Rc` (`&Rc<T>`) touches no count. Dereferencing one, `*rc`,
names the value, and a method of the value is called on the `Rc` as on the value
(**expr.method.deref.1**), and so is a field, `rc.name` (**expr.field.deref.1**). `Rc::strong_count(&rc)`
is how many owners the value has, and `Rc::ptr_eq(&a, &b)` whether two `Rc`s own the same value:
the only test of identity, as `==` compares the values. An `Rc` compares, orders, hashes and shows
as its value (`PartialEq`, `Eq`, `PartialOrd`, `Ord`, `Hash`, `Debug`), where the value does.

**rc.clone.1** `Rc<T>` and `Weak<T>` implement `Clone` as `share`, so generic code bounded by
`Clone` takes them: a `Vec<Rc<T>>` clones, sharing each value, and a derived `Clone` shares its
`Rc` fields. Written as a method call on an `Rc` or a `Weak`, `.clone()` is refused (E0417):
`.share()` says what it costs.

**rc.immutable.1** What an `Rc` owns cannot be changed through it: `Rc` implements `Deref` and
not `DerefMut` (E0205). A value shared and changed is in a sync cell ([Unsafe code](unsafe.md)).

**rc.weak.1** `rc.weak()` gives a `Weak<T>`, which keeps the allocation but not the value.
`w.upgrade()` gives `Some` new owner while the value has one, and `None` after. `w.share()` gives
another `Weak`, and `Rc::weak_count(&rc)` is how many there are. A back-edge (a child's link to
its parent) is a `Weak`, so that the two do not keep each other alive.

**rc.unwrap.1** `Rc::try_unwrap(rc)` gives `Ok` of the value if `rc` is its only owner, and
`Err(rc)` otherwise. Its weak handles then see it gone (`upgrade` gives `None`).

**rc.drop.1** Dropping an `Rc` takes one from the count. The last owner drops the value; the
allocation goes with the last owner or `Weak`, whichever is later. `Rc`s that own each other in a
cycle are never dropped: nothing collects them.

**rc.task.1** An `Rc` may be shared with, moved to and dropped in any task: its value is dropped
when its last owner is, in whichever task that is. The counts are biased to the task that made
the allocation ([`specs/memory.md`: Biased reference
counting](../../specs/memory.md#biased-reference-counting)): that task counts plainly, and the
others atomically.

An `Rc` that tasks swap is `moddil/sync`'s `AtomicRc`. Not yet: `Rc<dyn Trait>`, and the merge
queue a parallel runtime hands a last owner to its allocation's task through.

## Diagnostics

| Code | Meaning |
|---|---|
| E0417 | `.clone()` written on an `Rc` or a `Weak`: it is `.share()` |

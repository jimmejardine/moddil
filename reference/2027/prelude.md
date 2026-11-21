# The prelude

The prelude is the items every module can name without importing them. It is written in Moddil,
in [`prelude.moddil`](prelude.moddil), which is part of this reference: its items and their
behaviour are as written there.

**prelude.1** The prelude holds `assert` ([`assert`](expressions/assert.md)) and `discard`
([Errors](errors.md)), the types `Option`, `Result` and `Ordering`, the traits `Clone`, `Copy`,
`PartialEq`, `Eq`, `PartialOrd`, `Ord`, `Add`, `Sub`, `Mul`, `Div`, `Rem`, `Neg`, `Not` and
`Drop`, the function `drop`, and their impls for `int`, `bool` and `()`. A module may not define an item with the name of one of the prelude's items or
of `Option`'s and `Result`'s variants (E0103).

**prelude.option.1** `enum Option<T> { None, Some(T) }` is an optional value. Its variants are
named on their own: `Some(3)`, `None` (and also `Option::Some`). Its methods `is_some`, `is_none`
and `unwrap_or` are as the prelude writes them. It derives `Copy`, `Clone`, `PartialEq`, `Eq`,
`PartialOrd` and `Ord`, so `None` orders before every `Some`.

**prelude.result.1** `enum Result<T, E> { Ok(T), Err(E) }` is a value or an error: the value
form of a call that can fail ([Errors](errors.md)), made by `try`. Its variants are named on
their own (`Ok(3)`, `Err(e)`), and its methods `is_ok` and `is_err` are as the prelude writes
them. It derives the traits `Option` does.

**prelude.ordering.1** `enum Ordering { Less, Equal, Greater }` is the result of comparing two
values: `PartialOrd::partial_cmp` returns an `Option<Ordering>` (`None` for values that do not
compare), and `Ord::cmp` an `Ordering`. `PartialOrd`'s `lt`, `le`, `gt` and `ge`, which `<`, `<=`,
`>` and `>=` call ([Comparisons](expressions/comparison.md)), have defaults written with
`partial_cmp`. `int`s compare by value, `bool`s with `false` before `true`.

**prelude.clone.1** `x.clone()` makes an independent value with the same contents. `Copy` (a
subtrait of `Clone`) marks the types whose values are copied rather than moved
([Ownership](ownership.md)): `int`, `bool` and `()` implement both; and, built in, so do `!`, shared
borrows, and tuples whose elements are `Copy`, whose `clone` is a copy.

**prelude.drop.1** `drop(x)` takes `x` by value and does nothing with it, so `x` is dropped at the
end of the call ([Ownership](ownership.md)): it ends a value early.

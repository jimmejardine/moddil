# The prelude

The prelude is the items every module can name without importing them. It is written in Moddil,
in the files of [`prelude/`](prelude/), which are part of this reference: its items and their
behaviour are as written there. The files are one source, joined in this order:

| File | Holds |
|---|---|
| [`core.moddil`](prelude/core.moddil) | `Option`, `Result`, `Clone` and `Copy`, equality and ordering, the operator traits, `Drop` and `drop`, the closure traits |
| [`int.moddil`](prelude/int.moddil) | `int`, `bool` and `()` |
| [`numbers.moddil`](prelude/numbers.moddil) | the sized integers and the floats, and `Number`, written as templates (**comptime.derive.for.1**); `RadixText` (**expr.fmt.spec.4**) |
| [`convert.moddil`](prelude/convert.moddil) | conversions between the numbers, rounding, `RangeError` |
| [`float.moddil`](prelude/float.moddil) | the float functions |
| [`text.moddil`](prelude/text.moddil) | `TextWrite`, `Display`, `char`, `str`, `String`, `Pattern`, `Chars`, `CharIndices`, text positions, UTF-8, `RcStr` |
| [`fmt.moddil`](prelude/fmt.moddil) | `Debug` (and `debug_pretty` for `{x:#?}`) and `Default` |
| [`arrays.moddil`](prelude/arrays.moddil) | arrays and slices |
| [`iter.moddil`](prelude/iter.moddil) | `Iterator` and `IntoIterator`, the adapters, `FromIterator`, the ranges, the slice and array iterators (`Iter`, `IterMut`) |
| [`pointers.moddil`](prelude/pointers.moddil) | `Index`, `Deref`, `Box`, `Rc` |
| [`collections.moddil`](prelude/collections.moddil) | collection literals, `Buf`, `Vec`, `replace` and `swap`, `int`'s `rotate_left`, `Wrapping` and `WrappingInt` (**num.wrap.2**) |
| [`hash.moddil`](prelude/hash.moddil) | `Hash` and `Hasher` |
| [`program.moddil`](prelude/program.moddil) | `ExitCode`, `Log`, `RefinementError` and `Refined`, `FnInfo` and `ParamInfo` |
| [`concurrency.moddil`](prelude/concurrency.moddil) | scopes and tasks, channels, `Duration` and timers |
| [`derives.moddil`](prelude/derives.moddil) | the templates `#[derive]` instantiates for `Clone`, `PartialEq`, `PartialOrd`, `Ord`, `Hash`, `Debug` and `Default` (**attr.derive.1**) |

**prelude.1** The prelude holds `assert` ([`assert`](expressions/assert.md)), `unreachable` and
`todo` ([`unreachable` and `todo`](expressions/unreachable.md)) and `discard` ([Errors](errors.md)), the types `Option`, `Result` and `Ordering`, the traits `Clone`, `Copy`,
`PartialEq`, `Eq`, `PartialOrd`, `Ord`, `Add`, `Sub`, `Mul`, `Div`, `Rem`, `Neg`, `Not` and
`Drop`, the closure traits `FnOnce`, `FnMut` and `Fn`, `Display` and `TextWrite`, the
functions `drop`, `replace` and `swap`, their impls for `int`, `bool` and `()`, and the methods and impls of `char`, `str` and
`String` ([Text](text.md)), and of arrays and slices ([Arrays and indexing](expressions/arrays.md)).
It also holds `Index`, `IndexMut` and `FromArrayLiteral`, the iteration traits `Iterator` and
`IntoIterator`, the ranges `Range`, `RangeInclusive` and `RangeFrom`, `Iter` and `Chars`, `Buf`,
`Vec`, and `Hash` and `Hasher` ([Collections](collections.md)), and `Debug` and `Default`
(**expr.fmt.debug.1**, **attr.derive.default.1**), with `Default` for `int` (0), `bool`
(`false`), `String`, `()`, `Vec` and `Option` (`None`); `RefinementError` and
`Refined` ([Refinements](refinements.md)); `FnInfo` and `ParamInfo` ([Attributes](attributes.md)); `scope`, `Scope`, `Task`, `yield_now`, `stop_requested`, `Trapped`, `channel`, `Sender`,
`Receiver`, `Closed`, `Duration`, `timer` and `sleep` ([Concurrency](concurrency.md)); `ExitCode` (**main.exit.1**); `Log` and
`log` (**obs.log.1**); `Pattern` (**text.pattern.1**); `Option::map`; the impls and methods of
the sized integers and the floats, `Number` and `RangeError` (**num.types.1**,
**num.convert.1**, **num.float.fn.1**); `FromIterator` and `Iterator`'s `map`,
`filter` and `collect`, and the lazy `Map` and `Filter` (**iter.adapt.1**, **iter.collect.1**);
`Box`, `Deref` and `DerefMut` (**type.box.1**); `Rc`, `Weak` and `ToRc` ([Shared
values](rc.md)); and `Iterator`'s `all` and `any`,
which say whether a closure holds for every item, or some. `replace(&mut x, v)` puts `v` in
`x` and gives back what was there; `swap(&mut a, &mut b)` exchanges two values. The built-in
`core` mod's `collections` module holds the maps and sets (**coll.core.1**). Every module sees them, and
`Option`'s and `Result`'s variants, without importing them; an item or import of the module with the
same name shadows them ([Modules](modules.md)). The operators, `?` and `try` use the prelude's traits
and types whatever the module calls its own.

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

**prelude.fn.1** `FnOnce<Args>` (with `type Output` and `type Fails`, and `call_once(self, args)`),
`FnMut<Args>: FnOnce<Args>` (`call_mut(&mut self, args)`) and `Fn<Args>: FnMut<Args>`
(`call(&self, args)`) are the closure traits ([Closures](expressions/closures.md)): `Args` is the
tuple of a closure's parameters' types, `Output` its result and `Fails` its errors. A closure
implements them built in.

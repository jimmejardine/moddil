# Refinements

A refinement is a type of the values of another type, its **base**, for which a predicate holds:
a validated newtype ([`specs/correctness.md`: Validated newtypes](../../specs/correctness.md#validated-newtypes-lightweight-refinements)).

**refine.1** `type X = T where p;` declares the refined type `X`, of the values of `T` for which
`p` holds; in `p`, `self` is the value. `type X = T;`, without `where`, declares an alias: `X` is
`T`. A `type` item is not generic, for now. `p` is a `bool` and is compile-time code
([Compile-time evaluation](comptime.md)): pure, calling only `const fn`s and the prelude's
functions (E1001).

**refine.base.1** An `X` is used as its base: it converts to `T` where a `T` is expected (and a
`&X` to a `&T`), the base's
methods and operators apply to it and give values of the base (`p + 1` is an `int`), and it has
every trait its base has. A refined type has no impls of its own, for now (E0414). An `X` is not
made from a `T` by conversion (E0201): only as the rules below say.

**refine.new.1** `X::new(v)` is `v` as an `X` if `p` holds for it, and otherwise fails with the
prelude's `RefinementError { ty, clause, value }`: `X`'s name, `p` as written, and `v` as its
`Debug` shows it, all `String`s. So the base must implement `Debug` (E0402).

**refine.new.2** The base of a refinement holds no borrow (E0309).

**refine.trait.1** Every refinement implements the prelude's trait `Refined`, built in: its
`Base` is the refinement's base, and `refine(v)` is its `new(v)`. So generic code makes a value of
a refined type: `fn make<T: Refined>(v: T::Base) -> T with fail(RefinementError) { T::refine(v)? }`.
Nothing else implements it, and an impl of it is an error (E0406).

**refine.lit.1** A literal where an `X` is expected (a `let`'s type, a parameter, a function's
result) is an `X`, and compiling checks that `p` holds for it (E1101).

**refine.inline.1** A refinement may be written in a type, in parentheses: `(u32 where self >= 1)`,
in a parameter, a field, a result or a `let`'s type. It is a refinement of its own, without a
name, shown as written: its values come from literals (**refine.lit.1**) and from values of it,
so a value that must be checked when it runs is given a named refinement and its `X::new`. `type`
items may name each other in any order.

**refine.pat.1** At the top of a `match` arm, `q: X` matches a value of `X`'s base for which `p`
holds, and then matches it as an `X` with `q`. A base that is not `Copy` is given to the
predicate and back, so the value is the same for the next arm; matched through a borrow it must
be `Copy`, for now (I0001). Exhaustiveness treats the arm as one with a guard: it covers nothing.

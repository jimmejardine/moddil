# Attributes

```rust
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Version { major: int, minor: int }
```

**attr.derive.1** `#[derive(T₁, …, Tₙ)]` before a struct or enum implements the traits named, from
its fields. The traits that can be derived are `Copy`, `Clone`, `PartialEq`, `Eq`, `PartialOrd`, `Ord`,
`Hash`, `Debug` and `Default` ([Prelude](prelude.md)). The impls are **structural**:

- `clone` clones each field (or, for a `Copy` type with no type parameters, copies the value);
- `eq` holds when both values are the same variant and their fields are equal, field by field;
- `partial_cmp` and `cmp` order values by variant, in the order the variants are declared, and
  then by their fields, in declaration order, as the first field that differs does;
- `hash` hashes an enum's variant's index, then each field in order (**coll.hash.derive.1**);
- `fmt` (of `Debug`) writes the value as Moddil source writes it (**attr.derive.debug.1**), and
  `default` makes one of its fields' defaults (**attr.derive.default.1**).

The prelude writes these impls in Moddil, as templates
([`prelude/derives.moddil`](prelude/derives.moddil)): each of `Clone`, `PartialEq`,
`PartialOrd`, `Ord`, `Hash`, `Debug` and `Default` names its own with `#[derive_by]`
(**attr.derive-by.1**). `Copy` and `Eq`, which have no methods, and the copying `Clone` of a `Copy`
type with no type parameters are built in.

For a generic type, the impl has the type's parameters: each type parameter bounded by the
trait (`impl<T: Clone> Clone for Pair<T>`), and each const parameter as it is declared
(`impl<const S: int> Clone for Fixed<S>`).

**attr.derive.debug.1** A derived `Debug` writes `Point { x: 1, y: 2 }` for a struct with named
fields, `Pair(3, true)` for one with positional fields, and `Mode::On(4)` or `Mode::Off` for an
enum's variant, each field by its own `Debug`.

**attr.derive.default.1** A derived `Default` makes a struct of each field's
`Default::default()`. An enum derives it from the one unit variant marked `#[default]`, which it
must have (E0416).

Every field must implement the trait derived (E0402), except one whose type is a type parameter:
the derived impl of a generic type bounds each type parameter by the trait
(`impl<T: Clone> Clone for Wrap<T>`). A derive also needs the trait's supertraits (E0402), so
`Eq` comes with `PartialEq` and `Ord` with `PartialOrd` and `Eq`. `Copy` is described in
[Ownership](ownership.md).

**attr.derive-by.1** `#[derive_by(f)]` before a trait names its derive function `f`, declared in
the same module ([Compile-time evaluation](comptime.md)), E1006 otherwise. `#[derive(Tr)]` on a
struct, for such a trait `Tr`, instantiates `f`'s template for the struct (**comptime.template.1**),
whichever mod the struct is in: the impl asks of the fields only what the template's code does,
so the rule above, that every field implement the trait, does not apply to it. It does apply to
the traits of **attr.derive.1**, whose templates are the prelude's.

**attr.track-caller.1** `#[track_caller]` before a function makes a trap in it be reported where it
was called ([Traps](traps.md)), and through a tracked caller, where that one was called, up to the
first function that is not tracked. The prelude's functions that can trap are tracked, and so is
derived code. The attribute takes no arguments. Besides it, a function may have the wrappers
below; other attributes on functions are not supported yet (I0001).

## Wrappers

```rust
fn twice<A: Clone>(f: &FnInfo, args: A, inner: impl Fn(A) -> int) -> int {
    inner(args.clone()) + inner(args)
}

#[around(twice)]
fn add(a: int, b: int) -> int { a + b }

add(1, 2)                                          // 6
```

**attr.around.1** `#[around(w)]` before a function **wraps** it in the function `w`: its body is
the call `w(&INFO, f::Args { p₁, … }, |f::Args { p₁, … }| body)`, where `p₁, …` are its
parameters, `body` its body as written, and the call's errors pass on, as a call marked with `?`
passes them ([Errors](errors.md)). `w` is an ordinary function, and the call is checked as any
other, its problems reported at the attribute: `#[around]` without a wrapper, or with one that is
not a function's path or a call of one, is an error (E1005).

**attr.around.info.1** `INFO` is the function's description, a const `FnInfo` of the prelude
([The prelude](prelude.md)): `path`, the function's path in its mod (`accounts::transfer`), and
`params`, each parameter's `name` and `ty`, its type as written. `&INFO` is static data
([Compile-time evaluation](comptime.md)).

**attr.around.args.1** `f::Args` is a struct with a field of each parameter's type for each
parameter, named like it; it is a view if a parameter is a borrow ([Views](views.md)), and it
implements `Debug`, `Clone` and `PartialEq` when every parameter's type does. Its `Debug` writes
`f::Args { p₁: …, … }`; it cannot be named in code, for now. `#[around(w(c₁, …))]` passes the arguments `c₁, …` before `&INFO`;
they are compile-time code (E1001), evaluated at each call for now, and cannot name the
parameters (E0101).

**attr.around.order.1** With several wrappers, the first written is outermost: its closure calls
the second, with the arguments as it gives them, and the last's closure runs the body.

**attr.around.body.1** The closure returns the function's result type. In the body, `return`
leaves the closure, and `?` raises from it: what the wrapper's call of it gives or fails with. So
the body's errors reach the function's callers through the wrapper, and a wrapper adds no error
or effect the function does not have (E0501, E0603): a private function's are inferred.

**attr.around.contracts.1** The function's contracts ([Contracts](contracts.md)) are checked
around its wrappers: its preconditions before the first wrapper runs, and its postconditions on
what the first wrapper returns.

**attr.around.limits.1** For now, a wrapped function is a free function, not a `const fn`, whose
parameters are names (I0001).

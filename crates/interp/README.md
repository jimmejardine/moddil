# moddil-interp

The **MIR interpreter**, part of the compiler. It runs *analysed* or *runtime* MIR and never needs
target lowering ([`specs/mir.md`: Pipeline](../../specs/mir.md#pipeline)).

**Status:** slices S0 to S7c (traps, errors and observed values say which file of the program
they are in, and a trap in a `#[track_caller]` function where its caller called it): a closure's
call runs its own body, with its captures and its arguments as two tuples; `char`, `&str` (its
text) and `String` are values, and the prelude's `#[intrinsic]` methods run built in; an array
is a value of its elements, a `Buf` its capacity and elements, and a slice borrow points to either; a function's handlers arrive as hidden arguments after the others,
and a `dyn` borrow carries the type of what it points to, which chooses the methods called through
it; errors return through the error edge of the call that raised them
(a test, or a function the interpreter is asked to run, that an error leaves reports
`Stop::Failed`); `int`, `bool`, `()`, tuple, struct and enum values, borrows as
pointers into frames, places with projections, generic code run once for every type (each frame
has its type arguments, and a trait's method is found through a witness table built per type on
first use), drops (a moved-from place is empty, which is its drop flag, and destructors run as
calls), calls on its own frame stack (so deep recursion is a `stack overflow` trap at 10,000
frames, never a host crash), and every trap class. A failed `assert` returns the value of each part
of its condition with the trap. It runs every M0 slice, since no code-generation backend exists
until M1.

```rust
moddil_interp::call_fn(&db, file, index, &args, &mut |_, _| {})   // -> Result<Value, Trap>
moddil_interp::run_test(&db, file, index, &mut |range, value| …) // observed values
```

## Used for

- Running programs and tests before any backend exists.
- **Compile-time evaluation** ([`specs/comptime.md`](../../specs/comptime.md)).
- The **Miri-style `unsafe` checker** ([`specs/unsafe.md`: Checking](../../specs/unsafe.md#checking)).
- The **playground** ([`specs/toolchain.md`](../../specs/toolchain.md)).
- A deterministic scheduler for tasks (slice S9), the start of `moddil/runtime-sim`.

Not to be confused with the reference interpreter in [`refinterp/`](../refinterp/README.md).

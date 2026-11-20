# moddil-interp

The **MIR interpreter**, part of the compiler. It runs *analysed* or *runtime* MIR and never needs
target lowering ([`specs/mir.md`: Pipeline](../../specs/mir.md#pipeline)).

**Status:** slice S0: `int`, `bool` and `()` values, calls on its own frame stack (so deep recursion
is a `stack overflow` trap at 10,000 frames, never a host crash), and every trap class. It runs
every M0 slice, since no code-generation backend exists until M1.

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

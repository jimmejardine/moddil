# Numbers

```rust
let a: u8 = 250;
a + 5                          // 255
a + 10                         // traps: overflow at u8's width
a +% 10                        // 4: wrapping is written
let x: f32 = 0.1;              // the literal is an f32
let n: int = a;                // u8 widens to int: every value fits
let b = n.to::<i8>()?;         // fails with RangeError: 250 is not an i8
```

The sized integers and the floats ([`specs/numerics.md`](../../specs/numerics.md)). `int`
([Types](types.md)) is one of the integer types.

**num.types.1** The integer types are `i8`, `i16`, `i32`, `i64` (which is `int`) and `i128`,
signed two's-complement, and `u8`, `u16`, `u32`, `u64` and `u128`, unsigned; each has the width
its name says on every target. The float types are `f32` and `f64`, IEEE 754's binary32 and
binary64. Each is `Copy`, and has `Clone`, `PartialEq`, `PartialOrd`, `Display`, `Debug` and
`Default` (zero); the integers are also `Eq` and `Ord`, the floats not, as NaN is unordered.

**num.lit.1** A literal, and an operator whose operands are all literals (or such operations,
in parentheses), is an exact number until it gets a type, so `200 + 100` is `300` and `0.1 * 3.0`
is `0.3` exactly; a shift, a quotient of fractional literals, and anything that would overflow
`i128`, are left to the type. It gets its type from what it is used as (an
annotation, a parameter, a field, an operand of another number), or else `int` for an integer
literal and `f64` for a fractional one (`1.5`, `2e-3`). An integer literal may be of any number
type, a fractional one of a float type; there are no suffixes. An integer literal must fit its type
(E0204); a fractional one is rounded to the nearest value of its type, once.

**num.ops.1** The arithmetic operators (`+ - * / %`), the comparisons and, on the integers, the
bitwise operators and shifts apply to two operands of one number type (E0201), giving that type.
On the integers they trap as `int`'s do ([Arithmetic](expressions/arithmetic.md)), at the type's
width: on overflow, on division by zero, and on a shift by a negative amount or by the width or
more; `-` applies to the signed integers and the floats, `!` to the integers (E0205). On the
floats they follow IEEE 754: division by zero gives an infinity or NaN, nothing traps, `==` is
false for NaN and true for `-0.0 == 0.0`, and a float has no bitwise operators (E0205).

**num.wrap.1** `a +% b`, `a -% b` and `a *% b` (and `+%=`, `-%=`, `*%=`) on integers wrap around
the type's width instead of trapping: the result is the exact one modulo 2ⁿ, as the type holds it.

**num.widen.1** A number converts implicitly where a number of another type is expected only if
that type holds every value of its own exactly: a signed integer to a wider signed one, an unsigned
integer to a wider unsigned or a strictly wider signed one, `f32` to `f64`, and an integer to a
float whose significand holds it (up to 16 bits for `f32`, 32 for `f64`). Every other conversion
is written (**num.convert.1**); there is no `as`.

**num.convert.1** On every number type, with `T` a number type: `x.to::<T>()` is `x` as a `T`,
failing with the prelude's `RangeError` if `x` is not one exactly (an integer outside `T`'s range,
a float that is not a whole number in it, or a value a float `T` cannot hold exactly);
`x.checked_to::<T>()` gives it as an `Option`. On the integers, `x.truncate::<T>()` keeps `x`'s low
bits, `x.wrap::<T>()` reads them as two's complement, and `x.saturate::<T>()` is the nearest value
of `T`.

**num.round.1** `f.round::<T>(mode)`, on a float, is the whole number `mode` rounds `f` to, as a
`T` (as `to` gives it, so it fails with `RangeError` if it is not one): `Round::NearestEven` and
`Round::NearestAway` round to the nearest, a tie to the even one or away from zero; `Round::Down`
and `Round::Up` round toward −∞ and +∞; and `Round::TowardZero` drops the fraction. NaN and the
infinities fail.

**num.float.1** A float's value is written as the shortest decimal that reads back to it: `0.3`,
`0.30000000000000004`, `1.0`, `inf`, `NaN`.

**num.float.fn.1** The floats have the methods `sqrt`, `sin`, `cos`, `tan`, `asin`, `acos`,
`atan`, `atan2`, `exp`, `ln`, `log2`, `log10`, `powf`, `powi`, `abs`, `floor`, `ceil`, `trunc`,
`fract`, `min`, `max`, `clamp`, `is_nan` and `is_finite`. They give the same bits on every
machine; `sqrt` is correctly rounded, as IEEE 754 requires, and the others are within an ulp or so,
until correctly rounded ones replace them.

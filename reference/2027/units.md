# Units of measure

```rust
dimension Length;
dimension Time;

unit m: Length;
unit cm = m / 100;
unit km = 1000 * m;
unit s: Time;
unit h = 3600 * s;

fn speed(d: f64[km], t: f64[h]) -> f64[km/h] { d / t }

let length: f64[m] = 100.0[cm] + 50.5[mm];      // 1.0505
let v = speed(120.0[km], 2.0[h]);                 // 60.0, a f64[km/h]
let a = 1[m] + 5[cm];                             // 105, an int[cm]
```

A **quantity** is a number with a unit, which is part of its type: `f64[m]`, `int[ns]`,
`Fixed<4>[USD]`. Units exist only while a program is checked. A quantity is its number at run
time, and a conversion between two units of one dimension is a multiplication by a constant
(`specs/units.md`).

**unit.dim.1** A `dimension` item declares a base dimension: `dimension Length;`. A dimension's
identity is its item, so two mods' `Pixel` are two dimensions.

**unit.dim.2** `dimension Velocity = Length / Time;` names a product of dimensions, each to a
rational power. A dimension is such a product of base dimensions. The product with no base
dimension left is dimensionless.

**unit.decl.1** A `unit` item declares a unit. `unit m: Length;` gives a dimension a unit of
scale one: its base unit. `unit cm = m / 100;` defines a unit by others and numbers: its dimension
is theirs, and its scale (how many of its dimension's base unit it is) is their product.

**unit.decl.2** A definition's numbers are decimal (`0.3048`, `1e-3`) and exact. Its operators are
`*` and `/`, and `^` with an integer or a rational exponent (`m^-1`, `Hz^(1/2)`). A unit is not
defined in terms of itself, and every name in a definition is a unit (E0223).

**unit.decl.3** `unit degC = K + 273.15;` declares an affine unit: its zero is 273.15 of `K`. An
affine unit is not part of another unit's definition (E0223).

**unit.decl.4** Units and dimensions are items: they are named and imported as types are
(`use si::{m, cm};`), with the same visibility. A unit is not a type (E0102).

**unit.type.1** `T[u]` is a quantity of number type `T` in unit `u`: `T` is a number type, a
struct or enum (`Fixed<4>`), or a type parameter (E0229 otherwise). Its values are `T`'s.

**unit.type.2** A unit is a product of units and unit parameters to rational powers, written with
`*`, `/` and `^` (`m/s^2`, `kg*m^2/s^2`, `Hz^(1/2)`). Products are normalised: `m^(1/2) * m^(1/2)`
is `m`, and `m/m` is no unit, so `f64[m/m]` is `f64`.

**unit.type.3** Two quantity types are the same type when their units are the same product of
the same units. Two units of one dimension (`m`, `cm`) are different units, which convert
(`unit.conv.1`).

**unit.lit.1** `100[cm]` is a number literal with a unit: a quantity of the literal's type in that
unit. Arithmetic on unit literals is not folded as plain literals' is (`num.lit.1`); it is the
quantities'.

**unit.param.1** A type parameter written in a unit (`fn sqr<U>(x: f64[U]) -> f64[U^2]`) is a unit
parameter: its arguments are units. It is used only in units (E0102).

**unit.param.2** A unit parameter's argument is inferred from the arguments, or given (`to_unit::<cm>`).
A plain number where `f64[U]` is wanted gives `U` no unit.

**unit.conv.1** A quantity where another unit of its dimension is wanted is converted to it:
passed, returned, stored, or assigned. A float converts by its units' scale ratio. An integer
converts only when the ratio is a whole number (`cm` to `mm`): otherwise it converts with
`to_unit` and a rounding mode (E0225).

**unit.conv.2** A value converts by its units' scale ratio `n / d`, exactly as a constant: an
integer multiplied by `n` (trapping on overflow, as any multiplication does); a float multiplied
by `n`, divided by `d` when `n` is one, or else multiplied by `n / d` rounded to its type. An
affine unit's offset is then added.

**unit.conv.3** Different dimensions never convert (E0224). Neither does a quantity of a type
without built-in arithmetic, other than between units of the same scale (E0226).

**unit.op.add.1** `+`, `-`, `%` and the comparisons take two quantities of one dimension (E0224),
or two plain numbers. Two different units of it convert first: to the unit a type gives the sum
(`let x: f64[m] = a + b`), or else to the finer of the two, so `1[m] + 5[cm]` is `105[cm]`. A
comparison chain compares all its operands in its finest unit.

**unit.op.mul.1** `*` and `/` multiply and divide units: `f64[m] / f64[s]` is `f64[m/s]`, and
`2.0 * 3.0[m]` is `6.0[m]`.

**unit.op.mul.2** A quotient of two units of one dimension converts to the finer first, and is a
plain number: `1.0[m] / 5.0[cm]` is `20.0`. A product with no dimension left is a plain number,
converted by its scale: `3.0[m] * 2.0[km^-1]` is `0.006`.

**unit.op.3** `-x` of a quantity is of its unit. The bitwise operators and shifts are not defined
on quantities (E0227).

**unit.affine.1** A value of an affine unit is a point. A point and a difference (a quantity of
the dimension in a unit that is not affine) add, and subtract, to a point; two points subtract to
a difference in the dimension's base unit. Two points do not add, and a point is not multiplied
or divided (E0228).

**unit.affine.2** Converting a point to or from an affine unit applies the offsets:
`20.0[degC]` as `K` is `293.15`. Comparisons compare points as points.

**unit.trait.1** A quantity has its base's `Copy`, `Clone`, `PartialEq`, `Eq`, `PartialOrd`,
`Ord`, `Hash`, `Debug`, `Display` and `Default`, which compare and show it as its number.

**unit.method.1** A quantity has the methods of its number type that keep its unit (`abs`, `floor`,
`ceil`, `trunc`, `fract`, `min`, `max`, `clamp`, whose other operands are in its unit), those that
convert it to another number type in its unit (`round::<T>`, `to::<T>`), and `is_nan`,
`is_finite` and `signum`. Others are not defined on it (E0227).

**unit.method.2** `sqrt` gives the unit to the power one half, and `powi(n)` with a literal `n`
the unit to the power `n`.

**unit.to.1** `x.to_unit::<u>()` is `x` in unit `u` of its dimension, and `x.value_in::<u>()` its
number in `u`. An `int` quantity converts to a unit it is not a whole number of with a rounding
mode: `250[cm].to_unit::<m>(Round::NearestEven)` is `2[m]`.

**unit.with.1** `x.with_unit::<u>()` is plain number `x` as a quantity in `u`.

**unit.angle.1** The prelude declares `dimension Angle;` and its base unit `rad`. `sin`, `cos`
and `tan` take a quantity of an angle in any of its units, converted to radians, and give a plain
number.

**unit.angle.2** `x.per_radian()` is `x` with the angles in its unit converted to radians and
dropped: `f64[deg/s]` gives `f64[s^-1]`.

**unit.erase.1** Units are erased: a quantity's values, layout and operations are its base's, and
a function generic over a unit parameter is one function. A quantity shows, as a test observes it
or `{x}` prints it, as its number.

**unit.fmt.1** In a format spec, `[u]` after the rest (`{x:[mm]}`, `{x:.2[km]}`) writes a quantity
in unit `u` of its dimension (E0224), converted, then a space and `u` as written. The rest of the
spec applies to the number.

**unit.parse.1** `s.parse_quantity::<T[u]>()`, for `T` `f64` or `int`, reads text `s` as a number
and a unit's name (`12.5 km`, `250cm`): `Ok` of the quantity in `u` if the name is a unit of `u`'s
dimension the program declares, converted as `unit.conv.2` says (an `int` only by whole factors),
and `Err(UnitParseError)` otherwise.

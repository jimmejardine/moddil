# Units of Measure

> **Status:** complete: decided, with no open questions. Expands [Numerics and data](numerics.md). Snippets
> are **illustration only**.

## Summary

```rust
use moddil::units::si::{m, cm, mm, s, ms, kg};
use moddil::units::information::MiB;

let length: f64[m] = 100[cm] + 50.5[mm];    // 1.0505 m
let g              = 9.81[m/s^2];
let timeout        = 500[ms];
let budget         = 64[MiB];
```

- Units are part of the **type**: `f64[m]`, `i64[ns]`, `Fixed<4>[USD]`. Literals use the same syntax:
  `100[cm]`.
- **Units are written in square brackets (decided)**, after the number or the type: `100[cm]`,
  `f64[m]`, `Fixed<4>[USD]`, `Simd<f32, 8>[m]`, `BigDecimal[USD]`. Angle brackets stay for generic
  arguments only ([Why square brackets](#why-square-brackets)).
- **Zero runtime cost:** units are checked at compile time and erased; conversion factors are folded
  in at compile time.
- The set of dimensions and units is **open**: any mod can declare new ones.
- Related units convert automatically (`cm` + `mm`); different dimensions never do.

## Why square brackets

Moddil keeps F#'s model of units and changes only its brackets.

**What comes from F#** (Andrew Kennedy's design), which treats units as types:

- **Units are types, not values or functions.** `[<Measure>] type cm` declares a measure. `float<cm>`
  is a type and `100.0<cm>` is a literal of that type, so a literal reads exactly like its type.
- **Any unit expression inline:** `9.81<m/s^2>`, `1.0<kg m/s^2>`, `<m^-1>`. There is no need to name
  every combination or import a function for each.
- **Unit polymorphism:** `let sqr (x: float<'u>) : float<'u^2> = x * x`. Unit variables are inferred
  like generic types.
- **Open set:** any code can declare a new measure. Units form a free abelian group over whatever base
  measures exist (multiplying adds exponents, dividing subtracts them).
- **Erased at runtime.**

F#'s weaknesses, which Moddil addresses ([Declaring units](#declaring-units)):
- `cm` and `m` are unrelated measures; you convert by multiplying by constants such as `100.0<cm/m>`.
- No affine units (°C vs K).

**Decided: units are written in square brackets**, the one syntax for types and literals:

```rust
let length: f64[m] = 100[cm] + 50.5[mm];
let price: Fixed<4>[USD] = 101.2550[USD];
fn mean<U>(xs: &[f64[U]]) -> f64[U] { … }
let t = x * 1[s];                         // a unit on a computed value
```

- **The engineering convention:** tables, plots and datasheets already write "length [m]".
- **No whitespace-sensitive lexing.** Indexing a numeric literal means nothing, so `[…]` after a
  literal is always a unit; F#'s `<…>` would make `100<cm>` and `100 < cm` differ only by spaces.
- **Generics keep `<…>` to themselves.** A type's generic arguments and its unit never share a
  bracket: `Fixed<4>[USD]` and `Vec<f64[m]>`, and `Foo<m>` is always a type argument.
- **No clash with arrays and slices**, whose bracket comes before the type (`[f64; 3]`, `[f64]`).
- **Safe in Markdown**, unlike a backtick form, so documentation and agents can quote unit literals
  in inline code.
- **One form only** ([LLMs as first-class programmers](agent-interface.md)): there is no suffix form
  such as `100cm`, which would also clash with identifiers and hex digits.

## Dimensions: the SI vector and an open set

**The SI vector.** A dimension can be represented as a vector of exponents over the seven SI base
dimensions: length L, mass M, time T, electric current I, temperature Θ, amount of substance N,
luminous intensity J.

| Quantity | Dimension | Vector (L, M, T, I, Θ, N, J) |
|---|---|---|
| Velocity | L T⁻¹ | (1, 0, −1, 0, 0, 0, 0) |
| Force | M L T⁻² | (1, 1, −2, 0, 0, 0, 0) |
| Energy | M L² T⁻² | (2, 1, −2, 0, 0, 0, 0) |

Multiplying quantities adds vectors, dividing subtracts them, and `sqrt` halves them. Boost.Units,
Rust's `uom` and C++ mp-units work this way.

**Its limitation:** the basis is fixed. Currency, information (bits and bytes), pixels, "orders" or
"items" cannot be added without changing the library.

**Decided: an open set.** A dimension is a **sparse map from declared base dimensions to
exponents**: F#'s free abelian group, rather than a fixed seven-element vector.

- Any mod can declare a base dimension:
  ```
  dimension Length;
  dimension Currency;
  dimension Information;
  dimension Pixel;
  ```
- A dimension's identity is its fully qualified name, so two mods' `Pixel` never collide.
- The SI dimensions are simply the first such declarations, in `moddil/units/si`.
- Derived dimensions are expressions: `dimension Velocity = Length / Time`.

## Declaring units

A unit has a dimension and a scale relative to that dimension's base unit, plus an offset for affine
units:

```
unit m:  Length;              // base unit for Length
unit cm = m / 100;
unit mm = m / 1000;
unit km = 1000 * m;
unit s:  Time;
unit ms = s / 1000;
unit K:  Temperature;
unit degC = K + 273.15;       // affine
unit B:  Information;
unit MiB = 1048576 * B;
unit USD: Currency::USD;      // each currency is its own base dimension (see below)
```

**Unit expressions (decided)** in declarations use explicit operators over unit names: `*` and `/`
for scale (`1000 * m`, `m / 100`), `^` for powers, and `+` for an affine offset (`K + 273.15`).
There is no juxtaposition, so `1000 m` is not a unit expression.

Because `cm` and `m` are declared as related, they **convert automatically**, fixing F#'s main
weakness.

## Arithmetic, conversion and polymorphism

- `+`, `-` and comparisons require the **same dimension**. Units within a dimension are converted to
  a common unit: the expected type's unit if annotated, otherwise **the finer of the two**
  (decided): `1[m] + 5[cm]` is `105[cm]`. That is exact for integers and does not depend on the order
  of the operands.
- `*` and `/` combine dimensions: `f64[m] / f64[s]` is `f64[m/s]`. A dimensionless result is a plain
  number.
- **Conversion rules:**
  - floating-point representations convert implicitly;
  - integer representations convert implicitly only when exact (`cm` to `mm`); otherwise the
    conversion is explicit, with a rounding mode: `.to_unit::<cm>(Round::HalfEven)` (decided);
  - fractional integer quantities use fixed point, `Fixed<1>[cm]`; units never carry a scale of their
    own ([`numerics.md`: Decimals for money](numerics.md#decimals-for-money));
  - different dimensions never convert.
- **Rational exponents (decided):** exponents are fractions, as in F#, so `sqrt(f64[m^2])` is `f64[m]`
  and `sqrt(f64[Hz])` is `f64[Hz^(1/2)]` (noise densities such as `V/Hz^(1/2)` can be written).
  Exponents are normalised, so `m^(1/2) * m^(1/2)` is `m`.
- **Angles are their own dimension (decided).** `Angle` is a base dimension with `rad`, `deg` and
  `turn`, in `moddil/units/angle`. `sin` and `cos` accept any angle unit and convert, so passing
  degrees where radians are meant is a type error. Where the dimensions cancel (`rad/s * s`) the
  result is plain; where a formula needs a bare number, `.per_radian()` says so explicitly.
- **Currencies:** each currency is its own base dimension, so `USD` and `EUR` never mix. Conversion
  needs an explicit exchange rate of type `Fixed<6>[EUR/USD]`.
- **Points and differences (affine):** absolute temperatures and timestamps are distinct from
  differences. `Instant - Instant = Duration`; adding two `degC` points is an error; `degC` to `K`
  applies the offset to points but not to differences.
- **Unit polymorphism**, as in F#:
  ```rust
  fn sqr<U>(x: f64[U]) -> f64[U^2] { x * x }
  fn mean<U>(xs: &[f64[U]]) -> f64[U] { … }
  ```
- **Formatting and parsing:** `fmt"{length:[mm]}"` prints in the requested unit; parsing
  `"12.5 km"` returns a `Result`.

## Core and mods

Per R2 (small core):

| Core | Mods |
|---|---|
| Unit expressions in types and literals (`[m/s^2]`) | `dimension` and `unit` declarations |
| Unit algebra and inference in the type checker (free abelian group arithmetic, scale and offset constants) | `moddil/units/si`, `/information`, `/currency`, `/angle`, community packs |
| Erasure: units have no runtime representation | Formatting, parsing, conversions with explicit rounding |

Unit algebra has to be in the core because unit variables are inferred by the type checker. It is a
small piece: normalising products of base dimensions with rational exponents.

## Imports

Unit names are imported like types:

```rust
use moddil::units::si::{m, cm, mm, s, ms};
```

A whole unit pack can be imported through its declared prelude
(`use moddil::units::si::prelude::*`). Other glob imports are not allowed
([`modules.md`: Imports](modules.md#imports)).

## Across tiers

Wire contracts ([Distributed and multi-tier programs](distribution.md)) carry units, so a unit mismatch between services is a compile
error. The 1999 Mars Climate Orbiter was lost to exactly this: one system produced pound-force
seconds, the other expected newton-seconds.

## Fit with the reference examples

| Example | Units |
|---|---|
| [Game](examples/multiplayer-game.md) | Metres, seconds, radians, `m/s^2` in physics shared by client and server |
| [HFT](examples/hft-platform.md) | `Fixed<4>[USD]` prices, quantities, notional (`Price * Qty`); `i64[ns]` latencies |
| [Web store](examples/web-store.md) | Money per currency; shipping weights and dimensions |
| [Hashiverse](examples/hashiverse.md) | `500[ms]` durations and `64[MiB]` sizes, where today raw integer types (`TimeMillis`, `DurationMillis`) are used |

## Open questions

None at present.


## Prior art

- F# units of measure; Andrew Kennedy, *Programming Languages and Dimensions* (PhD thesis, 1996) and
  *Types for Units-of-Measure: Theory and Practice* (2010).
- C++ mp-units; Boost.Units.
- Rust `uom`.
- Frink.
- NASA, *Mars Climate Orbiter Mishap Investigation Board Phase I Report* (1999).

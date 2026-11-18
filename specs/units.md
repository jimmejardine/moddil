# Units of Measure

> **Status:** proposal. Expands [`rationale.md`](rationale.md) §2.5. Recommendations are marked
> **Recommended**; syntax follows the recommendation but is not final.

## 1. Summary

```
import moddil/units/si.{m, cm, mm, s, ms, kg}
import moddil/units/information.{MiB}

let length: f64<m> = 100<cm> + 50.5<mm>     // 1.0505 m
let g              = 9.81<m/s^2>
let timeout        = 500<ms>
let budget         = 64<MiB>
```

- Units are part of the **type**: `f64<m>`, `i64<ns>`, `Decimal<USD>`. Literals use the same syntax:
  `100<cm>`.
- **Zero runtime cost:** units are checked at compile time and erased; conversion factors are folded
  in at compile time.
- The set of dimensions and units is **open**: any mod can declare new ones.
- Related units convert automatically (`cm` + `mm`); different dimensions never do.

## 2. Why F#'s `100.0<cm>` syntax

F# (Andrew Kennedy's design) treats units as types:

- **Units are types, not values or functions.** `[<Measure>] type cm` declares a measure. `float<cm>`
  is a type and `100.0<cm>` is a literal of that type, so a literal reads exactly like its type.
- **Any unit expression inline:** `9.81<m/s^2>`, `1.0<kg m/s^2>`, `<m^-1>`. There is no need to name
  every combination or import a function for each.
- **Unit polymorphism:** `let sqr (x: float<'u>) : float<'u^2> = x * x`. Unit variables are inferred
  like generic types.
- **Open set:** any code can declare a new measure. Units form a free abelian group over whatever base
  measures exist (multiplying adds exponents, dividing subtracts them).
- **Unambiguous lexing:** no clash with identifiers, hex digits or numeric type suffixes (which a
  suffix form such as `100cm` would have).
- **Erased at runtime.**

F#'s weaknesses, which Moddil addresses (§4):
- `cm` and `m` are unrelated measures; you convert by multiplying by constants such as `100.0<cm/m>`.
- No affine units (°C vs K).

**Recommended:** adopt the F# form as Moddil's single syntax for units in types and literals. It is
consistent with Moddil's generic syntax and gives compound units for free. One form only (rationale
§2.18); the suffix form `100cm` is not used.

**Lexing rule:** a `<` immediately after a numeric literal (no space) starts a unit. The canonical
formatter always puts spaces around the comparison operator, so `100 < limit` is never ambiguous.
Swift uses similar whitespace-sensitive operator lexing.

## 3. Dimensions: the SI vector and an open set

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

**Recommended: an open set.** A dimension is a **sparse map from declared base dimensions to
exponents**: F#'s free abelian group, rather than a fixed seven-element vector.

- Any mod can declare a base dimension:
  ```
  dimension Length
  dimension Currency
  dimension Information
  dimension Pixel
  ```
- A dimension's identity is its fully qualified name, so two mods' `Pixel` never collide.
- The SI dimensions are simply the first such declarations, in `moddil/units/si`.
- Derived dimensions are expressions: `dimension Velocity = Length / Time`.

## 4. Declaring units

A unit has a dimension and a scale relative to that dimension's base unit, plus an offset for affine
units:

```
unit m:  Length               // base unit for Length
unit cm = m / 100
unit mm = m / 1000
unit km = 1000 m
unit s:  Time
unit ms = s / 1000
unit K:  Temperature
unit degC = K + 273.15        // affine
unit B:  Information
unit MiB = 1048576 B
unit USD: Currency.USD        // each currency is its own base dimension (§5)
```

Because `cm` and `m` are declared as related, they **convert automatically**, fixing F#'s main
weakness.

## 5. Arithmetic, conversion and polymorphism

- `+`, `-` and comparisons require the **same dimension**. Units within a dimension are converted to
  a common unit: the expected type's unit if annotated, otherwise the finer of the two.
- `*` and `/` combine dimensions: `f64<m> / f64<s>` is `f64<m/s>`. A dimensionless result is a plain
  number.
- **Conversion rules:**
  - floating-point representations convert implicitly;
  - integer representations convert implicitly only when exact (`cm` to `mm`); otherwise the
    conversion is explicit, with a rounding mode;
  - different dimensions never convert.
- **Currencies:** each currency is its own base dimension, so `USD` and `EUR` never mix. Conversion
  needs an explicit exchange rate of type `Decimal<EUR/USD>`.
- **Points and differences (affine):** absolute temperatures and timestamps are distinct from
  differences. `Instant - Instant = Duration`; adding two `degC` points is an error; `degC` to `K`
  applies the offset to points but not to differences.
- **Unit polymorphism**, as in F#:
  ```
  fn sqr<u>(x: f64<u>) -> f64<u^2> { x * x }
  fn mean<u>(xs: let [f64<u>]) -> f64<u> { … }
  ```
- **Formatting and parsing:** `fmt"{length:<mm>}"` prints in the requested unit; parsing
  `"12.5 km"` returns a `Result`.

## 6. Core and mods

Per R2 (small core):

| Core | Mods |
|---|---|
| Unit expressions in types and literals (`<m/s^2>`) | `dimension` and `unit` declarations |
| Unit algebra and inference in the type checker (free abelian group arithmetic, scale and offset constants) | `moddil/units/si`, `/information`, `/currency`, `/angle`, community packs |
| Erasure: units have no runtime representation | Formatting, parsing, conversions with explicit rounding |

Unit algebra has to be in the core because unit variables are inferred by the type checker. It is a
small piece: normalising products of base dimensions with integer exponents.

## 7. Imports

Unit names are imported like types:

```
import moddil/units/si.{m, cm, mm, s, ms}
```

*Open:* whether a whole unit pack can be imported at once (`import moddil/units/si.*`), given that
glob imports otherwise hurt local reasoning (rationale §2.18).

## 8. Across tiers

Wire contracts (rationale §2.10) carry units, so a unit mismatch between services is a compile
error. The 1999 Mars Climate Orbiter was lost to exactly this: one system produced pound-force
seconds, the other expected newton-seconds.

## 9. Fit with the reference examples

| Example | Units |
|---|---|
| [Game](examples/01-multiplayer-game.md) | Metres, seconds, radians, `m/s^2` in physics shared by client and server |
| [HFT](examples/02-hft-platform.md) | `Decimal<USD>` prices, quantities, notional (`Price * Qty`); `i64<ns>` latencies |
| [Web store](examples/03-web-store.md) | Money per currency; shipping weights and dimensions |
| [Hashiverse](examples/04-hashiverse.md) | `500<ms>` durations and `64<MiB>` sizes, where today raw integer types (`TimeMillis`, `DurationMillis`) are used |

## 10. Open questions

1. Confirm the lexing rule: no space between a literal and its `<unit>`.
2. Glob imports of unit packs (§7).
3. Angles: are radians and degrees units of a dimensionless-but-distinct "angle" kind?
4. Rational exponents (`m^(1/2)`), needed for some formulas: allow, or only integers?
5. The common-unit rule for unannotated mixed expressions: finer unit, or left operand?
6. Integer representations with fractional scales (for example `i64<mm>` from `cm`): exact-only
   conversion, or a fixed-point scale in the type?

## Prior art

- F# units of measure; Andrew Kennedy, *Programming Languages and Dimensions* (PhD thesis, 1996) and
  *Types for Units-of-Measure: Theory and Practice* (2010).
- C++ mp-units; Boost.Units.
- Rust `uom`.
- Frink.
- NASA, *Mars Climate Orbiter Mishap Investigation Board Phase I Report* (1999).

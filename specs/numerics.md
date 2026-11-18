# Numerics and Data

> **Status:** decided direction, open questions remain. Indexed in [`README.md`](README.md).
> Units of measure are a separate proposal in [`units.md`](units.md). Snippets are **illustration only**.

## Summary

| Rule | One line |
|---|---|
| **Overflow traps** | In every build, as in Swift. Wrapping is explicit: `+%`, `-%`, `*%` or `Wrapping<T>`. |
| **`int` for lengths and indices** | A signed 64-bit `int` on every target; `len - 1` on an empty list is `-1`, not 2^64−1. |
| **Exact literals** | A literal is an exact number until it is used, then checked to fit: `300` into a `u8` is a compile error; `0.10` into a decimal is exact. |
| **Lossless widening only** | `i32 → i64` and `f32 → f64` are implicit; anything that can lose information is an explicit, named conversion. No `as`. |
| **Strict IEEE floats** | `f16`, `bf16`, `f32`, `f64`; IEEE comparisons, `total_cmp` for sorting, a `Finite` refinement that is totally ordered. |
| **Decimals for money** | `Fixed<S>`, an i64 scaled by 10^S with the scale in the type, and `BigDecimal`, in `moddil/decimal`. |
| **SIMD in the core** | `Simd<T, N>` and masks are core primitives; tensors are a mod. |
| **Layout under control** | Field reordering unless `#[repr(C)]`; packed structs and bitfields; endian types (`u32be`); a comptime struct-of-arrays container. |

## Problem

- **Silent overflow**: C's signed overflow is undefined behaviour; Rust wraps in release builds;
  most languages' integers silently produce wrong answers.
- **Unsigned indices**: in Rust and C++, `len - 1` on an empty collection wraps to a huge number.
- **Lossy implicit conversions**: C silently truncates and changes sign.
- **Float literal surprises**: `0.1` is not 0.1; money stored in floats loses cents.
- **NaN breaks ordering**: in Rust, `f64` can't be sorted with `sort()`.
- **Layout**: performance code needs struct-of-arrays, packed wire formats and explicit endianness.

## State of the art

| Topic | Approaches |
|---|---|
| **Overflow** | **Swift** traps in every build (`&+` wraps). **Rust** traps in debug and wraps in release. **Zig** traps in safe modes (`+%` wraps). **Carbon** plans to trap. **C/C++**: signed overflow is undefined. |
| **Literals** | **Go**'s untyped constants are exact (arbitrary precision) until used; **Zig** has `comptime_int` and `comptime_float`; **Rust** types literals immediately. |
| **Conversions** | **Rust**: none implicit (`as` truncates silently). **C#, Zig, Kotlin**: implicit lossless widening. **C**: implicit and lossy. |
| **Index type** | **Rust, C++**: unsigned `usize`. **Swift, Go, Carbon, Java**: signed `Int`. |
| **Float ordering** | IEEE 754 defines a `totalOrder` predicate; Rust exposes it as `total_cmp`. |
| **Decimals** | **C#** `decimal` (128-bit); **Java** `BigDecimal`; IEEE 754-2008 decimal128; scaled integers (fixed point) in trading systems. |
| **Layout** | **Zig** `MultiArrayList`; **Jai** built-in SoA; Rust `#[repr(C)]`, `#[repr(packed)]`. |

## Decisions

### Integer types

- `i8`–`i128` and `u8`–`u128`.
- **`int` is i64 on every target**. It is the type of lengths, indices and counts, and the default
  for an integer literal with no other constraint.
- `usize` and `isize` exist only for FFI and memory-level code (allocators, pointer arithmetic in
  `unsafe`), and a lint flags them in cross-target code ([`determinism.md`: Sources of nondeterminism](determinism.md#sources-of-nondeterminism)).
- **Arbitrary widths** such as `u3` and `i24`, as in Zig ([`freestanding.md`: Hardware access](freestanding.md#hardware-access)).

### Overflow traps

- `+`, `-`, `*`, negation and shifts **trap on overflow in every build**: debug, release and
  every target behave the same. A trap is class 3 in [`error-handling.md`: Three outcome classes](error-handling.md#three-outcome-classes)
  (it terminates the task or process with a trace).
- Negating `MIN` traps; a shift amount equal to or larger than the width traps.
- **Wrapping is explicit**, for hashes, checksums, crypto and sequence numbers:

  ```rust
  let h = h *% 0x100000001b3 ^ byte          // wrapping operators
  let seq: Wrapping<u32> = last + 1           // or a wrapping type: every operation wraps
  let n = a.checked_add(b)?                   // Option, for "might not fit" logic
  let v = level.saturating_sub(drain)
  ```

- **Cost**: typically 1–5% in integer-heavy code before optimisation. Checks are removed where
  range analysis or refinements prove that overflow can't happen (loop counters bounded by a
  length, values of `type Percent = u8 where self <= 100`).
- **No global opt-out:** no build profile turns overflow checks off; `moddil perf checks` reports the
  ones that remain ([`performance.md`: Checks and their elision](performance.md#checks-and-their-elision)).

### Indices and lengths are signed

```rust
let last = xs.len() - 1        // -1 when xs is empty: an ordinary number
xs[last]                       // traps: index -1 out of bounds (not 18446744073709551615)
for i in (0..xs.len()).rev() { … }
```

- Negative or too-large indices are out of bounds, which is a trap (class 3).
- `int` has the same width everywhere, so index arithmetic behaves identically on wasm32 and
  64-bit targets.

### Exact literals

- A literal is an **exact compile-time number** (arbitrary precision, as Go's constants and Zig's
  `comptime_int`), and so is constant arithmetic on literals.
- It is **checked when it gets a type**:
  - `let b: u8 = 300` is a compile error;
  - `let p: Fixed<2> = 0.10` is exactly ten cents;
  - `let x: f64 = 0.1` rounds to the nearest double, as always.
- Without other constraints, integer literals are `int` and fractional literals are `f64`.
- This is comptime evaluation applied to numbers ([`comptime.md`](comptime.md)).

### Conversions

- **Implicit only when no information can be lost**:
  - `i32 → i64`, `u8 → u32`, `u32 → i64`;
  - `f32 → f64`;
  - `i32 → f64` and `u32 → f64` (exact), but **not** `i64 → f64`, which can round.
- **Everything else is a named, explicit conversion**:

  | Conversion | Meaning |
  |---|---|
  | `x.to::<u8>()` | Checked: fails with `fail(RangeError)` when the value doesn't fit |
  | `x.truncate::<u8>()` | Keeps the low bits |
  | `x.wrap::<i32>()` | Two's-complement reinterpretation |
  | `x.saturate::<u8>()` | Clamps to the range |
  | `f.round::<i64>(mode)` | Float to integer with an explicit rounding mode; NaN and out-of-range values fail |

- There is **no `as` for numbers**: Rust's `as` silently truncates and saturates, which is exactly
  the kind of hidden behaviour Moddil avoids.

### Floats

- `f16`, `bf16`, `f32`, `f64`.
- **Strict IEEE 754** ([`determinism.md`: Sources of nondeterminism](determinism.md#sources-of-nondeterminism)): no implicit FMA, no fast-math
  reordering unless opted into, and a correctly rounded `moddil/math`.
- `==` and `<` follow IEEE (NaN is unequal to everything), so floats are `PartialOrd`, not `Ord`.
- **Sorting** uses `total_cmp` (IEEE `totalOrder`).
- **A finite float** is a refinement: `type Finite = f64 where self.is_finite()`. It implements `Ord`
  and `Hash`, so it can be sorted and used as a map key. Constructing one from an `f64` is checked
  ([`correctness.md`](correctness.md)).
- Float division by zero gives ±∞ or NaN, as IEEE says; only integer division traps.

### Decimals for money

In `moddil/decimal`:
- **`Fixed<S>`**: an i64 holding the value × 10^S. It is exact, fast and allocation-free, which suits
  trading systems.
  - The **scale is part of the type**: adding `Fixed<2>` to `Fixed<4>` is a compile error until one
    is converted.
  - Multiplication widens the scale (`Fixed<2> * Fixed<4> → Fixed<6>`); rescaling takes an explicit
    rounding mode.
  - Overflow traps, like integers.
- **`BigDecimal`**: arbitrary precision with explicit rounding contexts, for accounting.
- Exact literals ([Exact literals](#exact-literals)) make `let price: Fixed<4> = 101.2550` safe.

```rust
let price: Fixed<4> = 101.2550
let qty:   Fixed<0> = 300
let notional = (price * qty).rescale::<2>(Round::HalfEven)   // Fixed<2>
```

### Big numbers

`BigInt` and `Rational` live in a standard mod, `moddil/num`.

### SIMD and tensors

- **`Simd<T, N>` and mask types are core primitives** ([`mods.md`: The core / mod boundary](mods.md#the-core--mod-boundary)), with portable
  lane-wise arithmetic, comparisons and shuffles that lower to the target's vector instructions.
- Higher-level operations live in `moddil/simd`; n-dimensional arrays and tensor expressions in
  `moddil/tensor` ([`accelerators.md`: Level 1: tensor expressions](accelerators.md#level-1-tensor-expressions)).

### Data layout

- **Field order**: the compiler may reorder fields to reduce padding, unless the type is
  `#[repr(C)]`.
- **Packed structs and bitfields** with arbitrary-width integers
  ([`freestanding.md`: Hardware access](freestanding.md#hardware-access)).
- **Endian types** `u16be`, `u32le` and so on, for wire formats and zero-copy views over network
  buffers ([`memory-and-resources.md`: Zero-copy I/O](memory-and-resources.md#zero-copy-io)). Arithmetic converts to the
  native type; the stored bytes are always in the declared order.
- **Struct of arrays**: `Soa<T>` in `moddil/collections` is derived at compile time from `T`'s fields
  (as Zig's `MultiArrayList`), so `orders.price` is one contiguous column. No language feature is
  needed.

## Units of measure

Units are a type-level feature (`f64<m/s>`, `i64<ns>`, `100<cm>`), proposed in
[`units.md`](units.md) with its own open questions.

## Fit check against the reference examples

| Example | Uses |
|---|---|
| [Multiplayer game](examples/01-multiplayer-game.md) | `f32` and `Simd<f32, 8>` for physics; strict IEEE for deterministic rollback |
| [HFT](examples/02-hft-platform.md) | `Fixed<4>` prices; overflow traps in risk limits; `Soa<Order>` for scans; `u64be` exchange formats |
| [Web store](examples/03-web-store.md) | `Fixed<2>` or `BigDecimal` for money; exact literals for prices |
| [Hashiverse](examples/04-hashiverse.md) | `Wrapping<u64>` hashes; `u32be` wire formats; `BigInt` where cryptography needs it |

## Directions (not yet decided)

- Overflow-check elision through range analysis and refinements, with the remaining checks shown
  in `moddil explain`.
- An `f128` type.

## Open questions

- How are units written on parameterised types (`Fixed<4>` in US dollars: `Fixed<4><USD>`?)?
- Should `Fixed` and `BigDecimal` have a default rounding mode, or must it always be stated?
- Should overflow checks ever be disabled per mod or per block (an `unchecked` region), or only
  replaced by explicit `+%`?

## Prior art

Swift integer overflow and `&+`; Zig wrapping operators, `comptime_int` and `MultiArrayList`; Go
untyped constants; Rust `total_cmp` and `Wrapping<T>`; Carbon's integer design; IEEE 754-2019
(`totalOrder`) and IEEE 754-2008 decimal arithmetic; C# `decimal`; Java `BigDecimal`; Jai SoA.

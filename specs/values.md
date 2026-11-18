# Values: Copy, Clone and Equality

> **Status:** decided direction, details open. Listed in the [index](README.md). Builds on
> [`memory.md`](memory.md). Snippets are **illustration only**.

## Summary

| Rule | One line |
|---|---|
| **Moves by default** | `let b = a` moves `a`, unless the type is `Copy`. |
| **Opt-in `Copy`, size-capped** | Only small plain data can be `Copy`, and only when declared: a 4 KB array never copies silently. |
| **`.clone()` is explicit and means a copy** | An independent value with the same contents; its cost grows with the value. |
| **`.share()` for reference counts** | `rc.share()` is an O(1) count increment returning the same object; `Rc<T>` has no `.clone()`. |
| **Explicit derives** | `#[derive(Clone, PartialEq, Eq, Hash, Ord, Debug)]`, structural and visible in diffs. |
| **Rust's equality traits** | `PartialEq`/`Eq` and `PartialOrd`/`Ord`; floats are partial, `Finite` is total. |
| **Equality is never identity** | Addresses are unobservable; `Rc::ptr_eq` is the only identity test. |
| **Seeded hashing and map order** | Keyed hashing seeded from the `rand` effect; maps iterate in seeded hash order; ordered maps are explicit. |

## Problem

- **Hidden copies.** C++ runs copy constructors implicitly; Swift copies every struct implicitly
  (arrays use copy-on-write); Rust lets a `[u8; 4096]` be `Copy`, so `let b = a` can hide a 4 KB
  memcpy. Each contradicts "cost is visible" ([Cross-cutting themes](vision.md#cross-cutting-themes)).
- **One name for two costs.** In Rust, `arc.clone()` is a count increment and `vec.clone()` is a deep
  copy. The reader cannot tell which from the call site.
- **Derive boilerplate vs accidental semantics.** Rust needs a derive list on almost every type;
  automatic structural equality (records, data classes) gives types equality they may not want, for
  example one that ignores a cache field.
- **NaN.** IEEE equality is not reflexive, which breaks hash maps and sorting unless the type system
  separates partial from total equality.
- **Iteration order.** Hash-map order that varies between runs makes tests flaky and replays
  diverge; order that never varies hides order-dependence until the implementation changes.

## State of the art

| Language | Copy and clone | Equality and hashing | Map order |
|---|---|---|---|
| **Rust** | Move by default; opt-in `Copy` of any size; `.clone()` for both `Arc` and `Vec` | Derived `PartialEq`/`Eq`/`Hash`/`Ord`; `Hash` + `Hasher` split | Random seed per map |
| **Rust 2025 ergonomic ref-counting** | Proposes a separate `Handle`/`share` operation for cheap handles | — | — |
| **Swift** | Value types copy implicitly; copy-on-write collections | Declared conformance (`: Hashable`), synthesised body | Per-process random seed |
| **C++** | Implicit copy constructors everywhere | Hand-written or defaulted `operator==`/`<=>` | Unspecified |
| **Mojo** | Values are explicitly copyable or movable | Traits | — |
| **Kotlin data classes, Java/C# records** | Reference types | Automatic structural equality and hashing | Insertion order (`LinkedHashMap`) on request |
| **Go** | Implicit copy of all values | `==` on comparable types | Deliberately randomised |
| **Python 3.7+, JavaScript** | References | Per-type | Insertion order |

## Decisions

### Moves by default

Assignment, argument passing and returning **move** a value: the source can't be used afterwards
([`memory.md`](memory.md)). `Copy` types are the only exception.

### Opt-in `Copy`, with a size cap

- A type is `Copy` only when it **says so** (`#[derive(Copy, Clone)]`).
- Only **plain data** may be `Copy`: no `Drop`, no owned heap data, no `Rc`, no sync cells, not
  linear.
- **Size cap**: a `Copy` type's size must not exceed a cap (proposed: 64 bytes). For generic types
  and arrays the cap is checked per instantiation.

| `Copy` | Not `Copy` |
|---|---|
| Integers, floats, `bool`, `char` | `String`, `Vec<T>`, `Box<T>` |
| `Fixed<S>`, `Simd<f32, 8>`, `[u8; 32]` | `[u8; 4096]` (over the cap: use `.clone()`) |
| `Point { x: f64, y: f64 }`, IDs, small enums | Anything with `Drop`, `Rc`, a sync cell, or a linear type |

- Adding or removing `Copy` changes what callers may do, so it is an API change caught by the
  compiler's semver check ([`packages-and-supply-chain.md`](packages-and-supply-chain.md)).

### `.clone()` is explicit and means a copy

- Duplicating a non-`Copy` value is always an explicit `.clone()`, and it always means **an
  independent value with the same contents**. Its cost grows with the value.
- A derived `Clone` duplicates the **owned** parts field by field and **shares** any `Rc` fields.
  This is safe, and observably the same as copying, because rc contents are immutable
  ([`memory.md`](memory.md)).
- Linear types do not derive `Clone` (a second copy would be a second obligation); they may
  implement it by hand where that makes sense. Sync cells are never `Clone`.

### `.share()` for reference counts

```rust
let shared = node.to_rc()        // Rc<Node>
let other  = shared.share()      // O(1): count + 1; the same node
let deep   = node2.clone()       // independent copy; cost grows with node2
fn render(n: &Rc<Node>) { … }    // borrowing an Rc touches no count
```

- `rc.share()` increments the count and returns another handle to **the same object**.
- `Rc<T>` has **no `.clone()` method**, so reading code tells you the cost: `.share()` is cheap,
  `.clone()` copies.
- `Weak<T>` uses `.share()` too.

### Equality and ordering traits

- **As in Rust**: `PartialEq` (`==`), `Eq` (a marker: equality is reflexive), `PartialOrd` (`<`),
  `Ord` (a total order). LLMs know this model well, and it is needed for floats.
- Floats are `PartialEq` and `PartialOrd` only; the `Finite` refinement is `Eq`, `Ord` and `Hash`
  ([`numerics.md`: Floats](numerics.md#floats)).
- Hash-map keys need `Eq + Hash`; tree-map keys need `Ord`.
- Comparing integers of different types works through lossless widening
  ([`numerics.md`: Conversions](numerics.md#conversions)): `a_i32 == b_i64` compiles; `a_i64 == b_u64`
  does not.

### Explicit derives

```rust
#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
struct OrderId(u64)

#[derive(Clone, PartialEq, Debug)]
struct Quote { price: Fixed<4>, qty: Fixed<0>, venue: VenueId }
```

- Derives are comptime functions returning typed impls ([`comptime.md`](comptime.md)). Nothing is
  derived unless asked for, so a type never gains equality it shouldn't have.
- Derived impls are **structural**, field by field.
- **Derived ordering uses declaration order** for fields and variant order for enums, even when the
  compiler reorders fields in memory ([`numerics.md`: Data layout](numerics.md#data-layout)).
- Generated code is visible through `moddil explain` and in the semantic diff.
- A derived `Debug` prints `Secret<T>` fields as redacted
  ([`observability.md`: Secrets never reach observability](observability.md#secrets-never-reach-observability)).

### Equality is never identity

- Safe code cannot observe addresses
  ([`determinism.md`: Sources of nondeterminism](determinism.md#sources-of-nondeterminism)).
- `==` is always value equality. The only identity test is the explicit `Rc::ptr_eq(&a, &b)`; there
  is no `===`.

### Hashing

- **`Hash` + `Hasher`**, as in Rust: a type says *what* it hashes; the hasher decides *how*.
- The default hasher is **keyed and flood-resistant**, keyed from the `rand` effect: random in
  production, fixed in simulation and replay ([`determinism.md`](determinism.md)).
- A faster, unkeyed hasher is an explicit opt-in for trusted keys.

### Map iteration order: seeded hash order

- The standard `HashMap` iterates in **hash order under its seed**. For a given seed the order is
  deterministic, so simulations and recorded replays reproduce exactly.
- The order is **not stable across seeds**. Test runs pick a fresh seed and print it, as with
  shuffled test order ([`determinism.md`](determinism.md)), so a test that depends on map order fails
  visibly and can be reproduced from the printed seed.
- When order matters, say so: `OrderedMap` (insertion order) and `TreeMap` (sorted) in
  `moddil/collections`.

## Fit check against the reference examples

| Example | Uses |
|---|---|
| [Multiplayer game](examples/01-multiplayer-game.md) | `Copy` vectors and entity IDs; seeded maps that replay identically in rollback |
| [HFT](examples/02-hft-platform.md) | `Copy` order IDs and `Fixed<4>` prices; `.share()` for reference data shared across strategy tasks; no hidden copies on the hot path |
| [Web store](examples/03-web-store.md) | Derived `Eq`/`Hash` on IDs; `OrderedMap` where API output order must be stable |
| [Hashiverse](examples/04-hashiverse.md) | `[u8; 32]` hashes as `Copy` keys; flood-resistant hashing for peer-supplied keys |

## Open questions

- The exact `Copy` size cap, and whether exceeding it is a hard error or a lint that can be
  overridden.
- Should `Copy` values still be moved explicitly in some positions (for example `sink` parameters)?
- Should `Debug` be available without a derive, for test and assertion output?

## Prior art

Rust (`Copy`, `Clone`, `PartialEq`/`Eq`, `Hash`/`Hasher`) and the 2025 ergonomic ref-counting
proposals; Swift synthesised `Equatable`/`Hashable`; Mojo value semantics; Kotlin data classes and
Java records; Python and JavaScript insertion-ordered maps; Go's randomised map iteration; SipHash.

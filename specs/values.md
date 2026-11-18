# Values: copy, clone and equality

> **Status:** not started. Nothing is decided yet; this page records the problem and the
> questions to settle. Listed in the [index](README.md).

## Problem

The theme "cost is visible" ([Cross-Cutting Themes](vision.md#cross-cutting-themes)) has not yet been applied to everyday values: which types copy
implicitly, whether duplicating a value is always explicit, and how equality, hashing and ordering
are defined and derived.

## Touched on in

[`memory-and-resources.md`](memory-and-resources.md),
[`paradigms.md`](paradigms.md), [`numerics.md`: Floats](numerics.md#floats) (float ordering).

## Questions to settle

- Which types copy implicitly (a `Copy`-like rule), and whether `clone` is always explicit.
- Derived `Eq`, `Hash` and `Ord`; structural vs nominal equality.
- Hashing and determinism (seeded hashing, [`determinism.md`: Sources of nondeterminism](determinism.md#sources-of-nondeterminism)).

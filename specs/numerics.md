# Numerics and Data

> **Status:** brainstorm, with integer and float semantics decided in [`error-handling.md`](error-handling.md) §2.
> Expands [`overview.md`](overview.md) §2.5.

## 1. Problem

Silent integer overflow (C), undefined behaviour, float surprises, no units.

## 2. Decided

From [`error-handling.md`](error-handling.md) §2:

- integer arithmetic wraps (two's complement, no UB, no checks);
- floats follow IEEE 754;
- integer division by zero and `MIN / -1` trap, as in Rust;
- explicit checked/saturating operators live in a mod.

## 3. Directions

- Units of measure (F#) as an optional type feature. Expanded in [`units.md`](units.md) (proposal):
  F#-style `100<cm>` and `f64<m/s>`, an open set of dimensions any mod can extend, and units declared
  with a scale (and an offset for affine units) so related units convert automatically.
- Arbitrary-width integers (`u3`, `i24`) and packed bitfields, proposed in
  [`freestanding.md`](freestanding.md) §6.
- SIMD vectors and tensors as first-class types (ties into overview §2.9).
- Arbitrary-precision integers in the standard library.

## 4. Open questions

None recorded yet. Units of measure have their own list in [`units.md`](units.md) §10.

# `unsafe` and the safety boundary

> **Status:** not started. Nothing is decided yet; this page records the problem and the
> questions to settle. Listed in the [index](README.md).

## Problem

`unsafe` is mentioned throughout as a capability and an escape hatch, but its semantics are
undefined: what undefined behaviour remains inside it, what aliasing rules raw pointers must obey,
and what the Miri-style checker actually checks.

## Touched on in

[`memory-and-resources.md`](memory-and-resources.md),
[`interop.md`: The safety boundary](interop.md#the-safety-boundary), [`correctness.md`](correctness.md),
[`effects-and-capabilities.md`: The effect catalogue](effects-and-capabilities.md#the-effect-catalogue).

## Questions to settle

- Which undefined behaviour remains inside `unsafe`, and how it is documented.
- The aliasing model for raw pointers (cf. Rust's Stacked and Tree Borrows).
- What the Miri-style checker checks, and when it runs.
- How `unsafe` code is reviewed and audited across mods.

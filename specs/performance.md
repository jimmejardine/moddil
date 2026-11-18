# Performance model

> **Status:** not started. Nothing is decided yet; this page records the problem and the
> questions to settle. Listed in the [index](README.md).

## Problem

Moddil promises that cost is visible ([`vision.md`: Cross-cutting themes](vision.md#cross-cutting-themes)), and two reference examples
(the HFT platform and the game) live or die by predictable performance. The pieces are spread across
memory, values, concurrency, numerics and accelerators, but there is no single performance model: what
each construct costs, how data is laid out, how hot paths are kept free of allocation and unbounded
latency, and how regressions are caught.

## Touched on in

[`memory-and-resources.md`](memory-and-resources.md), [`values.md`](values.md), [`concurrency.md`](concurrency.md),
[`numerics.md`](numerics.md) (overflow traps and their elision), [`build-times.md`](build-times.md) (debug and release profiles),
[`accelerators.md`](accelerators.md), [`examples/02-hft-platform.md`](examples/02-hft-platform.md),
[`examples/01-multiplayer-game.md`](examples/01-multiplayer-game.md).

## Questions to settle

- A **published cost model**: what each construct compiles to (calls through `dyn`, effects and
  handlers, rc operations, bounds and overflow checks, task switches).
- **Data layout control**: `repr`, field reordering, packing, cache-line alignment, and
  struct-of-arrays layouts without rewriting code.
- **Hot-path guarantees**: code checked never to allocate, lock or perform unbounded work. An effect,
  a capability, or an attribute?
- **Check elision**: which bounds and overflow checks the compiler promises to remove, and how a
  programmer sees the ones that remain.
- **Optimisation control**: inlining hints, PGO and LTO, and how they interact with build caching.
- **Debug-build speed**: how slow may debug builds be (witness tables, checks) before they are unusable
  for games?
- **Benchmarks and regression testing** in CI: a standard harness and noise-resistant comparison.

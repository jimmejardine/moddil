# Modules, visibility and program structure

> **Status:** not started. Nothing is decided yet; this page records the problem and the
> questions to settle. Listed in the [index](README.md).

## Problem

The mod boundary already carries a lot of meaning (explicit effects, capability ceilings, required
groups, semver checks), but the module system itself is still Rust's by default: visibility levels,
how files map to modules, and how code varies by target are undecided.

## Touched on in

[`syntax.md`: Mapping](syntax.md#mapping), [`docs-and-tests.md`: Overviews: mod, module and group docs](docs-and-tests.md#overviews-mod-module-and-group-docs),
[`build-times.md`: Open questions](build-times.md#open-questions), [`mods.md`](mods.md).

## Questions to settle

- Visibility levels (`pub`, `pub(mod)`, private) and what each exposes.
- Are cycles allowed between modules inside one mod?
- Conditional compilation per target and profile, and Cargo-style feature flags.
- The exact meaning of the mod boundary for effects, groups, capabilities and semver. (Its meaning
  for trait impls, including bridge mods, is in [`types.md`: Coherence: one impl per trait and type](types.md#coherence-one-impl-per-trait-and-type).)

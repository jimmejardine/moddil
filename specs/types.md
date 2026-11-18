# Type system core

> **Status:** not started. Nothing is decided yet; this page records the problem and the
> questions to settle. Listed in the [index](README.md).

## Problem

Moddil borrows Rust's traits, generics and sum types, but the core rules of the type system have not
been decided in their own right, and several other areas depend on them: coherence decides whether
two mods can implement the same trait for the same type, and the generics strategy decides both
build times and debug-build speed.

## Touched on in

[`syntax.md`: Mapping](syntax.md#mapping), [`paradigms.md`](paradigms.md),
[`comptime.md`: Generics are bounded](comptime.md#generics-are-bounded), [`build-times.md`: Language rules that decide build speed](build-times.md#language-rules-that-decide-build-speed), [`mods.md`: Open questions](mods.md#open-questions).

## Questions to settle

- Traits and **coherence across mods**: Rust's orphan rule, or something looser?
- Associated types and GATs (higher-kinded types are deferred, [Surface syntax and paradigms](syntax.md)).
- Generics compiled by monomorphisation, dictionary passing, or both by build profile.
- Variance, `dyn` safety, and exhaustiveness checking of sum types.

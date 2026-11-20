# Core model

The **mechanised model of Moddil's core in Lean**: the source of truth for the core rules
([`specs/definition.md`: Three artefacts, one meaning](../specs/definition.md#three-artefacts-one-meaning)).

**Status:** starts after slice S5, once ownership, traits, errors and effects have settled
([M0 slices](../specs/roadmap.md#m0-the-v0-language-in-slices)).

## Covers

- Dynamic semantics, type and effect soundness, and borrows, in the style of RustBelt
  ([`specs/correctness.md`: A formal semantics of the core](../specs/correctness.md#a-formal-semantics-of-the-core)).
- The reference shows the core rules as inference rules **generated from these definitions**, so the
  two cannot drift.
- It validates the reference interpreter.

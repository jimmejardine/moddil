# Reference

The **normative reference** for Moddil: what every implementation must do. It is one of three
artefacts with one meaning, alongside the reference interpreter (`crates/refinterp`) and the Lean
core model (`model/`); any disagreement between them is a bug
([`specs/definition.md`](../specs/definition.md)).

**Status:** the [grammar](grammar/README.md), and the [2027 edition](2027/README.md) up to slice S7a.
The reference grows with the compiler: a feature is done only when it has reference paragraphs and
conformance tests as well as an implementation
([roadmap](../specs/roadmap.md#milestones)).

## What goes here

- **Prose with formal rules for the core**, versioned by edition (`reference/2027/…`).
- **Stable paragraph IDs** such as `expr.call.effects.3`, in bold at the start of each paragraph. An ID never changes meaning; a rule that
  changes gets a new ID. Conformance tests cite the IDs they cover.
- Core rules are shown as inference rules generated from the Lean model, so the two cannot drift.
- [`grammar/`](grammar/README.md): the one grammar source.

Not here: design rationale and alternatives, which stay in [`specs/`](../specs/README.md).

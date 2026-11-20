# Reference

The **normative reference** for Moddil: what every implementation must do. It is one of three
artefacts with one meaning, alongside the reference interpreter (`crates/refinterp`) and the Lean
core model (`model/`); any disagreement between them is a bug
([`specs/definition.md`](../specs/definition.md)).

**Status:** empty until the grammar lands. The reference grows with the compiler: a feature is done
only when it has reference paragraphs and conformance tests as well as an implementation
([M0 slices](../specs/roadmap.md#m0-the-v0-language-in-slices)).

## What goes here

- **Prose with formal rules for the core**, versioned by edition (`reference/2027/…`).
- **Stable paragraph IDs** such as `expr.call.effects.3`. An ID never changes meaning; a rule that
  changes gets a new ID. Conformance tests cite the IDs they cover.
- Core rules are shown as inference rules generated from the Lean model, so the two cannot drift.
- [`grammar/`](grammar/README.md): the one grammar source.

Not here: design rationale and alternatives, which stay in [`specs/`](../specs/README.md).

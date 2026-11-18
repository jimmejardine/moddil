# Diffs, Code Review and Version Control

> **Status:** brainstorm. Expands [`rationale.md`](rationale.md) §2.19. Nothing here is decided.

## 1. Problem

Version control operates on lines of text, but programs are trees with meaning. Reviewers
see noise (formatting, renames, moved code) and miss signal (a new side effect, a changed error path, a
performance regression). As agents produce more code, review becomes the bottleneck.

## 2. State of the art

- **Git**: universal, fast, but line-based and snapshot-based; merges are textual.
- **Difftastic / SemanticDiff / GumTree**: AST-aware diffs that ignore formatting and track moves.
- **Unison**: code is stored by hash; names are metadata. Renames are free and never conflict.
- **Pijul / Darcs**: patch theory, so patches commute and merges are more predictable.
- **Jujutsu (jj)**: first-class conflicts, working copy as a commit, easy history rewriting.
- **Stacked diffs** (Phabricator, Gerrit, Graphite, Sapling): small, reviewable, dependent changes.
- **Refactoring-aware review tools** (RefactoringMiner, IDE "show refactors") detect renames and moves.

## 3. What still hurts

- A rename across 200 files is a 200-file diff.
- Reformatting obscures history (`git blame` breaks).
- Diffs show *textual* change, not *behavioural* change.
- Reviewers can't easily ask "which public APIs changed? which functions gained side effects? did
  allocation or complexity change?"
- Merge conflicts on code that is semantically independent.

## 4. Directions

- **Canonical AST as the diff unit.** Since formatting is canonical (rationale §2.18), whitespace and style
  diffs disappear. Tooling diffs trees, not lines.
- **Refactors as intents.** A rename, move or extract-function is recorded as a single operation
  ("rename `foo` → `bar`") that the tool replays, rather than as hundreds of line edits. Reviewers
  approve the intent, not every touched line.
- **Semantic change summary per PR**, computed by the compiler:
  - public API / signature changes
  - **effect and capability deltas** ("`parse()` now performs `fs`")
  - new `unsafe` / FFI
  - new dependencies and their capabilities
  - contract changes
  - optional: allocation / complexity / benchmark deltas
- **LLM-written review summaries that are checked** against the computed semantic diff, so the summary
  cannot claim something the diff does not show.
- **Definition-level history**: blame and history per function/type, surviving moves and renames.
- Work *with* Git (it is not going away), but store enough structure (e.g., stable node IDs, rename
  maps) to enable all of the above. Consider Unison-style hashing of definitions as an internal model.

## 5. Open questions

- Stable identity for definitions: hash of content (Unison) vs persistent IDs vs name-based?
- Is a custom VCS layer justified, or can everything be done as Git metadata + tooling?

## Prior art

Pijul theory of patches; Jujutsu; Difftastic; GumTree (Falleri et al., 2014); Unison codebase
model.

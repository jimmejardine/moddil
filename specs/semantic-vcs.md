# Diffs, Code Review and Version Control

## Problem

Version control operates on lines of text, but programs are trees with meaning. Reviewers
see noise (formatting, renames, moved code) and miss signal (a new side effect, a changed error path, a
performance regression). As agents produce more code, review becomes the bottleneck.

## State of the art

- **Git**: universal, fast, but line-based and snapshot-based; merges are textual.
- **Difftastic / SemanticDiff / GumTree**: AST-aware diffs that ignore formatting and track moves.
- **Unison**: code is stored by hash; names are metadata. Renames are free and never conflict.
- **Pijul / Darcs**: patch theory, so patches commute and merges are more predictable.
- **Jujutsu (jj)**: first-class conflicts, working copy as a commit, easy history rewriting.
- **Stacked diffs** (Phabricator, Gerrit, Graphite, Sapling): small, reviewable, dependent changes.
- **Refactoring-aware review tools** (RefactoringMiner, IDE "show refactors") detect renames and moves.
- **cargo-semver-checks**: computes whether a Rust API change is breaking.

## What still hurts

- A rename across 200 files is a 200-file diff.
- Reformatting obscures history (`git blame` breaks).
- Diffs show *textual* change, not *behavioural* change.
- Reviewers can't easily ask "which public APIs changed? which functions gained side effects? did
  allocation or complexity change?"
- Merge conflicts on code that is semantically independent.

## Decisions

- **Git is assumed.** There is no custom version control system and **no required metadata**: no
  commit trailers, git notes or special comments. Everything works on any Git repository, branch or
  tag, and with Git-compatible tools such as Jujutsu.
- **Everything is computed from two revisions.** `moddil diff` compares any two points in history:

  | Command | Compares |
  |---|---|
  | `moddil diff` | Uncommitted work (staged and unstaged) against `HEAD` |
  | `moddil diff --staged` | Staged changes against `HEAD` |
  | `moddil diff main..feature` | A branch against its base |
  | `moddil diff v1.2 v1.3` | Two tags (release notes) |
  | `moddil diff HEAD~3` | The last three commits |

- **Definition identity is inferred, not stored:**
  - the **fully qualified name** of each definition;
  - **rename and move detection** by matching canonical ASTs (GumTree and RefactoringMiner style),
    since formatting is canonical ([`toolchain.md`: Formatter](toolchain.md#formatter));
  - a **content hash per definition** (the same hash as the build-cache key,
    [`build-times.md`: Language rules that decide build speed](build-times.md#language-rules-that-decide-build-speed)) for instant "unchanged" checks.

  **Confidence (decided):**

  | Match | Shown as |
  |---|---|
  | Identical canonical AST after the rename | **verified** (behaviour-preserving) |
  | A unique best match above about 80% AST similarity | **inferred**, with its score: `rename (inferred, 87%)` |
  | Ambiguous, or below the threshold | A removal plus an addition |

  The threshold is configurable, and the JSON output carries every score.

## The report

Concise by default, most important first:

1. **Headline:** the **required semver bump**, computed (cargo-semver-checks style), and the number of
   behavioural changes versus refactors. New error variants or new effects in a public signature
   count as breaking.
2. **Behavioural changes**, per definition: signatures; effects, capabilities and error sets;
   contracts; new `unsafe` or FFI; changes to derive and compile-time expansions
   ([`comptime.md`: Generation: typed impls at the annotation site](comptime.md#generation-typed-impls-at-the-annotation-site)); wire-contract compatibility across tiers
   ([`distribution.md`](distribution.md)).
3. **Dependencies:** added, removed or upgraded, with their capability changes
   ([`packages-and-supply-chain.md`](packages-and-supply-chain.md)).
4. **Refactors, collapsed:** renames, moves, extractions and formatting. When the canonical ASTs are
   identical after applying the detected renames, the refactor is marked **verified
   behaviour-preserving**, and reviewers can skip it.
5. **Tests:** affected tests, listed; `--test` runs them (decided: on request, so `diff` stays fast
   and works on any two revisions; CI's pull-request report runs them by default, configurably); optional mutation-score
   ([`correctness.md`: Mutation testing](correctness.md#mutation-testing)) and benchmark changes.
6. **Suggested description:** a ready-to-paste **PR or commit description** generated from the
   report: summary, breaking changes, risk notes, testing. The user or an LLM edits and enriches it.
   Every statement maps to report items, so the description starts from facts, and
   `moddil diff --check-description pr.md` flags claims the diff does not support and changes the
   description does not mention.

Example (terminal output, abridged):

```text
$ moddil diff main..feature/checkout-retry
semver: MINOR (no breaking changes) · 4 behavioural changes · 12 refactors (all verified)

Behavioural
  store::checkout        effects +retry · error set +PaymentTimeout (internal, handled)
  store::payments::charge  new parameter `attempt: u32 = 0` (named, defaulted: not breaking)
  store::payments        capability +time
  store::api::Order      new field `retries` (wire: compatible, defaulted)

Dependencies
  moddil/retry 0.4.1     new · capabilities: time

Refactors (verified behaviour-preserving)
  rename  store::pay  →  store::payments          (9 references)
  move    checkout_total  →  store::pricing       ( 3 references)

Tests
  14 affected · run with --test

Suggested description
  Add bounded retries to checkout payments.
  - `checkout` retries `charge` up to 3 times on timeout (new `time` capability in `payments`).
  - `Order` gains a defaulted `retries` field; the wire format stays compatible.
  - Renames `store::pay` to `store::payments` (verified behaviour-preserving).
```

Output formats: terminal, **Markdown** (for pasting into a PR), and **JSON** (for agents and CI).
`--detail` expands any section into a per-definition semantic diff.

## Built on the report (optional, opt-in)

- **Git integration without metadata:**
  - a `.gitattributes` diff driver or difftool, so `git diff` can show the semantic view;
  - an optional semantic **merge driver** that merges on the AST. It is **conservative (decided)**:
    only structurally independent changes merge automatically (different definitions, import
    lists, additions at the same place, and a rename on one side with a body edit on the other). A
    conflict inside one body falls back to an ordinary text conflict. Every automatic merge is
    re-type-checked and listed in the merge report. Lockfiles and generated files are regenerated,
    never merged.
- **Definition-level history:** `moddil log store::payments::charge` and `moddil blame …` walk Git
  history and follow inferred renames and moves, so history survives refactors and reformatting.
- **Split suggestions:** `moddil diff --suggest-split` proposes separating refactor-only changes from
  behavioural ones into stacked commits, which suits large agent-written changes. Advisory only.
- **CI and review policy:** CI posts the Markdown report on each pull request. Optional rules in the
  manifest act on the report, for example "a new `Net` or `ffi` capability in this mod needs security
  review" or "a semver-major change needs an API owner": CODEOWNERS keyed on *semantic* change rather
  than file paths.

## Prior art

Git diff and merge drivers; Difftastic; SemanticDiff; GumTree (Falleri et al., 2014); RefactoringMiner;
mergiraf (syntax-aware merge driver); cargo-semver-checks; Jujutsu; Graphite and Sapling stacks;
CODEOWNERS; Pijul's theory of patches; the Unison codebase model.

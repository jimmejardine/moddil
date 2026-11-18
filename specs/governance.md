# Governance

> **Status:** decided direction, details open. Listed in the [index](README.md).

## Summary

| Rule | One line |
|---|---|
| **Staged decision making** | Founder-led now; written triggers hand decisions to a core team, then an elected steering council; a foundation holds the name and the money. |
| **Moddil Evolution** | Proposals move from draft to stabilised in an edition, each with an example, a migration, an ecosystem run, reference paragraphs and conformance tests. |
| **Agents as equals** | Agent reviews and approvals count as humans' do; contributions are marked as agent- or human-authored; humans break ties. |
| **Low-friction contributions** | MIT/Apache-2.0, DCO sign-off, no CLA. |
| **A code of conduct** | Contributor Covenant 2.1, moderated separately from technical decisions. |
| **A protected, permissive name** | "Moddil" and the armadillo are registered; community use is free; implementations need the conformance suite. |
| **An independent non-profit** | Funded by donations and sponsors, without corporate control. |
| **Security response in git** | OSV advisories in a project repository; community mods publish their own; `moddil audit` reads both. |

## Problem

- The language has no defined way to change, no named decision makers, and no plan for the community,
  security response or funding that a long-lived language needs.
- **Transitions made in a crisis go badly.** Python's founder stepped down abruptly in 2018, and the
  project had to invent its governance afterwards; Rust's moderation and leadership crises in 2021 and
  2023 showed how hard it is to fix governance under pressure.
- **Funding is fragile**: many languages depend on one company or a few sponsors.
- **The name** can be used for incompatible forks unless it is protected.

## State of the art

| Project | Decisions | Money and name |
|---|---|---|
| **Python** | Founder (BDFL) until 2018; then an elected five-person Steering Council; PEPs | The PSF owns the trademark and funds work |
| **Zig** | Founder-led | The Zig Software Foundation, a non-profit funded by donations, pays developers |
| **Rust** | Teams, RFCs with a final comment period; the Leadership Council since 2023 | The Rust Foundation, funded by corporate members, owns the trademark |
| **Go, Swift, Kotlin** | Company teams (Google, Apple, JetBrains) | One company owns everything |
| **Gleam, Odin** | Founder-led | GitHub Sponsors, Open Collective |

| Practice | Examples |
|---|---|
| **Security advisories** | RustSec (an advisory database in git), the Go vulnerability database, the OSV format |
| **Contributor terms** | DCO sign-off (Linux, many Rust projects) vs CLAs (Apache, Google) |
| **Codes of conduct** | Contributor Covenant 2.1; Rust's code of conduct and moderation team |

## Decisions

### Staged decision making

| Stage | Who decides | Moves on when (proposed) |
|---|---|---|
| **1. Founder** | The founder, in the open, with public reasoning | v1 ships, or ten people contribute regularly |
| **2. Core team** | A small named team under a written charter | A set number of years at stage 2, or enough contributors to hold an election |
| **3. Steering council** | An elected council, with teams for the compiler, standard mods, tooling and the reference | — |
| **Foundation** (alongside 2 or 3) | Holds the trademark, money and infrastructure; does not make technical decisions | Created when there is money or a trademark to hold |

- The triggers are written down now, so each transition is planned rather than forced by a crisis.

### Moddil Evolution

The proposal process sketched in [`evolution.md`](evolution.md), formalised:

- Proposals are **pull requests** to a proposals repository.
- **Stages**: draft → review → final comment period → accepted → implemented → stabilised in an
  edition.
- **Every proposal** must:
  - cite a reference example that needs it ([`examples/README.md`](examples/README.md));
  - include its migration, if it changes anything;
  - pass the ecosystem run over opted-in public mods ([`evolution.md`](evolution.md));
  - land its **reference paragraphs and conformance tests** before stabilising
    ([`definition.md`](definition.md)).
- The decision body of the current stage accepts or rejects proposals, with written reasons.

### Agents as equals

- Agents may author **proposals, implementations and reviews**.
- **Agent reviews and approvals count as humans' do.**
- Every contribution and review is recorded as **agent- or human-authored**, as in the audit trail
  ([`packages-and-supply-chain.md`: Audit trail in git](packages-and-supply-chain.md#audit-trail-in-git)).
- **Humans break ties**: when reviews split, or agents and humans disagree, the human reviewers
  decide.
- An agent cannot certify that it has the right to contribute, so **its operator signs the DCO**.

### Contributions

- Code, documentation, the reference and the conformance suite are dual-licensed **MIT or
  Apache-2.0**.
- Contributors **sign off** each commit under the Developer Certificate of Origin. There is no CLA.

### Standard mods

The `moddil/*` mods ([`stdlib.md`](stdlib.md)) follow these rules on top of the general process:

- **RFCs** for new standard mods and breaking changes.
- **Capability review**: any increase in a standard mod's declared capabilities needs review.
- **Audit trail** ([Package management and supply-chain security](packages-and-supply-chain.md)): standard mod releases are reproducibly built, signed, and reviewed. The
  review record says whether the reviewer was a human or an agent.
- **Wrappers are allowed**: a mod may wrap a C or Rust library ([`interop.md`](interop.md)),
  but it carries the `ffi` capability, which is visible to every consumer.
- **Editions**: standard mods follow language editions, with automated migrations ([Language evolution](evolution.md)).

### Code of conduct

- **Contributor Covenant 2.1.**
- A named moderation contact, later a moderation team, **separate from the technical decision
  makers**, with its own appeals path.

### Name, logo and trademark

- **"Moddil" and the armadillo logo are registered**, held by the founder until the foundation
  exists ([`adoption.md`](adoption.md)).
- **The policy is permissive**: community mods, books, talks, courses and "for Moddil" products may
  use the name freely.
- **Implementations** may call themselves "Moddil <edition>" only if they pass that edition's
  conformance suite ([`definition.md`: Open implementations, protected name](definition.md#open-implementations-protected-name)).
- **Forks** that change the language must use a different name.

### Funding

- An **independent non-profit foundation** (the Zig Software Foundation model), funded by donations,
  sponsorships and corporate support **without board seats or control**.
- The money pays core developers and infrastructure: CI, ecosystem runs and the playground
  ([`toolchain.md`](toolchain.md)).
- Until the foundation exists: GitHub Sponsors or Open Collective.
- Finances are published.

### Security response

- A **security response team**, initially the founder and one other person, handles private
  disclosure (through `SECURITY.md`) and fixes for the **toolchain and standard mods**
  ([`security.md`](security.md)).
- Advisories are published in the **OSV format** in a **project git repository**, consistent with
  plain-git hosting ([`packages-and-supply-chain.md`](packages-and-supply-chain.md)).
- **Community mods** publish OSV advisories in their own repositories.
- `moddil audit` and `moddil outdated` read every advisory repository the user lists; the project's
  is listed by default. An advisory never bypasses the cooldown automatically; taking a fix early
  is an explicit override ([`packages-and-supply-chain.md`: Randomised cooldown](packages-and-supply-chain.md#randomised-cooldown)).

## Fit check against the reference examples

Governance doesn't change what the examples can express; it decides how features they need get into
the language (each proposal cites one), and how fast a security fix reaches them (advisories and the
cooldown override).

## Open questions

- The exact triggers for each stage, and how council members are chosen.
- Do agents take part in elections or hold council seats, or only review?
- Does the human-review default for `unsafe` and `ffi` changes
  ([`unsafe.md`: Review and audit](unsafe.md#review-and-audit)) also apply to the project's own
  standard mods, given that agents are equals here?
- Who holds the trademark, and in which jurisdictions, before the foundation exists?

## Prior art

Python's PEPs, PEP 8016 and the Steering Council; the Zig Software Foundation; Rust's RFCs, teams,
Leadership Council and Foundation; Swift Evolution; RustSec, the Go vulnerability database and OSV;
the Developer Certificate of Origin; Contributor Covenant 2.1.

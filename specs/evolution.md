# Language Evolution

> **Status:** brainstorm. Expands [`rationale.md`](rationale.md) §2.20. Nothing here is decided.

## 1. Problem

Languages either freeze (C) or break their ecosystems (Python 2→3).

## 2. State of the art

Rust editions (per-crate, interoperable); Go 1 compatibility promise plus `go fix`; Swift's
evolution process.

## 3. Directions

- Editions from day one.
- **Automated migrations** as part of the toolchain, which agents can run and humans can review as
  intents (rationale §2.19).
- A stable, small surface syntax that changes rarely, so training data stays valid (rationale §2.18).

## 4. Open questions

None recorded yet.

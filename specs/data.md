# Data interchange and persistence

> **Status:** not started. Nothing is decided yet; this page records the problem and the
> questions to settle. Listed in the [index](README.md).

## Problem

Serialisation, schema evolution and databases appear in several places (standard mods, distributed
contracts, the web store), but there is no single design for how data leaves and re-enters a
program.

## Touched on in

[`mods.md`: Initial standard-mod catalogue](mods.md#initial-standard-mod-catalogue) (`moddil/serde`, `moddil/sql`),
[`distribution.md`](distribution.md) (contracts), [`examples/03-web-store.md`](examples/03-web-store.md).

## Questions to settle

- Serialisation and derives; binary and text formats.
- Schema evolution, shared with distributed contracts.
- Zero-copy formats and validated views.
- Typed SQL, migrations and connection pools.

# Tour

The **dogfood tour**: one small, complete Moddil program whose documentation is the tour of the
language ([`specs/tour.md`](../specs/tour.md)).

**Status:** slice S9d: the tour is the [`shorty/`](shorty/README.md) mod, and
`python tools/ci.py tour` checks it (M0's gate is met); its libraries are the signature-only stubs
in [`stubs/moddil/`](../stubs/moddil/README.md) until M2.

## Gates

- **M0:** the tour parses and passes the type, effect and ownership checks, in CI.
- **M2:** its tests pass, including the simulated-network test under `moddil/runtime-sim`.
- A failing tour blocks a release: it is the first program written in Moddil.

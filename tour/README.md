# Tour

The **dogfood tour**: one small, complete Moddil program whose documentation is the tour of the
language ([`specs/tour.md`](../specs/tour.md)).

**Status:** M2 slice M2c: the tour is the [`shorty/`](shorty/README.md) mod;
`python tools/ci.py tour` checks it (M0's gate), and its three tests pass natively, on the MIR
interpreter and as WASM, over the standard mods written in Moddil ([`mods/`](../mods/README.md))
(M2's gate).

## Gates

- **M0:** the tour parses and passes the type, effect and ownership checks, in CI.
- **M2:** its tests pass, including the simulated-network test under `moddil/runtime-sim`.
- A failing tour blocks a release: it is the first program written in Moddil.

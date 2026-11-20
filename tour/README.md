# Tour

The **dogfood tour**: one small, complete Moddil program whose documentation is the tour of the
language ([`specs/tour.md`](../specs/tour.md)).

**Status:** until M0, the tour lives in [`specs/tour.md`](../specs/tour.md). At M0 it becomes the
[`shorty/`](shorty/README.md) mod.

## Gates

- **M0:** the tour parses and passes the type, effect and ownership checks, in CI.
- **M2:** its tests pass, including the simulated-network test under `moddil/runtime-sim`.
- A failing tour blocks a release: it is the first program written in Moddil.

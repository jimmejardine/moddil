# Standard mods

The **standard mods**, written in Moddil: `core-*`, `collections`, `text`, `net`, `serde` and the
rest of the catalogue ([`specs/stdlib.md`](../specs/stdlib.md)).

**Status:** empty until slice S7, when `Vec` and `Map` become the first mods written in Moddil.

## Layout and releases

- One directory per mod (`mods/collections/`, `mods/net/`, …), each a real mod with its own manifest.
- **`core-*` ships inside the toolchain** and changes only with editions. **Every other mod** is an
  ordinary dependency: released with a path-prefixed tag (`mods/net/v0.3.1`), reached through a vanity
  name (`moddil.com/net`), and subject to cooldowns and capability ceilings like any third-party mod
  ([`specs/stdlib.md`: Distribution and versioning](../specs/stdlib.md#distribution-and-versioning)).
- Standard mods follow the extra rules in [`specs/governance.md`: Standard mods](../specs/governance.md#standard-mods):
  RFCs for new mods, capability review, reproducible signed releases.

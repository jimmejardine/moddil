# shorty

A link shortener, and the tour of Moddil written as real source
([`specs/tour.md`: From markdown to source](../../specs/tour.md#from-markdown-to-source)).

**Status:** M2 slice M2c: it passes the type, effect and ownership checks
(`python tools/ci.py tour`, or `moddil check tour/shorty`), and its three tests pass natively, on
the MIR interpreter and as WASM (`moddil test tour/shorty`), over the standard mods written in
Moddil ([`mods/`](../../mods/README.md)), named through [`stubs/moddil/`](../../stubs/moddil/README.md).
`main` checks but does not run: the root capabilities are still signatures. Where the code had to differ from the tour as first written, the gaps table in
[`specs/tour.md`](../../specs/tour.md#gaps-the-tour-exposed) says why.

## Layout

| Module | Tour section |
|---|---|
| `src/types.moddil` | Types that validate themselves |
| `src/links.moddil` | A user-defined effect for storage |
| `src/routes.moddil` | Request handling, with every error handled |
| `src/serve.moddil` | Serving, isolated per connection |
| `src/main.moddil` | `main`: the only source of authority |

- Each module's `//!` overview carries the tour's explanations and spec links; the mod overview gives
  the reading order.
- The tests are attached `test` blocks (`for app`, `for serve`); the `// =>` example in a doc
  comment is to run as a test once doc tests do.
- `moddil doc` renders the mod as the tour.
- Until M2, its library dependencies (HTTP, serde, `Url`) are signature-only stubs.

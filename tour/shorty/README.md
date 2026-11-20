# shorty

A link shortener, and the tour of Moddil written as real source
([`specs/tour.md`: From markdown to source](../../specs/tour.md#from-markdown-to-source)).

**Status:** placeholder until M0; the program is still in [`specs/tour.md`](../../specs/tour.md).

## Planned layout

| Module | Tour section |
|---|---|
| `src/types.moddil` | Types that validate themselves |
| `src/links.moddil` | A user-defined effect for storage |
| `src/routes.moddil` | Request handling, with every error handled |
| `src/serve.moddil` | Serving, isolated per connection |
| `src/main.moddil` | `main`: the only source of authority |

- Each module's `//!` overview carries the tour's explanations and spec links; the mod overview gives
  the reading order.
- The tests are attached `test` blocks, and `// =>` examples in doc comments run as tests.
- `moddil doc` renders the mod as the tour.
- Until M2, its library dependencies (HTTP, serde, `Url`) are signature-only stubs.

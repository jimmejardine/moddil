# Examples

The **reference examples as real programs**: the yardstick every language decision is measured
against ([`specs/examples/README.md`](../specs/examples/README.md)).

**Status:** [`mandelbulb/`](mandelbulb/README.md) is written and parses; it checks once slice S10
(numbers) lands, and runs in browsers at M2. The others are empty until their milestones. Their design documents are in [`specs/examples/`](../specs/examples/README.md).

| Directory | Example | Milestone |
|---|---|---|
| `mandelbulb/` | Showcase: fly through the Mandelbulb, ray-marched on WebGPU; the first to be built | M2 |
| [`zlib/`](zlib/src/lib.moddil) | C from Moddil: zlib (vendored) behind a typed API, through `extern "C"`; its strings, its CRC table, and `inflateBack` calling Moddil back | M3a, M3c |
| [`cabi/`](cabi/src/lib.moddil) | Moddil from C: `#[export]` functions, `#[repr(C)]` structs by pointer and by value, the generated header; C called with the same structs, and calling back | M3a, M3c |
| [`python/`](python/src/lib.moddil) | A Python module in Moddil: `#[export(python)]` functions and a class, built as a wheel | M3b |
| `hashiverse/` | Hashiverse client: CLI/native first, then the browser | M3, M5 |
| `web-store/` | Web store API with `sql`, HTTP and deterministic simulation | M4 |
| `game/` | Multiplayer game: browser client, server, WebGPU | M5 |
| `hft/` | HFT platform: same code in simulation and live | Later |
| `fumoca/` | Showcase: the fumoca Monte Carlo spreadsheet, a Svelte 5 UI over a Moddil core | M5 |
| `talk/` | Showcase: text to speech for Python; the CPU pool first, then WebGPU | M3 |

- Each is a real mod (or a few), **built and tested in CI on every change**, so a language change that
  breaks an example is caught at once.
- Hashiverse, a real product, may move to its own repository once it ships.

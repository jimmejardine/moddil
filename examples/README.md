# Examples

The **reference examples as real programs**: the yardstick every language decision is measured
against ([`specs/examples/README.md`](../specs/examples/README.md)).

**Status:** [`mandelbulb/`](mandelbulb/README.md) is written and parses; it checks once slice S10
(numbers) lands, and runs in browsers at M2. The others are empty until their items in the
roadmap ([`specs/roadmap.md`: On the horizon](../specs/roadmap.md#on-the-horizon)). Their design documents are in [`specs/examples/`](../specs/examples/README.md).

| Directory | Example | Built in, or roadmap item |
|---|---|---|
| `mandelbulb/` | Showcase: fly through the Mandelbulb, ray-marched on WebGPU; the first to be built | M2 |
| [`zlib/`](zlib/src/lib.moddil) | C from Moddil: zlib (vendored) behind a typed API, through `extern "C"`; its strings, its CRC table, and `inflateBack` calling Moddil back | M3a, M3c |
| [`cabi/`](cabi/src/lib.moddil) | Moddil from C: `#[export]` functions, `#[repr(C)]` structs by pointer and by value, the generated header; C called with the same structs, and calling back | M3a, M3c |
| [`python/`](python/src/lib.moddil) | A Python module in Moddil: `#[export(python)]` functions and a class, built as a wheel | M3b |
| `hashiverse/` | Hashiverse client: CLI/native first, then the browser | P42, P48 |
| `web-store/` | Web store API with `sql`, HTTP and deterministic simulation | P46 |
| `game/` | Multiplayer game: browser client, server, WebGPU | P48 |
| `hft/` | HFT platform: same code in simulation and live | Later |
| `fumoca/` | Showcase: the fumoca Monte Carlo spreadsheet, a Svelte 5 UI over a Moddil core | P49 |
| `talk/` | Showcase: text to speech for Python; the CPU pool first, then WebGPU | P45 |

- Each is a real mod (or a few), **built and tested in CI on every change**, so a language change that
  breaks an example is caught at once.
- Hashiverse, a real product, may move to its own repository once it ships.

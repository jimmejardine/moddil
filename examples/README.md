# Examples

The **reference examples as real programs**: the yardstick every language decision is measured
against ([`specs/examples/README.md`](../specs/examples/README.md)).

**Status:** empty until M3. Their design documents are in [`specs/examples/`](../specs/examples/README.md).

| Directory | Example | Milestone |
|---|---|---|
| `hashiverse/` | Hashiverse client: CLI/native first, then the browser | M3, M5 |
| `web-store/` | Web store API with `sql`, HTTP and deterministic simulation | M4 |
| `game/` | Multiplayer game: browser client, server, WebGPU | M5 |
| `hft/` | HFT platform: same code in simulation and live | Later |

- Each is a real mod (or a few), **built and tested in CI on every change**, so a language change that
  breaks an example is caught at once.
- Hashiverse, a real product, may move to its own repository once it ships.

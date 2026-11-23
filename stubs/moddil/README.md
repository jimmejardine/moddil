# moddil (stubs)

The **umbrella** for the standard mods: it re-exports those written in Moddil
([`mods/`](../../mods/README.md)), so a program names them as `moddil::net` and so on, and holds the
**signatures** of those still to be written
([`specs/roadmap.md`: Milestones](../../specs/roadmap.md#milestones)).

**Status:** M2 slice M2c. A stub mod (`stub = true` in its manifest,
[`reference/2027/modules.md`](../../reference/2027/modules.md)): its functions may have no body, so
a program checks against them, and calling one traps (`not implemented yet`). A derive of one of
its traits declares an empty impl, and `#[root_capability]` marks the structs that only `main`
takes. Each item is written just far enough for [the tour](../../tour/shorty/README.md) to check.

## Modules

| Module | Stands in for |
|---|---|
| `root` | The root capabilities: `Args`, `Env`, `Stdio`, `FsRoot`, `NetRoot`, `Clock`, `Entropy`, and `Port` (re-exported from `net`) |
| `io` | `Stderr` and `eprintln` |
| `runtime` | `serve_until_stopped` and `Signal`; `Duration` is the prelude's |
| `collections`, `sync`, `rand`, `serde`, `net`, `sim` | Re-exported from [`mods/`](../../mods/README.md), written in Moddil |

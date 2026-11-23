# moddil (stubs)

The **signatures** of the standard mods the tour uses, until they are written in Moddil at M2
([`specs/roadmap.md`: Milestones](../../specs/roadmap.md#milestones)).

**Status:** slice S9d. A stub mod (`stub = true` in its manifest,
[`reference/2027/modules.md`](../../reference/2027/modules.md)): its functions may have no body, so
a program checks against them, and calling one traps (`not implemented yet`). A derive of one of
its traits declares an empty impl, and `#[root_capability]` marks the structs that only `main`
takes. Each item is written just far enough for [the tour](../../tour/shorty/README.md) to check.

## Modules

| Module | Stands in for |
|---|---|
| `root` | The root capabilities: `Args`, `Env`, `Stdio`, `FsRoot`, `NetRoot`, `Clock`, `Entropy`, and `Port` |
| `io` | `Stderr` and `eprintln` |
| `collections` | `Map` |
| `sync` | `Mutex` |
| `rand` | `Rand` and `seeded` |
| `net`, `net::http` | `Net`, `Url`, `Listener`, `Conn`; the router, requests and responses |
| `serde`, `serde::json` | Written in Moddil (M2a): `Serialize` and `Deserialize` with their derives (impl templates), `DecodeError`, and JSON's `encode` and `decode`; its tests are in [`../tests/serde`](../tests/serde) |
| `runtime` | `serve_until_stopped`, `Signal`, `Duration` |
| `sim` | `Sim` and `drop_messages` |

At M2 the real mods replace these, and the tour's tests run.

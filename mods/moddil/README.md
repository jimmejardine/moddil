# moddil

The **umbrella** for the standard mods: it re-exports them ([`mods/`](../README.md)), so a
program names them as `moddil::net`, `moddil::root` and so on
([`specs/roadmap.md`: How work is built](../../specs/roadmap.md#how-work-is-built)).

**Status:** M2 slice M2d: every module is a re-export of a mod written in Moddil. Until M2d it was
a stub mod (`stub = true`, [`reference/2027/modules.md`](../../reference/2027/modules.md)) holding
the signatures of the standard mods the tour uses, in `stubs/moddil`; with nothing left to stub,
it moved here beside the mods it re-exports.

## Modules

| Module | Re-exports |
|---|---|
| `root` | `runtime::root`: the root capabilities `Args`, `Env`, `Stdio`, `FsRoot`, `NetRoot`, `Clock`, `Entropy`, and `Port` |
| `io` | `runtime::io`: `Stderr` and `eprintln` |
| `runtime` | `serve_until_stopped` and `Signal`; `Duration` is the prelude's |
| `collections`, `sync`, `rand`, `serde`, `net`, `sim` | The mods of the same names |

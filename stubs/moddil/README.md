# moddil

The **umbrella** for the standard mods: it re-exports them ([`mods/`](../../mods/README.md)), so a
program names them as `moddil::net`, `moddil::root` and so on
([`specs/roadmap.md`: Milestones](../../specs/roadmap.md#milestones)).

**Status:** M2 slice M2d: every module is a re-export of a mod written in Moddil. Until M2d it was
a stub mod (`stub = true`, [`reference/2027/modules.md`](../../reference/2027/modules.md)) holding
the signatures of the standard mods the tour uses; it stays in `stubs/` until mods are published
under their own names.

## Modules

| Module | Re-exports |
|---|---|
| `root` | `runtime::root`: the root capabilities `Args`, `Env`, `Stdio`, `FsRoot`, `NetRoot`, `Clock`, `Entropy`, and `Port` |
| `io` | `runtime::io`: `Stderr` and `eprintln` |
| `runtime` | `serve_until_stopped` and `Signal`; `Duration` is the prelude's |
| `collections`, `sync`, `rand`, `serde`, `net`, `sim` | The mods of the same names |

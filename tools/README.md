# Tools

Small scripts for working on this repository. Language tooling lives in [`crates/`](../crates/README.md).

| Script | Does |
|---|---|
| `check_spec_links.py` | Checks every relative link and `#heading` anchor in `specs/`, and that every spec is listed in the index. `--align` re-aligns the index table. Run it after every spec edit; CI runs it too. |
| `check_grammars.py` | Runs the grammars generated from `reference/grammar/moddil.ebnf` through their own tools: Lark and tree-sitter must accept every valid corpus case and sample. Part of `ci.py grammars`. |
| `zonedata.py` | Generates `mods/time/src/tzdata.moddil`, the IANA time zone database the time mod carries, from the TZif files of Python's `tzdata` package: each zone's transitions until its POSIX TZ rule takes over, and the rule. `--check` compares the generated file with `zoneinfo`. Regenerate after `pip install --upgrade tzdata`. |
| `ci.py` | The single definition of CI: `fmt`, `clippy`, `deny`, `test`, `tour`, `examples`, `links` and `grammars`. GitHub Actions calls it step by step; locally it runs on Windows, and `--wsl` or `--all` runs it in WSL as well. `quick`, not one of the steps, is the check while working: `cargo fmt`, the layout of the changed `.moddil` files, the tests of the crates changed (`git status`), and the conformance areas changed or named (`quick borrows`). |

```
python tools/check_spec_links.py
python tools/check_spec_links.py --align
python tools/ci.py [steps…] [--wsl | --all]
python tools/ci.py quick [area | package]...
python tools/check_grammars.py [lark | tree-sitter]
python tools/zonedata.py [--check]
```

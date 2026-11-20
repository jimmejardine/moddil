# Tools

Small scripts for working on this repository. Language tooling lives in [`crates/`](../crates/README.md).

| Script | Does |
|---|---|
| `check_spec_links.py` | Checks every relative link and `#heading` anchor in `specs/`, and that every spec is listed in the index. `--align` re-aligns the index table. Run it after every spec edit. |

```
python tools/check_spec_links.py
python tools/check_spec_links.py --align
```

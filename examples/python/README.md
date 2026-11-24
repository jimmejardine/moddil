# python

A Python module written in Moddil: the mod `stats`, built by `moddil build --wheel` into an
extension module, its stubs and a wheel that `pip` installs. It is the example for **exporting to
Python** ([`reference/2027/interop.md`](../../reference/2027/interop.md), `ffi.python.1` and
`ffi.python.2`). For C calling Moddil, see [`../cabi`](../cabi/README.md).

## Build it and use it from Python

```
moddil build --wheel examples/python -o out/python
pip install --target out/site out/python/stats-0.2.0-cp311-abi3-*.whl
PYTHONPATH=out/site python examples/python/py/check.py
```

The build needs Python 3.11 or later (found as `python3` or `python`, or named by `MODDIL_PYTHON`)
and the system's C compiler, which compiles the generated glue. It writes into `out/python`:

| File | What it is |
|---|---|
| `stats.pyd` (Windows) or `stats.abi3.so` | The extension module, against CPython's stable ABI |
| `stats.pyi` | Its stubs, for editors and type checkers |
| `stats-0.2.0-cp311-abi3-<platform>.whl` | The wheel, versioned from the manifest's `[mod] version` |

Then, in Python:

```python
import array, stats

stats.mean(array.array("d", [1.0, 2.0, 3.0]))   # 2.0, read in place
stats.rolling_mean([1.0], 3)                     # raises stats.StatsError
c = stats.Counter(5); c.add(2); c.count          # 7
```

## What crosses

| Moddil | Python | Shows |
|---|---|---|
| `mean(xs: &[f64]) -> f64` | `mean(xs: Buffer) -> float` | a buffer of 8-byte floats, without a copy (numpy arrays too) |
| `rolling_mean(xs: &[f64], window: int) -> Vec<f64> with fail(StatsError)` | a `stats.Array`, or `StatsError` | a `Vec` as a read-only buffer without a copy; a `fail` error as an exception, a subclass of `stats.Error`, its fields as attributes (`e.window`); keyword arguments (`window=2`) |
| `steps(start: int, n: int, step: int) -> Vec<int>` | a `stats.Array` of `int`s | a `Vec<int>`, format `q` |
| `scale_in_place(xs: &mut [f64], k: f64)` | changes the caller's buffer | a `&mut` slice, written in place |
| `greet(name: &str) -> String`, `checksum(data: &[u8]) -> u32`, `parse(s: &str) -> Option<int>` | `str`, `int`, `int` or `None` | text, bytes, and `Option` |
| `divide(a: int, b: int) -> int` | raises `stats.Trap` on 0 | a trap, after which the module keeps working |
| `struct Counter` with `new`, `add`, `doubled`, `fork`, `zero`, `finish` | `class Counter` | a class: its constructor, methods, static methods and properties; `finish(self)` takes the instance's value |
| `geo::area`, `geo::Rect` (in [`src/geo.moddil`](src/geo.moddil)) | `stats.geo.area`, `stats.geo.Rect` | a submodule, importable as `stats.geo` |

## How it fits together

| File | Part |
|---|---|
| [`moddil.toml`](moddil.toml) | The mod: its name and version |
| [`src/lib.moddil`](src/lib.moddil) | The `#[export(python)]` functions and the class, with their Moddil tests |
| [`src/geo.moddil`](src/geo.moddil) | The submodule `stats.geo` |
| [`py/check.py`](py/check.py) | Every function and the class from Python: buffers, errors, traps, and 10,000 objects made and freed; numpy arrays where numpy is installed |

`moddil build --wheel` writes three layers over the mod (the `moddil-py` crate): Moddil shims that
take C pieces and give a status, C glue against the stable ABI, and the stubs.

## Where it is checked

- `crates/moddil/tests/python.rs` builds the wheel, installs it with `pip` into a temporary
  directory, and runs `py/check.py`. On CI (where `CI` is set) it fails rather than skips if
  Python or numpy is missing.
- `crates/moddil/tests/projects.rs` runs the Moddil tests natively, on the MIR interpreter, and as
  WASM.

## Status

Built in M3 slice M3b ([`specs/roadmap.md`](../../specs/roadmap.md#milestones)). Not yet: `async`
functions, keyword arguments, methods taking `self` by value, and `Vec` results as arrays.

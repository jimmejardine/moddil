<img src="moddil.png" alt="Moddil logo" align="right" width="150">

# Moddil

[![checks](https://github.com/jimmejardine/moddil/actions/workflows/checks.yml/badge.svg?branch=main)](https://github.com/jimmejardine/moddil/actions/workflows/checks.yml)
[![Linux](https://github.com/jimmejardine/moddil/actions/workflows/test-linux.yml/badge.svg?branch=main)](https://github.com/jimmejardine/moddil/actions/workflows/test-linux.yml)
[![Windows](https://github.com/jimmejardine/moddil/actions/workflows/test-windows.yml/badge.svg?branch=main)](https://github.com/jimmejardine/moddil/actions/workflows/test-windows.yml)
[![macOS](https://github.com/jimmejardine/moddil/actions/workflows/test-macos.yml/badge.svg?branch=main)](https://github.com/jimmejardine/moddil/actions/workflows/test-macos.yml)
[![Licence: MIT OR Apache-2.0](https://img.shields.io/badge/licence-MIT%20OR%20Apache--2.0-blue)](#licence)
[![Rust 1.95.0](https://img.shields.io/badge/rust-1.95.0-orange)](rust-toolchain.toml)

**Moddil: the programming language with armour, not baggage.**

Moddil is a programming language being designed for three shifts happening at once:

- **LLM agents write more of the code**, and humans mostly review and approve it.
- **The machine is heterogeneous**: many CPU cores, GPUs and other accelerators.
- **A program is a fleet**: browser clients, servers, workers and databases working together.

Its core idea is **semantics that are explicit and checked by machines**. Effects, capabilities,
ownership and resources, and where code and data live all appear in the types and are checked by
the compiler. That makes code easier for agents to write correctly and for humans to review.

## Status

Moddil is at the **design stage**. The Rust workspace for the compiler is set up, but there is no
language code yet. The documents in `specs/` range
from brainstorm to decided direction; none of them is a formal specification. Code snippets are
illustration only and do not commit to any syntax.

## Start here

Read [`specs/README.md`](specs/README.md) first. It is the **index**: every area with a status
(⚪ not started, 🔴 superficial, 🟠 major questions, 🟡 minor questions, 🟢 complete) and a link to
the document that holds its detail. Then read [Why Moddil](specs/vision.md) for the thesis, and the
[firm requirements](specs/requirements.md#firm-requirements-decided). People and agents editing
the specs should follow [Working on these specs](specs/README.md#working-on-these-specs).

## Layout

```
specs/   the design: one document per area, indexed by specs/README.md
crates/  the Rust implementation: the stage-0 compiler and its tools
tools/   helper scripts: the spec link checker and the CI script
```

The full layout, including the directories that fill up during M0, is in
[`specs/compiler.md`: Repository layout](specs/compiler.md#repository-layout).

## Building

Needs [rustup](https://rustup.rs) and Python 3.8 or later. The pinned Rust toolchain installs
itself on first use.

```
cargo build --workspace
python tools/ci.py        # fmt, clippy, deny, test, links: the same checks GitHub runs
```

See [`crates/README.md`](crates/README.md) for more.

## Licence

Moddil is dual-licensed under either of

- [MIT license](LICENSE-MIT) ([`https://opensource.org/licenses/MIT`](https://opensource.org/licenses/MIT))
- [Apache License, Version 2.0](LICENSE-APACHE) ([`https://www.apache.org/licenses/LICENSE-2.0`](https://www.apache.org/licenses/LICENSE-2.0))

at your option. This is the same dual-licence arrangement used by the Rust project itself, and by the majority of the Rust ecosystem.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in Moddil by you, as defined in the Apache-2.0 licence, shall be dual-licensed as above, without any additional terms or conditions.

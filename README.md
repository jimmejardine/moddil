<img src="moddil.png" alt="Moddil logo" align="right" width="150">

# Moddil

[![checks](https://github.com/jimmejardine/moddil/actions/workflows/checks.yml/badge.svg?branch=main)](https://github.com/jimmejardine/moddil/actions/workflows/checks.yml)
[![grammars](https://github.com/jimmejardine/moddil/actions/workflows/grammars.yml/badge.svg?branch=main)](https://github.com/jimmejardine/moddil/actions/workflows/grammars.yml)
[![Linux](https://github.com/jimmejardine/moddil/actions/workflows/test-linux.yml/badge.svg?branch=main)](https://github.com/jimmejardine/moddil/actions/workflows/test-linux.yml)
[![Windows](https://github.com/jimmejardine/moddil/actions/workflows/test-windows.yml/badge.svg?branch=main)](https://github.com/jimmejardine/moddil/actions/workflows/test-windows.yml)
[![macOS](https://github.com/jimmejardine/moddil/actions/workflows/test-macos.yml/badge.svg?branch=main)](https://github.com/jimmejardine/moddil/actions/workflows/test-macos.yml)
[![Licence: MIT OR Apache-2.0](https://img.shields.io/badge/licence-MIT%20OR%20Apache--2.0-blue)](#licence)
[![Rust 1.100.0](https://img.shields.io/badge/rust-1.100.0-orange)](rust-toolchain.toml)

**Moddil: Rust's armour, without the baggage.**

Moddil is a programming language being designed for three shifts happening at once:

- **LLM agents write more of the code**, and humans mostly review and approve it.
- **The machine is heterogeneous**: many CPU cores, GPUs and other accelerators.
- **A program is a fleet**: browser clients, servers, workers and databases working together.

Its core idea is **semantics that are explicit and checked by machines**. Effects, capabilities,
ownership and resources, and where code and data live all appear in the types and are checked by
the compiler. That makes code easier for agents to write correctly and for humans to review.

## Start here

Read [`specs/README.md`](specs/README.md) first. It is the **index**: every area with a status
(⚪ not started, 🔴 superficial, 🟠 major questions, 🟡 minor questions, 🟢 complete) and a link to
the document that holds its detail. Then read [Why Moddil](specs/vision.md) for the thesis, and the
[firm requirements](specs/requirements.md#firm-requirements). People and agents editing
the specs should follow [Working on these specs](specs/README.md#working-on-these-specs).

The language itself is defined by the [2027 reference](reference/README.md) and its grammar,
[`reference/grammar/moddil.ebnf`](reference/grammar/moddil.ebnf); the
[conformance suite](conformance/README.md) checks every implementation against it. Code in the
specs follows [`syntax.md`](specs/syntax.md) and the grammar, but not all of it runs yet: the
[roadmap](specs/roadmap.md) has the milestones and what is still to build.

## Layout

```
specs/        the design: one document per area, indexed by specs/README.md
reference/    the normative reference, and the grammar (reference/grammar/moddil.ebnf)
conformance/  the shared test suite every implementation must pass
crates/       the Rust implementation: the compiler, the MIR and reference interpreters, the
              native and WASM backends, the C and Python bindings, and the language server
mods/         the standard mods, written in Moddil
tour/         the dogfood tour, the tour/shorty mod
examples/     the reference examples as programs: Mandelbulb, zlib, a C ABI, a Python module
editors/      editor support: the tree-sitter grammar
model/        the Lean model of the core
bootstrap/    reserved for after self-hosting
tools/        helper scripts: the CI script, the spec link checker and the grammar check
```

The full layout, including the directories that fill up during M0, is in
[`specs/compiler.md`: Repository layout](specs/compiler.md#repository-layout).

## Building

Needs [rustup](https://rustup.rs) and Python 3.8 or later. The pinned Rust toolchain installs
itself on first use. Node (with JSPI) and Chrome are optional: without them the WASM and browser
tests are skipped.

```
cargo build --workspace
python tools/ci.py        # fmt, clippy, deny, test, tour, examples, links, grammars: as GitHub runs
python tools/ci.py quick  # while working: the tests of what you changed (name areas: quick borrows)
```

A small program, saved as `hello.moddil`:

```rust
fn add(a: int, b: int) -> int { a + b }

fn main() -> int { add(2, 3) }

test "add" {
    assert(add(2, 2) == 4);
    add(2, 3)                 // => 5
}
```

```
cargo run -p moddil -- run hello.moddil     # prints 5
cargo run -p moddil -- test hello.moddil    # test "add" ... ok
```

A test checks values in two ways: `// =>` states the value of the expression before it, and
`assert` checks a condition. When an `assert` fails, the trap shows the value of every part of
the condition ([`specs/testing.md`: Assertions](specs/testing.md#assertions)).

Each command takes a file, or a mod's directory (with a `moddil.toml`):

```
moddil check     reports the errors in a program
moddil run       runs fn main(), compiled to native code (--interp: on the MIR interpreter;
                 --wasm: as WebAssembly, in Node), and prints its result
moddil test      runs the mod's tests, natively (--interp, --wasm as for run)
moddil fmt       rewrites source in canonical format (--check: lists what is not)
moddil build     compiles fn main() into an executable (--release: a release build)
  --wasm         WebAssembly and the JS glue that runs it in a browser or in Node
  --lib          the mod's #[export] functions as a C library and its header (--static: static)
  --wheel        the mod's #[export(python)] functions and structs as a Python module and wheel
moddil lsp       the language server, for an editor
```

See [`crates/README.md`](crates/README.md) for more.

## Licence

Moddil is dual-licensed under either of

- [MIT license](LICENSE-MIT) ([`https://opensource.org/licenses/MIT`](https://opensource.org/licenses/MIT))
- [Apache License, Version 2.0](LICENSE-APACHE) ([`https://www.apache.org/licenses/LICENSE-2.0`](https://www.apache.org/licenses/LICENSE-2.0))

at your option. This is the same dual-licence arrangement used by the Rust project itself, and by the majority of the Rust ecosystem.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in Moddil by you, as defined in the Apache-2.0 licence, shall be dual-licensed as above, without any additional terms or conditions.

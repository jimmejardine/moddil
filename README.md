<img src="moddil.png" alt="Moddil logo" align="right" width="150">

# Moddil

[![checks](https://github.com/jimmejardine/moddil/actions/workflows/checks.yml/badge.svg?branch=main)](https://github.com/jimmejardine/moddil/actions/workflows/checks.yml)
[![grammars](https://github.com/jimmejardine/moddil/actions/workflows/grammars.yml/badge.svg?branch=main)](https://github.com/jimmejardine/moddil/actions/workflows/grammars.yml)
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

Moddil is **early in M0**, the first milestone. The design in `specs/` ranges from brainstorm to
decided direction; none of it is a formal specification yet. Implemented so far:

- **The grammar** for all decided syntax, in
  [`reference/grammar/moddil.ebnf`](reference/grammar/moddil.ebnf), checked LL(3). GBNF, Lark and
  tree-sitter grammars are generated from it.
- **A lexer and parser** for the whole grammar.
- **Slices S0 to S7a, end to end**: functions, `let`, `int` and `bool`, arithmetic with overflow
  traps, `if`, blocks and calls (S0); tuples, structs, enums, exhaustive `match`, `if let`,
  `let … else`, loops and `assert` (S1); moves, `Copy`, borrows checked on MIR, and `defer` (S2);
  generics, traits, methods, operators through traits, derives, destructors and a prelude
  written in Moddil (S3); errors with `fail`, `?`, `?!`, `try` and `catch`, `errdefer` and
  `Result` (S4); effects: traits and context data after `with`, handlers, inference of a private
  function's effects and errors, and `dyn` borrows (S5); several files and mods: the module tree,
  visibility, `use`, the `moddil.toml` manifest and dependencies by path (S6); closures, the
  closure traits, anonymous impls and `#[track_caller]` (S7a).
  They go through names and types, the MIR and the MIR interpreter, and run on the independent
  reference interpreter too. Both are checked by the
  [conformance suite](conformance/README.md), whose tests cite the
  [2027 reference](reference/README.md).

Code in the specs follows [`syntax.md`](specs/syntax.md) and the grammar, but beyond S7a none of it
runs yet; the [M0 slices](specs/roadmap.md#m0-the-v0-language-in-slices) add the rest.

## Start here

Read [`specs/README.md`](specs/README.md) first. It is the **index**: every area with a status
(⚪ not started, 🔴 superficial, 🟠 major questions, 🟡 minor questions, 🟢 complete) and a link to
the document that holds its detail. Then read [Why Moddil](specs/vision.md) for the thesis, and the
[firm requirements](specs/requirements.md#firm-requirements-decided). People and agents editing
the specs should follow [Working on these specs](specs/README.md#working-on-these-specs).

## Layout

```
specs/        the design: one document per area, indexed by specs/README.md
reference/    the normative reference, and the grammar (reference/grammar/moddil.ebnf)
conformance/  the shared test suite every implementation must pass
crates/       the Rust implementation: the stage-0 compiler and its tools
editors/      editor support: the tree-sitter grammar
tools/        helper scripts: the spec link checker, the grammar check and the CI script
```

The full layout, including the directories that fill up during M0, is in
[`specs/compiler.md`: Repository layout](specs/compiler.md#repository-layout).

## Building

Needs [rustup](https://rustup.rs) and Python 3.8 or later. The pinned Rust toolchain installs
itself on first use.

```
cargo build --workspace
python tools/ci.py        # fmt, clippy, deny, test, links, grammars: the same checks GitHub runs
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

See [`crates/README.md`](crates/README.md) for more.

## Licence

Moddil is dual-licensed under either of

- [MIT license](LICENSE-MIT) ([`https://opensource.org/licenses/MIT`](https://opensource.org/licenses/MIT))
- [Apache License, Version 2.0](LICENSE-APACHE) ([`https://www.apache.org/licenses/LICENSE-2.0`](https://www.apache.org/licenses/LICENSE-2.0))

at your option. This is the same dual-licence arrangement used by the Rust project itself, and by the majority of the Rust ecosystem.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in Moddil by you, as defined in the Apache-2.0 licence, shall be dual-licensed as above, without any additional terms or conditions.

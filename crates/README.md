# Crates

The **Rust implementation** of Moddil: the stage-0 compiler and its tools, in one Cargo workspace
whose `Cargo.toml` sits at the repository root
([`specs/compiler.md`](../specs/compiler.md), [Repository layout](../specs/compiler.md#repository-layout)).

**Status:** the grammar checker and the parser cover the whole grammar; names, types, MIR, the
MIR interpreter, the reference interpreter, the `moddil` command and the conformance runner cover
slices S0 to S7a ([M0 slices](../specs/roadmap.md#m0-the-v0-language-in-slices)).

## Building and checking

```
cargo build --workspace
cargo run -p moddil -- --version
python tools/ci.py             # every CI step on this machine: fmt, clippy, deny, test, links
python tools/ci.py --wsl       # the same steps in WSL (Linux); --all runs both
```

- **Toolchain:** pinned to Rust 1.100.0 in [`rust-toolchain.toml`](../rust-toolchain.toml); rustup
  installs it on first use.
- **Dependencies** are declared once, in the root [`Cargo.toml`](../Cargo.toml) under
  `[workspace.dependencies]`; a crate takes one with `name.workspace = true` when a slice needs it.
- **Lints** are workspace-wide (`[workspace.lints]`); CI denies warnings.
- **Supply chain:** [`deny.toml`](../deny.toml) checks licences, advisories and sources with
  `cargo-deny`.
- **CI** is [`tools/ci.py`](../tools/ci.py), called step by step by four GitHub workflows in
  [`.github/workflows/`](../.github/workflows): `checks.yml`, and `test-linux.yml`,
  `test-windows.yml` and `test-macos.yml`, which share `_test.yml`. Each has a badge in the root
  README.

| Crate | Package | Role |
|---|---|---|
| [`grammar/`](grammar/README.md) | `moddil-grammar` | Checks the EBNF grammar and generates tests and grammars |
| [`syntax/`](syntax/README.md) | `moddil-syntax` | Lexer, parser and AST; later the formatter |
| [`sema/`](sema/README.md) | `moddil-sema` | Names, types and effects; produces the typed HIR |
| [`mir/`](mir/README.md) | `moddil-mir` | The MIR, its phases, and ownership checking ([`specs/mir.md`](../specs/mir.md)) |
| [`interp/`](interp/README.md) | `moddil-interp` | The MIR interpreter |
| [`refinterp/`](refinterp/README.md) | `moddil-refinterp` | The reference interpreter |
| [`conformance/`](conformance/README.md) | `moddil-conformance` | The conformance runner |
| [`native/`](native/README.md) | `moddil-native` | The native backend: MIR to Cranelift, run by its JIT (M1) |
| [`wasm/`](wasm/README.md) | `moddil-wasm` | The WASM backend: the native lowering's Cranelift IR translated to core WebAssembly modules, run in Node (M1) |
| [`rt/`](rt/README.md) | `moddil-rt` | The runtime native code calls: traps, the heap, tasks, numbers and text |
| [`rt-main/`](rt-main/README.md) | `moddil-rt-main` | A built program's `main`: the static library `moddil build` links with |
| [`gpu/`](gpu/src/lib.rs) | `moddil-gpu` | Kernels (`#[kernel]`) to WGSL: their MIR built as naga IR, with structured control flow and a thread per pixel, validated by naga, and written out (M2e, M2g) |
| [`lsp/`](lsp/src/lib.rs) | `moddil-lsp` | The language server (`moddil lsp`): diagnostics, hover, go to definition, inlay hints and completion over the compiler's salsa queries (M2f) |
| [`os/`](os/src/lib.rs) | `moddil-os` | The host's side of `core::os` for the runtimes written in Rust (the MIR interpreter, and moddil-rt natively): sockets, arguments, standard error, randomness and signals |
| [`rt-wasm/`](rt-wasm/README.md) | `moddil-rt-wasm` | `moddil-rt` built for wasm32: the runtime a WASM module shares its memory with |
| [`project/`](project/README.md) | `moddil-project` | Reads a program from disk: the manifest, the module tree, dependencies by path |
| [`moddil/`](moddil/README.md) | `moddil` | The `moddil` command |

## Rules

- **Query-based and incremental** (Salsa), demand-driven, down to individual declarations.
- **Many small crates:** start coarse and split as they grow, so the compiler itself builds fast.
- **`refinterp` depends only on `syntax`** (and general-purpose crates). The test
  `crates/refinterp/tests/independence.rs` enforces this, so the reference interpreter stays
  independent of the compiler's semantics.
- **Interpreter first:** no code-generation backend exists until M1 (Cranelift, then LLVM and the own
  WASM emitter).

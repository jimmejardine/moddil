# Crates

The **Rust implementation** of Moddil: the stage-0 compiler and its tools, in one Cargo workspace
whose `Cargo.toml` sits at the repository root
([`specs/compiler.md`](../specs/compiler.md), [Repository layout](../specs/compiler.md#repository-layout)).

**Status:** no code yet. The grammar comes first; the crates follow slice by slice
([M0 slices](../specs/roadmap.md#m0-the-v0-language-in-slices)).

| Crate | Package | Role |
|---|---|---|
| [`grammar/`](grammar/README.md) | `moddil-grammar` | Checks the EBNF grammar and generates tests and grammars |
| [`syntax/`](syntax/README.md) | `moddil-syntax` | Lexer, parser and AST; later the formatter |
| [`sema/`](sema/README.md) | `moddil-sema` | Names, types and effects; produces the typed HIR |
| [`mir/`](mir/README.md) | `moddil-mir` | The MIR, its phases, and ownership checking ([`specs/mir.md`](../specs/mir.md)) |
| [`interp/`](interp/README.md) | `moddil-interp` | The MIR interpreter |
| [`refinterp/`](refinterp/README.md) | `moddil-refinterp` | The reference interpreter |
| [`conformance/`](conformance/README.md) | `moddil-conformance` | The conformance runner |
| [`moddil/`](moddil/README.md) | `moddil` | The `moddil` command |

## Rules

- **Query-based and incremental** (Salsa), demand-driven, down to individual declarations.
- **Many small crates:** start coarse and split as they grow, so the compiler itself builds fast.
- **`refinterp` depends only on `syntax`** (and general-purpose crates). CI enforces this, so the
  reference interpreter stays independent of the compiler's semantics.
- **Interpreter first:** no code-generation backend exists until M1 (Cranelift, then LLVM and the own
  WASM emitter).

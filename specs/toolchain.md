# The Toolchain

> **Status:** proposal. Expands [`rationale.md`](rationale.md) §2.17. Docs and tests are detailed in
> [`docs-and-tests.md`](docs-and-tests.md); the agent-facing API in
> [`agent-interface.md`](agent-interface.md); packages in
> [`packages-and-supply-chain.md`](packages-and-supply-chain.md). Commands are illustrative.

## 1. Principle

**One binary (`moddil`), one manifest, one front end.**

Every tool (build, test, formatter, linter, docs, language server, debugger, profiler, agent API) is
a client of the same query engine running in the compiler daemon
([`build-times.md`](build-times.md) §4.2). Tools therefore never disagree about what the code means,
and all of them are fast because the daemon is warm.

**Lessons:**
- *Fragmentation:* JavaScript (webpack, vite, esbuild; eslint, prettier; jest, vitest) and Python
  (pip, poetry, conda; black, flake8) spend enormous effort choosing and configuring tools that
  disagree with each other.
- *Success:* Go (`go build`, `gofmt`, `go vet`), Rust (Cargo, rustfmt, Clippy, rust-analyzer), Zig,
  Deno (format, lint, test, doc, bench, compile and LSP in one binary), and uv and ruff (one fast tool
  replacing many).

## 2. Commands

| Command | Purpose |
|---|---|
| `new`, `add`, `update`, `publish`, `audit`, `vet` | Mods and the supply chain ([`packages-and-supply-chain.md`](packages-and-supply-chain.md)) |
| `build`, `run`, `check` | Build, run, and check without code generation ([`build-times.md`](build-times.md)) |
| `test`, `bench` | Attached tests and benchmarks ([`docs-and-tests.md`](docs-and-tests.md)) |
| `fmt` | Canonical formatter (§3) |
| `lint`, `fix` | Built-in lints and machine-applicable fixes (§4) |
| `doc` | Checked and generated documentation ([`docs-and-tests.md`](docs-and-tests.md)) |
| `explain` | Expansions, desugarings, effect sets, error sets, handler stacks |
| `migrate` | Edition and API migrations ([`evolution.md`](evolution.md)) |
| `sim`, `replay` | Deterministic simulation and replay ([`determinism.md`](determinism.md)) |
| `debug`, `profile` | Debugger (§6) and profiler (§7) |
| `lsp`, `serve` | Language server; tool and agent API ([`agent-interface.md`](agent-interface.md)) |
| `graph` | Export the build graph (§10) |
| `toolchain` | Install and select toolchain versions (§8) |

Every command supports `--format json`, with stable diagnostic codes, so tools and agents never parse
human-oriented text.

## 3. Formatter

- **Canonical.** Like `gofmt` and Black, the formatter has no style options. Source text is a print
  of the canonical AST (rationale §2.18, §2.19), so formatting never appears in a diff.
- Comments and blank-line grouping are part of the AST and are preserved.
- `moddil fmt --check` runs in CI; editors format on save.
- **One setting, recommended:** a project-wide line width in the manifest (default 120). Teams
  genuinely differ here (Hashiverse, for example, uses a 250-character width in rustfmt), and a width
  changes no meaning. Everything else is fixed.

## 4. Linter

- **Built into the compiler.** Lints are compiler queries like any other diagnostic, so they are
  incremental and identical in the CLI, CI and the language server.
- Levels `allow`, `warn` and `deny` per mod in the manifest, with lint groups.
- Every lint that can be fixed mechanically has a machine-applicable fix, applied with `moddil fix`.
- **The lint set stays small.** Many classic lints are type errors in Moddil: unhandled errors (R3),
  undeclared effects and capabilities, unit mismatches, data races.
- **Custom lints are mods:** pure, sandboxed comptime functions over the semantic model
  ([`comptime.md`](comptime.md)). They give Clippy-style power without plugin trust problems, and run
  in the same daemon.

## 5. Editor support

- The daemon speaks **LSP**, so any LSP editor works.
- Moddil-specific features:
  - hovers showing effects, error sets, units, capabilities and contracts;
  - inlay hints at suspension points ([`closures.md`](closures.md) §9);
  - the active handler stack in tests and simulations ([`determinism.md`](determinism.md) §4);
  - `explain` expansions inline;
  - a semantic diff view (rationale §2.19).
- An official **tree-sitter grammar** for syntax highlighting elsewhere (GitHub, other editors and
  tools).
- Official editor integrations: VS Code, Zed, Neovim and JetBrains (via LSP). Priority order is open.

## 6. Debugger

- Debug information as DWARF (via `gimli`); a built-in **DAP** (Debug Adapter Protocol) server, so
  any DAP editor can debug.
- **Task-aware:** the task tree, each task's stack, and what each task is blocked on
  ([`observability.md`](observability.md)).
- Pretty-printers generated from static reflection, so user types display well without extra code.
- **Time travel** through record and replay ([`determinism.md`](determinism.md) §5).
- Works across live update ([`build-times.md`](build-times.md) §4.1).
- Bare metal: through the GDB stubs of QEMU and Renode ([`freestanding.md`](freestanding.md)).

## 7. Profiler

- A built-in **sampling profiler**, task-aware, producing flame graphs.
- **Allocation profiling** through the allocator capability.
- Counters for slow-path reference-count operations
  ([`memory-and-resources.md`](memory-and-resources.md) §5.3).
- Effect-level tracing ([`observability.md`](observability.md)).
- Build profiling with `moddil build --timings` ([`build-times.md`](build-times.md) §6).

## 8. Toolchain installation and versions

- One self-contained download per host; every toolchain is a cross-compiler
  ([`targets.md`](targets.md)).
- **The toolchain version is pinned in the manifest and fetched automatically** (like Go's
  `toolchain` directive since Go 1.21: rustup, but built in).
- Toolchains are reproducible and verifiable ([`bootstrap.md`](bootstrap.md),
  [`determinism.md`](determinism.md) §6).

## 9. Configuration

- **One manifest file** per mod or workspace holds everything: dependencies, capabilities, targets,
  profiles, lint levels, line width and toolchain version.
- No tool-specific dotfiles (`.eslintrc`, `.prettierrc`, `rustfmt.toml`, `clippy.toml`).

## 10. Extensibility without fragmentation

- **Extension points are mods:** lints, derives, tagged-literal DSLs, test strategies, simulation
  handlers. There are no alternative formatters or build systems to choose between.
- **Monorepo build systems** (Bazel, Buck2) integrate through an exported build graph
  (`moddil graph --format json`) and the remote cache protocol, rather than reimplementing the
  compiler.
- The **compiler-as-a-service API** ([`agent-interface.md`](agent-interface.md)) is versioned, so
  third-party tools see exactly what official tools see.

## 11. Budgets

Tool latencies follow [`build-times.md`](build-times.md) §6: keystroke to diagnostics under 100 ms;
`fmt` and `lint` effectively instant on a warm daemon.

## 12. Open questions

1. The formatter's line-width setting (§3): accept the single knob, or none at all?
2. A REPL or notebook mode: worth having, given Moddil's focus?
3. Editor priority order (§5).
4. Custom lint mods are pure and sandboxed, but they still cost build time. Do they need limits or
   review?
5. Should `moddil` host third-party subcommands (`cargo-xyz` style), or keep a closed command set?

## Prior art

- Go toolchain: `gofmt`, `go vet`, the `toolchain` directive.
- Rust: Cargo, rustfmt, Clippy, rust-analyzer, rustup.
- Zig; Deno; Bun; uv and ruff; Black; Prettier.
- tree-sitter; the Language Server Protocol; the Debug Adapter Protocol.
- Bazel; Buck2.

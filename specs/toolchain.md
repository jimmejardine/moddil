# The Toolchain

> **Status:** proposal. Indexed in [`README.md`](README.md). Docs and tests are detailed in
> [`docs-and-tests.md`](docs-and-tests.md); the agent-facing API in
> [`agent-interface.md`](agent-interface.md); packages in
> [`packages-and-supply-chain.md`](packages-and-supply-chain.md). Commands are illustrative.

## Principle

**One binary (`moddil`), one manifest, one front end.**

Every tool (build, test, formatter, linter, docs, language server, debugger, profiler, agent API) is
a client of the same query engine running in the compiler daemon
([`build-times.md`: Cached builds](build-times.md#cached-builds)). Tools therefore never disagree about what the code means,
and all of them are fast because the daemon is warm.

**Lessons:**
- *Fragmentation:* JavaScript (webpack, vite, esbuild; eslint, prettier; jest, vitest) and Python
  (pip, poetry, conda; black, flake8) spend enormous effort choosing and configuring tools that
  disagree with each other.
- *Success:* Go (`go build`, `gofmt`, `go vet`), Rust (Cargo, rustfmt, Clippy, rust-analyzer), Zig,
  Deno (format, lint, test, doc, bench, compile and LSP in one binary), and uv and ruff (one fast tool
  replacing many).

## Commands

| Command | Purpose |
|---|---|
| `new`, `add`, `update`, `outdated`, `vendor`, `publish`, `audit`, `vet` | Mods and the supply chain ([`packages-and-supply-chain.md`](packages-and-supply-chain.md)) |
| `build`, `run`, `check` | Build, run, and check without code generation ([`build-times.md`](build-times.md)); `run --watch` re-checks and re-runs on save ([Playground: learning without a REPL](#playground-learning-without-a-repl)) |
| `test`, `bench` | Attached tests and benchmarks ([`docs-and-tests.md`](docs-and-tests.md)) |
| `fmt` | Canonical formatter ([Formatter](#formatter)) |
| `lint`, `fix` | Built-in lints and machine-applicable fixes ([Linter](#linter)) |
| `doc` | Checked and generated documentation ([`docs-and-tests.md`](docs-and-tests.md)) |
| `diff`, `log`, `blame` | Semantic report between two Git revisions, with a suggested PR description; definition-level history ([`semantic-vcs.md`](semantic-vcs.md)) |
| `explain` | Expansions, desugarings, effect sets, error sets, handler stacks |
| `migrate` | Edition migrations, checked equivalent; library replacement rules are applied by `fix` ([`evolution.md`](evolution.md)) |
| `sim`, `replay` | Deterministic simulation and replay ([`determinism.md`](determinism.md)) |
| `debug`, `profile` | Debugger ([Debugger](#debugger)) and profiler ([Profiler](#profiler)) |
| `lsp`, `serve` | Language server; tool and agent API ([`agent-interface.md`](agent-interface.md)) |
| `graph` | Export the build graph ([Extensibility without fragmentation](#extensibility-without-fragmentation)) |
| `toolchain` | Install and select toolchain versions ([Toolchain installation and versions](#toolchain-installation-and-versions)) |

Every command supports `--format json`, with stable diagnostic codes, so tools and agents never parse
human-oriented text.

## Formatter

- **Canonical.** Like `gofmt` and Black, the formatter has no style options. Source text is a print
  of the canonical AST ([LLMs as first-class programmers](agent-interface.md), [Diffs, code review and version control](semantic-vcs.md)), so formatting never appears in a diff.
- Comments and blank-line grouping are part of the AST and are preserved.
- `moddil fmt --check` runs in CI; editors format on save.
- **One setting, recommended:** a project-wide line width in the manifest (default 120). Teams
  genuinely differ here (Hashiverse, for example, uses a 250-character width in rustfmt), and a width
  changes no meaning. Everything else is fixed.

## Linter

- **Built into the compiler.** Lints are compiler queries like any other diagnostic, so they are
  incremental and identical in the CLI, CI and the language server.
- Levels `allow`, `warn` and `deny` per mod in the manifest, with lint groups.
- Every lint that can be fixed mechanically has a machine-applicable fix, applied with `moddil fix`.
- **The lint set stays small.** Many classic lints are type errors in Moddil: unhandled errors (R3),
  undeclared effects and capabilities, unit mismatches, data races.
- **Custom lints are mods:** pure, sandboxed comptime functions over the semantic model
  ([`comptime.md`](comptime.md)). They give Clippy-style power without plugin trust problems, and run
  in the same daemon.

## Editor support

- The daemon speaks **LSP**, so any LSP editor works.
- Moddil-specific features:
  - hovers showing effects, error sets, units, capabilities and contracts;
  - inlay hints at suspension points ([`closures.md`: Seeing suspension points](closures.md#seeing-suspension-points));
  - the active handler stack in tests and simulations ([`determinism.md`: Handlers are scoped, not global](determinism.md#handlers-are-scoped-not-global));
  - `explain` expansions inline;
  - a semantic diff view ([Diffs, code review and version control](semantic-vcs.md)).
- An official **tree-sitter grammar** for syntax highlighting elsewhere (GitHub, other editors and
  tools).
- Official editor integrations: VS Code, Zed, Neovim and JetBrains (via LSP). Priority order is open.

## Debugger

- Debug information as DWARF (via `gimli`); a built-in **DAP** (Debug Adapter Protocol) server, so
  any DAP editor can debug.
- **Task-aware:** the task tree, each task's stack, and what each task is blocked on
  ([`observability.md`](observability.md)).
- Pretty-printers generated from static reflection, so user types display well without extra code.
- **Time travel** through record and replay ([`determinism.md`: Record and replay](determinism.md#record-and-replay)).
- Works across live update ([`build-times.md`: Live update (debug builds)](build-times.md#live-update-debug-builds)).
- Bare metal: through the GDB stubs of QEMU and Renode ([`freestanding.md`](freestanding.md)).

## Profiler

- A built-in **sampling profiler**, task-aware, producing flame graphs.
- **Allocation profiling** through the allocator capability.
- Counters for slow-path reference-count operations
  ([`memory-and-resources.md`: Biased reference counting](memory-and-resources.md#biased-reference-counting)).
- Effect-level tracing ([`observability.md`](observability.md)).
- Build profiling with `moddil build --timings` ([`build-times.md`: Budgets](build-times.md#budgets)).

## Toolchain installation and versions

- One self-contained download per host; every toolchain is a cross-compiler
  ([`targets.md`](targets.md)).
- **The toolchain version is pinned in the manifest and fetched automatically** (like Go's
  `toolchain` directive since Go 1.21: rustup, but built in).
- Toolchains are reproducible and verifiable ([`bootstrap.md`](bootstrap.md),
  [`determinism.md`: Reproducible builds](determinism.md#reproducible-builds)).

## Configuration

- **One manifest file** per mod or workspace holds everything: dependencies, capabilities, targets,
  profiles, lint levels, line width and toolchain version.
- No tool-specific dotfiles (`.eslintrc`, `.prettierrc`, `rustfmt.toml`, `clippy.toml`).

## Extensibility without fragmentation

- **Extension points are mods:** lints, derives, tagged-literal DSLs, test strategies, simulation
  handlers. There are no alternative formatters or build systems to choose between.
- **Monorepo build systems** (Bazel, Buck2) integrate through an exported build graph
  (`moddil graph --format json`) and the remote cache protocol, rather than reimplementing the
  compiler.
- The **compiler-as-a-service API** ([`agent-interface.md`](agent-interface.md)) is versioned, so
  third-party tools see exactly what official tools see.

## Budgets

Tool latencies follow [`build-times.md`: Budgets](build-times.md#budgets): keystroke to diagnostics under 100 ms;
`fmt` and `lint` effectively instant on a warm daemon.

### Playground: learning without a REPL

A REPL, script mode and notebook mode are non-goals ([`requirements.md`: Non-goals (draft)](requirements.md#non-goals-draft)): a
REPL session would need exceptions to R3, borrow scopes, linear values, task scopes and
redefinition. Quick, REPL-style learning comes from **compilation fast enough to re-run the whole
program on every edit**, in an **in-browser playground**:

- **No server.** The compiler itself runs as WASM in the page (it builds to WASM anyway,
  [`bootstrap.md`: Bootstrapping after self-hosting](bootstrap.md#bootstrapping-after-self-hosting)), and the program compiles to WASM and runs there too.
- **REPL feel, program semantics.** Every keystroke re-checks the program (diagnostics, with
  inferred types and effects as hints); every successful build re-runs it. There is no session
  state, so none of a notebook's hidden-state problems, and no exceptions to the language's rules.
- **Small in the browser.** The page contains the front end, the MIR interpreter (instant for small
  programs) and Moddil's own WASM emitter (for speed). LLVM and Cranelift are not needed.
- **Standard mods** arrive as precompiled interface artefacts and code from the cache
  ([`build-times.md`: Cached builds](build-times.md#cached-builds)), loaded lazily.
- **Effects in the playground:** `fs`, `net`, `time` and `rand` use simulation handlers
  ([`determinism.md`](determinism.md)): a virtual file system, a simulated network, virtual time
  and a seeded RNG. Real `fetch` is optional, with the user's consent. Suspension uses JSPI; threads
  use cross-origin isolation (the playground sets COOP/COEP); kernels use WebGPU.
- **Shareable and reproducible.** A link encodes the source and the seed. Runs are deterministic, so
  everyone who opens the link sees exactly the same output.
- **Learning material:** a guided tour (in the style of the Go tour and the Rust Book); a "Run"
  button on every documentation example (examples are tests,
  [`docs-and-tests.md`: Examples are tests](docs-and-tests.md#examples-are-tests)); runnable error-explanation pages.
- **On the desktop**, `moddil run --watch` gives the same loop: re-check and re-run on save, with
  live update for long-running programs ([`build-times.md`: Live update (debug builds)](build-times.md#live-update-debug-builds)).
- **Costs and risks:** the compiler's download size (estimated in the tens of MB; loaded lazily; to
  be measured) and compile speed in the browser against the budget above. The playground is a
  tier-1 target in CI.

Prior art: the TypeScript playground (compiler in the browser); the Go playground and tour, and the
Rust playground (server-side compilation); Elm reactor; Swift Playgrounds; Zig's WASM build of its
compiler.

## Open questions

1. The formatter's line-width setting ([Formatter](#formatter)): accept the single knob, or none at all?
2. Editor priority order ([Editor support](#editor-support)).
3. Custom lint mods are pure and sandboxed, but they still cost build time. Do they need limits or
   review?
4. Should `moddil` host third-party subcommands (`cargo-xyz` style), or keep a closed command set?

Decided: no REPL, script or notebook mode (a non-goal, [`requirements.md`: Non-goals (draft)](requirements.md#non-goals-draft));
quick learning comes from the in-browser playground ([Playground: learning without a REPL](#playground-learning-without-a-repl)).

## Prior art

- Go toolchain: `gofmt`, `go vet`, the `toolchain` directive.
- Rust: Cargo, rustfmt, Clippy, rust-analyzer, rustup.
- Zig; Deno; Bun; uv and ruff; Black; Prettier.
- tree-sitter; the Language Server Protocol; the Debug Adapter Protocol.
- Bazel; Buck2.

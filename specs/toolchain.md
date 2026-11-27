# The Toolchain

Docs and tests are detailed in [`docs.md`](docs.md); the agent-facing API in
[`agent-interface.md`](agent-interface.md); packages in
[`packages-and-supply-chain.md`](packages-and-supply-chain.md).

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
| `test`, `bench` | Attached tests and benchmarks ([`docs.md`](docs.md)) |
| `fmt` | Canonical formatter ([Formatter](#formatter)) |
| `lint`, `fix` | Built-in lints and machine-applicable fixes ([Linter](#linter)) |
| `doc` | Checked and generated documentation ([`docs.md`](docs.md)) |
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

**As built:** `moddil explain <file> [item]` prints, as source in the formatter's layout, what
the compiler writes for the program's own items, or for one (`Point`, `Counter::bump`): the impls
its derives expand to (one whose members a derive function's template gives says which
function's), the hidden items of its wrapped functions and of its tagged literals with fields,
and each wrapped function's body as the wrapper call it is. Each piece comes under a comment
naming the attribute or literal it comes from and where. Desugarings, effect sets, error sets
and handler stacks come later.

## Formatter

- **Canonical.** Like `gofmt` and Black, the formatter has no style options. Source text is a print
  of the canonical AST ([LLMs as first-class programmers](agent-interface.md), [Diffs, code review and version control](semantic-vcs.md)), so formatting never appears in a diff.
- Comments and blank-line grouping are part of the AST and are preserved.
- `moddil fmt --check` runs in CI; editors format on save.
- **No line width, and no setting.**
  - Lines break where the structure says, never because a line is long; editors wrap long lines
    for display, each as its user likes.
  - So the layout depends on the code and its comments alone, and the same code always prints
    the same way, whoever formats it.
  - A trailing comment always stays on the line of the code it follows, which keeps line markers
    such as the conformance suite's `//~` where they are.
- **A line comment is how a writer breaks a line by choice:** inside an argument list, an array or
  any other list, a comment (a bare `//` where there is nothing to say) ends its line, and nothing
  else breaks.
- **The layout** (a function's body always on lines of its own, `where` and contract clauses on
  lines of their own, method chains and match arms, and the rest) is in
  [`crates/fmt/README.md`](../crates/fmt/README.md).

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
- **Limits:** lint mods follow comptime's fuel, memory and caching rules, and
  `moddil build --timings` attributes their cost per lint. A lint runs only if the consuming project
  enables it, never just because a dependency ships it. Being pure, they need no security review.
- **Diagnostics for people**, in the rustc and Elm style: one primary message, labelled
  source snippets, notes, and a suggested fix when one is mechanical. Every error has a code with a
  long explanation (`moddil explain E0xxx`) whose examples are tests. Effect, borrow and coherence
  errors show the causal chain ("`parse` uses `Net` via `fetch_schema`, line 40"). A published style
  guide covers the wording: plain British English, and no jargon without a glossary link.

## Editor support

- The daemon speaks **LSP**, so any LSP editor works.
- Moddil-specific features:
  - hovers showing effects, error sets, units, capabilities and contracts;
  - inlay hints at suspension points ([`closures.md`: Seeing suspension points](closures.md#seeing-suspension-points));
  - the active handler stack in tests and simulations ([`determinism.md`: Handlers are scoped, not global](determinism.md#handlers-are-scoped-not-global));
  - `explain` expansions inline;
  - a semantic diff view ([Diffs, code review and version control](semantic-vcs.md)).
- **As built (M2f):** `moddil lsp` gives diagnostics, hover (types, signatures with effects,
  `suspend` and error sets), go to definition, inlay hints at suspension points, and completion
  (slice M2f). Units, capabilities and contracts on hover, the handler
  stack, `explain` inline and the semantic diff come later.
- An official **tree-sitter grammar** for syntax highlighting elsewhere (GitHub, other editors and
  tools).
- Official editor integrations, in priority order: VS Code (most users; the playground's
  editor shares its grammar), Zed, Neovim, then JetBrains (via LSP).

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
  ([`memory.md`: Biased reference counting](memory.md#biased-reference-counting)).
- Effect-level tracing ([`observability.md`](observability.md)).
- Build profiling with `moddil build --timings` ([`build-times.md`: Budgets](build-times.md#budgets)).

## Toolchain installation and versions

- One self-contained download per host; every toolchain is a cross-compiler
  ([`targets.md`](targets.md)).
- **The toolchain version is pinned in the manifest and fetched automatically** (like Go's
  `toolchain` directive since Go 1.21: rustup, but built in).
- Toolchains are reproducible and verifiable ([`compiler.md`](compiler.md),
  [`determinism.md`: Reproducible builds](determinism.md#reproducible-builds)).

## Configuration

- **One manifest file** per mod or workspace holds everything: dependencies, capabilities, targets,
  profiles, lint levels and toolchain version.
- No tool-specific dotfiles (`.eslintrc`, `.prettierrc`, `rustfmt.toml`, `clippy.toml`).

## Extensibility without fragmentation

- **Extension points are mods:** lints, derives, tagged-literal DSLs, test strategies, simulation
  handlers. There are no alternative formatters or build systems to choose between.
- **Monorepo build systems** (Bazel, Buck2) integrate through an exported build graph
  (`moddil graph --format json`) and the remote cache protocol, rather than reimplementing the
  compiler.
- The **compiler-as-a-service API** ([`agent-interface.md`](agent-interface.md)) is versioned, so
  third-party tools see exactly what official tools see.
- **Tools are pinned mods, not PATH plugins.** There is no `moddil-xyz` discovery on
  `PATH`. A tool is a mod with an executable, pinned in the manifest's `[tools]` section (in the
  lockfile, hash-checked), and run as `moddil tool <name> …` under the capabilities it declares. The
  core command set stays closed and the same everywhere.

## Budgets

Tool latencies follow [`build-times.md`: Budgets](build-times.md#budgets): keystroke to diagnostics under 100 ms;
`fmt` and `lint` effectively instant on a warm daemon.

### Playground: learning without a REPL

A REPL, script mode and notebook mode are non-goals ([`requirements.md`: Non-goals (draft)](requirements.md#non-goals-draft)): a
REPL session would need exceptions to R3, borrow scopes, linear values, task scopes and
redefinition. Quick, REPL-style learning comes from **compilation fast enough to re-run the whole
program on every edit**, in an **in-browser playground**:

- **No server.** The playground is static files at `moddil.com/play` ([`web.md`](web.md)).
  The compiler itself runs as WASM in the page (the Rust compiler built for
  `wasm32`; [`compiler.md`](compiler.md)), and the program compiles to WASM and runs there too.
- **REPL feel, program semantics.** Every keystroke re-checks the program (diagnostics, with
  inferred types and effects as hints); every successful build re-runs it. There is no session
  state, so none of a notebook's hidden-state problems, and no exceptions to the language's rules.
- **Small in the browser.** The page contains the front end, the MIR interpreter (instant for small
  programs) and Moddil's own WASM emitter (for speed). LLVM and Cranelift are not needed.
- **Standard mods** arrive as precompiled interface artefacts and code from the cache
  ([`build-times.md`: Cached builds](build-times.md#cached-builds)), loaded lazily.
- **Effects in the playground:** `Fs`, `Net`, `Time` and `Rand` use simulation handlers
  ([`determinism.md`](determinism.md)): a virtual file system, a simulated network, virtual time
  and a seeded RNG. Real `fetch` is optional, with the user's consent. Suspension uses JSPI; threads
  use cross-origin isolation (the playground sets COOP/COEP); kernels use WebGPU.
- **Shareable and reproducible.** A link encodes the source and the seed. Runs are deterministic, so
  everyone who opens the link sees exactly the same output.
- **Learning material:** a guided tour (in the style of the Go tour and the Rust Book); a "Run"
  button on every documentation example (examples are tests,
  [`testing.md`: Examples are tests](testing.md#examples-are-tests)); runnable error-explanation pages.
- **On the desktop**, `moddil run --watch` gives the same loop: re-check and re-run on save, with
  live update for long-running programs ([`build-times.md`: Live update (debug builds)](build-times.md#live-update-debug-builds)).
- **Costs and risks:** the compiler's download size (loaded lazily) and compile speed in the
  browser against the budget above ([Open questions](#open-questions)). The playground is a tier-1
  target in CI.

Prior art: the TypeScript playground (compiler in the browser); the Go playground and tour, and the
Rust playground (server-side compilation); Elm reactor; Swift Playgrounds; Zig's WASM build of its
compiler.

## Open questions

- How large the compiler's download for the playground is (estimated in the tens of MB) and how
  fast it compiles in the browser against the budget: both are still to be measured.

## Alternatives considered

- **A line width in the manifest** for the formatter: not taken. With no width the layout depends
  on the code and its comments alone, so the same code always prints the same way.
- **Tools discovered on `PATH`** (`moddil-xyz` plugins): not taken. A tool is a mod pinned in the
  manifest and the lockfile, hash-checked and run under its declared capabilities, and the core
  command set stays closed and the same everywhere.
- **A REPL, script mode or notebook mode**: not taken. A REPL session would need exceptions to R3,
  borrow scopes, linear values, task scopes and redefinition; the playground and `run --watch`
  re-run the whole program instead.

## Prior art

- Go toolchain: `gofmt`, `go vet`, the `toolchain` directive.
- Rust: Cargo, rustfmt, Clippy, rust-analyzer, rustup.
- Zig; Deno; Bun; uv and ruff; Black; Prettier.
- tree-sitter; the Language Server Protocol; the Debug Adapter Protocol.
- Bazel; Buck2.

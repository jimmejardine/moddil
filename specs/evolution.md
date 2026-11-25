# Language Evolution

## Problem

Languages either freeze (C) or break their ecosystems (Python 2→3). The failures are instructive:

| Case | What went wrong |
|---|---|
| Python 2 → 3 | Old and new code could not be mixed module by module; many changes were runtime semantics (`str` vs `bytes`) that no tool could detect statically; the split lasted over a decade |
| Perl 6 / Raku | The new version became a different language; the community split |
| Scala 2 → 3 | Macros had no migration path |
| C++ | The ABI freeze blocks fixes (`std::regex` stays slow) |
| Rust | Crates depending on nightly-only features; the async ecosystem split between runtimes |
| Swift 1 → 3 | Repeated source-breaking churn |
| Go | Ten years to add generics, because the compatibility promise left no room to change |

## State of the art

- **Rust editions:** chosen per crate; crates of different editions interoperate in one build;
  `cargo fix --edition` migrates; a three-year cadence (2015, 2018, 2021, 2024); **crater** builds
  the whole ecosystem to test compiler changes.
- **Go:** the Go 1 compatibility promise; the `go` line in each module's `go.mod` selects semantics
  for that module (Go 1.22 gave each loop iteration its own variable only in modules that declare
  `go 1.22`); `GODEBUG` settings; `go fix`.
- **Swift:** the Swift Evolution proposal process; language modes per module (Swift 5, Swift 6); a
  source-compatibility suite of real projects.
- **Opt-in experiments:** Java preview and incubator features; Kotlin `@OptIn`; Python
  `from __future__ import …`.
- **Data-driven library migrations:** Dart's `dart fix` rules shipped with packages; OpenRewrite for
  Java; Go's inline-directed fixes.
- **Unison:** code addressed by content hash, so existing code never breaks.

## Decisions

1. **Editions per mod**, declared in the manifest (`edition = "2027"`).
   - Mods of different editions **work together in one program**. This is guaranteed because a mod's
     **interface artefact is edition-neutral** ([`build-times.md`: Language rules that decide build speed](build-times.md#language-rules-that-decide-build-speed)).
   - An edition may change syntax, defaults, lints and name resolution. It may change semantics only
     **inside a mod, and only when that mod moves to the new edition** (Go 1.22's loop variable is
     the model), and **only if `moddil migrate` can rewrite old code to keep its old meaning**,
     checked by comparing MIR (decided). A change that no tool can migrate, such as Python 3's
     `str`/`bytes` split, is forbidden.
   - Nothing changes meaning across a mod boundary.
2. **A compatibility promise between editions.** Within an edition, every new toolchain release is
   backwards compatible (as with Go 1 and Rust's stability guarantee). The only exceptions are
   soundness and security fixes, each preceded by a lint period.
3. **Cadence (decided): on demand, with rising minimum gaps.** An edition comes only when there is
   something worth changing, and is kept small. The second edition may follow the first after at
   least one year, the third after at least two more, and every later one after at least three,
   while the language is young enough to need faster fixes.
4. **Automated, verified migrations.** `moddil migrate --edition 2030` rewrites a mod through
   compiler-driven AST transformations.
   - The result is **checked equivalent**: the old program under the old edition elaborates to the
     same MIR as the new program under the new edition.
   - It is reviewed with `moddil diff` ([`semantic-vcs.md`](semantic-vcs.md)), which reports, for
     example, "143 verified edition migrations, 0 behavioural changes".
   - Agents can run it.
5. **Libraries ship their own migrations.** A breaking release can carry machine-applicable rules:

   ```rust
   #[deprecated(since = "2.0", replace_with = "connect_with(host, Options { port, ..Default::default() })")]
   pub fn connect(host: &str, port: u16) -> Conn with Net, suspend, fail(NetError) { … }
   ```

   Rename rules cover types and modules. `moddil fix` applies the rules across downstream code (as
   Dart's data-driven fixes, Go's inline fixes and OpenRewrite do). Deprecated items warn for at
   least one released minor version **and** six months before a major release may remove them
   (decided); standard mods must ship `replace_with` rules, so `moddil fix` migrates users.
6. **No unstable features in published code.** Feature gates exist in development builds of the
   toolchain, but **`moddil publish` refuses to tag a mod that uses unstable features**, and consumers'
   builds refuse such dependencies (there is no registry,
   [`packages-and-supply-chain.md`](packages-and-supply-chain.md)). This avoids Rust's split
   between stable crates and crates that build only on nightly. Experiments live in development
   toolchains and forks, not in the ecosystem.
7. **Process.** A proposal process ([Moddil Evolution](governance.md#moddil-evolution), in the spirit of Rust RFCs and Swift
   Evolution). Every proposal must:
   - cite a reference example that needs it ([`examples/README.md`](examples/README.md));
   - include its migration if it changes anything;
   - pass an **ecosystem run**: build and test a **corpus of public mods** (a list kept in a public
     git repository, which authors opt into) from their tags with the new toolchain (Rust's crater,
     Swift's source-compatibility suite). Reproducible builds and the shared cache
     ([`build-times.md`: Cached builds](build-times.md#cached-builds)) keep this cheap.
8. **Long-term support (decided).** One toolchain release a year is designated LTS and receives
   security and soundness fixes for two years. Standard mods state the oldest LTS they support.
9. **Standard mods** follow semver, with the bump computed by `moddil diff`. They remove items only
   across editions, always with replacement rules ([`stdlib.md`](stdlib.md)).
10. **Agents and training data.**
   - The surface syntax stays small and stable ([`syntax.md`](syntax.md)).
   - Editions, deprecations and migration rules are machine-readable through the agent API
     ([`agent-interface.md`](agent-interface.md)).
   - The compiler recognises constructs from older editions and offers the machine-applicable fix,
     so models trained on older code still converge quickly.
11. **Binary and wire compatibility.** There is no stable native ABI; artefacts are rebuilt from
    source through the caches ([`types.md`](types.md)). WIT components and wire contracts are
    versioned and checked for compatibility ([`distribution.md`](distribution.md)).

## Fit with the reference examples

| Example | What evolution means for it |
|---|---|
| [Game](examples/multiplayer-game.md) | Client and server code shared across editions; each side upgrades independently |
| [HFT](examples/hft-platform.md) | Pinned toolchains; migrations verified as behaviour-preserving, which supports audit |
| [Web store](examples/web-store.md) | A large codebase moves edition with `moddil migrate`; library upgrades applied with `moddil fix` |
| [Hashiverse](examples/hashiverse.md) | Long-lived volunteer nodes run mixed versions, so wire compatibility matters more than source compatibility |

## Prior art

Rust editions, RFCs and crater; the Go 1 compatibility promise, `go.mod` version semantics,
`GODEBUG` and `go fix`; Swift Evolution, language modes and the source-compatibility suite; Java
preview features; Kotlin `@OptIn`; Python `__future__`; Dart data-driven fixes; OpenRewrite; Unison.

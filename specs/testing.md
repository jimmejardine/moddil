# Testing

> **Status:** complete: decided, with no open questions. Listed in the [index](README.md). Tests, examples,
> property tests, fuzzing, simulation and benchmarks attach to the code they test and run under one
> command, `moddil test`. Documentation is in [`docs.md`](docs.md). Snippets are **illustration only**.

## Summary

| Rule | One line |
|---|---|
| **Examples are tests** | Examples carry expected results (`// => value`), compile in the same build, and run under the deterministic simulation runtime ([Examples are tests](#examples-are-tests)). |
| **Tests name what they test** | `test "…" for withdraw { }` sits next to its subject, with private access. Tooling knows every test's subject ([Attached tests](#attached-tests)). |
| **Handlers, not mocks** | Tests run under the simulation runtime by default: virtual time, seeded randomness, simulated network and filesystem. |
| **Correctness tools attach the same way** | Property tests, fuzz targets, snapshots, interleaving tests, model checking and proofs all attach to a subject; contracts become tests automatically ([Correctness tools in the same place](#correctness-tools-in-the-same-place)). |
| **Only affected tests run** | Test impact analysis plus results cached by content hash ([Test impact and caching](#test-impact-and-caching)). |
| **Benchmarks attach too** | Benchmarks attach to their subject; budgets on deterministic counters fail CI ([Benchmarks](#benchmarks)). |

## Problem

Rust gets a lot right (`cargo test`, unit tests next to the code, doc tests), but tests are a bag of
functions with no link to what they test, doc tests are slow and cannot use private items, tests
perform real I/O and so flake, and every run re-runs everything unless outside tools are added. Mocks
are bolted on through traits written by hand. For agents that iterate dozens of times per task, slow
and flaky tests are the main brake.

## The testing toolbox

| Need | Mechanism | Defined in |
|---|---|---|
| Unit tests, examples | Attached `test` blocks and checked examples | This document |
| Fakes for time, network, files, randomness | Scoped effect handlers and the simulation runtime | [`determinism.md`: Deterministic simulation testing](determinism.md#deterministic-simulation-testing) |
| Properties, fuzzing, interleavings, proofs | Attached correctness blocks | [`correctness.md`](correctness.md) |
| Test strength | Mutation score per declaration | [`correctness.md`: Mutation testing](correctness.md#mutation-testing) |
| Performance | Benchmarks and counter budgets | [`performance.md`: Benchmarks and budgets](performance.md#benchmarks-and-budgets) |
| Whole systems | Many components in one simulated process | [`distribution.md`: Whole-system runs and simulation](distribution.md#whole-system-runs-and-simulation) |

## Examples are tests

- An example line may end with `// => value` (or `// => fail(E)`) as its expected result, like
  Python's and Elixir's doctests and Go's `// Output:`. Larger values use snapshot-style expected
  output stored next to the example.
- Examples compile **in the same build** as the mod, so they are fast and can use private helpers.
- Examples run under the **deterministic simulation runtime** ([`determinism.md`](determinism.md)):
  an example that "fetches a URL" gets a simulated network, so docs never flake and never need the
  internet.
- When a signature changes, broken examples are compile errors immediately, not stale text.
- Every example has a **Run** button that opens it in the in-browser playground
  ([`toolchain.md`: Playground: learning without a REPL](toolchain.md#playground-learning-without-a-repl)).

## Attached tests

- `test "name" for subject { … }` sits next to its subject (like Zig's `test` blocks and Pyret's
  `where:` clauses) and can see private items in its module.
- Tests without a single subject are module-level `test "name" { … }` blocks; larger scenarios live in
  separate integration-test modules.
- **Subjects are qualified paths (decided):** `test "…" for OrderBook::add` (a method),
  `for <Money as Display>` (a trait implementation) and `for sort::<int>` (one instantiation). A
  generic test is written once and run for listed types:
  `test "sorts" for sort::<T> with T in [int, f64, String] { … }`.
- **Fixtures and helpers (decided):** `#[test_only]` items and modules compile only in test builds,
  and the compiler checks that only tests and other test-only items use them. Helpers meant for other
  mods (fake handlers, builders) go in a separate `-testing` mod, so release builds never contain test
  code.
- **Capabilities in tests:** a test gets no root capabilities unless it declares them, and test code
  counts against the mod's ceiling ([`security.md`: Tests are held to the same rules](security.md#tests-are-held-to-the-same-rules)).
- **Tooling knows each test's subject**:
  - "which tests cover `withdraw`?" is a query;
  - the semantic diff ([Diffs, code review and version control](semantic-vcs.md)) reports "**`withdraw` changed, but none of its tests did**",
    a sharp signal when reviewing agent-written changes.
- **Tests use effect handlers, not mocks.** The simulation runtime is the default: virtual time,
  seeded randomness, simulated network and filesystem. Fakes are scoped handlers
  ([`determinism.md`: Handlers are scoped, not global](determinism.md#handlers-are-scoped-not-global)), not global monkey-patching.
- Tests, examples, notes and benchmarks are **excluded from the mod's interface hash and from release
  artefacts**, so editing a test never rebuilds dependents ([`build-times.md`](build-times.md)).

### Errors inside tests

**Decided: an error that escapes a test fails the test.** A test is an entry point with an explicit
exemption from R3 ([`requirements.md`: R3](requirements.md#r3-no-crash-from-an-unhandled-error)):
its body may use `?`, and any error that escapes, like any trap, fails that test and is reported with
its error return trace ([`error-handling.md`: Traces](error-handling.md#traces)). `main`, task
bodies and callbacks keep the full R3 rule.

## Correctness tools in the same place

The tools of [`correctness.md`](correctness.md) attach to subjects in the same way, so a reader sees a
function's whole specification and evidence in one place:

| Block / mechanism | What it is | See |
|---|---|---|
| `property "…" for f` | Property-based test; generators derived from types, automatic shrinking | [`correctness.md`: Property-based testing, with contracts as free oracles](correctness.md#property-based-testing-with-contracts-as-free-oracles) |
| **Contracts as implicit tests** | Every function with contracts gets an automatic property test: generate inputs satisfying `requires`, check `ensures` and invariants (Eiffel AutoTest). Listed in the docs. | [`correctness.md`: Property-based testing, with contracts as free oracles](correctness.md#property-based-testing-with-contracts-as-free-oracles) |
| `fuzz "…" for f` | Coverage-guided fuzz target; failing inputs are saved as attached regression tests | [`correctness.md`: Fuzzing, doc tests and snapshot tests](correctness.md#fuzzing-doc-tests-and-snapshot-tests) |
| Snapshot expectations | For comptime expansions and generated code | [`correctness.md`: Fuzzing, doc tests and snapshot tests](correctness.md#fuzzing-doc-tests-and-snapshot-tests) |
| `test.interleavings "…" for f` | Explores task interleavings under the simulation runtime | [`correctness.md`: Systematic concurrency testing](correctness.md#systematic-concurrency-testing) |
| Model-checking harnesses, `proof` blocks | Bounded model checking and deductive proofs; erased from builds | [`correctness.md`: Proofs in Moddil itself](correctness.md#proofs-in-moddil-itself), [`correctness.md`: Bounded model checking](correctness.md#bounded-model-checking) |
| **Mutation score per declaration** | How many deliberate bugs in `f` its tests catch: test *strength*, not just presence. Shown in docs and PR summaries. | [`correctness.md`: Mutation testing](correctness.md#mutation-testing) |
| Miri-style mode | A test-runner mode that runs tests on the interpreter with extra checks | [`correctness.md`: A Miri-style checker](correctness.md#a-miri-style-checker) |

## Test impact and caching

- Builds are content-addressed, and the compiler knows the call graph and every test's subject. So
  `moddil test` runs **only the tests whose inputs changed** ([`compiler.md`: Compiler architecture](compiler.md#compiler-architecture)).
- Results are **cached by hash**, locally or in a shared remote cache, and reused across machines and
  CI runs.
- Under the simulation runtime tests are deterministic, so a cached pass is trustworthy. Tests that
  use real effects are marked and re-run according to policy.
- For agents iterating dozens of times per task, this turns "run the tests" from minutes into
  seconds.

## Benchmarks

- `bench "…" for f { … }` attaches a benchmark to its subject (Criterion-style statistics), the
  same way `test "…" for f` attaches a test.
- `#[budget(allocations: 0, instructions: 2_000)]` on a `bench` block sets limits on deterministic
  counters, which fail CI ([`performance.md`: Benchmarks and budgets](performance.md#benchmarks-and-budgets)).
- Results are recorded per build hash, so the semantic diff can report **performance deltas** for
  changed functions ([Diffs, code review and version control](semantic-vcs.md)).
- **Noise handling (decided in [`performance.md`: Benchmarks and budgets](performance.md#benchmarks-and-budgets)):**
  budgets on deterministic counters gate CI; wall-clock results come with confidence intervals and
  only warn. Wall-clock benchmarks that gate merges run on dedicated or pinned machines, configured
  per project.

## Open questions

None at present.


## Prior art

Rust (`cargo test`, doc tests, nextest); Zig `test` blocks; Pyret `where:` clauses; Python and Elixir
doctests; Go `// Output:` examples; QuickCheck and Hypothesis; Criterion; iai; FoundationDB and
TigerBeetle deterministic simulation; Bazel test caching.

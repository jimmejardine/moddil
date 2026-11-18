# Correctness: Contracts, Testing and Verification

> **Status:** proposal. Indexed in [`README.md`](README.md). Nothing here is decided yet;
> recommendations are marked **Recommended**. Syntax is illustrative.

## Summary

Moddil should make correctness tooling **cheap by default and deep on demand**:
- **v1 (easy wins):** contracts as syntax, validated newtypes, built-in property testing and fuzzing,
  mutation testing, a Miri-style checker, systematic concurrency testing, and differential testing of
  the compiler's backends ([Easy wins (v1)](#easy-wins-v1)).
- **Designed in now, built later:** proofs written in Moddil itself (SMT-backed), refinement types,
  bounded model checking, protocol verification, and assurance levels per mod ([State of the art not to miss (design now, build later)](#state-of-the-art-not-to-miss-design-now-build-later)).
- A handful of **decisions must be locked early**, because they are expensive to retrofit ([Decisions to lock early](#decisions-to-lock-early)).

## Why Moddil is unusually well placed

Most languages bolt verification on afterwards and fight their own semantics. Several Moddil
decisions remove the usual obstacles:

| Existing decision | What it gives correctness tooling |
|---|---|
| **Effects** ([Effects and capabilities as a unifying idea](effects-and-capabilities.md)) | The compiler can *check* that contract expressions are pure; most languages can only trust them to be. Effect handlers swap real I/O, time and scheduling for test versions. |
| **Ownership; immutable rc contents; shared mutation only in sync cells** ([`memory.md`](memory.md)) | No unrestricted aliasing, so a verifier knows what a function can modify (the "frame problem"). This is why Verus, Prusti and Creusot work for Rust and why C/C++ verification is so hard. |
| **Linear resources** | Typestate: protocols such as "open → write → commit or rollback" checked by the type system. |
| **Two interpreters** ([`compiler.md`: Two interpreters](compiler.md#two-interpreters)) | The MIR interpreter is a Miri-style undefined-behaviour checker; the separate reference interpreter gives a reference semantics for testing the compiler and backends against. |
| **Deterministic runtime** (`moddil/runtime-sim`, [Determinism and reproducibility](determinism.md)) | Reproducible tests and systematic exploration of task interleavings. |
| **Comptime and static reflection** | Test-data generators derived from types (`#[derive(Arbitrary)]`); contracts on literals checked at compile time. |

## Easy wins (v1)

### Contracts as syntax

```rust
fn withdraw(account: &mut Account, amount: Money) -> Money
    requires amount > 0
    requires account.balance >= amount
    ensures  account.balance == old(account.balance) - amount
{ … }

struct OrderBook {
    bids: SortedVec<Level>,
    asks: SortedVec<Level>,
    invariant bids.is_empty() || asks.is_empty() || bids.best() < asks.best()
}
```

- `requires`, `ensures`, `invariant`, and `old(expr)` in postconditions (Eiffel, Ada 2012, SPARK,
  Dafny). **Decided:** contracts are clauses after the signature, not attributes
  ([`syntax.md`](syntax.md)).
- **Contract expressions must be pure**, checked by the effect system.
- Contracts are part of the **mod interface**: shown by the language server, included in generated
  docs, subject to semver, and visible in semantic diffs ([Diffs, code review and version control](semantic-vcs.md)). A trait impl may weaken
  preconditions and strengthen postconditions, never the reverse (Liskov).
- Runtime checking policy: see [Decisions to lock early](#decisions-to-lock-early).

### Validated newtypes (lightweight refinements)

```
type Port = u16 where self >= 1024
type Price = i64 where self > 0

let p = Port::new(raw)?;      // returns Result: invalid input is an error (class 1)
let q: Port = 8080;           // literal checked at compile time
```

- **Decided:** `where` is the only refinement form; ranges are written `where self in 1..`
  ([`syntax.md`](syntax.md)).
- "Parse, don't validate": once you hold a `Port`, the invariant holds, and it is never checked again.
- Construction from untrusted input returns a `Result`, so R3 forces callers to handle bad input.
- These are the natural boundary between *errors* (bad input) and *bugs* (a broken contract inside
  trusted code).

### Property-based testing, with contracts as free oracles

```rust
#[property_test]
fn withdraw_never_overdraws(mut account: Account, amount: Money) {
    try { withdraw(&mut account, amount) } catch { _ => {} }
}
```

- Built into the test runner (`moddil/prop`), with generators derived from types
  (`#[derive(Arbitrary)]`) and automatic **shrinking**, which reduces a failing input to the smallest
  case that still fails (QuickCheck, Hypothesis, proptest).
- **Contracts are oracles.** Any function with contracts can be tested automatically: generate inputs
  that satisfy `requires`, run, check `ensures` and invariants (Eiffel AutoTest). Tests come for free
  from specifications.

### Fuzzing, doc tests and snapshot tests

- **Coverage-guided fuzzing** in the standard test runner (Go `go test -fuzz`, cargo-fuzz): fuzz
  targets look like tests, and failing inputs are saved as regression tests.
- **Doc tests**: examples in documentation are compiled and run (Rust). In Moddil they carry
  expected results and run under the simulation runtime; all of this section's tools attach to the
  declaration they test ([`docs.md`](docs.md)).
- **Snapshot / golden tests** (`insta`), useful for generated code and comptime expansions.

### Mutation testing

The tool makes small deliberate changes (`<` to `<=`, deleting a statement, swapping a branch) and
checks that some test fails (cargo-mutants, PIT, Stryker). A surviving mutant points at an untested
behaviour.

This matters more than usual for Moddil: **LLM-written tests are often weak or tautological**
([LLMs as first-class programmers](agent-interface.md)). Mutation scores give reviewers an objective measure of whether a PR's tests
actually test anything, and can be shown in the PR summary.

### A Miri-style checker

Run tests on the MIR interpreter with extra checks: `unsafe` code obeying aliasing and validity rules,
FFI boundaries, use of uninitialised memory (Rust's Miri). This is nearly free, since the interpreter
exists for comptime. It must pass before a mod with the `unsafe` capability is published
([`unsafe.md`: Checking](unsafe.md#checking)).

### Systematic concurrency testing

See also [`determinism.md`: Deterministic simulation testing](determinism.md#deterministic-simulation-testing).

Because scheduling is an effect, `runtime-sim` can explore task interleavings: random, PCT (a
probabilistic strategy with good bug-finding guarantees), or exhaustive for small tests (Loom,
shuttle, Coyote, FoundationDB). Every failure replays from a seed ([Determinism and reproducibility](determinism.md)).

### Differential testing of the compiler

Run generated programs (Csmith, YARPGen style) through the interpreter, Cranelift, LLVM and WASM
backends; any disagreement is a compiler bug. The interpreter is the executable form of the
reference ([`definition.md`](definition.md)).

## State of the art not to miss (design now, build later)

### Proofs in Moddil itself

Deductive verification in the same language and toolchain, as Verus (Rust), Dafny, SPARK, Prusti and
Creusot do:
- **ghost code** (`ghost` variables, spec-only functions) and `proof { … }` blocks, **erased at
  compile time**, so proofs cost nothing at runtime;
- loop invariants, and `decreases` clauses for termination;
- an SMT solver (Z3 or cvc5) discharges proof obligations.

Verus's linear ghost permissions are a particularly good fit for Moddil's ownership model.
**Recommended:** reserve the syntax and erasure semantics now, even if the verifier ships much later.

### Assurance levels per mod

Following SPARK's graded levels (Stone, Bronze, Silver, Gold, Platinum), a mod declares how far it is
verified:

| Level | Meaning |
|---|---|
| **L0** | Contracts checked at runtime; tests and property tests pass |
| **L1** | **No traps**, proven: no division by zero, out-of-bounds index or `unreachable` reachable. This complements R3: errors are handled, *and* nothing crashes. |
| **L2** | Functional correctness proven against the contracts |

The level is part of mod metadata and the audit trail ([Package management and supply-chain security](packages-and-supply-chain.md)), so a dependency's assurance
is visible before you depend on it. The HFT risk module (example 2) would target L2.

### Refinement types with automatic proofs

Liquid Haskell, Flux (for Rust) and F\* attach logical predicates to types and prove them
automatically, without proof scripts. In Moddil this drives **bounds-check elision** (error-handling
D7) and index safety, and generalises [Validated newtypes (lightweight refinements)](#validated-newtypes-lightweight-refinements)'s `where` clauses from runtime checks to static proofs.

### Bounded model checking

Kani (Rust, via CBMC) proves a property for *all* inputs up to a bound. Its proof harnesses look like
tests, so engineers adopt it easily. A Moddil equivalent could run on MIR.

### Protocol and distributed verification

- **Choreographies and session types** ([Distributed and multi-tier programs](distribution.md)) are deadlock-free by construction.
- **TLA+** and **P** (used at AWS and Microsoft) model-check distributed protocols. Moddil
  choreographies could be exported to such a checker, and deterministic simulation ([Determinism and reproducibility](determinism.md)) gives a
  cheaper complement.

### Information flow

Type-level taint tracking and non-interference: secrets never reach the `client` placement, and untrusted
input reaches sensitive sinks only after validation ([Distributed and multi-tier programs](distribution.md)). This is both a security and a
correctness property.

### A formal semantics of the core

A mechanised model of Moddil-core (for example in Lean), as WebAssembly (WasmCert), CakeML and
CompCert have, used to validate the reference interpreter. This keeps "what Moddil means" precise,
which every other tool here depends on. How the model, the reference and the interpreter relate is
decided in [`definition.md`: Three artefacts, one meaning](definition.md#three-artefacts-one-meaning).

### LLMs plus verification

Agents can now write code *and* its proofs, with the verifier as the arbiter ("vericoding"). The
spec is what the human reviews; the proof certifies that the code meets it. To support this:
- verifier output must be **structured**: failing obligations, counterexamples, and suggested
  invariants, in a form agents can act on ([LLMs as first-class programmers](agent-interface.md));
- **spec inference**: tools or LLMs propose candidate contracts and invariants (Daikon-style), and
  humans approve them.

### Static analysers

Abstract-interpretation tools (Infer, Astrée, IKOS) can be added later as tooling. They don't
require language design decisions beyond a well-specified MIR.

## Decisions to lock early

These are cheap now and expensive to change once code exists.

1. **Contract syntax**: `requires`, `ensures`, `invariant`, `old`; contract expressions pure,
   enforced by effects.
2. **Ghost code and erasure**: what can appear only in specs and proofs, and the guarantee that it is
   erased.
3. **Contracts in interfaces**: part of the mod's public surface, under semver, in semantic diffs,
   with Liskov rules for trait impls.
4. **A runtime contract failure traps (class 3).** **Recommended.** A broken contract is a bug, not an
   error. Untrusted input goes through validated newtypes or explicit checks that return errors
   (class 1). This would close [`error-handling.md`](error-handling.md) open question 2.
5. **Release-build checking policy, per mod**: `all`, `preconditions` (the Ada and Eiffel default),
   or `none`. **Recommended default:** preconditions on public mod functions stay on in release
   builds; internal contracts are checked in debug builds only, unless the mod opts in.
6. **Assurance levels** as mod metadata ([Assurance levels per mod](#assurance-levels-per-mod)).

## Fit with the reference examples

| Example | What correctness tooling gives it |
|---|---|
| [Game](examples/01-multiplayer-game.md) | Differential tests that the shared simulation gives identical results on WASM and native; simulation tests with many clients |
| [HFT](examples/02-hft-platform.md) | Risk limits as contracts, with the risk module at L1 or L2; property tests on the order book; interleaving tests for lock-free queues |
| [Web store](examples/03-web-store.md) | Validated newtypes for money, IDs and quantities; contracts on checkout; property tests on pricing |
| [Hashiverse](examples/04-hashiverse.md) | Fuzzing the wire protocol against the Rust implementation; contracts and L1 on crypto and PoW code |

## Open questions

1. Is the verifier part of the core toolchain or a mod (`moddil/verify`)?
2. Which SMT solver(s), and how are solver versions pinned for reproducible proofs?
3. How much annotation burden is acceptable before L1/L2 stop being used?
4. Should a mod be able to require proof that no overflow trap can occur?
5. How are contracts on effectful operations expressed (for example, "this handler is called at most
   once")?

## Background

The problem and the state of the art that led to this design.

- **Problem.** Tests show the presence of bugs, not their absence; specifications live in comments or
  nowhere.
- **State of the art.** Ada/SPARK, Eiffel (design by contract), Dafny, Verus (Rust), Liquid Haskell,
  Lean 4, Prusti/Kani for Rust, property-based testing (QuickCheck, Hypothesis).
- **Directions.** Contracts are syntax, not library calls; checked at runtime in debug, optionally
  proven by an SMT-backed verifier for critical modules; property tests generated from contracts.
  This becomes much more valuable with LLMs, which can write proofs and code against a fixed spec.

## Prior art

- Bertrand Meyer, *Object-Oriented Software Construction* (Eiffel, design by contract); Eiffel AutoTest.
- Ada 2012 contracts; SPARK and its assurance levels (AdaCore, *Implementation Guidance for the
  Adoption of SPARK*).
- Dafny (Leino); Verus; Prusti; Creusot; Kani.
- Liquid Haskell; Flux; F\*.
- QuickCheck (Claessen & Hughes, 2000); Hypothesis; proptest.
- Go native fuzzing; libFuzzer; cargo-fuzz.
- cargo-mutants; PIT; Stryker.
- Rust Miri; Loom; shuttle; Coyote; PCT (Burckhardt et al., 2010).
- Csmith (Yang et al., PLDI 2011); YARPGen.
- TLA+ (Lamport); the P language.
- WasmCert; CakeML; CompCert.
- Daikon (Ernst et al.) for invariant inference.

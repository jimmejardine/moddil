# Security model

Ties together the security mechanisms specified in other areas and states the guarantee they add
up to.

## Summary

| Topic | Rule |
|---|---|
| **The guarantee** | **Capabilities are a real security boundary for safe code.** Code in a mod without the `unsafe` or `ffi` capability can use only the authority it is passed. Compiler soundness bugs are therefore security vulnerabilities ([The guarantee: capabilities as a boundary](#the-guarantee-capabilities-as-a-boundary)). |
| **Stronger isolation** | Crash containment, resource limits and untrusted `unsafe` or native code go in **WASM components** ([Isolation beyond capabilities](#isolation-beyond-capabilities)). |
| **Information flow** | A **small, fixed set of labels**: `Secret<T>`, `Personal<T>` and `Untrusted<T>`, checked by the compiler ([Information-flow labels](#information-flow-labels)). |
| **Constant time** | `#[constant_time]` functions are **checked by the compiler**, and the backends preserve constant-time behaviour ([Constant-time code](#constant-time-code)). |
| **Security fixes** | Advisories inform; taking a fix before its cooldown ends is a **manual, recorded override** ([Supply chain](#supply-chain)). |

## Problem

Moddil's security mechanisms were each designed in their own area: capabilities and root authority,
capability ceilings on dependencies, the randomised cooldown, the `unsafe` boundary, trust boundaries
between distributed components, and secrets kept out of logs. Each is useful on its own, but without a
threat model nobody can say which attackers Moddil defends against, what it promises as a single
claim, or where the gaps between the pieces are.

## Threat model

| Attacker | Main defences | Defined in | Not covered |
|---|---|---|---|
| **A malicious or compromised dependency** | Capability ceilings per dependency; no ambient authority; no code at fetch or build time; randomised cooldown; audit trail | [`packages-and-supply-chain.md`](packages-and-supply-chain.md), [`effects-and-capabilities.md`](effects-and-capabilities.md) | A dependency granted `unsafe` or `ffi` is trusted with everything |
| **Hostile input** (network, files, IPC) | Memory safety; `Untrusted<T>`; parse-don't-validate decoding; overflow and bounds traps; typed errors (R3) | [`data.md`](data.md), [`numerics.md`](numerics.md), [`error-handling.md`](error-handling.md) | Algorithmic denial of service beyond keyed hashing, bounded by opt-in budgets ([Resource budgets](#resource-budgets)) |
| **A hostile client** (browser or app on a machine the attacker controls) | `Untrusted<T>` from `client`; secrets cannot be placed on `client`; server-side validation | [`distribution.md`: Trust, secrets and capabilities](distribution.md#trust-secrets-and-capabilities) | Anything that runs on the client itself |
| **Untrusted plugins or tenant code** | WASM components with explicit imports and resource limits | [Isolation beyond capabilities](#isolation-beyond-capabilities), [`interop.md`](interop.md) | |
| **A compromised build or toolchain** | Reproducible builds; hash-pinned sources; pure, sandboxed comptime; double-compiling check of the compiler | [`determinism.md`: Reproducible builds](determinism.md#reproducible-builds), [`comptime.md`](comptime.md), [`compiler.md`](compiler.md) | A compromised machine that produces both build and check |
| **Leaks through observability** | `Secret<T>` and `Personal<T>` cannot reach logs, traces or metrics | [`observability.md`: Secrets never reach observability](observability.md#secrets-never-reach-observability) | Leaks through ordinary program output the programmer wrote deliberately |
| **Timing side channels** | `#[constant_time]` code for secrets | [Constant-time code](#constant-time-code) | Cache and power channels outside constant-time code |

**Out of scope, stated explicitly:** physical access and hardware attacks; Spectre-class speculative
leaks beyond what constant-time code and opt-in mitigations provide; and malicious code that has been
granted `unsafe` or `ffi`, which can do anything its process can.

## The guarantee: capabilities as a boundary

Moddil's capabilities are a security boundary for safe code, not just an audit aid.

> Code in a mod that does not hold the `unsafe` or `ffi` capability can perform only the effects it
> declares, using only the capability values and handlers it is passed. It cannot read or write
> memory it was not given a reference to, and it cannot obtain authority from anywhere else.

The claim holds given two conditions:

1. **`unsafe` and `ffi` code elsewhere in the program is correct.** Such code is contained and may
   sit behind safe interfaces ([`unsafe.md`](unsafe.md)), but it is trusted: it is visible in manifests,
   requires human review by default, and is checked before publishing.
2. **The compiler is sound.** A soundness bug that lets safe code exceed its authority is a **security
   vulnerability**, handled with an advisory like any other ([Security response](#security-response)).

What makes the claim possible:

- **No ambient authority.** `main` receives the root capabilities; nothing else can create them
  ([`effects-and-capabilities.md`: Authority comes only from root capabilities](effects-and-capabilities.md#authority-comes-only-from-root-capabilities)).
- **No global mutable state** that could smuggle authority between unrelated code.
- **No reflection that bypasses visibility.** Reflection is static and respects privacy
  ([`comptime.md`](comptime.md)).
- **Comptime is pure and sandboxed**: no build scripts, no I/O at compile time.
- **Effects are checked**, including across mod boundaries, where they are written out.
- **A closed list of undefined behaviour**, reachable only inside `unsafe`
  ([`unsafe.md`: The closed list of undefined behaviour](unsafe.md#the-closed-list-of-undefined-behaviour)).

*Why it holds:* authority is a value, its absence is a compile error, and reflection cannot reach
private state, so there is no ambient authority, unrestricted reflection or run-time permission
check to get wrong.

**In practice:**

```rust
// A third-party template engine, declared with no capabilities in its manifest.
// It can only use what this call hands it: here, a read-only view of one directory.
with Fs = fs.read_only(theme_dir) {
    let html = templates::render(&page, &ctx)?;
}
```

- **Capability diffs in review:** an upgrade that adds `Net` to a logging library shows up in the
  semantic diff and needs explicit acceptance ([`semantic-vcs.md`](semantic-vcs.md),
  [`packages-and-supply-chain.md`: Capability-scoped dependencies](packages-and-supply-chain.md#capability-scoped-dependencies)).
- **Narrow before delegating:** handlers are narrowed (`fs.read_only`, `net.only("api.example.com")`)
  before untrusted code runs ([`effects-and-capabilities.md`](effects-and-capabilities.md)).

## Isolation beyond capabilities

Capabilities control authority, not resources or crashes. Use a **WASM component** when you need:

- **untrusted native or `unsafe` code** (a C library you have not audited, a plugin from a user);
- **resource limits**: fuel (instruction budgets) and memory caps for tenant code;
- **crash containment**: a trap inside the component does not take down the host;
- **a stable plugin interface** described in WIT ([`interop.md`: WASM components: the sandboxed path](interop.md#wasm-components-the-sandboxed-path)).

The cost is a copy at the boundary for non-scalar data and a calling-convention overhead; inside the
component, code runs at full speed.

## Information-flow labels

A small, fixed set of labels, checked by the compiler.

| Label | Meaning | Rules |
|---|---|---|
| **`Secret<T>`** | Credentials, keys, tokens | Cannot reach logs, traces, metrics, `Debug`, `Display` or serialisation; cannot be placed on `client`; zeroed on drop; `==` is constant time ([Secret handling](#secret-handling)) |
| **`Personal<T, C = General>`** | Personal data: names, emails, addresses, identifiers of people; the category `C` marks regulated kinds (`Health`, `Financial`, `ExportControlled`, or a user-declared category) | Cannot reach logs, traces or metrics; may be stored or sent only to placements and stores declared for its category in the topology ([`distribution.md`](distribution.md)) |
| **`Untrusted<T>`** | Data from `client`, peers or other untrusted sources | Must be validated (into a validated newtype, or by explicit checks) before use ([`correctness.md`: Validated newtypes (lightweight refinements)](correctness.md#validated-newtypes-lightweight-refinements)) |

- **Labels nest.** A struct with a `Secret` field cannot be logged as a whole; a derived `Debug`
  prints the field as redacted ([`values.md`](values.md)).
- **Categories, not more labels.** Domains such as health or export-controlled data use a
  category on `Personal`, e.g. `Personal<Diagnosis, Health>`, and the topology declares which stores
  and placements accept each category. The compiler rules stay the same; there is no policy
  language.
- **Declassification is explicit and reviewed.** `secret.redacted()` and
  `personal.hash_for_correlation()` are projections; `x.declassify("reason")` removes a label and
  takes a mandatory string literal, like `unsafe("…")`. Every declassification is listed by
  `moddil audit` and reported by the semantic diff ([`semantic-vcs.md`](semantic-vcs.md)), and a new
  or changed one needs a human review under the default `vet` policy, which a project can relax.
- **The checks are type-based**, not a separate analysis: the observability effects and the
  serialisation traits are simply not implemented for labelled types.

```rust
struct Customer { id: CustomerId, email: Personal<Email>, card_token: Secret<Token> }

log.info(fmt"checkout for {customer.id}");          // ✓
log.info(fmt"checkout for {customer.email}");       // ✗ compile error: Personal<Email> in a log event
log.info(fmt"checkout for {customer:?}");           // ✓ prints email and card_token as redacted
```

## Secret handling

- **Zeroed on drop, guaranteed.** The compiler emits volatile zeroing that the optimiser may not
  remove, including on error and cancellation paths ([`memory.md`](memory.md)).
- **No accidental copies.** `Secret<T>` is never `Copy`; cloning is explicit
  ([`values.md`](values.md)).
- **Constant-time equality.** `==` on secrets compares in constant time.
- **Where secrets come from:** secret-provider handlers (Vault, Kubernetes secrets, cloud KMS), never
  manifests or source ([`distribution.md`](distribution.md)).
- **Secrets in recordings:** secret- and personal-typed effect results are stored encrypted, and
  replaying them needs the `ReplayKey` capability
  ([`observability.md`: Flight recorder and replay](observability.md#flight-recorder-and-replay)).

## Constant-time code

Constant-time code is checked by the compiler, and the backends preserve it.

```rust
#[constant_time]
fn verify_mac(expected: &Secret<[u8; 32]>, actual: &[u8; 32]) -> bool {
    let mut diff: u8 = 0;
    for i in 0..32 { diff |= expected.expose()[i] ^ actual[i]; }   // fixed bounds: OK
    diff == 0
}
```

- **Inside a `#[constant_time]` function**, the compiler rejects branches, indexing, division and
  remainder, early returns and loop bounds that depend on `Secret` values.
- **The backends preserve it**: secret-dependent selections lower to conditional moves or masked
  arithmetic, and optimisation passes that could introduce branches or table lookups on those values
  are disabled for them, in Cranelift and in LLVM.
- **LLVM without a fork:** Moddil uses LLVM's upstream constant-time intrinsics and
  carries no patches. CI runs binary analysis on the emitted code of every `#[constant_time]`
  function, looking for secret-dependent branches and memory addresses. A function LLVM cannot
  preserve on some target is compiled with Cranelift there instead.
- **Speculation:** `#[constant_time]` functions also get speculative load hardening. Other
  functions opt in with `#[speculation_hardened]`; there is no global flag. Process and component
  isolation stay the main defence across trust boundaries.
- **Testing:** `moddil/test` includes statistical timing tests (dudect style) for
  `#[constant_time]` functions.
- `moddil/crypto` is written this way, so applications rarely write constant-time code themselves.

## Tests are held to the same rules

Test code counts against its mod's capability ceiling, like any other code:

- A test gets **no root capabilities** unless it declares them (`test "reads real config" with FsRoot`),
  so the default test runs against simulation handlers only.
- **Test-only dependencies** have their own ceilings, which never leak to users of the mod.
- `moddil test` reports which tests used real capabilities
  ([`testing.md`](testing.md)).

Comptime is already pure and sandboxed ([`comptime.md`](comptime.md)).

## Resource budgets

Soft budgets per scope; hard limits per component.

- **Opt-in per-scope budgets:** an allocator handler with a byte limit, where an allocation past it
  fails with a typed error, and deadlines that cancel the scope
  ([`lifecycle.md`: Cancellation and shutdown](lifecycle.md#cancellation-and-shutdown)).
- **Hard, enforced limits** (fuel, memory caps, crash containment) come from WASM components
  ([Isolation beyond capabilities](#isolation-beyond-capabilities)).
- Nothing is charged per task by default.

## Supply chain

Specified in [`packages-and-supply-chain.md`](packages-and-supply-chain.md) and summarised here:

- **Capability ceilings** per dependency; capability increases need explicit acceptance.
- **Hash-pinned sources** in plain git; the source is the release.
- **No code at fetch or build time**: no build scripts, no install hooks, sandboxed comptime.
- **Randomised cooldown** of 7–28 days before a new version can be adopted.
- **An audit trail in git**, recording whether a human or an agent reviewed each version.

**Security fixes: advisories inform; a person or agent decides.**

- `moddil audit` checks the dependency graph against advisory feeds the project chooses (such as
  OSV) and flags affected versions in every build and in CI.
- Taking a fixed version before its cooldown ends is a **manual override**, per mod and version,
  recorded in the manifest so that it shows up in review
  ([`packages-and-supply-chain.md`: Randomised cooldown](packages-and-supply-chain.md#randomised-cooldown)).
  Advisories never bypass the cooldown automatically. *Why:* an attacker who can publish a fake
  advisory must not be able to push a release straight into every project.

## Memory safety and `unsafe`

- Safe Moddil is memory-safe and free of data races ([`memory.md`](memory.md),
  [`concurrency.md`](concurrency.md)).
- `unsafe` is contained, carries safety contracts, is checked before publishing and needs human review
  by default ([`unsafe.md`](unsafe.md)).
- Errors cannot be left unhandled ([R3](requirements.md#firm-requirements)); traps end a task
  or process with a full trace, never with undefined behaviour ([`error-handling.md`](error-handling.md)).

## Security response

- **Reporting:** a private channel for vulnerabilities in the compiler, the standard mods and the
  toolchain, with a published disclosure policy and embargo period.
- **Scope:** compiler soundness bugs that let safe code exceed its authority, bugs in the label checks
  or constant-time checking, and vulnerabilities in standard mods.
- **Advisories:** published in OSV format, so `moddil audit` and existing tools can read them.
- **Ownership** of the process sits with governance ([`governance.md`](governance.md)).

## Fit with the reference examples

| Example | Main threats | Moddil's defences |
|---|---|---|
| [Web store](examples/web-store.md) | Payment credentials, customer data, hostile clients | `Secret` payment keys; `Personal` customer data kept out of logs; `Untrusted` cart contents validated on the server |
| [Hashiverse](examples/hashiverse.md) | Hostile peers, key theft, timing attacks | `Untrusted` peer data; keys as `Secret` behind a key-locker handler; constant-time crypto |
| [HFT platform](examples/hft-platform.md) | Credential theft, insiders, compromised dependencies | Capability ceilings on market-data and FIX mods; secrets from providers; audit trail |
| [Multiplayer game](examples/multiplayer-game.md) | Cheating clients, malformed packets | Authoritative server; `Untrusted` inputs; parse-don't-validate decoding |

## Alternatives considered

- **Run-time permission checks, as in Java's SecurityManager.** Not taken: it failed because of
  ambient authority (any code could call `new FileInputStream`), unrestricted reflection and checks
  that libraries had to remember to call at run time.
- **More labels, or a label lattice and policy language (as in Jif and FlowCaml).** Not taken: such
  labels are noisy and rarely adopted; categories on `Personal` cover regulated domains with the
  same compiler rules.
- **A patched or forked LLVM for constant-time code.** Not taken: Moddil uses the upstream
  intrinsics, checks the emitted code in CI, and falls back to Cranelift on a target where LLVM
  cannot preserve a function.
- **A global speculation-hardening flag.** Not taken: hardening is per function, and process and
  component isolation are the main defence across trust boundaries.
- **Letting advisories bypass the cooldown automatically.** Not taken: a fake advisory would then
  push a release straight into every project.

## Prior art

| System | What it does | Lesson |
|---|---|---|
| **E, Joe-E, Austral, Pony** | Object-capability languages: no ambient authority; references are permissions | Capabilities can be a real boundary inside one process when the language removes ambient authority |
| **Java SecurityManager** | Runtime permission checks inside the JVM; deprecated for removal | Failed because of ambient authority, reflection and a huge trusted library surface; checks at run time are easy to forget |
| **Deno** | Process-wide permissions (`--allow-net`) | Coarse, but proves that users accept declared authority |
| **Rust** | Memory safety; RustSec advisories and `cargo audit`; the `subtle` crate for constant time | Safety without authority control; constant time is best effort because the optimiser may break it |
| **WASM components** | Memory-isolated modules with explicit imports | Strong isolation at a calling-convention cost |
| **Jif, FlowCaml** | Information-flow type systems with label lattices | Expressive, but rarely adopted because labels are noisy |
| **Spectre and its variants** | Speculative execution leaks across boundaries | No language can fully close microarchitectural channels in software |

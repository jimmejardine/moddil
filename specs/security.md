# Security model

> **Status:** not started. Nothing is decided yet; this page records the problem and the
> questions to settle. Listed in the [index](README.md).

## Problem

Moddil's security pieces are each designed in their own area: capabilities and root authority,
capability-scoped dependencies and the supply chain, the `unsafe` boundary, trust boundaries and
secrets between distributed components, and WASM components as a sandbox. What is missing is the
whole picture: a threat model that says which attackers Moddil defends against, which guarantees it
makes as a single claim, and the gaps between the pieces (secrets in logs, timing side channels,
personal data flowing where it should not).

## Touched on in

[`effects-and-capabilities.md`: Authority comes only from root capabilities](effects-and-capabilities.md#authority-comes-only-from-root-capabilities),
[`packages-and-supply-chain.md`: Capability-scoped dependencies](packages-and-supply-chain.md#capability-scoped-dependencies), [`unsafe.md`](unsafe.md),
[`distribution.md`](distribution.md) (trust boundaries, secrets), [`interop.md`: WASM components: the sandboxed path](interop.md#wasm-components-the-sandboxed-path),
[`observability.md`](observability.md) and [`correctness.md`](correctness.md) (personal data).

## Questions to settle

- The **threat model**: malicious or compromised dependencies, hostile network input, untrusted
  plugins, insiders, and which of these each mechanism addresses.
- **Memory safety and authority as one stated guarantee**: what safe Moddil code can never do,
  whatever its dependencies do.
- **Secret types**: values that cannot be logged or formatted, are zeroed on drop, and compare in
  constant time.
- **Information flow**: tracking personal or sensitive data (taint) through a program, and whether
  that belongs in types, effects or a lint.
- **Constant-time code** for cryptography: can the compiler promise not to introduce branches or
  table lookups on secret data?
- **Sandboxing untrusted code**: WASM components, capability narrowing, resource limits.
- **Security response**: advisories, CVEs, how fixes reach users through pinned versions and the
  randomised cooldown, and who owns the process ([`governance.md`](governance.md)).

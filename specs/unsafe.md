# `unsafe` and the Safety Boundary

> **Status:** complete: decided, with no open questions. Listed in the [index](README.md). Snippets are
> **illustration only**.

## Summary

| Rule | One line |
|---|---|
| **Contained, not contagious** | A safe function may use `unsafe` inside and still be safe to call; visibility comes from the mod's `unsafe` capability, audits and diffs. |
| **Safety contracts** | An `unsafe fn` states its preconditions as `requires` clauses; every `unsafe` block carries a justification. |
| **A closed list of UB** | Eight kinds of undefined behaviour, all inside `unsafe`; everything else is defined or traps. |
| **Scoped-borrow aliasing** | A borrow made from a raw pointer is valid for its lexical scope; raw pointers alias freely. |
| **Strict provenance** | Pointers carry provenance; integer↔pointer conversion is explicit and `unsafe`. |
| **Checked before publishing** | Debug builds check cheap conditions; the interpreter-based checker must pass before a mod with `unsafe` is published. |
| **Part of the security model** | Safe code's authority is bounded by its capabilities only if `unsafe` code is correct; that is why `unsafe` is trusted, visible and reviewed ([`security.md`](security.md)). |
| **Human review by default** | New or changed `unsafe` and `ffi` code needs a human reviewer under the default `vet` policy. |

## Problem

- `unsafe` appears throughout the specs as a capability and an escape hatch, but its meaning is
  undefined.
- **C and C++** have hundreds of kinds of undefined behaviour, most of them reachable from ordinary
  code.
- **Rust** limits UB to `unsafe`, but its aliasing model (Stacked Borrows, now Tree Borrows) is hard
  for humans, and harder for LLMs, to reason about. Its safety obligations are free-text
  `// SAFETY:` comments that no tool can check.
- **Unsafe code in dependencies** is rarely reviewed: a single mistake deep in the graph undermines
  every guarantee above it.

## State of the art

| System | What it does well | Limits |
|---|---|---|
| **Rust** | `unsafe` blocks, functions, traits and impls; since the 2024 edition an `unsafe fn` body still needs blocks; strict provenance (stable since 1.84); Miri finds UB in tests | Tree Borrows is subtle; `// SAFETY:` comments are unchecked; Miri is slow and opt-in |
| **Zig** | "Illegal behaviour" checked at runtime in Debug and ReleaseSafe; `noalias` is explicit | No `unsafe` keyword: unsafe operations aren't marked |
| **C** | `restrict`; type-based aliasing | Hundreds of kinds of UB; unsafe everywhere |
| **Verus, Kani, Prusti** | Safety conditions as contracts, proved or model-checked | Annotation effort |
| **Sanitizers** (ASan, TSan, UBSan) | Find memory errors and races at runtime | Partial coverage; overhead |
| **CHERI** | Provenance enforced in hardware | Special hardware |
| **cargo-geiger, cargo-vet** | Count and review `unsafe` in dependencies | Opt-in, outside the compiler |

## Decisions

### What needs `unsafe`

| Operation | Why |
|---|---|
| Dereferencing a raw pointer | It may be invalid, dangling or misaligned |
| Calling an `unsafe fn`, including raw FFI bindings ([`interop.md`](interop.md)) | Its preconditions must be met by the caller |
| Implementing an `unsafe trait` | The implementation must uphold the trait's invariant |
| Converting between integers and pointers | Provenance must be preserved (see [Strict provenance](#strict-provenance)) |
| Forgetting a linear value | It discharges an obligation without fulfilling it ([`memory.md`](memory.md)) |
| Unchecked operations (`get_unchecked`, `assume_init`) | They skip a check the caller must have made |
| GPU scatter writes ([`accelerators.md`](accelerators.md)) | Writes may race |
| Inline assembly (also needs the `asm` capability, [`freestanding.md`](freestanding.md)) | The compiler can't see what it does |

### Contained, not contagious

- **`unsafe` does not propagate to callers.** A safe function that uses `unsafe` inside, such as
  `Vec::push`, is safe to call. That is what makes safe abstractions possible.
- Visibility comes instead from:
  - the mod's **`unsafe` capability ceiling**
    ([`packages-and-supply-chain.md`: Capability-scoped dependencies](packages-and-supply-chain.md#capability-scoped-dependencies));
  - audits and the semantic diff (see [Review and audit](#review-and-audit)).
- Inside an `unsafe fn`, each unsafe operation still needs its own `unsafe` block (the Rust 2024
  rule), so the dangerous lines stay marked.

### Safety contracts

An `unsafe fn` states what its caller must guarantee as **`requires` clauses**
([`correctness.md`: Contracts as syntax](correctness.md#contracts-as-syntax)), and every `unsafe`
block says why those guarantees hold:

```rust
pub unsafe fn copy_nonoverlapping<T>(src: *const T, dst: *mut T, n: int)
    requires valid: src.is_valid_for_read(n) && dst.is_valid_for_write(n)
    requires disjoint: !overlaps(src, dst, n)
{ … }

fn append(buf: &mut Vec<u8>, data: &[u8]) {
    buf.reserve(data.len());
    unsafe("valid: reserve guarantees capacity; disjoint: data is a separate borrow") {
        copy_nonoverlapping(data.as_ptr(), buf.end_ptr(), data.len());
        buf.set_len(buf.len() + data.len())
    }
}
```

- The preconditions are **checked at runtime** in debug builds and checker runs. In release builds
  they follow the contract-checking policy; preconditions written **`requires always`** are checked
  in every build (decided, [`correctness.md`: Checking policy: the `Contracts` handler](correctness.md#checking-policy-the-contracts-handler)).
  Null and alignment checks are debug-only.
- **Citations (decided):** a `requires` clause may be named (`valid:` above), and a justification
  may cite names (`unsafe("valid: …")`). Tools link each claim to the obligation it discharges, a
  lint flags named preconditions that no block cites, and the verifier knows what each block claims.
- They can be **proved** later by the verifier
  ([`correctness.md`: Proofs in Moddil itself](correctness.md#proofs-in-moddil-itself)).
- The justification is required: an `unsafe` block without one is a compile error.

### The closed list of undefined behaviour

Undefined behaviour exists **only inside `unsafe`**, and only these kinds:

| # | Undefined behaviour |
|---|---|
| 1 | Dereferencing an invalid, dangling or misaligned pointer |
| 2 | A data race through raw pointers |
| 3 | Violating the [scoped-borrow aliasing rules](#aliasing-the-scoped-borrow-model) |
| 4 | Producing an invalid value: a `bool` other than 0 or 1, an invalid enum tag, null where a non-null type is required |
| 5 | Reading uninitialised memory |
| 6 | Calling foreign code with the wrong signature or ABI |
| 7 | Breaking the invariant of an `unsafe trait` |
| 8 | Forgetting a linear value |

Everything else is **defined or traps**: integer overflow, division, bounds and float behaviour are
specified in [`error-handling.md`](error-handling.md) and [`numerics.md`](numerics.md).

### Aliasing: the scoped-borrow model

Moddil's borrows are **second-class**: they cannot be stored, so every borrow has a lexical scope
([`memory.md`: Borrowing](memory.md#borrowing)). That makes the aliasing
rules much simpler than Rust's:

- **Raw pointers alias freely.** Any number of raw pointers may point at the same memory, and the
  optimiser assumes nothing about them.
- **A borrow made from a raw pointer is valid for exactly its lexical scope.**
  - While a `&mut` is in scope, **no other access** to that memory may happen, through any pointer.
  - While a `&` is in scope, **no write** to that memory may happen.
- The optimiser may assume `noalias` for borrows during their scope, as it does for safe code.
- Views and owning views follow the borrow they were made from
  ([`memory.md`: View types](memory.md#view-types)). **A view that outlives its `unsafe` block
  (decided)** keeps the borrow alive until the view's own scope ends: no conflicting access to that
  memory may happen meanwhile, through any pointer. Views are second-class, so that scope is lexical,
  and the checker verifies it. This is what makes zero-copy `FromBytes` views over raw buffers safe to
  hand to safe code.

```rust
unsafe("p is valid and uniquely owned here") {
    let r: &mut Node = &mut *p;     // r's scope starts
    r.count += 1
    // (*q).count += 1              // UB if q points at the same Node while r is in scope
}                                   // r's scope ends: raw access through p or q is fine again
```

### Strict provenance

- Every pointer carries **provenance**: which allocation it may access.
- **`ptr.addr()`** (pointer to integer) and **`ptr.with_addr(a)`** (integer back to a pointer, keeping
  `ptr`'s provenance) are the only conversions, and both need `unsafe`.
- Safe code never observes an address
  ([`determinism.md`: Sources of nondeterminism](determinism.md#sources-of-nondeterminism)).

### Unsafe traits

An `unsafe trait` has an invariant the compiler can't check, so implementing it needs `unsafe impl`.
Example: **`FromBytes`**, "every bit pattern of the right size is a valid value", which zero-copy
views rely on ([`memory.md`: Zero-copy I/O](memory.md#zero-copy-io)).
Derives can implement it safely when they can check the invariant from the type's fields.

### Checking

| When | What is checked |
|---|---|
| **Debug builds** | Null, alignment, bounds where the size is known, and `requires` preconditions of `unsafe fn` |
| **Release builds** | `requires always` preconditions, and others as the contract policy chooses |
| **The checker** (a test-runner mode on the MIR interpreter, [`correctness.md`: A Miri-style checker](correctness.md#a-miri-style-checker)) | Aliasing, provenance, uninitialised reads, invalid values, FFI signatures, and data races under the simulation scheduler |
| **GPU kernels** | The CPU-simulator handler with its race detector ([`accelerators.md`](accelerators.md)) |

- **The checker must pass before `moddil publish`** tags a release of a mod with the `unsafe`
  capability. Audits show the release as checked.
- Locally it is `moddil test --check-unsafe`; CI runs it on changed unsafe code.
- **Incremental and cached (decided).** Only tests that reach `unsafe` code run under the checker
  (test-impact analysis, [`testing.md`: Test impact and caching](testing.md#test-impact-and-caching)),
  and results are cached per function hash, so a publish re-checks only unsafe paths changed since
  the last checked release. The published budget: a typical mod's checker run takes under ten minutes
  in CI; a mod that is slower splits its unsafe core into a mod of its own.

### Review and audit

- **`moddil audit`** lists every `unsafe` block in a mod and its dependencies, with its
  justification.
- The **semantic diff** flags new or changed unsafe code ([`semantic-vcs.md`](semantic-vcs.md)).
- **The default `vet` policy requires a human review** for new or changed `unsafe` or `ffi` code,
  both in your own mods and in dependency deltas
  ([`packages-and-supply-chain.md`: Audit trail in git](packages-and-supply-chain.md#audit-trail-in-git)).
  Agent reviews are recorded but don't satisfy it. A project can relax the policy explicitly.

### Where `unsafe` lives

The goal is that **almost no application mod needs `unsafe`**. Collections, sync cells, allocators,
runtimes and FFI wrappers live in audited standard mods
([`stdlib.md`: Initial standard-mod catalogue](stdlib.md#initial-standard-mod-catalogue)), so
applications get their performance without holding the capability.

**Built so far (M2b, November 2026):** raw pointers, `unsafe("why") { … }` with its justification
required, the `unsafe` capability in the manifest, and the `core` mod's `ptr`, `cell`
(`UnsafeCell`) and `atomic` modules ([`reference/2027/unsafe.md`](../reference/2027/unsafe.md)).
`requires` contracts on `unsafe fn`, the checker and the audit rules come later.

## Fit check against the reference examples

| Example | Uses |
|---|---|
| [Multiplayer game](examples/multiplayer-game.md) | GPU scatter writes checked on the CPU simulator; no `unsafe` in game logic |
| [HFT](examples/hft-platform.md) | Kernel-bypass networking and `FromBytes` wire views in a small, human-reviewed, checker-verified mod |
| [Web store](examples/web-store.md) | No `unsafe` at all; dependencies that hold it are flagged in audits |
| [Hashiverse](examples/hashiverse.md) | Crypto through FFI wrappers with safety contracts; human review of every unsafe delta |

## Open questions

None at present.


## Prior art

Rust's `unsafe`, the 2024 `unsafe_op_in_unsafe_fn` rule, Stacked and Tree Borrows, strict provenance
and Miri; Zig's illegal behaviour and `noalias`; C `restrict`; Verus, Kani and Prusti; ASan, TSan and
UBSan; CHERI; cargo-geiger and cargo-vet.

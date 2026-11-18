# Why Moddil

> **Status:** the reasoning behind the whole design: what Moddil builds on, why it should exist, the
> themes that recur across areas, and where its goals pull against each other. Each area's detail is
> in its own document; the [index](README.md) lists them all.

## Preamble

Every serious language exists because it solved something better than what came before it:

- **C** made a portable abstraction over the machine that became the world's ABI.
- **C++** showed that zero-cost abstractions and RAII can scale to huge systems.
- **Rust** proved that memory safety and data-race freedom can be checked at compile time without a GC.
- **Go** showed that simplicity, fast builds, and one blessed toolchain are features in their own right.
- **Erlang/Elixir** made failure a normal, designed-for event, using isolated processes and supervisors.
- **Haskell/OCaml/Koka** demonstrated what types and effects can say about a program.
- **Zig** treated cross-compilation, explicit allocation, and C interop as first-class concerns.
- **Swift** shipped real-world automatic reference counting and serious C++ interop.
- **Unison** asked what happens if code is identified by its content rather than by its text.

**Syntax is deliberately not where Moddil innovates:** its surface syntax is Rust's, with a short
list of justified deltas ([`syntax.md`](syntax.md)). Snippets in every spec follow it.

Moddil should not be a remix of syntax. It is only justified if the *combination* of problems below
cannot be solved well by evolving an existing language, either because the solutions need guarantees
that existing languages cannot add without breaking their ecosystems, or because the target user has
changed.

**What this document is not:** a grammar, a feature list, or a promise. Where a snippet appears it only
illustrates a point.

## Thesis

Three shifts are happening together, and no existing language was designed for all of them:

1. **The author is changing.** More and more code is written by LLM agents. Humans type less, but they
   must *understand, review and approve* more. The bottleneck moves from writing code to verifying it.
   Languages built to save keystrokes (implicit conversions, clever macros, overloads, magic)
   now cost the human who has to review the result.

2. **The machine is changing.** "A computer" now means many CPU cores, one or more GPUs, and often other
   accelerators (NPUs, TPUs, DSPs), with non-uniform memory between them. Most languages still treat
   anything beyond a single CPU thread as a library concern.

3. **The program is a fleet.** A deployed system is a group of cooperating processes: browser
   client, API servers, message brokers, background workers, databases. Languages describe one
   process at a time; the system as a whole is held together by hand-maintained contracts,
   YAML and hope.

All three shifts point the same way: **semantics that are explicit and checked by machines.** That is
Moddil's reason to exist. A language where effects, resources, capabilities, concurrency and memory placement are visible in the types and
checked by the compiler is easier for an LLM to write correctly, easier for a human to review, easier to
schedule onto heterogeneous hardware, easier to split safely across machines, and safer to pull dependencies into.

## Cross-cutting themes

When the problem areas are laid side by side, four ideas keep recurring:

1. **Explicitness that machines enforce.** Effects, capabilities, resources, memory spaces and errors
   are *in the types*, checked by the compiler, and mostly inferred inside a module. Explicit for the
   reader; cheap for the writer (especially when the writer is a model).

2. **One canonical representation.** A single formatting and a canonical AST give smaller diffs,
   cleaner training data, semantic review, faster caching and simpler tools.

3. **Safe by default, with explicit escape hatches.** Unsafe, FFI, manual memory and wrapping
   arithmetic are always possible, but always *visible*: in signatures, in package metadata and in PR
   summaries. The same goes for **cost**: the default is zero-cost (owned values and borrows), and
   anything more expensive is spelled out where it happens (`.to_rc()`, a sync cell, a `remote`
   call).

4. **Placement is one concept at every scale.** Thread → core → GPU → machine → tier: work is placed
   and coordinated by the same structured-concurrency model, and *where data lives* is part of its
   type, whether that is GPU memory ([Concurrency across GPUs and accelerators](accelerators.md)) or a client browser ([Distributed and multi-tier programs](distribution.md)). Algorithms are separate from
   schedules, and logical programs are separate from physical topologies. Crossing a boundary is
   always explicit.

An optional fifth: **the compiler is a service for agents and humans.** Its structured output is as
important as the binaries it produces.

## Tensions and trade-offs

Being honest about where goals collide:

| Tension                                            | Notes                                                                                                                                                                                            |
| -------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Explicitness vs brevity                            | Effects in every signature could be noisy. Mitigate with inference inside modules and explicit effects only at boundaries.                                                                       |
| Compile-time safety vs build speed                 | Borrow checking, effect inference and verification all cost time. Keep the expensive checks incremental and per-module. Second-class borrows help: the borrow check is local to each function. |
| No lifetimes vs expressiveness                     | Second-class borrows and view types remove lifetime annotations, but lose structs with several independent lifetimes and borrows stored in long-lived data. Owning views, rc and indices fill the gap. |
| Visible sharing vs single-threaded convenience     | rc contents are immutable and shared mutation needs sync cells, so a mutable graph used by one thread still pays (cheap) atomic costs. Arenas, indices and library collections are the idiomatic answer. |
| Monomorphisation vs build time vs runtime speed    | Dual-mode generics ([Build times](build-times.md)) is promising but unproven at scale.                                                                                                                                    |
| Strict capabilities vs ergonomics                  | Passing capabilities everywhere is tedious. Need good defaults (e.g., `main` receives all; libraries receive nothing unless asked).                                                              |
| Canonical syntax vs expressiveness                 | One way to do things limits DSLs and clever code, which is usually good, and sometimes painful.                                                                                                  |
| Easy interop vs safety guarantees                  | Every FFI call weakens guarantees. Making it visible is the compromise.                                                                                                                          |
| Novelty vs adoptability                            | Effect systems and linear types are unfamiliar. Each novel feature must pay for itself.                                                                                                          |
| GPU portability vs peak performance                | Portable code is rarely fastest; schedules per target are the escape valve.                                                                                                                      |
| Location transparency vs explicitness              | One language across tiers is attractive, but hiding the network repeats the RPC mistakes of the 1990s. Moddil chooses explicit boundaries.                                                       |
| Language vs infrastructure scope                   | Every step toward describing infrastructure risks turning Moddil into a cloud platform.                                                                                                          |
| Whole-program view vs independent deploys          | Whole-system checking is powerful, but teams must still ship components separately.                                                                                                              |
| WASM as a required target vs native-first features | Threads and GPU access are weaker on WASM, and stack switching does not ship in browsers yet, so browsers need a JSPI-based lowering in the meantime. Every core feature needs a WASM implementation strategy, even if it is slower. |
| Overflow checks vs speed | Integer overflow traps in every build, which costs typically 1–5% before checks are elided by range analysis. Wrapping is always available but explicit (`+%`, `Wrapping<T>`). |
| Small core vs compiler-checked guarantees | R2 wants a tiny core, but anything the compiler must check (ownership, effects, capabilities, placement, contracts) has to be in it. The core is "small" only relative to its guarantees. |
| "For LLMs" vs "for humans"                         | Must not produce a language only machines can comfortably write. Humans still need to read, debug and occasionally write it.                                                                     |

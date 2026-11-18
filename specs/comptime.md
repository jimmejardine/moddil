# Compile-time Evaluation (instead of Macros)

> **Status:** decided direction, details open. Expands [`overview.md`: Metaprogramming](overview.md#metaprogramming).
> Snippets are **illustration only**; no syntax is committed.

## Summary

| Rule | One line |
|---|---|
| **No macro language** | There are no token-level macros (`macro_rules!`, proc macros, a preprocessor). Metaprogramming is ordinary Moddil code that the compiler runs. |
| **Comptime is pure** | Code run at compile time has an empty effect set: no `net`, `fs`, `time` or `rand`. It is deterministic, fuel-limited, sandboxed and cached. |
| **Generics stay bounded** | Generic code is checked against trait bounds where it is defined. Comptime handles values and code generation, not untyped generics. |
| **Static reflection** | `reflect(T)` describes a type's fields, variants, annotations and generic parameters, read-only. |
| **Typed generation at the annotation site** | `#[derive(serde::Serialize)]` calls a comptime function that returns a *typed* trait impl. It is never a token stream and never appears anywhere the annotation isn't. |
| **Tagged string literals for DSLs** | `sql"…"`, `re"…"` and `fmt"…"` are parsed and checked at compile time and produce typed code. There is no new syntax. |
| **Everything is visible** | Every expansion can be printed as canonical source and appears in semantic diffs. |

## Background

**Problem.** Macros are powerful but break tooling, readability, caching and LLM understanding
(C preprocessor, C++ templates, Rust proc macros). Proc macros also run arbitrary code at build time.

### How Zig does it

Zig has no macro language. It has one idea: ordinary Zig code can be evaluated by the compiler.

- **Comptime parameters and blocks.** A `comptime` argument must be known at compile time, and the
  compiler's interpreter evaluates the code that depends on it.
- **Types are values at compile time.** A generic is a function from `type` to `type`:
  ```zig
  fn ArrayList(comptime T: type) type {
      return struct { items: []T, len: usize };
  }
  ```
- **Static reflection.**
  - `@typeInfo(T)` returns a plain struct describing fields, variants and declarations.
  - `inline for` unrolls a loop over them.
  - `@field(x, name)` accesses a field by a computed name.
  - `@Type(info)` builds a new type from such a description.
  - `@compileError` emits a custom diagnostic.
- **Uses:**
  - `std.fmt` parses format strings at compile time, so a bad format string is a compile error;
  - JSON serialisation walks `@typeInfo`;
  - lookup tables and specialised code are computed by the compiler.
- **Constraints:**
  - the only compile-time I/O is `@import` and `@embedFile`;
  - evaluation is deterministic and limited by a branch quota;
  - comptime cannot invent syntax or inject names into the caller's scope, so it is hygienic by
    construction.

**Gaps that matter for Moddil:**
- **Duck-typed generics.** `anytype` and comptime `type` parameters have no declared bounds. As with
  C++ templates, errors appear at instantiation, deep inside library code, and a signature does not
  say what it requires. That conflicts with local reasoning ([overview: LLMs as first-class programmers](overview.md#llms-as-first-class-programmers)) and with debug-mode
  dictionary passing ([overview: Build times](overview.md#build-times)).
- **Lazy analysis.** Code that is never instantiated is never checked.
- **No generated declarations.** `@Type` cannot create methods or attach an implementation to an
  existing type, so there is no `derive`. Consumers reflect at the call site instead.

### How Rust does it

| | `macro_rules!` | Proc macros (derive, attribute, function-like) |
|---|---|---|
| Mechanism | Pattern matching on token trees | Arbitrary Rust compiled as a compiler plugin, transforming token streams |
| Power | Moderate | Unlimited: `serde` derive, `async-trait`, `tokio::select!`, `sqlx::query!` |
| Costs | An opaque mini-language; partial hygiene | Opaque to IDEs, reviewers and LLMs; slow builds (`syn`, poor caching); **arbitrary code execution at build time**; every macro is new syntax |

`sqlx::query!` is the extreme case: it connects to a live database during compilation.

### Others

C++ `constexpr` and C++26 static reflection; Swift's sandboxed, typed macros; Lisp/Racket
macros.

## Evaluation model

- Comptime code is **ordinary Moddil**: the same syntax, types, ownership rules and standard mods.
- It must have an **empty effect set**, which the effect system checks ([overview: Effects and capabilities as a unifying idea](overview.md#effects-and-capabilities-as-a-unifying-idea)).
  - Allocation within the evaluator is allowed.
  - `fail` becomes a compile error.
  - Every other effect is rejected.
- **Deterministic**: the same inputs always give the same output.
- **Fuel-limited**: a default step budget, raisable per call site with a visible annotation.
- **Errors**: `compile_error(msg, span)` reports a diagnostic at a source location chosen by the
  comptime code, such as a position inside a literal. Diagnostics are structured, for agents ([overview: LLMs as first-class programmers](overview.md#llms-as-first-class-programmers)).
- **Sandboxed**: the evaluator has no ambient authority (R1 makes WASM the natural sandbox; see [Open questions](#open-questions)).
- **Cached** by a hash of the function, its arguments and the reflected inputs it read. This makes it
  fit the query-based, content-addressed compiler of [overview: Build times](overview.md#build-times).
- **External inputs** (schemas, IDL files, protocol definitions) come only through `embed("path")`.
  - Embedded files are declared in the mod manifest.
  - They are hashed into the cache key.
  - They are visible to audits ([overview: Package management and supply-chain security](overview.md#package-management-and-supply-chain-security)).

## Comptime parameters and values

```rust
fn ring<const N: usize, T>() -> RingBuffer<T, N>          // size known at compile time
const CRC_TABLE: [u32; 256] = const { crc_table(0xEDB88320) };
fn pow<const N: u32>(x: f64) -> f64 { … }                  // specialised per N
```

Compile-time evaluation uses Rust's spelling ([`syntax.md`](syntax.md)): `const` generic
parameters, `const fn`, and `const { … }` blocks. "Comptime" remains the name of the concept.

Comptime values cover sizes, constants, lookup tables, format strings and specialisation. A comptime
value can be anything with a defined compile-time representation. Borrows of compile-time data
become static data in the binary.

## Generics are bounded

```rust
fn max<T: Ord>(a: &T, b: &T) -> &T                         // the bound is part of the signature
```

- Generic functions and types declare trait bounds and are **checked once, at their definition**,
  not at each instantiation.
- Comptime does not replace parametric generics. `type` values exist in comptime code for
  reflection and generation, but any generic signature visible to callers states its bounds.
- This preserves:
  - local reasoning: a signature tells you what is required;
  - debug builds with dictionary passing and release builds with specialisation, with identical
    semantics ([overview: Build times](overview.md#build-times));
  - errors that point at the call site, not into library internals.

## Static reflection

`reflect(T)` returns a read-only description of `T`:
- **Structs**: fields (name, type, annotations, visibility), in declaration order.
- **Sum types**: variants and their payloads.
- **Generics**: generic parameters and their bounds.
- **Attributes** attached to the type and its fields, e.g. `#[serde(rename = "id")]`.
- **Functions**: signature, including parameter modes and effects.

Visibility is respected: comptime code in one mod cannot read the private fields of another mod's
type unless that type opts in (e.g. by deriving in its own mod).

## Generation: typed impls at the annotation site

```rust
#[derive(serde::Serialize, serde::Deserialize, Eq, Hash)]
struct Order {
    id: OrderId,
    #[serde(rename = "px")] price: Price,
    qty: Qty,
}
```

- `#[derive(X)]` calls a compile-time function published by a mod, e.g.
  `serde::Serialize: const fn(type) -> Impl<Serialize>`.
- The function builds its result through a **typed construction API** (impls, functions, match
  arms, expressions), not token streams. Output is guaranteed well-formed, and it is type-checked
  against the trait like hand-written code.
- **Generation happens only where the annotation is.** A derive can add the impl it promises and
  nothing else: no new names in the caller's scope, no edits elsewhere.
- Comptime functions can also return **types** (Zig-style, e.g. a struct-of-arrays layout computed
  from a struct), constants and specialised functions.
- **Visible expansion:**
  - `moddil explain Order` prints the generated impls as canonical source ([overview: LLMs as first-class programmers](overview.md#llms-as-first-class-programmers)).
  - Expansions are part of the semantic diff ([overview: Diffs, code review and version control](overview.md#diffs-code-review-and-version-control)), so a derive version bump that changes generated
    code shows up in review.

## Tagged string literals (embedded DSLs)

A literal with a prefix, `prefix"…"`, calls the comptime function `prefix` in scope. The function
receives the literal text plus any interpolated expressions as **typed holes**, and returns typed
values or code.

```
let line  = fmt"{order.id}: {order.qty} @ {order.price}"     // checked against the argument types
let ident = re"[a-z_][a-z0-9_]*"                               // regex compiled to a DFA at comptime

let open_orders = sql"""
    SELECT id, price, qty FROM orders WHERE user_id = {user} AND status = 'open'
"""                                    // checked against embed("schema.sql"); returns rows of a generated type
```

- Errors point *into* the literal: the column of a bad regex token, an unknown SQL column.
- `sql` checks against a **declared, embedded schema file**, never a live database. The migration
  tooling keeps that file current.
- Interpolations are typed holes, never string concatenation, so `sql` and `html` are
  injection-safe by construction.
- This replaces most function-like macros (`format!`, `regex!`, `sqlx::query!`, `html!`) without
  adding syntax.

## Interaction with ownership, effects and capabilities

Generated code is ordinary code once expanded:
- it is checked for borrows, linearity, effects and capabilities like anything hand-written;
- a derive **cannot grant capabilities**: if it generates code that performs `net`, the mod using it
  must declare `net`;
- generated effects and capabilities appear in signatures and PR summaries like any others.

## Security and supply chain

- No code runs at build time with ambient authority. Comptime is pure and sandboxed, unlike
  `build.rs`, proc macros or `postinstall` ([overview: Package management and supply-chain security](overview.md#package-management-and-supply-chain-security)).
- External inputs are declared (`embed`), hashed and auditable.
- A malicious derive can still generate bad *code*. That code is visible in `explain` and in diffs,
  and is limited to the capabilities the consuming mod declares.

## What we give up

| Rust feature | Moddil answer |
|---|---|
| New syntax forms (`tokio::select!`, `html!`, `json!`) | Language features where fundamental (select is part of the structured-concurrency API); otherwise tagged literals or plain functions |
| `vec![a, b, c]` and other variadic sugar | Array/collection literals; comptime variadic parameters |
| Attribute macros that rewrite a function body (`#[tokio::main]`, `#[async_trait]`) | Not needed: effects remove function colouring and runtime selection happens in `main` (overview R2) |
| Generating items at arbitrary places, or names chosen by the macro | Not possible: generation only at the annotation site, through the typed API |
| Token-level tricks | Not possible, by design |

## Fit check against the reference examples

| Example | Uses |
|---|---|
| [Multiplayer game](examples/01-multiplayer-game.md) | `#[derive]` for network snapshot codecs and delta encoding; comptime struct-of-arrays layouts for ECS components |
| [HFT](examples/02-hft-platform.md) | Exchange codecs (SBE/FIX) generated from embedded schemas; comptime lookup tables; zero-copy accessor views (see [`memory-and-resources.md`: Zero-copy I/O](memory-and-resources.md#zero-copy-io)) |
| [Web store](examples/03-web-store.md) | `sql"…"` checked against the schema; `#[derive]` for API contracts and validation; `html"…"` templates for the web client |
| [Hashiverse](examples/04-hashiverse.md) | Wire-protocol codecs that must match existing Rust servers; comptime crypto constant tables; `fmt` for logs |

None of them needed token-level macros.

## Open questions

- **Where does comptime run?** Options:
  - an interpreter inside the compiler (Zig; fast start, slow for heavy work);
  - compiling comptime code to WASM and running it in Wasmtime (fast for heavy work; type values
    must cross the boundary);
  - a hybrid of the two.
- **Allocation:** what allocator does comptime use, and how does comptime data become static data in
  the binary?
- **Budget:** what is the default fuel and the per-mod compile-time budget ([overview: Build times](overview.md#build-times))?
- **Ordering:** can one derive depend on another's output (e.g. `Hash` relying on `Eq`), and how are
  cycles detected?
- **Reflection and privacy:** exactly which private details may a derive in another mod see?
- **IDE responsiveness:** comptime must be incremental enough to run on every keystroke for agents
  ([overview: LLMs as first-class programmers](overview.md#llms-as-first-class-programmers)).
- **Construction API:** how expressive is it, and does it stay stable across editions ([overview: Language evolution](overview.md#language-evolution))?

## Prior art

Zig `comptime`, `@typeInfo`, `@Type`; Rust `macro_rules!`, proc macros, `syn`, `serde`, `sqlx`; C++
`constexpr`/`consteval` and C++26 static reflection (P2996); D CTFE and `static foreach`; Jai `#run`
and `#insert`; Nim macros; Terra; Lisp and Racket hygienic macros; Swift macros (sandboxed, typed
expansion); Mojo parameters; Scala 3 `inline` and quotes; JavaScript tagged templates.

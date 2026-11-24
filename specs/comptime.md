# Compile-time Evaluation (instead of Macros)

> **Status:** complete: decided, with no open questions. Indexed in [`README.md`](README.md).
> Snippets are **illustration only**; no syntax is committed.

## Summary

| Rule                                        | One line                                                                                                                                                               |
| ------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **No macro language**                       | There are no token-level macros (`macro_rules!`, proc macros, a preprocessor). Metaprogramming is ordinary Moddil code that the compiler runs.                         |
| **Comptime is pure**                        | Code run at compile time has an empty effect set: no `Net`, `Fs`, `Time` or `Rand`. It is deterministic, fuel-limited, sandboxed and cached.                           |
| **Generics stay bounded**                   | Generic code is checked against trait bounds where it is defined. Comptime handles values and code generation, not untyped generics.                                   |
| **Static reflection**                       | `reflect(T)` describes a type's fields, variants, annotations and generic parameters, read-only.                                                                       |
| **Typed generation at the annotation site** | `#[derive(serde::Serialize)]` calls a comptime function that returns a *typed* trait impl. It is never a token stream and never appears anywhere the annotation isn't. |
| **Typed function wrappers**                 | `#[around(trace::instrument)]` wraps a function in an ordinary generic function that sees its signature, never its body ([Function wrappers](#function-wrappers)).     |
| **Tagged string literals for DSLs**         | `sql"…"`, `re"…"` and `fmt"…"` are parsed and checked at compile time and produce typed code. There is no new syntax.                                                  |
| **Everything is visible**                   | Every expansion can be printed as canonical source and appears in semantic diffs.                                                                                      |

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
  say what it requires. That conflicts with local reasoning ([LLMs as first-class programmers](agent-interface.md)) and with debug-mode
  dictionary passing ([Build times](build-times.md)).
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
- It must have an **empty effect set**, which the effect system checks ([Effects and capabilities as a unifying idea](effects-and-capabilities.md)).
  - Allocation within the evaluator is allowed: each evaluation has its own arena, freed
    afterwards. A value that ends up in a `const` is deep-copied into read-only static data (a
    `Vec` becomes a static slice, an `Rc` is frozen); a type that cannot be made static (raw
    pointers, a `Mutex`) is a compile error there.
  - `fail` becomes a compile error.
  - Every other effect is rejected.
- **Deterministic**: the same inputs always give the same output.
- **Fuel-limited**: 10 million interpreter steps per evaluation by default, raisable at the call
  site with a visible `#[fuel(…)]`. Steps are counted identically on every machine. Each mod's
  comptime time appears in `moddil build --timings`, with a warning above 2 s; a hard limit applies
  only if a project sets one in CI.
- **Errors**: `compile_error(msg, span)` reports a diagnostic at a source location chosen by the
  comptime code, such as a position inside a literal. Diagnostics are structured, for agents ([LLMs as first-class programmers](agent-interface.md)).
- **Sandboxed**: the evaluator has no ambient authority. It is the compiler's MIR interpreter
  ([`compiler.md`: Two interpreters](compiler.md#two-interpreters)). Later, a second tier may
  compile hot comptime functions (large derives, parser generators) to WASM and run them in
  Wasmtime, with identical results checked in CI.
- **Cached** by a hash of the function, its arguments and the reflected inputs it read. This makes it
  fit the query-based, content-addressed compiler of [Build times](build-times.md): editing a body
  does not rerun derives on unrelated types.
- **Never blocks the editor:** an evaluation that takes longer than 50 ms runs in the background and
  its diagnostics stream in; the keystroke path never waits for it.
- **External inputs** (schemas, IDL files, protocol definitions) come only through `embed("path")`.
  - Embedded files are declared in the mod manifest.
  - They are hashed into the cache key.
  - They are visible to audits ([Package management and supply-chain security](packages-and-supply-chain.md)).

## Comptime parameters and values

```rust
fn ring<const N: int, T>() -> RingBuffer<T, N>          // size known at compile time
const CRC_TABLE: [u32; 256] = const { crc_table(0xEDB88320) };
fn pow<const N: u32>(x: f64) -> f64 { … }                  // specialised per N
```

Compile-time evaluation uses Rust's spelling ([`syntax.md`](syntax.md)): `const` generic
parameters, `const fn`, `const { … }` blocks and `if const`. "Comptime" remains the name of the
concept. The one exception is unrolling over reflected fields, which uses Zig's spelling,
`inline for f in fields(t) { … }` (decided November 2026, M2 slice M2a), since Rust has no
equivalent and `const for` would read as a loop run at compile time.

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
    semantics ([Build times](build-times.md));
  - errors that point at the call site, not into library internals.

## Static reflection

`reflect(T)` returns a read-only description of `T`:
- **Structs**: fields (name, type, annotations, visibility), in declaration order.
- **Sum types**: variants and their payloads.
- **Generics**: generic parameters and their bounds.
- **Attributes** attached to the type and its fields, e.g. `#[serde(rename = "id")]`.
- **Functions**: signature, including parameter modes and effects.

Visibility is respected: comptime code in one mod cannot read the private fields of another mod's
type unless that type opts in. **Writing `#[derive(X)]` is the opt-in:** it grants X's comptime code
read access to that type's private fields, attributes and variants, for that derive only. Other
comptime code sees only public items, and the grant shows in semantic diffs.

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
  against the trait like hand-written code. The API is a full typed syntax tree, able to build any
  item a person could write. It is versioned per edition: older derives keep working through
  adapters, and migrations update them ([`evolution.md`](evolution.md)).
- **Built first: impl templates (decided November 2026, M2 slice M2a).** Before the full
  construction API, a derive function's body is a template: `const fn f(t: type) -> Impl<Tr> {
  impl Tr for t { … } }`.
  - A trait names its derive with `#[derive_by(f)]`.
  - Instantiation: names resolve where the template is written, `t` and `Self` are the struct,
    and the result is checked like hand-written code.
  - In the template, `inline for f in fields(t) { … }` unrolls over the struct's fields, and
    `f.name`, `x.[f]`, `[f]: e`, `f.is_refined()` and `f.new(e)` reflect each one.
  - `if const` chooses by reflection, and only the branch chosen is compiled.
  - The construction API comes later and builds on this, for enums, generic types and impls
    that templates cannot express ([`reference/2027/comptime.md`](../reference/2027/comptime.md)).
- **Templates for built-in types: `#[derive_for]` (decided 24 November 2026).** A built-in type
  such as `i8` has no declaration to put `#[derive]` on. So a derive function can list the types
  it is for: `#[derive_for(i8, i16, …)] const fn int_add(t: type) -> Impl<Add> { … }`. The impl is
  written once, and the types it is made for are written in the same place. Generation stays
  visible at one annotation, typed and checked like any template. It replaced about 1,100 lines of
  per-type impls in the prelude (`prelude/numbers.moddil`). It is prelude-only for now: user
  types keep `#[derive]` at their declaration.
- **Templates for generic structs (decided 24 November 2026).** A template's derive for a generic
  struct bounds each type parameter by the trait, as the built-in derives do (`impl<T: Tr> Tr for
  S<T>`), in place of the I0001 it was.
- **Templates for fieldless enums (decided 24 November 2026).** A template reflects on an enum
  by its variants: `inline for v in variants(t)`, `v.name`, `v.is(x)` and `v.make()`, and
  `if const t.is_enum()` lets one template serve structs and enums. Every variant must be a unit
  variant for now. That covers C-like enums (serde, display and parse by name, variant lists)
  with no new syntax.
- **Variants with fields (decided 24 November 2026).** `inline for f in fields(v)` reflects a
  variant's fields. `x.[f]` reads one through a borrow (a match on the variant, trapping on
  another), and `v { [f]: e, … }` builds the variant, of any shape. `v.index`, `t.name`, and
  `is_unit()`/`is_positional()` of `t` and `v` in `if const` give what `Debug`, `Hash` and the
  orderings need. Templates cannot bind fields in patterns, so a variant is chosen by `v.is(x)`
  rather than by a match: an if chain, one test per variant.
- **The built-in derives are prelude templates (decided 24 November 2026).** `Clone`,
  `PartialEq`, `PartialOrd`, `Ord`, `Hash`, `Debug` and `Default` are derived by templates in
  `prelude/derives.moddil`, named by `#[derive_by]` on each trait. That replaced the generators in
  `crates/sema/src/derive.rs`. `Copy`, `Eq` and a `Copy` type's `*self` clone stay built in. The
  reference interpreter keeps its structural derives, so the conformance suite compares the two.
  Every field must still implement the trait, checked at the `#[derive]`. On a loop of derived
  operations on an enum and a struct, the templates' if chains ran about 7% slower than the
  generated matches.
- **Tuples in `#[derive_for]` (decided 24 November 2026).** `#[derive_for(tuples(2..=12))]`
  makes a template's impl for each size of tuple, with every element type bounded. A tuple's
  elements reflect as fields named `0`, `1`, …, so the prelude's `Clone`, `Debug` and `Hash` for
  tuples are one template each, in place of about 380 lines of impls written per size.
- **Derives can depend on each other.** Derives are compiler queries: a derive may ask whether
  `T: Eq` holds, which runs that derive first, and a cycle is a compile error that shows it. A derive
  sees which traits are implemented, never another derive's generated body.
- **Generation happens only where the annotation is.** A derive can add the impl it promises and
  nothing else: no new names in the caller's scope, no edits elsewhere. A wrapper likewise changes
  only the function it annotates ([Function wrappers](#function-wrappers)).
- Comptime functions can also return **types** (Zig-style, e.g. a struct-of-arrays layout computed
  from a struct), constants and specialised functions.
- **Visible expansion:**
  - `moddil explain Order` prints the generated impls as canonical source ([LLMs as first-class programmers](agent-interface.md)).
  - Expansions are part of the semantic diff ([Diffs, code review and version control](semantic-vcs.md)), so a derive version bump that changes generated
    code shows up in review.

## Function wrappers

**Decided: `#[around(w)]` wraps a function in an ordinary generic function `w`.** This covers
what Rust's attribute macros are best at (entry and exit logging, permission checks, timing,
retries, memoisation) without rewriting tokens.

```rust
#[around(trace::instrument)]
fn transfer(from: AccountId, to: AccountId, amount: Money) -> Receipt with Db, fail(TransferError) {
    …
}
```

- **The expansion is fixed by the language.** The body becomes a call to the wrapper, which is
  given the function's description, its arguments as one value, and the original body as a closure
  that takes the arguments back:

  ```rust
  fn transfer(from: AccountId, to: AccountId, amount: Money) -> Receipt with Db, fail(TransferError) {
      trace::instrument(&INFO, transfer::Args { from, to, amount },
                        |transfer::Args { from, to, amount }| { … })
  }
  ```

  - `INFO: FnInfo` is the function's [static reflection](#static-reflection), as static data: its
    path, its parameters' names and types, its attributes and its effects.
  - `transfer::Args` is generated with one field per parameter, named like it. It is a **view
    type** whenever a parameter is a borrow (`&T`, `&mut T`), because borrows cannot be stored in
    owned data such as a tuple ([`memory.md`: View types](memory.md#view-types)). A view can be
    passed down the call stack and to a non-escaping closure, which is all a wrapper does, so
    borrowed parameters work.
  - The arguments pass through the wrapper and back, so moves and borrows are checked as in the
    original body.
- **A wrapper is ordinary Moddil:** a generic, effect-polymorphic function, checked once where it
  is defined ([Generics are bounded](#generics-are-bounded)):

  ```rust
  pub fn instrument<A: Debug, R: Debug, E>(f: &FnInfo, args: A,
                                           inner: impl FnOnce(A) -> R with E) -> R with E {
      span(f.path, args: &args) {
          log::debug("enter");
          let r = inner(args);
          log::debug("exit", result: &r);
          r
      }
  }
  ```

  `A: Debug` holds when every parameter is `Debug`; a parameter that is not is an error at the
  `#[around]`, not inside the library.
- **Arguments and order:** `#[around(auth::require(Role::Admin))]` passes comptime arguments.
  Several wrappers apply in written order, the first outermost.
- **What a wrapper can do:** run code before and after `inner`; call `inner` never (refusing),
  once, or several times (retrying, which needs `A: Clone`); read context data; and use the ambient
  `log`, `span` and `metric` ([`observability.md`](observability.md)).
- **What it cannot do:** see or change the body, add names to any scope, change the signature, or
  add effects and errors the function does not have. A wrapper that fails with `Forbidden` needs
  `fail(Forbidden)` in the function's error set: inferred inside a mod, written on a `pub`
  signature, where the compiler suggests it
  ([`effects-and-capabilities.md`: Signatures and inference](effects-and-capabilities.md#signatures-and-inference)).
- **Visible:** `moddil explain transfer` prints the expansion; the wrapper appears in the function's
  docs; and a wrapper's version bump that changes behaviour shows in semantic diffs
  ([`semantic-vcs.md`](semantic-vcs.md)).
- **Cost:** the wrapper and its closure inline like any generic code; in a debug build a wrapper
  is one extra call.

## Rust's macros, the Moddil way

Rust's macros are at their best on cross-cutting concerns that would otherwise repeat in every
function. Each pair below shows the Rust idiom, then Moddil's. Sometimes the answer is a wrapper;
often a type or an effect does the job better.

### Logging entry and exit

```rust
// Rust (tracing)
#[tracing::instrument(skip(password), err)]
fn login(user: &str, password: &str) -> Result<Session, AuthError> { … }
```

```rust
// Moddil
#[around(trace::instrument)]
fn login(user: &str, password: Secret<&str>) -> Session with fail(AuthError) { … }
```

- **No `skip` list to forget:** `Secret<T>` prints as redacted, so a secret never reaches a log
  ([`observability.md`: Secrets never reach observability](observability.md#secrets-never-reach-observability)).
- **Errors are already traced:** an error that leaves the function carries an error return trace
  ([`error-handling.md`: Traces](error-handling.md#traces)), so the wrapper need not log it.
- **Every task and scope is already a span**
  ([`observability.md`: Spans and context propagation](observability.md#spans-and-context-propagation)),
  so `#[around(trace::instrument)]` is for logical operations worth naming, not for every function.

### Security preambles

```rust
// Rust (actix-web-grants): a runtime check inserted at the top of the body
#[has_permissions("ADMIN")]
async fn delete_user(id: web::Path<UserId>) -> HttpResponse { … }
```

**Preferred in Moddil: make the check a type.** The function takes context data that only the
check can produce, so every caller is proven to have checked, and no preamble can be forgotten or
reordered:

```rust
pub struct Admin { by: UserId }                    // no public constructor

pub fn admin(user: &User) -> Admin with fail(Forbidden) {
    if !user.roles.contains(Role::Admin) { fail(Forbidden) }
    Admin { by: user.id }
}

fn delete_user(id: UserId) with admin: &Admin, Db { … }

// at the edge, once per request
with Admin = auth::admin(&user)? { delete_user(id) }
```

([`effects-and-capabilities.md`: Context data](effects-and-capabilities.md#context-data)). **When
policy must run at entry**, such as writing an audit record for every call, a wrapper does it:

```rust
#[around(auth::require(Role::Admin))]
pub fn delete_user(id: UserId) with user: &User, Db, fail(Forbidden) { … }
```

The wrapper reads `user`, writes the audit log, and either calls `inner` or fails with
`Forbidden`, which is visible in the signature.

### Timing, retries and memoisation

| Rust | Moddil | Notes |
|---|---|---|
| `#[timed]` (metrics crates) | `#[around(metric::timed)]` | Records a duration histogram; `metric` is ambient, so the signature does not change |
| `#[retry(3)]` (retry crates) | `#[around(retry::times(3, backoff: exp(10ms)))]` | Re-calls `inner`, so the arguments must be `Clone`; sleeping uses `Time`, which the function must already have. At a single call site, the plain function `retry::times(3, \|\| fetch(url))` does the same |
| `#[cached]` (cached) | `#[around(cache::memo)]` | Rust's version hides a global map; Moddil's store is context data, `with memo: &Memo<K, V>`, installed by the caller and replaced in tests |

### Other common macros

| Rust | Moddil |
|---|---|
| `#[derive(…)]` | `#[derive(…)]`, through the typed construction API ([Generation](#generation-typed-impls-at-the-annotation-site)) |
| `format!`, `println!`, `write!` | `fmt"…"`, checked against the argument types ([Tagged string literals](#tagged-string-literals-embedded-dsls)) |
| `vec![a, b, c]`, `vec![x; n]` | `[a, b, c]` and `[x; n]`, typed by context: `let v: Vec<int> = [1, 2, 3];` ([`stdlib.md`: Collections](stdlib.md#collections)) |
| `include_str!`, `include_bytes!` | `embed("path")`, declared in the manifest ([Evaluation model](#evaluation-model)) |
| `sqlx::query!` | `sql"…"`, checked against an embedded schema, never a live database |
| `assert!`, `assert_eq!`, `assert_ne!` | The power `assert(cond)`, which shows every part's value ([`testing.md`: Assertions](testing.md#assertions)) |
| `#[cfg(…)]` | `if const`, whose excluded branch is still checked ([`modules.md`](modules.md#conditional-code-is-comptime-if)) |
| `#[test]` | `test "…" { }` items ([`testing.md`: Attached tests](testing.md#attached-tests)) |
| `#[tokio::main]`, `#[async_trait]` | Not needed: effects remove function colouring, and `main` chooses the runtime |
| `lazy_static!`, `once_cell` for tables | `const { … }`, evaluated at compile time ([Comptime parameters and values](#comptime-parameters-and-values)) |
| `bitflags!` | `EnumSet<E>` over a fieldless enum ([`stdlib.md`: Collections](stdlib.md#collections)) |
| `thiserror` | Error sets with payloads, and `#[derive(Display)]` ([`error-handling.md`](error-handling.md)) |
| `matches!` | `if let` and `let … else` |

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
- `sql` checks against a **declared, embedded schema file**, never a live database. The file is
  derived from the migrations ([`data.md`: Databases: migrations-first](data.md#databases-migrations-first)).
- Interpolations are typed holes, never string concatenation, so `sql` and `html` are
  injection-safe by construction.
- This replaces most function-like macros (`format!`, `regex!`, `sqlx::query!`, `html!`) without
  adding syntax.

## Interaction with ownership, effects and capabilities

Generated code is ordinary code once expanded:
- it is checked for borrows, linearity, effects and capabilities like anything hand-written;
- a derive **cannot grant capabilities**: if it generates code that performs `Net`, the mod using it
  must declare `Net`;
- generated effects and capabilities appear in signatures and PR summaries like any others.

## Security and supply chain

- No code runs at build time with ambient authority. Comptime is pure and sandboxed, unlike
  `build.rs`, proc macros or `postinstall` ([Package management and supply-chain security](packages-and-supply-chain.md)).
- External inputs are declared (`embed`), hashed and auditable.
- A malicious derive can still generate bad *code*. That code is visible in `explain` and in diffs,
  and is limited to the capabilities the consuming mod declares.

## What we give up

| Rust feature                                                                       | Moddil answer                                                                                                                           |
| ---------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------- |
| New syntax forms (`tokio::select!`, `html!`, `json!`)                              | Language features where fundamental (select is part of the structured-concurrency API); otherwise tagged literals or plain functions    |
| `vec![a, b, c]` and other variadic sugar                                           | Array literals typed by context (`let v: Vec<int> = [1, 2, 3];`); comptime variadic parameters |
| Attribute macros that rewrite a function body (`#[tokio::main]`, `#[async_trait]`) | Rewriting is not possible. `#[tokio::main]` and `#[async_trait]` are not needed (effects remove function colouring, and runtime selection happens in `main`, [R2](requirements.md#firm-requirements-decided)); logging, permission checks, timing, retries and memoisation use [typed wrappers](#function-wrappers) |
| Generating items at arbitrary places, or names chosen by the macro                 | Not possible: generation only at the annotation site, through the typed API                                                             |
| Token-level tricks                                                                 | Not possible, by design                                                                                                                 |

## Fit check against the reference examples

| Example | Uses |
|---|---|
| [Multiplayer game](examples/multiplayer-game.md) | `#[derive]` for network snapshot codecs and delta encoding; comptime struct-of-arrays layouts for ECS components |
| [HFT](examples/hft-platform.md) | Exchange codecs (SBE/FIX) generated from embedded schemas; comptime lookup tables; zero-copy accessor views (see [`memory.md`: Zero-copy I/O](memory.md#zero-copy-io)) |
| [Web store](examples/web-store.md) | `sql"…"` checked against the schema; `#[derive]` for API contracts and validation; `html"…"` templates for the web client |
| [Hashiverse](examples/hashiverse.md) | Wire-protocol codecs that must match existing Rust servers; comptime crypto constant tables; `fmt` for logs |

None of them needed token-level macros.

## Open questions

None at present.

## Prior art

Zig `comptime`, `@typeInfo`, `@Type`; Rust `macro_rules!`, proc macros, `syn`, `serde`, `sqlx`; C++
`constexpr`/`consteval` and C++26 static reflection (P2996); D CTFE and `static foreach`; Jai `#run`
and `#insert`; Nim macros; Terra; Lisp and Racket hygienic macros; Swift macros (sandboxed, typed
expansion); Mojo parameters; Scala 3 `inline` and quotes; JavaScript tagged templates.

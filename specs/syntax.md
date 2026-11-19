# Syntax

> **Status:** decided direction, details open. Indexed in [`README.md`](README.md).
> Moddil's surface syntax is **based on Rust**. Every
> snippet in these specs follows this document; snippets remain illustrative where a feature's own
> design is still open.

## Rule

**Rust syntax, unless Moddil's semantics genuinely differ.** Every difference from Rust is listed
here with its reason ([Deltas from Rust (and why)](#deltas-from-rust-and-why)). A Rust programmer should be able to read Moddil code on day one.

## Mapping

| Concept | Moddil | Notes |
|---|---|---|
| Parameters | `x: &T` (shared borrow), `x: &mut T` (exclusive borrow), `x: T` (move) | Borrows are second-class ([`memory.md`: Borrowing](memory.md#borrowing)), so **no lifetime annotations** ever appear |
| Methods | `&self`, `&mut self`, `self` | As in Rust |
| Locals | `let x`, `let mut x` | As in Rust |
| Types | `struct`, `enum`, `trait`, `impl`, `type` aliases | As in Rust; the rules (coherence, no subtyping, specialisation) are in [`types.md`](types.md) |
| Generics and bounds | `fn max<T: Ord>(a: &T, b: &T) -> &T`, `where` clauses | Bounds checked at the definition ([`comptime.md`: Generics are bounded](comptime.md#generics-are-bounded)) |
| Attributes | `#[derive(…)]`, `#[repr(C)]`, `#[entry]`, `#[interrupt(…)]`, `#[component(server)]`, `#[group(…)]` | As in Rust; Moddil-specific features use attributes rather than new syntax |
| Modules and imports | `use moddil::units::si::{m, cm};`, `::` paths | As in Rust |
| Visibility | `fn f` (this module), `pub(mod) fn f` (this mod), `pub fn f` (exported) | No `pub(super)` or `pub(in path)` ([`modules.md`](modules.md#three-visibility-levels)) |
| Closures | `\|x\| expr`, `\|x\| { … }`; `impl Fn`, `impl FnMut`, `impl FnOnce`; `Box<dyn Fn…>` | **No `move`**: captures are inferred ([`closures.md`](closures.md)) |
| Concurrency | `scope(\|s\| { s.spawn(\|\| …); })` | Like `std::thread::scope`; the only way to start a task ([`concurrency.md`](concurrency.md)) |
| Labelled exits | `'label: { … break 'label value … }` | As in Rust |
| Compile-time evaluation | `const fn`, `const { … }` blocks, `<const N: int>` | Rust's spelling for the comptime concept ([`comptime.md`](comptime.md)) |
| Errors | `?`, `try { … }`, `Result`, `Option` | As in Rust, with `try … catch` sugar ([`error-handling.md`: Propagation, matching and try blocks](error-handling.md#propagation-matching-and-try-blocks)) |
| Optional values | `Option<T>` only (decided); no `T?` sugar; `x.unwrap_or(d)` rather than `x ?? d` | As in Rust; one canonical spelling, and no clash with the `?` operator |
| Prefix literals | `sql"…"`, `re"…"`, `fmt"…"` | Rust already has `b"…"` and `r"…"`; Moddil lets mods define prefixes ([`comptime.md`: Tagged string literals (embedded DSLs)](comptime.md#tagged-string-literals-embedded-dsls)) |
| Never type | `-> !` | As in Rust |
| Dynamic dispatch | `dyn Trait` | As in Rust, but any trait can be `dyn`, including generic methods ([`types.md`: `dyn`: existentials](types.md#dyn-existentials)) |

## Deltas from Rust (and why)

| Delta | Example | Why |
|---|---|---|
| **Effects in signatures, keyword `with`** (decided) | `fn fetch(id: UserId) -> User with net: Net, suspend, fail(HttpError)` | Effects are traits listed after `with` (`net: Net` names the handle; bare `Net` passes it through); `suspend`, `spawn` and `fail` are keywords. Effects are core semantics ([`effects-and-capabilities.md`](effects-and-capabilities.md)). `with` reads naturally and leaves `!` as the never type. |
| Effects are traits | `trait Time { fn now(&self) -> Instant; }`, used as `with time: Time` | No `effect` keyword: any trait listed after `with` is an effect, resolved by type in the dynamic scope ([`effects-and-capabilities.md`: Effects are traits passed implicitly](effects-and-capabilities.md#effects-are-traits-passed-implicitly)) |
| Effect variables | `fn map<T, U, E>(xs: &[T], f: impl FnMut(&T) -> U with E) -> Vec<U> with E` | Effect polymorphism ([`error-handling.md`: Why `Result`-only languages duplicate higher-order APIs](error-handling.md#why-result-only-languages-duplicate-higher-order-apis)); written only to name or constrain: an unannotated closure parameter is implicitly polymorphic ([`effects-and-capabilities.md`: Effect polymorphism](effects-and-capabilities.md#effect-polymorphism)) |
| Handler definitions | `impl Links for MemoryLinks { … }`; anonymous `impl Time { fn now(&self) -> Instant { t0 } }` | A handler is an ordinary impl; the anonymous impl expression captures like a closure. There is no `effect` keyword ([`effects-and-capabilities.md`: Defining handlers](effects-and-capabilities.md#defining-handlers)) |
| Handler blocks | `with Time = sim.clock(node), Net = sim.net.endpoint(node) { … }` | Installing effect handlers ([`determinism.md`: Handlers are scoped, not global](determinism.md#handlers-are-scoped-not-global)); the same keyword as in signatures |
| **Named and default arguments** (decided) | `urls.parallel_map(\|u\| fetch(u)?, limit: 32)`; `fn connect(host: &str, port: u16 = 443)` | Readability at call sites (Swift, Kotlin, Python). Positional arguments come first, then named ones. |
| Expression-level optional chaining | `try { users.get(id)?.profile?.email }` | `?` always propagates, as in Rust (inside a function returning `Option`, the chain works as is). A `try` block keeps the chain inside one expression, giving Swift-style chaining without a new operator ([`types.md`](types.md)). |
| Cannot-fail assertion | `Code::new(s) ?! "built from ALPHABET"` | Traps with the reason if the call fails anyway ([`error-handling.md`: This cannot fail here](error-handling.md#this-cannot-fail-here)) |
| Error context | `load(p)? fmt"loading config {p}"` | A note attached to the error, not a wrapper ([`error-handling.md`: Error types](error-handling.md#error-types)) |
| Wrapping operators | `h *% prime`, `a +% b`; `Wrapping<u32>` | Integer overflow traps by default; wrapping is explicit ([`numerics.md`: Overflow traps](numerics.md#overflow-traps)) |
| No `as` for numbers | `x.to::<u8>()?`, `x.truncate::<u8>()`, `f.round::<i64>(mode)` | Only lossless widening is implicit; every lossy conversion is named ([`numerics.md`: Conversions](numerics.md#conversions)) |
| Units of measure | `100<cm>`, `f64<m/s>` | Type-level units ([`units.md`](units.md)) |
| Generators | `gen { for x in xs { yield f(x) } }`, `gen fn lines(…) -> Line` | Stable in v0 (unstable in Rust); compile to state-machine iterators ([`stdlib.md`: Generators](stdlib.md#generators)) |
| Dedented string blocks | `sql"""` … `"""`; the closing quotes' indentation is stripped | Embedded SQL, JSON and HTML keep the code's indentation without a macro ([`text.md`: Literals](text.md#literals)) |
| No integer string indexing | `let at = s.find('@')?; &s[at..s.end()]` | Positions are `StrPos` values on character boundaries, so slicing never traps ([`text.md`: Positions and slicing](text.md#positions-and-slicing)) |
| Refinements (decided: one form) | `type Port = u16 where self >= 1024`; ranges as `type Port = u16 where self in 1..` | Validated newtypes ([`correctness.md`: Validated newtypes (lightweight refinements)](correctness.md#validated-newtypes-lightweight-refinements)). The compiler recognises range predicates and uses excluded values as layout niches, so `Option<Port>` stays 2 bytes ([`types.md`](types.md)). |
| Contracts (decided: clauses) | `requires …`, `ensures …`, `invariant …` after the signature, positioned like `where` | First-class syntax for docs, tooling and verification ([`correctness.md`: Contracts as syntax](correctness.md#contracts-as-syntax)) |
| Anonymous records (decided) | `fn bounds(xs: &[f64]) -> { min: f64, max: f64 }` | [Inference: "generic on the fly"](#inference-generic-on-the-fly) |
| Linear and view types | `linear struct Txn { … }`, `view RingIter<T> { … }` | [`memory.md`: View types](memory.md#view-types), [`resources.md`: Resources: affine and linear](resources.md#resources-affine-and-linear) |
| Comparison chaining | `0 <= i < len`, `lo < x <= hi` | One direction only; `==` does not chain ([`types.md`: Operators](types.md#operators)) |
| Matrix multiply | `a @ b` | Tensor code ([`accelerators.md`](accelerators.md)) |
| Structured `select` | `select { conn = listener.accept() => …, _ = stop_requested() => break, }` | A language construct, not a macro ([`concurrency.md`](concurrency.md)) |
| Inferred signatures for private items | `fn add(a, b) { a + b }` | [Inference: "generic on the fly"](#inference-generic-on-the-fly) |

**What disappears from Rust:** lifetimes, `async`/`.await`, `Send`/`Sync`, `Pin`, `move`, and macros
(`macro_rules!`, procedural macros). Suspension is an effect, every type is sendable, captures are
inferred, and metaprogramming is compile-time code ([`comptime.md`](comptime.md)).

## Inference: "generic on the fly"

TypeScript lets one unannotated function work for several types. The sound equivalent, used by
OCaml, F# and Haskell, is **generalisation with inferred trait bounds**:

```rust
fn add(a, b) { a + b }
// inferred: fn add<T: Add<Output = T>>(a: T, b: T) -> T

let x = add(1, 2);        // i32
let y = add(1.5, 2.25);   // f64
```

- **Private functions** (not visible outside their mod) and closures **may omit parameter and return
  types**. The compiler infers the most general signature: generic parameters, trait bounds, effects
  and error sets.
- **Public functions at mod boundaries must write their signatures.** This keeps the fast-build and
  local-reasoning rules ([`build-times.md`: Language rules that decide build speed](build-times.md#language-rules-that-decide-build-speed)) and makes interfaces reviewable.
- `moddil fix` writes an inferred signature into the source; the language server shows inferred
  signatures as inlay hints ([`toolchain.md`](toolchain.md)).
- Recursive and mutually recursive functions are solved by fixpoint; an ambiguous case asks for an
  annotation.
- Inferred bounds are still bounds, checked once ([`comptime.md`: Generics are bounded](comptime.md#generics-are-bounded)).
- **Structural typing** (TypeScript's object shapes) is not the default: Moddil types are nominal,
  with traits.
- **Anonymous records (decided):** `{ min: f64, max: f64 }` is a type that needs no declaration.
  Two records with the same field names and types are the same type. There is **no width
  subtyping**: a record with extra fields is not accepted where fewer are expected, so records
  behave like tuples with named fields.
  ```rust
  fn bounds(xs: &[f64]) -> { min: f64, max: f64 } {
      { min: xs.min(), max: xs.max() }
  }
  let b = bounds(&prices);
  println(fmt"{b.min}..{b.max}");
  ```

## Patterns and control flow

As in Rust, unless a delta says otherwise:

| Construct | Moddil | Notes |
|---|---|---|
| `match` | Exhaustive, with guards (`x if x > 0`), or-patterns (`A \| B`), ranges, slice patterns (`[first, .., last]`), bindings (`n @ 1..=9`) | Exhaustiveness rules in [`types.md`: Sum types and exhaustiveness](types.md#sum-types-and-exhaustiveness) |
| Conditional binding | `if let`, `if let` chains (`if let Some(x) = a && x > 0`), `while let`, `let … else` | |
| Loops | `loop`, `while`, `for x in iter`; `break value` from `loop`; labelled `break` and `continue` | `for` uses `IntoIterator`, including effectful iterators ([`stdlib.md`: Iteration](stdlib.md#iteration)) |
| Expressions | `if`, `match` and blocks are expressions; the last expression is the value | |
| Binding modes | Matching through a borrow binds borrows (`&T` in, `&T` fields out) | No lifetimes involved: the bindings are second-class like any borrow ([`memory.md`: Borrowing](memory.md#borrowing)) |
| Evaluation order | Left to right, specified | [`determinism.md`: Sources of nondeterminism](determinism.md#sources-of-nondeterminism) |

## Named and default arguments

- Any parameter may be passed by name: `connect(host, port: 8443)`.
- Parameters may declare defaults: `fn connect(host: &str, port: u16 = 443, timeout: Duration = 10<s>)`.
- Positional arguments come first, named arguments after, in any order.
- Defaults are part of the public interface, under semver and in semantic diffs
  ([`semantic-vcs.md`](semantic-vcs.md)).
- **Name-only parameters (decided, opt-in):** parameters after a `*` must be passed by name, as in
  Python. A lint suggests this for `bool` parameters, so `resize(w, h, true)` cannot happen:
  ```rust
  fn resize(w: u32, h: u32, *, keep_aspect: bool = false) { … }
  resize(800, 600, keep_aspect: true);
  ```
- *Open:* how defaults interact with trait methods and their implementations.

## What Swift has that we lack

| Swift feature | Moddil |
|---|---|
| Argument labels and default parameter values | **Adopted:** named and default arguments ([`syntax.md`: Named and default arguments](syntax.md#named-and-default-arguments)) |
| Optional chaining `a?.b?.c` | **Adopted** ([`syntax.md`: Deltas from Rust (and why)](syntax.md#deltas-from-rust-and-why)) |
| Computed properties | **Not adopted (decided):** methods only, so a call is visibly a call and cost and effects are never hidden behind a field-like read |
| Property observers (`willSet`, `didSet`) | Not adopted: hidden control flow |
| Property wrappers, `@dynamicMemberLookup` | Not adopted: compile-time derives cover the useful cases, and dynamic lookup harms local reasoning |
| Result builders (the SwiftUI DSL style) | Covered by compile-time code plus closures; relevant to the UI question in [`examples/03-web-store.md`](examples/03-web-store.md) |
| Key paths (`\Person.name`) | **Later (decided):** field-path values through static reflection, when UI binding and typed query builders need them; closures cover the rest |
| **Library evolution: a stable binary ABI** for frameworks | **Decided:** the C ABI plus **WASM components (WIT)** are Moddil's stable binary boundaries, for plugins and binary SDKs ([`interop.md`](interop.md)). A Swift-style resilience mode (fields and methods reached through indirection so binaries survive upgrades) is deferred. |
| Classes with shared mutable references (ARC) | Deliberately not: `Rc` contents are immutable; shared mutation goes through sync cells |
| Objective-C interop; Playgrounds | Objective-C through C ([`interop.md`](interop.md)); REPL, script and notebook modes are a non-goal ([`requirements.md`: Non-goals (draft)](requirements.md#non-goals-draft)); an in-browser playground replaces Playgrounds ([`toolchain.md`: Playground: learning without a REPL](toolchain.md#playground-learning-without-a-repl)) |
| Actors, `async`/`await`, typed throws, `some`/`any`, enums with payloads, `defer`, `Codable`, macros | Already covered: tasks and actors, effects, error handling, `impl`/`dyn`, ADTs, RAII and `errdefer`, derives, compile-time code |

## Open questions

1. Default arguments on trait methods and their implementations ([Named and default arguments](#named-and-default-arguments)).
2. Patterns: should refinements be usable as patterns (`match port { p: Port => … }`), and are
   named-argument records destructurable by name ([Patterns and control flow](#patterns-and-control-flow))?
3. Do any of Rust's binding-mode subtleties (the 2024 edition changes to `ref` and `&` in patterns)
   need to exist without lifetimes, or can the rules be simpler?
4. A Swift-style resilience mode for native binary libraries ([What Swift has that we lack](#what-swift-has-that-we-lack)).
5. The key-path API on top of static reflection.

Decided in this round: name-only parameters (opt-in, [Named and default arguments](#named-and-default-arguments)); anonymous records without width subtyping
([Inference: "generic on the fly"](#inference-generic-on-the-fly)); one refinement form, `where` ([Deltas from Rust (and why)](#deltas-from-rust-and-why)); contracts as clauses ([Deltas from Rust (and why)](#deltas-from-rust-and-why)); `Option<T>` only, with
expression-level chaining through `try { }` ([Mapping](#mapping), [Deltas from Rust (and why)](#deltas-from-rust-and-why)).

## Prior art

Rust (the baseline); Swift (argument labels, default arguments, optional chaining); Kotlin and Python
(named and default arguments); OCaml, F# and Haskell (inferred generalisation); TypeScript (inferred
generic use); Koka and Effekt (effect rows and handlers).

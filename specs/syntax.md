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
| Parameters | `x: &T` (shared borrow), `x: &mut T` (exclusive borrow), `x: T` (move) | Borrows are second-class ([`memory-and-resources.md`: Borrowing](memory-and-resources.md#borrowing)), so **no lifetime annotations** ever appear |
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
| Compile-time evaluation | `const fn`, `const { … }` blocks, `<const N: usize>` | Rust's spelling for the comptime concept ([`comptime.md`](comptime.md)) |
| Errors | `?`, `try { … }`, `Result`, `Option` | As in Rust, with `try … catch` sugar ([`error-handling.md`: Propagation, matching and try blocks](error-handling.md#propagation-matching-and-try-blocks)) |
| Optional values | `Option<T>` only (decided); no `T?` sugar; `x.unwrap_or(d)` rather than `x ?? d` | As in Rust; one canonical spelling, and no clash with the `?` operator |
| Prefix literals | `sql"…"`, `re"…"`, `fmt"…"` | Rust already has `b"…"` and `r"…"`; Moddil lets mods define prefixes ([`comptime.md`: Tagged string literals (embedded DSLs)](comptime.md#tagged-string-literals-embedded-dsls)) |
| Never type | `-> !` | As in Rust |
| Dynamic dispatch | `dyn Trait` | As in Rust, but any trait can be `dyn`, including generic methods ([`types.md`: `dyn`: existentials](types.md#dyn-existentials)) |

## Deltas from Rust (and why)

| Delta | Example | Why |
|---|---|---|
| **Effects in signatures, keyword `with`** (decided) | `fn fetch(id: UserId) -> User with net, fail(HttpError)` | Effects are core semantics ([`effects-and-capabilities.md`](effects-and-capabilities.md)). `with` reads naturally and leaves `!` as the never type. |
| Effect declarations | `effect time { fn now() -> Instant }` | User-defined effects ([`effects-and-capabilities.md`: Effects are interfaces; handlers are scoped implementations](effects-and-capabilities.md#effects-are-interfaces-handlers-are-scoped-implementations)) |
| Effect variables | `fn map<T, U, E>(xs: &[T], f: impl FnMut(&T) -> U with E) -> Vec<U> with E` | Effect polymorphism ([`error-handling.md`: Why `Result`-only languages duplicate higher-order APIs](error-handling.md#why-result-only-languages-duplicate-higher-order-apis)); written only to name or constrain: an unannotated closure parameter is implicitly polymorphic ([`effects-and-capabilities.md`: Effect polymorphism](effects-and-capabilities.md#effect-polymorphism)) |
| Handler blocks | `with time = sim.clock(node), net = sim.net.endpoint(node) { … }` | Installing effect handlers ([`determinism.md`: Handlers are scoped, not global](determinism.md#handlers-are-scoped-not-global)); the same keyword as in signatures |
| **Named and default arguments** (decided) | `urls.parallel_map(\|u\| fetch(u)?, limit: 32)`; `fn connect(host: &str, port: u16 = 443)` | Readability at call sites (Swift, Kotlin, Python). Positional arguments come first, then named ones. |
| Expression-level optional chaining | `try { users.get(id)?.profile?.email }` | `?` always propagates, as in Rust (inside a function returning `Option`, the chain works as is). A `try` block keeps the chain inside one expression, giving Swift-style chaining without a new operator ([`null-safety.md`](null-safety.md)). |
| Wrapping operators | `h *% prime`, `a +% b`; `Wrapping<u32>` | Integer overflow traps by default; wrapping is explicit ([`numerics.md`: Overflow traps](numerics.md#overflow-traps)) |
| No `as` for numbers | `x.to::<u8>()?`, `x.truncate::<u8>()`, `f.round::<i64>(mode)` | Only lossless widening is implicit; every lossy conversion is named ([`numerics.md`: Conversions](numerics.md#conversions)) |
| Units of measure | `100<cm>`, `f64<m/s>` | Type-level units ([`units.md`](units.md)) |
| Dedented string blocks | `sql"""` … `"""`; the closing quotes' indentation is stripped | Embedded SQL, JSON and HTML keep the code's indentation without a macro ([`text.md`: Literals](text.md#literals)) |
| No integer string indexing | `let at = s.find('@')?; &s[at..s.end()]` | Positions are `StrPos` values on character boundaries, so slicing never traps ([`text.md`: Positions and slicing](text.md#positions-and-slicing)) |
| Refinements (decided: one form) | `type Port = u16 where self >= 1024`; ranges as `type Port = u16 where self in 1..` | Validated newtypes ([`correctness.md`: Validated newtypes (lightweight refinements)](correctness.md#validated-newtypes-lightweight-refinements)). The compiler recognises range predicates and uses excluded values as layout niches, so `Option<Port>` stays 2 bytes ([`null-safety.md`](null-safety.md)). |
| Contracts (decided: clauses) | `requires …`, `ensures …`, `invariant …` after the signature, positioned like `where` | First-class syntax for docs, tooling and verification ([`correctness.md`: Contracts as syntax](correctness.md#contracts-as-syntax)) |
| Anonymous records (decided) | `fn bounds(xs: &[f64]) -> { min: f64, max: f64 }` | [Inference: "generic on the fly"](#inference-generic-on-the-fly) |
| Linear and view types | `linear struct Txn { … }`, `view RingIter<T> { … }` | [`memory-and-resources.md`: View types](memory-and-resources.md#view-types), [`memory-and-resources.md`: Resources: affine and linear](memory-and-resources.md#resources-affine-and-linear) |
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

## Open questions

1. Default arguments on trait methods and their implementations ([Named and default arguments](#named-and-default-arguments)).

Decided in this round: name-only parameters (opt-in, [Named and default arguments](#named-and-default-arguments)); anonymous records without width subtyping
([Inference: "generic on the fly"](#inference-generic-on-the-fly)); one refinement form, `where` ([Deltas from Rust (and why)](#deltas-from-rust-and-why)); contracts as clauses ([Deltas from Rust (and why)](#deltas-from-rust-and-why)); `Option<T>` only, with
expression-level chaining through `try { }` ([Mapping](#mapping), [Deltas from Rust (and why)](#deltas-from-rust-and-why)).

## Prior art

Rust (the baseline); Swift (argument labels, default arguments, optional chaining); Kotlin and Python
(named and default arguments); OCaml, F# and Haskell (inferred generalisation); TypeScript (inferred
generic use); Koka and Effekt (effect rows and handlers).

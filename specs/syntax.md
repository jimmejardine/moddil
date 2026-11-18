# Syntax

> **Status:** decided direction, details open. Moddil's surface syntax is **based on Rust**. Every
> snippet in these specs follows this document; snippets remain illustrative where a feature's own
> design is still open.

## 1. Rule

**Rust syntax, unless Moddil's semantics genuinely differ.** Every difference from Rust is listed
here with its reason (§3). A Rust programmer should be able to read Moddil code on day one.

## 2. Mapping

| Concept | Moddil | Notes |
|---|---|---|
| Parameters | `x: &T` (shared borrow), `x: &mut T` (exclusive borrow), `x: T` (move) | Borrows are second-class ([`memory-and-resources.md`](memory-and-resources.md) §3), so **no lifetime annotations** ever appear |
| Methods | `&self`, `&mut self`, `self` | As in Rust |
| Locals | `let x`, `let mut x` | As in Rust |
| Types | `struct`, `enum`, `trait`, `impl`, `type` aliases | As in Rust |
| Generics and bounds | `fn max<T: Ord>(a: &T, b: &T) -> &T`, `where` clauses | Bounds checked at the definition ([`comptime.md`](comptime.md) §5) |
| Attributes | `#[derive(…)]`, `#[repr(C)]`, `#[entry]`, `#[interrupt(…)]`, `#[component(server)]`, `#[group(…)]` | As in Rust; Moddil-specific features use attributes rather than new syntax |
| Modules and imports | `use moddil::units::si::{m, cm};`, `::` paths | As in Rust |
| Closures | `\|x\| expr`, `\|x\| { … }`; `impl Fn`, `impl FnMut`, `impl FnOnce`; `Box<dyn Fn…>` | **No `move`**: captures are inferred ([`closures.md`](closures.md)) |
| Concurrency | `scope(\|s\| { s.spawn(\|\| …); })` | Like `std::thread::scope`; the only way to start a task ([`concurrency.md`](concurrency.md)) |
| Labelled exits | `'label: { … break 'label value … }` | As in Rust |
| Compile-time evaluation | `const fn`, `const { … }` blocks, `<const N: usize>` | Rust's spelling for the comptime concept ([`comptime.md`](comptime.md)) |
| Errors | `?`, `try { … }`, `Result`, `Option` | As in Rust, with `try … catch` sugar ([`error-handling.md`](error-handling.md) §4) |
| Optional values | `Option<T>` only (decided); no `T?` sugar; `x.unwrap_or(d)` rather than `x ?? d` | As in Rust; one canonical spelling, and no clash with the `?` operator |
| Prefix literals | `sql"…"`, `re"…"`, `fmt"…"` | Rust already has `b"…"` and `r"…"`; Moddil lets mods define prefixes ([`comptime.md`](comptime.md) §8) |
| Never type | `-> !` | As in Rust |
| Dynamic dispatch | `dyn Trait` | As in Rust |

## 3. Deltas from Rust (and why)

| Delta | Example | Why |
|---|---|---|
| **Effects in signatures, keyword `with`** (decided) | `fn fetch(id: UserId) -> User with net, fail(HttpError)` | Effects are core semantics ([`effects-and-capabilities.md`](effects-and-capabilities.md)). `with` reads naturally and leaves `!` as the never type. |
| Effect declarations | `effect time { fn now() -> Instant }` | User-defined effects ([`effects-and-capabilities.md`](effects-and-capabilities.md) §4.1) |
| Effect variables | `fn map<T, U, E>(xs: &[T], f: impl FnMut(&T) -> U with E) -> Vec<U> with E` | Effect polymorphism ([`error-handling.md`](error-handling.md) §3.2); written only to name or constrain: an unannotated closure parameter is implicitly polymorphic ([`effects-and-capabilities.md`](effects-and-capabilities.md) §4.7) |
| Handler blocks | `with time = sim.clock(node), net = sim.net.endpoint(node) { … }` | Installing effect handlers ([`determinism.md`](determinism.md) §4); the same keyword as in signatures |
| **Named and default arguments** (decided) | `urls.parallel_map(\|u\| fetch(u)?, limit: 32)`; `fn connect(host: &str, port: u16 = 443)` | Readability at call sites (Swift, Kotlin, Python). Positional arguments come first, then named ones. |
| Expression-level optional chaining | `try { users.get(id)?.profile?.email }` | `?` always propagates, as in Rust (inside a function returning `Option`, the chain works as is). A `try` block keeps the chain inside one expression, giving Swift-style chaining without a new operator ([`null-safety.md`](null-safety.md)). |
| Wrapping operators | `h *% prime`, `a +% b`; `Wrapping<u32>` | Integer overflow traps by default; wrapping is explicit ([`numerics.md`](numerics.md) §4.2) |
| No `as` for numbers | `x.to::<u8>()?`, `x.truncate::<u8>()`, `f.round::<i64>(mode)` | Only lossless widening is implicit; every lossy conversion is named ([`numerics.md`](numerics.md) §4.5) |
| Units of measure | `100<cm>`, `f64<m/s>` | Type-level units ([`units.md`](units.md)) |
| Refinements (decided: one form) | `type Port = u16 where self >= 1024`; ranges as `type Port = u16 where self in 1..` | Validated newtypes ([`correctness.md`](correctness.md) §3.2). The compiler recognises range predicates and uses excluded values as layout niches, so `Option<Port>` stays 2 bytes ([`null-safety.md`](null-safety.md)). |
| Contracts (decided: clauses) | `requires …`, `ensures …`, `invariant …` after the signature, positioned like `where` | First-class syntax for docs, tooling and verification ([`correctness.md`](correctness.md) §3.1) |
| Anonymous records (decided) | `fn bounds(xs: &[f64]) -> { min: f64, max: f64 }` | §4 |
| Linear and view types | `linear struct Txn { … }`, `view RingIter<T> { … }` | [`memory-and-resources.md`](memory-and-resources.md) §4, §7 |
| Matrix multiply | `a @ b` | Tensor code ([`accelerators.md`](accelerators.md)) |
| Structured `select` | `select { conn = listener.accept() => …, _ = stop_requested() => break, }` | A language construct, not a macro ([`concurrency.md`](concurrency.md)) |
| Inferred signatures for private items | `fn add(a, b) { a + b }` | §4 |

**What disappears from Rust:** lifetimes, `async`/`.await`, `Send`/`Sync`, `Pin`, `move`, and macros
(`macro_rules!`, procedural macros). Suspension is an effect, every type is sendable, captures are
inferred, and metaprogramming is compile-time code ([`comptime.md`](comptime.md)).

## 4. Inference: "generic on the fly"

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
  local-reasoning rules ([`build-times.md`](build-times.md) §3) and makes interfaces reviewable.
- `moddil fix` writes an inferred signature into the source; the language server shows inferred
  signatures as inlay hints ([`toolchain.md`](toolchain.md)).
- Recursive and mutually recursive functions are solved by fixpoint; an ambiguous case asks for an
  annotation.
- Inferred bounds are still bounds, checked once ([`comptime.md`](comptime.md) §5).
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

## 5. Named and default arguments

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

## 6. Open questions

1. Default arguments on trait methods and their implementations (§5).

Decided in this round: name-only parameters (opt-in, §5); anonymous records without width subtyping
(§4); one refinement form, `where` (§3); contracts as clauses (§3); `Option<T>` only, with
expression-level chaining through `try { }` (§2, §3).

## Prior art

Rust (the baseline); Swift (argument labels, default arguments, optional chaining); Kotlin and Python
(named and default arguments); OCaml, F# and Haskell (inferred generalisation); TypeScript (inferred
generic use); Koka and Effekt (effect rows and handlers).

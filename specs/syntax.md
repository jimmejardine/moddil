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
| Prefix literals | `sql"…"`, `re"…"`, `fmt"…"` | Rust already has `b"…"` and `r"…"`; Moddil lets mods define prefixes ([`comptime.md`](comptime.md) §8) |
| Never type | `-> !` | As in Rust |
| Dynamic dispatch | `dyn Trait` | As in Rust |

## 3. Deltas from Rust (and why)

| Delta | Example | Why |
|---|---|---|
| **Effects in signatures, keyword `with`** (decided) | `fn fetch(id: UserId) -> User with net, fail(HttpError)` | Effects are core semantics ([`effects-and-capabilities.md`](effects-and-capabilities.md)). `with` reads naturally and leaves `!` as the never type. |
| Effect variables | `fn map<T, U, E>(xs: &[T], f: impl FnMut(&T) -> U with E) -> Vec<U> with E` | Effect polymorphism ([`error-handling.md`](error-handling.md) §3.2) |
| Handler blocks | `with time = sim.clock(node), net = sim.net.endpoint(node) { … }` | Installing effect handlers ([`determinism.md`](determinism.md) §4); the same keyword as in signatures |
| **Named and default arguments** (decided) | `urls.parallel_map(\|u\| fetch(u)?, limit: 32)`; `fn connect(host: &str, port: u16 = 443)` | Readability at call sites (Swift, Kotlin, Python). Positional arguments come first, then named ones. |
| Optional chaining | `users.get(id)?.profile?.email` | From Swift; fits `?` ([`null-safety.md`](null-safety.md)) |
| Optional types | `T?` as sugar for `Option<T>` | Proposed in [`null-safety.md`](null-safety.md) (open there) |
| Units of measure | `100<cm>`, `f64<m/s>` | Type-level units ([`units.md`](units.md)) |
| Refinements | `type Port = u16 where self >= 1024` | Validated newtypes ([`correctness.md`](correctness.md) §3.2). *Open:* unify with `null-safety.md`'s `u16 in 1..` |
| Contracts | `requires …`, `ensures …`, `invariant …` clauses | [`correctness.md`](correctness.md) §3.1 |
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
  with traits. *Open:* anonymous records for local data shaping.

## 5. Named and default arguments

- Any parameter may be passed by name: `connect(host, port: 8443)`.
- Parameters may declare defaults: `fn connect(host: &str, port: u16 = 443, timeout: Duration = 10<s>)`.
- Positional arguments come first, named arguments after, in any order.
- Defaults are part of the public interface, under semver and in semantic diffs
  ([`semantic-vcs.md`](semantic-vcs.md)).
- *Open:* whether a parameter can be declared name-only (as with Swift labels), and how defaults
  interact with trait methods.

## 6. Open questions

1. Name-only parameters (§5).
2. Anonymous structural records (§4).
3. One refinement syntax (`where self …` vs `in 1..`).
4. Contracts as attributes (`#[requires(…)]`) or as clauses (current).
5. `T?` sugar vs plain `Option<T>`.

## Prior art

Rust (the baseline); Swift (argument labels, default arguments, optional chaining); Kotlin and Python
(named and default arguments); OCaml, F# and Haskell (inferred generalisation); TypeScript (inferred
generic use); Koka and Effekt (effect rows and handlers).

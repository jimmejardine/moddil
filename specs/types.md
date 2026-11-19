# Type system core

> **Status:** major questions. The core rules below are decided; the exact specialisation rules and
> the details of bridge mods are still open. Listed in the [index](README.md). Syntax per
> [`syntax.md`](syntax.md); snippets are **illustration only**.

## Summary

| Topic | Decision |
|---|---|
| **Baseline** | Rust's type system: structs, enums with payloads, traits, generics with bounds, associated types, GATs. **No lifetimes**, because borrows are second-class ([`memory.md`](memory.md)). |
| **Subtyping** | **None.** No variance rules. A short, fixed list of implicit coercions; every other conversion is explicit ([No subtyping](#no-subtyping)). |
| **Coherence** | **Exactly one impl per trait and type in any program.** Rust's orphan rule, plus: the application mod and mods from the same repository may write any impl, and declared **bridge mods** may join two other mods ([Coherence: one impl per trait and type](#coherence-one-impl-per-trait-and-type)). |
| **Specialisation** | **Restricted form in v0.** Overlapping impls only when one is strictly more specific, and only beside the impl it specialises ([Specialisation](#specialisation)). |
| **Generics** | Checked once, at the definition. Debug builds pass **witness tables**; release builds **monomorphise**. Same semantics in both ([Generics](#generics)). |
| **`dyn`** | **Swift-style existentials.** Any trait can be used as `dyn`; generic methods dispatch through the witness table ([`dyn`: existentials](#dyn-existentials)). |
| **Sum types** | Exhaustive `match`; `#[non_exhaustive]` for public enums that may grow ([Sum types and exhaustiveness](#sum-types-and-exhaustiveness)). |
| **Inference** | Local only. Private signatures may be inferred and generalised within a mod; nothing is inferred across a mod boundary ([Inference](#inference)). |
| **Optional values** | No null in safe code: `Option<T>` only, no `T?` sugar; definite initialisation; guaranteed niches ([Optional values and invalid states](#optional-values-and-invalid-states)). |
| **Paradigms** | Functional and object-oriented styles both supported; no inheritance, traits plus delegation instead ([Paradigms: functional and object-oriented](#paradigms-functional-and-object-oriented)). |
| **Higher-kinded types** | Not now; GATs cover the known needs ([Paradigms: functional and object-oriented](#paradigms-functional-and-object-oriented)). |

## Problem

Moddil takes Rust's traits, generics and sum types, but several other areas depend on rules that Rust
either gets painfully wrong for an ecosystem or leaves unstable:

- **Coherence decides how an ecosystem grows.** If two mods may both implement `Serialize` for
  `DateTime`, programs can contain two meanings for the same call. If neither may, every pair of mods
  must coordinate through feature flags or newtypes.
- **The generics strategy decides build times.** Monomorphising everything makes debug builds slow;
  dictionary passing everywhere makes release builds slow ([`build-times.md`](build-times.md)).
- **Readers and agents need local reasoning.** Which impl runs, which conversion happens and what a
  generic function requires should all be visible from the code at hand, not from imports elsewhere.
- **Subtyping and variance are a large source of confusion** in Rust (lifetime variance), Java
  (array covariance) and Scala, and Moddil has removed most of the reasons for them.

## Prior art

| Language | What it does | Lesson |
|---|---|---|
| **Rust** | Orphan rule; `min_specialization` (unstable for about ten years, mostly because of lifetimes); "dyn compatibility" rules | Coherence works, but the orphan workarounds are a constant tax; specialisation is hard *because of lifetimes* |
| **Swift** | Protocols, witness tables, existentials (`any P`), retroactive conformances (warned about since Swift 6) | One generics implementation that can be unspecialised or specialised; existentials that handle generic requirements |
| **Haskell** | Type classes; orphan instances allowed with a warning; `OverlappingInstances` | Orphans cause real conflicts; unrestricted overlap makes instance choice hard to predict |
| **Scala** | `given`/`using` (Scala 3), implicits (Scala 2) | Scoped instances are flexible, but Scala 2's implicits are the standard example of code whose behaviour depends on imports |
| **OCaml** | Modules and functors instead of type classes | Explicit instance passing is fully local, but verbose for everyday operations like `==` and hashing |
| **Go** | Structural interfaces, no explicit impls | No coherence problem, but no associated types, and accidental conformance |

Further reading: Hoare, "Null References: The Billion Dollar Mistake" (QCon 2009); Swift `Optional`;
Kotlin null safety; TypeScript `strictNullChecks`; Dart sound null safety; C# 8 nullable reference
types; Java Valhalla null-restricted types and JSpecify; Rust niche optimisation and pattern types;
F* and Liquid Haskell refinement types; typestate (Strom and Yemini, 1986). Kotlin `by` delegation
and Go embedding; Scala and Swift traits and protocols with default implementations; Haskell and
Koka (purity, effects); Lean 4 and Koka (Perceus reuse).

## Kinds of type

This document defines the rules; the types themselves are mostly defined elsewhere.

| Kind | Notes |
|---|---|
| Primitives | Integers, floats, `bool`, `char` ([`numerics.md`](numerics.md)) |
| Tuples, arrays `[T; N]`, slices `[T]` | As in Rust |
| `struct`, `enum` | Nominal, as in Rust; enums carry payloads |
| Anonymous records `{ min: f64, max: f64 }` | **Structural**, without width subtyping ([`syntax.md`: Inference: "generic on the fly"](syntax.md#inference-generic-on-the-fly)) |
| Function types `fn(A) -> B with E` | Effects are part of the type ([`effects-and-capabilities.md`: Effect polymorphism](effects-and-capabilities.md#effect-polymorphism)) |
| Closures | Unnameable types implementing `Fn`, `FnMut` or `FnOnce` ([`closures.md`](closures.md)) |
| `!` | The never type |
| Refinements `u16 where self >= 1024` | A validated newtype over a base type ([`correctness.md`: Validated newtypes (lightweight refinements)](correctness.md#validated-newtypes-lightweight-refinements)) |
| Unit quantities `f64<m/s>` | Dimensions are type-level ([`units.md`: Dimensions: the SI vector and an open set](units.md#dimensions-the-si-vector-and-an-open-set)) |
| `&T`, `&mut T`, view types | Second-class borrows ([`memory.md`: View types](memory.md#view-types)) |
| `linear struct` | Must be consumed explicitly ([`resources.md`: Resources: affine and linear](resources.md#resources-affine-and-linear)) |
| `dyn Trait` | An existential, always behind a pointer ([`dyn`: existentials](#dyn-existentials)) |

## No subtyping

**Decision: Moddil has no subtyping, so it has no variance.** Rust's variance exists almost entirely
for lifetimes, Java's and Scala's for inheritance, and TypeScript's for width subtyping. Moddil has
none of these.

The cases where a value of one type is accepted as another are a **fixed list of coercions**, applied
only where the expected type is known (an argument, an annotated `let`, a return):

| Coercion | Example |
|---|---|
| `!` to any type | `let x: u32 = match r { Ok(v) => v, Err(_) => return };` |
| Refinement to its base type | A `Port` (`u16 where …`) is passed where `u16` is expected |
| `&mut T` to `&T` | Lending an exclusive borrow as a shared one |
| Unsizing | `&[T; N]` to `&[T]`; `Box<T>` to `Box<dyn Trait>` when `T: Trait` |

Everything else is explicit: numeric conversions (`x.to::<u8>()?`, `x.truncate::<u8>()`; there is no `as` for numbers,
[`numerics.md`](numerics.md)), refinement checks in the other direction (`Port::try_from(n)?`), and
unit conversions ([`units.md`](units.md)).

**Consequences:**

- `Vec<Port>` is not a `Vec<u16>`, and never needs to be: the coercion applies to values, not to
  containers. A whole container is converted explicitly (`ports.map(u16::from)`) or viewed as its
  base type through a method the refinement provides.
- Generic code never has to ask "is `F<A>` a subtype of `F<B>`?", which removes a whole class of
  confusing errors.

## Traits

As in Rust, with the omissions that follow from other decisions:

- **Supertraits and default methods**; associated types and associated consts.
- **Generic associated types** (lending iterators, collection families). **Higher-kinded types are
  deferred**.
- **No inheritance.** Delegation (`#[delegate(Trait)]`) forwards a trait's methods to a field
  ([Multiple inheritance](#multiple-inheritance)).
- **Derives** (`#[derive(Eq, Hash, …)]`) are comptime code ([`comptime.md`](comptime.md)); what they
  generate for equality, hashing and copying is settled in [`values.md`](values.md).
- **No `Send` or `Sync`**, and no other auto traits: every type is sendable
  ([`concurrency.md`: No `Send` / `Sync`](concurrency.md#no-send--sync)).
- **Any trait can be used as an effect** by listing it after `with` (`with time: Time`). The
  implementation is then passed implicitly, found by trait type in the dynamic scope; coherence is
  unaffected, because each handler is an ordinary impl on its own type
  ([`effects-and-capabilities.md`: Effects are traits passed implicitly](effects-and-capabilities.md#effects-are-traits-passed-implicitly)).
- **Anonymous impls**: `impl Trait { … }` is an expression that implements a trait for an unnamed
  type and captures its environment like a closure ([`effects-and-capabilities.md`: Defining handlers](effects-and-capabilities.md#defining-handlers)).
- Trait methods may carry the **control effects** `suspend` and `fail(E)`; implementations may
  perform fewer than the trait declares, never more. Other dependencies of an implementation are
  captured when the value is built ([`effects-and-capabilities.md`: Effects inside handlers](effects-and-capabilities.md#effects-inside-handlers)).

## Coherence: one impl per trait and type

**Decision: in any program there is at most one implementation of a trait for a type.** Which impl
runs is therefore a property of the program, not of the imports in scope.

### Who may write an impl

An `impl Trait for Type` is allowed in a mod when **any** of these holds:

1. **The mod owns the trait or the type** (Rust's orphan rule, including its rules for local type
   parameters such as `impl<T> From<MyType> for Vec<T>`).
2. **The mod is the application**: the root mod being built as an executable, test binary or
   component. Nothing depends on it, so nothing downstream can clash with it.
3. **The mods share an owner**: mods published from the **same repository** count as one owner for
   this rule. This removes Rust's "split crate" problem, where a project that owns both crates still
   cannot write the impl in a third.
4. **The mod is a declared bridge** between the trait's mod and the type's mod (below).

### Bridge mods

A **bridge mod** exists to join two mods that do not know about each other, such as a serialisation
mod and a date-time mod:

```toml
# manifest (illustration only)
[mod]
name = "github.com/acme/chrono-serde"
version = "1.0.0"
bridge = ["moddil.dev/serde", "github.com/chrono/chrono"]   # traits from the first, types from the second
```

- A bridge may implement **traits of the first mod for types of the second**, and nothing else
  outside the orphan rule.
- **Duplicate bridges are a build error.** When a program is built, the toolchain checks that no two
  mods in the dependency graph implement the same trait for the same type. Each mod's **interface
  artefact** ([`build-times.md`: Language rules that decide build speed](build-times.md#language-rules-that-decide-build-speed)) lists
  the impls it provides, so the check is a set union over artefacts, not a re-analysis.
- Because mods form a DAG and versions are selected once per program
  ([`packages-and-supply-chain.md`](packages-and-supply-chain.md)), one program contains one version of a bridge; two
  libraries that both use `chrono-serde` share it.
- The owning mods can later absorb the impl. Moving an impl from a bridge into the owning mod is a
  migration `moddil` can verify ([`evolution.md`](evolution.md)).

### Why not scoped or named impls

Scala's `given`s and Haskell's local-instance proposals let different parts of a program use
different impls of the same trait for the same type. Moddil rejects this:

- **Data structures can disagree.** A `HashMap` built where one `Hash` impl is in scope and queried
  where another is in scope misses its own keys. The fixes (carrying the impl in the type, as in
  `Set<T, by = MyOrd>`) complicate every generic type.
- **Behaviour depends on imports.** Changing a `use` line can change which code runs without any
  visible change at the call site: bad for reviewers and for agents.
- **Upgrades break builds at a distance.** A dependency that adds an impl can make existing code
  ambiguous.

### What Rust's workarounds cost, and what remains

| Rust workaround | Cost | In Moddil |
|---|---|---|
| Newtype wrapper (`struct MyDate(DateTime)`) | Wrap and unwrap at every boundary; re-implement every other trait; `Vec<DateTime>` cannot become `Vec<MyDate>` | Rarely needed; when it is, `#[delegate]` forwards the other traits |
| Optional feature flags (chrono's `serde` feature) | The type's owner must depend on every trait mod; N × M coordination | Bridge mods |
| Remote derive (`#[serde(with = …)]`) | A parallel, hand-maintained description of a foreign type | Bridge mods, with derives run inside the bridge |
| Same owner, different crates | Still forbidden | Same-repository rule |

## Specialisation

**Decision: a restricted form of specialisation is in v0.** Rust's `specialization` has been unstable
for about ten years mainly because impl selection could depend on lifetimes, which are erased before
code generation. Moddil has no lifetime parameters, which removes that problem.

```rust
impl<T: Clone> Extend<T> for Vec<T> {
    default fn extend_from_slice(&mut self, items: &[T]) {       // general: clone one at a time
        for x in items { self.push(x.clone()); }
    }
}

impl<T: Copy> Extend<T> for Vec<T> {
    fn extend_from_slice(&mut self, items: &[T]) {               // specialised: one memcpy
        self.copy_from_slice_at_end(items);
    }
}
```

Rules (proposal, to be settled in detail):

1. **Strictly more specific.** Two impls may overlap only if one applies to a strict subset of the
   other's types. Overlaps that neither contains are an error, as today.
2. **`default` marks what may be overridden.** Only items marked `default` in the general impl can be
   replaced.
3. **No action at a distance.** A specialising impl must be in the same mod as the general impl, or in
   the mod that owns the type. A third mod cannot change how a library behaves for a library's own
   types.
4. **Same choice in debug and release.** The impl is selected when the witness table for a concrete
   type is built ([Generics](#generics)), the same point at which a release build monomorphises, so
   both builds run the same code.
5. **Specialisation is not observable as a semantic change.** The specialised impl must be
   behaviourally equivalent to the general one. This is not checked by the compiler; property tests
   can compare the two ([`correctness.md`](correctness.md)).

## Generics

- **Checked once, at the definition** (decided, [`comptime.md`: Generics are bounded](comptime.md#generics-are-bounded)).
  A generic function's bounds are the whole contract; errors point at the call site, not into the
  library.
- **Two compilation strategies, one semantics** (decided):
  - debug builds compile one copy of a generic function, which receives a hidden **witness table**
    (size, alignment, copy, drop and the trait methods it uses);
  - release builds produce a specialised copy per type (monomorphisation);
  - anything layout-dependent lives in the witness table, which is what makes the two identical
    ([`build-times.md`: Language rules that decide build speed](build-times.md#language-rules-that-decide-build-speed), rule 5).
- **Const generics** use Rust's spelling, with `int` for sizes (`<const N: int>`,
  [`numerics.md`: Integer types](numerics.md#integer-types)). **Proposal:** a const parameter may
  have any type that is comptime-evaluable and has structural equality (integers, `bool`, `char`,
  strings, and enums and structs made of them), so `Matrix<f32, 3, 3>` and `Format<"utf-8">` are
  both possible.
- **`impl Trait`** in argument position (an anonymous generic) and in return position (an opaque
  type), including in trait methods.
- **Inferred generalisation for private items:** `fn add(a, b) { a + b }` is generalised to
  `fn add<T: Add<Output = T>>(a: T, b: T) -> T` ([`syntax.md`: Inference: "generic on the fly"](syntax.md#inference-generic-on-the-fly)).
- **Effect polymorphism** is implicit for closure parameters ([`effects-and-capabilities.md`: Effect polymorphism](effects-and-capabilities.md#effect-polymorphism)).

## `dyn`: existentials

**Decision: any trait can be used as `dyn Trait`**, as with Swift's `any P`, rather than only the
"dyn-compatible" traits Rust allows.

- A `dyn Trait` value is a pointer to the data plus a **witness table** for its concrete type, the
  same table debug builds already pass to generic code.
- **Generic methods work through `dyn`.** The call passes the witness tables for the method's type
  parameters, just as an unspecialised generic call does:

  ```rust
  trait Sink {
      fn write<T: Serialize>(&mut self, value: &T) with fail(IoError);
  }

  fn log_all(sinks: &mut [Box<dyn Sink>], event: &Event) with fail(IoError) {
      for s in sinks { s.write(event)?; }         // a generic method, called through dyn
  }
  ```

  Rust rejects `dyn Sink` here; the usual workaround is a second, object-safe trait plus a blanket
  impl.
- **What is unavailable on `dyn`:** methods with no receiver (`fn new() -> Self`), and methods that
  take or return `Self` by value, because the concrete type is unknown. These methods are simply not
  callable on a `dyn` value; the rest of the trait remains usable. There is no
  `where Self: Sized` ritual.
- `dyn Trait` is unsized and always lives behind a pointer: `&dyn`, `&mut dyn`, `Box<dyn>`, or an rc.
- Associated types must be fixed to be used: `dyn Iterator<Item = u8>`.

## Operators

Operators are traits, as in Rust: `a + b` calls `Add::add`, `a[i]` calls `Index::index`, `-a` calls
`Neg::neg`. There are no user-defined operator symbols.

- Moddil's extra operators are traits too: the wrapping operators (`+%`, `-%`, `*%`;
  [`numerics.md`](numerics.md)) and matrix multiply (`@`; [`accelerators.md`](accelerators.md)).
- **Output types follow the operands.** `f64<m> / f64<s>` is `f64<m/s>`: the `Output` associated
  type is computed at compile time from the unit dimensions ([`units.md`: Arithmetic, conversion and polymorphism](units.md#arithmetic-conversion-and-polymorphism)).
  The same holds for `Fixed<S>` scales ([`numerics.md`: Decimals for money](numerics.md#decimals-for-money)).
- `==` and `<` come from `Eq` and `Ord` ([`values.md`: Equality and ordering traits](values.md#equality-and-ordering-traits)).
- Operators may perform effects only if the trait allows it; the standard operator traits are
  pure apart from traps (open question 8).

## Sum types and exhaustiveness

- `match` must be **exhaustive**, as in Rust; so must `let … else` and `if let` chains used as
  expressions.
- **`#[non_exhaustive]`** on a public enum means other mods must include a wildcard arm. Adding a
  variant to an exhaustive public enum is a breaking change, and the compiler's semver check reports
  it ([`packages-and-supply-chain.md`: Semver is checked by the compiler](packages-and-supply-chain.md#semver-is-checked-by-the-compiler)).
- **Refinements in patterns:** matching on a refinement's base value narrows it (`n if n >= 1024`
  binds a `u16`; converting to `Port` remains explicit, or proven automatically where
  [`correctness.md`: Refinement types with automatic proofs](correctness.md#refinement-types-with-automatic-proofs) applies).

## Optional values and invalid states

Absence and invalid states are handled by the type system, not by runtime checks.

### Why null is a problem

Null references (Hoare's "billion-dollar mistake"). More generally: types that admit
values the domain does not, such as zero values nobody chose, unchecked ranges, mixed-up IDs, and
objects used in the wrong state.

### How other languages handle absence

Every language that has tackled null is converging on *absence in the type,
plus compiler-checked narrowing*, in one of two styles:

| Style | Languages | Strength | Weakness |
|---|---|---|---|
| Wrapper sum type `Option<T>` | Rust, Haskell (`Maybe`), OCaml, Scala, Zig (`?T`) | Ordinary type; composes; nests (`Option<Option<T>>` ≠ `Option<T>`) | Ceremony without sugar |
| Nullable union `T \| null` + flow typing | Kotlin, TypeScript (`strictNullChecks`), Dart 3, C# 8, Java (Valhalla null-restricted types, JSpecify) | Ergonomic; can be retrofitted onto a language with null | `T??` collapses to `T?`: "absent" vs "present but null" is lost |

- **Swift** is the synthesis most new designs copy. `T?` is sugar for a real `Optional<T>` sum type,
  so it nests correctly, and it adds `if let`, `guard let`, optional chaining `a?.b` and `x ?? d`.
- **Rust** has exactly one `Option`, kept separate from `Result`: absence is a normal outcome,
  failure is an error.
  - Raw pointers can be null only in `unsafe` code.
  - The **niche optimisation** makes `Option<&T>` pointer-sized, with `None` as address 0.
  - Ergonomics keep improving: `?` on `Option`, `let … else`, `if let` chains (2024 edition).
  - The *pattern types* work (`u32 is 1..`) generalises niches.
- **The frontier is invalid states in general:**
  - definite initialisation (no Go-style zero values);
  - refinement types (Liquid Haskell, F*);
  - newtypes;
  - typestate.

### Decision: no null, and no invalid states

- **No null in safe code.** Absence is `Option<T>` = `None | Some(T)`. It is an ordinary sum type in a
  core-adjacent mod (R2); the compiler adds only sugar and layout guarantees.
- **`Option<T>` only, as in Rust** (decided; [`syntax.md`](syntax.md)). There is no `T?` sugar.
  Nesting is explicit: `Option<Option<T>>`. It comes with:
  - exhaustive `match`, `if let` and `let … else`;
  - `?`, which always **propagates** `None`, as in Rust. Inside a function returning `Option`,
    `a?.b?.c` works directly;
  - **expression-level chaining** through a `try` block: `try { a?.b?.c }` yields an `Option`
    without leaving the function (Swift's chaining, with no new operator);
  - defaulting with `x.unwrap_or(d)` and `x.unwrap_or_else(|| …)`.

  There is no implicit unwrap. `unwrap()` is an explicit, visible panic point.
- **Absence is data; failure is an effect.** `fail(E)` ([Error handling](error-handling.md)) plays the role of Rust's `Result`,
  so `Option` is the only "maybe" type in everyday code.
  - `opt.or_fail(e)` turns absence into failure.
  - Catching a failure can produce an `Option`.
- **Guaranteed niches:** `Option<&T>`, `Option<Rc<T>>`, owned pointers, non-zero and range-typed
  integers, and enums with spare values are all the same size as `T`. Nullable C pointers import as
  `Option<…>` ([Interoperability with C and Rust](interop.md)).
- **Fits the ownership model:**
  - `Option<&T>`, or an `Option` holding a view, is itself a view
    ([`memory.md`: View types](memory.md#view-types)).
  - Moving out of an `Option` is an ordinary move (`take()`).
- **Definite initialisation:** no zero values, and no reads of uninitialised memory in safe code.
  Values are created only by literals that supply every field; there are no special constructors
  ([`memory.md`: Ownership and RAII](memory.md#ownership-and-raii)).
- **Invalid states beyond null:**
  - **Cheap newtypes:** zero-cost and distinct, with no implicit conversion (`UserId` ≠ `OrderId`).
  - **Range refinements** (`u32 where self in 1..`): values that are excluded become free niches for
    `Option`. This is the same `where` form as other refinements (decided).
  - **Refinement types** (`Int where 0 <= self < 100`): checked statically where provable,
    dynamically otherwise. They share machinery with contracts ([Correctness tooling: contracts and verification](correctness.md)).
  - **Typestate through linear types** ([Memory: ownership, borrowing and sharing](memory.md)): states are types, and transitions consume the old
    state, e.g. `Conn<Unauthed>` → `Conn<Authed>`.

Illustration only:
```rust
struct UserId(u64);
type Port = u16 where self in 1..;                // 0 excluded: Option<Port> is still 2 bytes

fn email_of(users: &Map<UserId, User>, id: UserId) -> Option<&str> {
    users.get(id)?.profile?.email                 // `?` propagates None out of email_of
}

fn greeting(users: &Map<UserId, User>, id: UserId) -> String {
    let email = try { users.get(id)?.profile?.email };   // chaining within one expression
    fmt"Hello {email.unwrap_or("guest")}"
}

fn connect(host: &str, port: Option<Port>) with Net, suspend, fail(NetError) {
    let Some(addr) = resolve(host) else { return };   // early exit on absence
    let Some(p) = port else { fail(NetError::NoPort) };
    …
}
```

## Inference

- **Local only** ([`build-times.md`](build-times.md) rule 7). Types flow within a function body,
  as in Rust.
- **Private items may omit their signatures** and are generalised within their mod
  ([Generics](#generics)).
- **Nothing is inferred across a mod boundary.** Public signatures, effects and error sets are written
  out and recorded in the interface artefact (rule 2), so editing a body never re-checks dependents.

## Paradigms: functional and object-oriented

### Paradigms in brief

| Question | Answer |
|---|---|
| Functional programming? | **Yes, strongly.** Closures, immutability by default, algebraic data types, pattern matching, iterators, and purity tracked by effects. |
| Object-oriented programming? | **Yes, the useful parts:** encapsulation, methods, interfaces (traits), static and dynamic polymorphism, actors. |
| Inheritance? | **No class inheritance**, single or multiple. Traits with supertraits and default methods, plus delegation, cover the use cases. Trait rules are in [`types.md`](types.md). |

### Functional programming

- **First-class functions and closures**, including closures that fail and suspend
  ([`closures.md`](closures.md)).
- **Immutable by default:** `let` bindings, and shared data (`Rc`) is always immutable
  ([`memory.md`: Sharing: `.to_rc()`](memory.md#sharing-to_rc)).
- **Algebraic data types** (`enum` with payloads) and exhaustive `match`.
- **Iterators and combinators**, with one `map`/`filter`/`fold` for pure, fallible and suspending
  functions (effect polymorphism, [`error-handling.md`: Why `Result`-only languages duplicate higher-order APIs](error-handling.md#why-result-only-languages-duplicate-higher-order-apis)).
- **Purity is tracked by effects:** a function with an empty effect set is pure, and the compiler
  knows it. Few languages outside Haskell and Koka can say this.
- **Algebraic effect handlers** ([`determinism.md`: Handlers are scoped, not global](determinism.md#handlers-are-scoped-not-global)).
- **Persistent collections** in `moddil/collections`.
- **Functional updates in place:** compile-time reference-count elision with reuse (Perceus style)
  turns "build a new value" into an in-place update when the old one is no longer used.
- **Guaranteed tail calls** with `become f(x)` (Rust's accepted design; WASM tail calls are standard).
- **Laziness (decided):** evaluation is eager. Iterators are lazy, as in Rust, and a `Lazy<T>`
  library type computes a value once, on first use.
- **Higher-kinded types (decided): not now.** Generic associated types (GATs), as in Rust, cover
  the concrete needs (lending iterators, collection families). HKTs are revisited later; adding
  them is backwards compatible. Rationale: they clash with ownership and lifetimes, monadic
  `bind` breaks `?`/`return`/`break` inside closures, inference gets harder, and Moddil's effects
  already provide what monads give Haskell (errors, state, async).

### Object-oriented programming

| OOP idea | Moddil |
|---|---|
| Encapsulation | Privacy per module; values come into existence only through literals the module controls ([`memory.md`: Ownership and RAII](memory.md#ownership-and-raii)) |
| Methods | `impl` blocks with `&self`, `&mut self`, `self` |
| Interfaces | Traits, with default methods and supertraits |
| Polymorphism | Static (generics) and dynamic (`dyn Trait`) |
| Objects with identity and a mailbox | Actors: a task owning its state plus a channel, as a library pattern ([`concurrency.md`: Patterns the model makes easy](concurrency.md#patterns-the-model-makes-easy)) |
| Inheritance hierarchies | Not supported ([Multiple inheritance](#multiple-inheritance)) |

### Multiple inheritance

**No class inheritance.** Rust, Go and Zig made the same choice. Inheritance couples
implementations tightly, and multiple inheritance adds the diamond problem for state.

What replaces it:

1. **Traits with several supertraits and default methods.** This gives multiple inheritance of
   *behaviour*, with no shared state to conflict:
   ```rust
   trait Shape: Area + Perimeter + Display {
       fn describe(&self) -> String { fmt"{self}: area {self.area()}" }   // default method
   }
   ```
2. **Delegation** replaces implementation inheritance. An attribute forwards a trait's methods to a
   field (Kotlin's `by`):
   ```rust
   struct AuditedFile {
       #[delegate(Read, Write)]
       inner: File,
       log: AuditLog,
   }
   impl Write for AuditedFile {
       fn write(&mut self, buf: &[u8]) -> int with suspend, fail(IoError) {
           self.log.record(buf.len());
           self.inner.write(buf)      // override one method; the rest are delegated
       }
   }
   ```
3. **Default methods act as mixins:** a trait can supply reusable behaviour to every type that
   implements its required methods.

## Open questions

1. **Specialisation details:** strictly-more-specific only, or Rust's "always applicable" lattice
   rule? Can a specialising impl change an associated type (Rust forbids it)?
2. **Bridge mods:** the exact manifest syntax; whether a bridge may join more than two mods; how a
   program chooses between two competing bridges for the same pair (today: a build error, resolved
   by removing one).
3. **Same-repository ownership:** is "same repository" the right unit, or should it be a declared
   mod family?
4. **Const generic types** beyond integers, `bool` and `char`: are strings and structs worth the
   compile-time cost?
5. **Default type parameters** (`HashMap<K, V, S = RandomState>`): keep Rust's form, or drop it now
   that named arguments exist ([`syntax.md`](syntax.md))?
6. **Trait aliases** (`trait Num = Add + Sub + Mul + Copy`).
7. Revisit **higher-kinded types** once GAT-based libraries show what is missing ([Paradigms: functional and object-oriented](#paradigms-functional-and-object-oriented)).
8. **How far operator overloading goes:** may operator traits fail or suspend (`a + b` on big numbers
   that allocate)? Should comparison chaining (`a < b < c`) exist?
9. **Can `==` differ from `Eq`?** Rust's `PartialEq` allows it (floats); Moddil's equality rules are
   in [`values.md`](values.md). Is a separate overloadable `==` ever needed?
10. **Guidance against surprising overloads:** a lint or review rule (for people and agents) that
    operators keep their arithmetic meaning, as the `+` on strings is absent ([`text.md`](text.md)).
11. **Refinements: static or dynamic?** How much refinement checking is done at compile time, and what
    does it cost in build time ([`build-times.md`](build-times.md))?
12. **Niche packing:** how far should layout optimisation of nested options (`Option<Option<T>>`) go?

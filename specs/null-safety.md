# Null and Invalid States

> **Status:** decided direction, details open. Expands [`rationale.md`](rationale.md) §2.4.

## 1. Problem

Null references (Hoare's "billion-dollar mistake"). More generally: types that admit
values the domain does not, such as zero values nobody chose, unchecked ranges, mixed-up IDs, and
objects used in the wrong state.

## 2. State of the art

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

## 3. Decision

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
- **Absence is data; failure is an effect.** `fail(E)` (rationale §2.3) plays the role of Rust's `Result`,
  so `Option` is the only "maybe" type in everyday code.
  - `opt.or_fail(e)` turns absence into failure.
  - Catching a failure can produce an `Option`.
- **Guaranteed niches:** `Option<&T>`, `Option<Rc<T>>`, owned pointers, non-zero and range-typed
  integers, and enums with spare values are all the same size as `T`. Nullable C pointers import as
  `Option<…>` (rationale §2.13).
- **Fits the ownership model:**
  - `Option<&T>`, or an `Option` holding a view, is itself a view
    ([`memory-and-resources.md`](memory-and-resources.md) §4).
  - Moving out of an `Option` is an ordinary move (`take()`).
- **Definite initialisation:** no zero values, and no reads of uninitialised memory in safe code.
  Values are created only by literals that supply every field; there are no special constructors
  ([`memory-and-resources.md`](memory-and-resources.md) §2).
- **Invalid states beyond null:**
  - **Cheap newtypes:** zero-cost and distinct, with no implicit conversion (`UserId` ≠ `OrderId`).
  - **Range refinements** (`u32 where self in 1..`): values that are excluded become free niches for
    `Option`. This is the same `where` form as other refinements (decided).
  - **Refinement types** (`Int where 0 <= self < 100`): checked statically where provable,
    dynamically otherwise. They share machinery with contracts (rationale §2.7).
  - **Typestate through linear types** (rationale §2.2): states are types, and transitions consume the old
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

fn connect(host: &str, port: Option<Port>) with net, fail(NetError) {
    let Some(addr) = resolve(host) else { return };   // early exit on absence
    let Some(p) = port else { fail(NetError::NoPort) };
    …
}
```

## 4. Open questions

- ~~What does postfix `?` mean?~~ **Resolved:** it propagates, as in Rust (`None`, or a failure
  through the `fail` effect, [`error-handling.md`](error-handling.md)). Swift-style chaining within
  an expression is `try { a?.b?.c }`.
- How much refinement checking is static vs dynamic, and what does it cost in compile time (rationale §2.14)?
- Layout of nested options (`Option<Option<T>>`): how far should niche packing go?

## Prior art

Hoare, "Null References: The Billion Dollar Mistake" (QCon 2009);
Swift `Optional`; Kotlin null safety; TypeScript `strictNullChecks`; Dart sound null safety; C# 8
nullable reference types; Java Project Valhalla null-restricted types and JSpecify; Rust niche
optimisation and pattern types; F* and Liquid Haskell refinement types; typestate (Strom and
Yemini, 1986).

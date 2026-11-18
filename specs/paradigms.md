# Paradigms: Functional, Object-Oriented, and Inheritance

> **Status:** decided direction, details open. Syntax per [`syntax.md`](syntax.md).

## 1. Summary

| Question | Answer |
|---|---|
| Functional programming? | **Yes, strongly.** Closures, immutability by default, algebraic data types, pattern matching, iterators, and purity tracked by effects. |
| Object-oriented programming? | **Yes, the useful parts:** encapsulation, methods, interfaces (traits), static and dynamic polymorphism, actors. |
| Inheritance? | **No class inheritance**, single or multiple. Traits with supertraits and default methods, plus delegation, cover the use cases. |

## 2. Functional programming

- **First-class functions and closures**, including closures that fail and suspend
  ([`closures.md`](closures.md)).
- **Immutable by default:** `let` bindings, and shared data (`Rc`) is always immutable
  ([`memory-and-resources.md`](memory-and-resources.md) §5).
- **Algebraic data types** (`enum` with payloads) and exhaustive `match`.
- **Iterators and combinators**, with one `map`/`filter`/`fold` for pure, fallible and suspending
  functions (effect polymorphism, [`error-handling.md`](error-handling.md) §3.2).
- **Purity is tracked by effects:** a function with an empty effect set is pure, and the compiler
  knows it. Few languages outside Haskell and Koka can say this.
- **Algebraic effect handlers** ([`determinism.md`](determinism.md) §4).
- **Persistent collections** in `moddil/collections`.
- **Functional updates in place:** compile-time reference-count elision with reuse (Perceus style)
  turns "build a new value" into an in-place update when the old one is no longer used.
- **Guaranteed tail calls** with `become f(x)` (Rust's accepted design; WASM tail calls are standard).
- Not included: laziness by default, and higher-kinded types (as in Rust). *Open.*

## 3. Object-oriented programming

| OOP idea | Moddil |
|---|---|
| Encapsulation | Privacy per module; values come into existence only through literals the module controls ([`memory-and-resources.md`](memory-and-resources.md) §2) |
| Methods | `impl` blocks with `&self`, `&mut self`, `self` |
| Interfaces | Traits, with default methods and supertraits |
| Polymorphism | Static (generics) and dynamic (`dyn Trait`) |
| Objects with identity and a mailbox | Actors: a task owning its state plus a channel, as a library pattern ([`concurrency.md`](concurrency.md) §5) |
| Inheritance hierarchies | Not supported (§4) |

## 4. Multiple inheritance

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
       fn write(&mut self, buf: &[u8]) -> usize with fs, fail(IoError) {
           self.log.record(buf.len());
           self.inner.write(buf)      // override one method; the rest are delegated
       }
   }
   ```
3. **Default methods act as mixins:** a trait can supply reusable behaviour to every type that
   implements its required methods.

## 5. What Swift has that we lack

| Swift feature | Moddil |
|---|---|
| Argument labels and default parameter values | **Adopted:** named and default arguments ([`syntax.md`](syntax.md) §5) |
| Optional chaining `a?.b?.c` | **Adopted** ([`syntax.md`](syntax.md) §3) |
| Computed properties | *Open*; getter methods cover most uses |
| Property observers (`willSet`, `didSet`) | Not adopted: hidden control flow |
| Property wrappers, `@dynamicMemberLookup` | Not adopted: compile-time derives cover the useful cases, and dynamic lookup harms local reasoning |
| Result builders (the SwiftUI DSL style) | Covered by compile-time code plus closures; relevant to the UI question in [`examples/03-web-store.md`](examples/03-web-store.md) |
| Key paths (`\Person.name`) | *Open:* possible through static reflection |
| **Library evolution: a stable binary ABI** for frameworks | **A real gap.** Like Rust, Moddil has no stable ABI beyond C. *Open*, linked to prebuilt registry artefacts ([`build-times.md`](build-times.md) §4.2) |
| Classes with shared mutable references (ARC) | Deliberately not: `Rc` contents are immutable; shared mutation goes through sync cells |
| Objective-C interop; Playgrounds | Objective-C through C ([`interop.md`](interop.md)); REPL or notebook mode open ([`toolchain.md`](toolchain.md)) |
| Actors, `async`/`await`, typed throws, `some`/`any`, enums with payloads, `defer`, `Codable`, macros | Already covered: tasks and actors, effects, error handling, `impl`/`dyn`, ADTs, RAII and `errdefer`, derives, compile-time code |

## 6. Open questions

1. Higher-kinded types.
2. Laziness (opt-in lazy values or streams).
3. Computed properties.
4. Key paths.
5. Library evolution and a stable ABI.

## Prior art

Rust (traits, no inheritance); Go (embedding); Kotlin (`by` delegation); Scala and Swift (traits and
protocols with default implementations); Haskell and Koka (purity, effects); Lean 4 and Koka (Perceus
reuse); Swift (the features in §5).

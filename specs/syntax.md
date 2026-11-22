# Syntax

> **Status:** complete: decided, with no open questions. Indexed in [`README.md`](README.md).
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
| Effects are traits | `trait Time { fn now(&self) -> Instant; }`, used as `with time: Time`; data as `with config: &AppConfig` | No `effect` keyword: any trait listed after `with` is an effect, resolved by type in the dynamic scope ([`effects-and-capabilities.md`: Effects are traits passed implicitly](effects-and-capabilities.md#effects-are-traits-passed-implicitly)) |
| Effect variables | `fn map<T, U, E>(xs: &[T], f: impl FnMut(&T) -> U with E) -> Vec<U> with E` | Effect polymorphism ([`error-handling.md`: Why `Result`-only languages duplicate higher-order APIs](error-handling.md#why-result-only-languages-duplicate-higher-order-apis)); written only to name or constrain: an unannotated closure parameter is implicitly polymorphic ([`effects-and-capabilities.md`: Effect polymorphism](effects-and-capabilities.md#effect-polymorphism)) |
| Handler definitions | `impl Links for MemoryLinks { … }`; anonymous `impl Time { fn now(&self) -> Instant { t0 } }` | A handler is an ordinary impl; the anonymous impl expression captures like a closure. There is no `effect` keyword ([`effects-and-capabilities.md`: Defining handlers](effects-and-capabilities.md#defining-handlers)) |
| Handler blocks | `with Time = sim.clock(node), Net = sim.net.endpoint(node) { … }` | Installing effect handlers ([`determinism.md`: Handlers are scoped, not global](determinism.md#handlers-are-scoped-not-global)); the same keyword as in signatures |
| **Named and default arguments** (decided) | `urls.parallel_map(\|u\| fetch(u)?, limit: 32)`; `fn connect(host: &str, port: u16 = 443)` | Readability at call sites (Swift, Kotlin, Python). Positional arguments come first, then named ones. |
| Expression-level optional chaining | `try { users.get(id)?.profile?.email }` | `?` always propagates, as in Rust (inside a function returning `Option`, the chain works as is). A `try` block keeps the chain inside one expression, giving Swift-style chaining without a new operator ([`types.md`](types.md)). |
| Early exit from lambdas | `escape Some(p)` | Returns from the enclosing `fn` across non-escaping lambdas; in task scopes the first `escape` wins ([`closures.md`: Control flow out of a lambda](closures.md#control-flow-out-of-a-lambda)) |
| Multi-dimensional slicing | `a[.., 1..3]`, `a[i, ..]` | Ranges, not `:`, which named arguments use ([`accelerators.md`: Level 1: tensor expressions](accelerators.md#level-1-tensor-expressions)) |
| Power assert | `assert(fee(amount) + amount == 260)` | A built-in in the prelude, not a macro; when it fails, the trap shows every part's value, so there is no `assert_eq!` ([`testing.md`: Assertions](testing.md#assertions)) |
| Collection literals | `let v: Vec<int> = [1, 2, 3]` | No `vec!`: an array literal takes the type the context expects ([`stdlib.md`: Collections](stdlib.md#collections)) |
| Function wrappers | `#[around(trace::instrument)] fn transfer(…)` | Logging, permission checks and retries without body-rewriting macros ([`comptime.md`: Function wrappers](comptime.md#function-wrappers)) |
| Cannot-fail assertion | `Code::new(s) ?! "built from ALPHABET"` | Traps with the reason if the call fails anyway ([`error-handling.md`: This cannot fail here](error-handling.md#this-cannot-fail-here)) |
| Error context | `load(p)? fmt"loading config {p}"` | A note attached to the error, not a wrapper ([`error-handling.md`: Error types](error-handling.md#error-types)) |
| Wrapping operators | `h *% prime`, `a +% b`; `Wrapping<u32>` | Integer overflow traps by default; wrapping is explicit ([`numerics.md`: Overflow traps](numerics.md#overflow-traps)) |
| No `as` for numbers | `x.to::<u8>()?`, `x.truncate::<u8>()`, `f.round::<i64>(mode)` | Only lossless widening is implicit; every lossy conversion is named ([`numerics.md`: Conversions](numerics.md#conversions)) |
| Units of measure | `100[cm]`, `f64[m/s]` | Type-level units ([`units.md`](units.md)) |
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
| Binding modes | **One rule (decided):** matching through a borrow binds borrows (`&mut` in, `&mut` out); matching an owned value moves or copies; `&x` in a pattern only dereferences `Copy` values. There are no `ref` or `ref mut` keywords | No lifetimes involved: the bindings are second-class like any borrow ([`memory.md`: Borrowing](memory.md#borrowing)). `moddil migrate rust` rewrites `ref` patterns ([`adoption.md`](adoption.md#moddil-migrate-rust)) |
| Refinement patterns | `match s { p: Port => …, _ => … }` (decided) | Runs the refinement's pure predicate and binds `p` as the refined type; exhaustiveness treats it as a guard, so a fallback arm is needed |
| Destructuring by name | `let { host, port, .. } = opts` (decided) | Records and structs destructure by field name, matching named arguments |
| Evaluation order | Left to right, specified | [`determinism.md`: Sources of nondeterminism](determinism.md#sources-of-nondeterminism) |

## Named and default arguments

- Any parameter may be passed by name: `connect(host, port: 8443)`.
- Parameters may declare defaults: `fn connect(host: &str, port: u16 = 443, timeout: Duration = 10[s])`.
- Positional arguments come first, named arguments after, in any order.
- Defaults are part of the public interface, under semver and in semantic diffs
  ([`semantic-vcs.md`](semantic-vcs.md)).
- **Name-only parameters (decided, opt-in):** parameters after a `*` must be passed by name, as in
  Python. A lint suggests this for `bool` parameters, so `resize(w, h, true)` cannot happen:
  ```rust
  fn resize(w: u32, h: u32, *, keep_aspect: bool = false) { … }
  resize(800, 600, keep_aspect: true);
  ```
- **Defaults on trait methods (decided):** defaults are written only on the trait method;
  implementations inherit them and may not redeclare them (Kotlin's rule). A call through `dyn`, a
  generic bound or the concrete type therefore uses the same defaults. Changing a trait's default is
  a semver-visible change.

## Grammar rules

Decided while building the grammar inventory
([`reference/grammar/inventory.md`](../reference/grammar/inventory.md)) and writing the grammar
([`reference/grammar/moddil.ebnf`](../reference/grammar/README.md)):

| Rule | Decision |
|---|---|
| **Statements** | Rust's rule: a statement ends with `;`; a block's final expression without `;` is its value; items and block-ended expressions (`if`, `match`, `loop`, `with … { }`) need none |
| **Comptime conditionals** | `if const target.os == Os::Linux { … }` ([`modules.md`](modules.md#conditional-code-is-comptime-if)) |
| **Generated modules** | `#[c_import(…)] mod sqlite3;`: `mod name;` only with a generating attribute ([`modules.md`](modules.md#mods-and-modules)) |
| **Unit declarations** | Explicit operators: `unit km = 1000 * m`, `unit cm = m / 100`, `unit degC = K + 273.15` ([`units.md`](units.md#declaring-units)) |
| **Refinements** | A bare `where` refinement only in a `type` alias (`type Port = u16 where self >= 1024`); elsewhere it is parenthesised, `max_conns: (u32 where self >= 1) = 1000`. In a signature, `where` always means bounds |
| **Literal suffixes** | None: `0u8` is written `let d: u8 = 0` ([`numerics.md`](numerics.md#exact-literals)) |
| **Struct literals in heads** | As in Rust, a struct literal in the head of `if`, `while` or `match` must be parenthesised: `if x == (Foo { a: 1 }) { … }` |
| **Signature clause order** | `fn f<T>(a: &T, b: &T) -> &T from a with fail(E) where T: Ord requires … ensures … { … }`: provenance, effects, bounds, contracts; the formatter enforces it |
| **Effects inside types** | In type position one effect may be bare (`fn() -> T with E`, `impl Fn() with suspend`); several are parenthesised: `Box<dyn Fn(Event) with (suspend, fail(E))>`. Named effects (`time: Time`) appear only in signatures, whose `with` lists stay bare. A signature's return type takes no effects (`-> impl Handler with Links` gives `Links` to the function); a returned closure type with effects is parenthesised, `-> (impl Fn() with E)` |
| **Anonymous records** | No shorthand fields: `{ min: lo, max: hi }`. `{` followed by a name and `:` starts a record; anything else starts a block. Named struct literals keep shorthand (`Conn { socket, session }`) |
| **Heads before a block** | The struct-literal rule covers every head that a block follows: `if`, `while`, `match`, `for … in`, handler values in `with … = value { }`, and contract expressions before a body. A bare block or `const { }` block is not allowed there either; parenthesise it |
| **`if const`** | After `if`, the keyword `const` always starts a comptime conditional |
| **Refinement patterns** | `p: Port` is only allowed at the top of a `match` arm; in `let` and parameters, `:` is the type annotation |
| **Membership** | `x in a..b` takes its range at comparison precedence, so `self in 1..10 && self != 5` groups as written |
| **Lowest precedence** | `return`, `break`, `continue`, `escape`, `yield` and closures take the rest of the expression, so nothing continues after them: `return (a, b)` returns the tuple. `let` conditions are operands of `&&` chains in `if` and `while` heads |
| **Key paths** | `\T.a.b` takes every `.name` after it; to call a method on a key path, parenthesise it: `(\T.f).then(g)` |
| **Where clauses** | No trailing comma; a tuple struct's `where` clause comes after its fields, as in Rust |
| **Longest readings** | Four places take the longer reading, as Rust does: `else` belongs to the nearest `if`; `x.f(…)` is a method call (a field then a call is `(x.f)(…)`); `[…]` right after a number is a unit; `&mut p` in a pattern matches through a mutable borrow |
| **Items** | Items that do not end in a block end with `;`: `use`, `type`, `const`, tuple and unit structs, `mod`, `group`, `unit`, `dimension` |
| **Tagged-literal holes** | Inside a tagged literal, `{` opens a hole that runs to its matching `}`, so holes may contain strings (`fmt"Hi {name.or("guest")}"`); `{{` and `}}` are literal braces |
| **Keywords** | All reserved, none contextual. A keyword may still name a **member**: a field or method, where it is declared (`pub fn view(&self)`, `default: Port`) and where it is used after `.` or `::` (`Type::from(x)`, `cart.view()`), and a named-argument label (`default: 8080`) |

**Reserved keywords:** Rust's strict keywords except `async`, `await`, `move` and `macro_rules`, plus
`with`, `fail`, `suspend`, `spawn`, `nondet`, `try`, `catch`, `escape`, `gen`, `yield`, `resume`,
`defer`, `errdefer`, `select`, `linear`, `view`, `requires`, `ensures`, `always`, `invariant`, `from`,
`default`, `test`, `bench`, `property`, `note`, `group`, `unit` and `dimension`.

## What Swift has that we lack

| Swift feature | Moddil |
|---|---|
| Argument labels and default parameter values | **Adopted:** named and default arguments ([`syntax.md`: Named and default arguments](syntax.md#named-and-default-arguments)) |
| Optional chaining `a?.b?.c` | **Adopted** ([`syntax.md`: Deltas from Rust (and why)](syntax.md#deltas-from-rust-and-why)) |
| Computed properties | **Not adopted (decided):** methods only, so a call is visibly a call and cost and effects are never hidden behind a field-like read |
| Property observers (`willSet`, `didSet`) | Not adopted: hidden control flow |
| Property wrappers, `@dynamicMemberLookup` | Not adopted: compile-time derives cover the useful cases, and dynamic lookup harms local reasoning |
| Result builders (the SwiftUI DSL style) | Covered by compile-time code plus closures; relevant to the UI question in [`examples/web-store.md`](examples/web-store.md) |
| Key paths (`\Person.name`) | **Decided:** `\Order.customer.email` is a typed `KeyPath<Order, Email>`, generated by static reflection: composable (`a.then(b)`), `Eq` and `Hash`, and privacy-respecting; `WritableKeyPath` gives `&mut` access. It ships when UI binding or typed queries first need it; closures cover the rest |
| **Library evolution: a stable binary ABI** for frameworks | **Decided:** the C ABI plus **WASM components (WIT)** are Moddil's stable binary boundaries, for plugins and binary SDKs ([`interop.md`](interop.md)). A Swift-style resilience mode (fields and methods reached through indirection so binaries survive upgrades) is a **non-goal** ([`requirements.md`: Non-goals (draft)](requirements.md#non-goals-draft)). |
| Classes with shared mutable references (ARC) | Deliberately not: `Rc` contents are immutable; shared mutation goes through sync cells |
| Objective-C interop; Playgrounds | Objective-C through C ([`interop.md`](interop.md)); REPL, script and notebook modes are a non-goal ([`requirements.md`: Non-goals (draft)](requirements.md#non-goals-draft)); an in-browser playground replaces Playgrounds ([`toolchain.md`: Playground: learning without a REPL](toolchain.md#playground-learning-without-a-repl)) |
| Actors, `async`/`await`, typed throws, `some`/`any`, enums with payloads, `defer`, `Codable`, macros | Already covered: tasks and actors, effects, error handling, `impl`/`dyn`, ADTs, RAII and `errdefer`, derives, compile-time code |

## Open questions

- **Keywords as names of parameters and arguments.** `default` is a keyword, so a
  parameter cannot be called `default`, and a call cannot pass `default: 8080`
  ([`tour.md`: Gaps](tour.md#gaps-the-tour-exposed), 1). Whether a keyword may name a parameter,
  and so an argument, where nothing else can follow, is open.


## Prior art

Rust (the baseline); Swift (argument labels, default arguments, optional chaining); Kotlin and Python
(named and default arguments); OCaml, F# and Haskell (inferred generalisation); TypeScript (inferred
generic use); Koka and Effekt (effect rows and handlers).

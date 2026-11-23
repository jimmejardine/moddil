# Grammar inventory

Every syntactic construct the specs decide, with an example and the spec that decides it: the
checklist for `moddil.ebnf` ([`README.md`](README.md)). Built by sweeping every code block and syntax
table in [`specs/`](../../specs/README.md).

**Status values:** ✅ decided · ⚠️ conflict or undecided spelling. All eleven conflicts found are now
decided ([Conflicts, resolved](#conflicts-resolved)).

The base rule: **Rust's syntax unless a delta says otherwise**
([`specs/syntax.md`](../../specs/syntax.md)). Rows marked "as Rust" follow Rust's grammar.

## Lexical

| Construct | Example | Decided in | Status |
|---|---|---|---|
| Source encoding, identifiers | UTF-8; UAX #31, NFC-normalised; ASCII keywords; no bidi controls | [definition](../../specs/definition.md#lexical-structure) | ✅ |
| Line, block comments | `// …`, `/* … */` | as Rust | ✅ |
| Doc comments | `/// item`, `//! module overview` | [docs](../../specs/docs.md) | ✅ |
| Integer literals | `42`, `0x1F`, `0b1010`, `1_000_000` | as Rust | ✅ |
| Integer type suffixes | none: `let d: u8 = 0` | [syntax](../../specs/syntax.md#grammar-rules) | ✅ C6 |
| Float literals | `1.5`, `6.02e23` | as Rust | ✅ |
| Unit literals | `100[cm]`, `9.81[m/s^2]`, `101.2550[USD]`, `x * 1[s]` | [units](../../specs/units.md#why-square-brackets) | ✅ |
| Strings, raw strings, bytes, chars | `"…"`, `r#"…"#`, `b"…"`, `'c'`, `b'c'` | [text](../../specs/text.md#literals) | ✅ |
| Dedented blocks | `"""` … `"""`, raw `r"""` … `"""` | [text](../../specs/text.md#literals) | ✅ |
| Tagged literals | `fmt"…"`, `sql"…"`, `re"…"`, `html"…"`, `sh"…"`, `sql"""…"""` | [comptime](../../specs/comptime.md#tagged-string-literals-embedded-dsls) | ✅ |
| Interpolation holes | `fmt"{order.id} @ {price}"`, `{x:>8}`, `{x:?}`, `{length:[mm]}` | [text](../../specs/text.md#formatting) | ✅ |
| Labels | `'outer: loop { … break 'outer }` | as Rust | ✅ |
| Keywords | all reserved; keywords allowed as member names and argument labels | [syntax](../../specs/syntax.md#grammar-rules) | ✅ C9 |

## Items

| Construct | Example | Decided in | Status |
|---|---|---|---|
| Functions | `fn f(x: T) -> U { … }`, `pub fn`, `pub(mod) fn` | [modules](../../specs/modules.md#three-visibility-levels) | ✅ |
| Inferred private signatures | `fn add(a, b) { a + b }` | [syntax](../../specs/syntax.md#inference-generic-on-the-fly) | ✅ |
| Never-returning | `fn reset(board: Board) -> ! { … }` | as Rust | ✅ |
| `const fn`, `const` items | `const ALPHABET: &str = "…";` | [comptime](../../specs/comptime.md) | ✅ |
| `unsafe fn` | `pub unsafe fn copy_nonoverlapping<T>(…)` | [unsafe](../../specs/unsafe.md) | ✅ |
| Generators | `gen fn read_records(file: &mut File) -> Record { … }` | [stdlib](../../specs/stdlib.md#generators) | ✅ |
| Coroutines | `gen fn guard_ai(start: Pos) -> Action resume first: Frame { … }` | [stdlib](../../specs/stdlib.md#coroutines) | ✅ |
| Structs (named, tuple, unit) | `struct Created { code: Code }`, `struct OrderId(u64)` | as Rust | ✅ |
| Field defaults | `port: Port = 8080` | [lifecycle](../../specs/lifecycle.md) | ✅ |
| Inline refinement | `max_conns: (u32 where self >= 1) = 1000` (parenthesised) | [syntax](../../specs/syntax.md#grammar-rules) | ✅ C5 |
| Struct invariants | `invariant bids.is_empty() \|\| …` inside a struct body | [correctness](../../specs/correctness.md#contracts-as-syntax) | ✅ |
| Linear and view types | `linear struct Txn { … }`, `view RingIter<T> { buf: &[T], idx: int }` | [resources](../../specs/resources.md), [memory](../../specs/memory.md#view-types) | ✅ |
| Enums | `pub enum LinkError { Taken(Code) }` | as Rust | ✅ |
| Traits | `trait Shape: Area + Display { fn describe(&self) -> String { … } }`, `type Item;` | [types](../../specs/types.md) | ✅ |
| Impls | `impl Links for MemoryLinks { … }`, `impl<T: Db> DbInput for T {}` | as Rust | ✅ |
| Specialisation | `default fn extend_from_slice(…)`, `default type Repr = Vec<T>;` | [types](../../specs/types.md) | ✅ |
| Type aliases, refinements | `type Port = u16 where self >= 1024`, `where self in 1..` | [correctness](../../specs/correctness.md#validated-newtypes-lightweight-refinements) | ✅ |
| Imports | `use a::b::{c, d};`, `pub use x as Y;`, `use …::prelude::*;`, `{self, …}` | [modules](../../specs/modules.md#imports) | ✅ |
| Generated modules | `#[c_import(header = "…")] mod sqlite3;` (only with a generating attribute) | [modules](../../specs/modules.md#mods-and-modules) | ✅ C2 |
| C functions | `extern "C" { pub fn strlen(s: *const u8) -> u64; }` (hand-written bindings; header import later) | [interop](../../specs/interop.md#as-built-m3a) | ✅ M3a |
| Units and dimensions | `dimension Length`, `unit m: Length`, `unit km = 1000 * m`, `unit degC = K + 273.15` | [units](../../specs/units.md#declaring-units) | ✅ C4 |
| Attributes | `#[derive(…)]`, `#[repr(C)]`, `#[kernel]`, `#[interrupt(TIMER0, priority = 2)]`, `#[no_alloc, no_lock]`, `#[budget(…)]` | [syntax](../../specs/syntax.md#mapping) | ✅ |
| Tests | `test "name" for subject { … }`, `for <Money as Display>`, `for sort::<T> with T in [int, f64]` | [testing](../../specs/testing.md#attached-tests) | ✅ |
| Properties, benchmarks | `property "…" for withdraw (acct: Account, amount: Money) { … }`, `bench "…" for f { … }` | [correctness](../../specs/correctness.md), [testing](../../specs/testing.md#benchmarks) | ✅ |
| Notes, groups | `note "…"`, `group levels "Price levels"` | [docs](../../specs/docs.md) | ✅ |

## Signatures

| Construct | Example | Decided in | Status |
|---|---|---|---|
| Generics, const generics | `<T: Ord>`, `<const N: int>`, `where T: Clone` | [types](../../specs/types.md#generics) | ✅ |
| Parameters | `x: &T`, `x: &mut T`, `x: T`, `&self`, `&mut self`, `self` | [syntax](../../specs/syntax.md#mapping) | ✅ |
| Defaults, name-only | `port: u16 = 443`; `fn resize(w: u32, *, keep_aspect: bool = false)` | [syntax](../../specs/syntax.md#named-and-default-arguments) | ✅ |
| Borrow provenance | `fn first_word(text: &str, sep: &str) -> &str from text` | [memory](../../specs/memory.md) | ✅ |
| Effects | `with net: Net, Rand, suspend, spawn, fail(A \| B)`, `with config: &AppConfig`, `with E`, `with nondet` | [effects](../../specs/effects-and-capabilities.md) | ✅ |
| Generic effects | `with Store<User>` | [effects](../../specs/effects-and-capabilities.md#which-traits-are-effects) | ✅ |
| Contracts | `requires amount > 0`, `requires always …`, `requires valid: …`, `ensures x == old(x) - a` | [correctness](../../specs/correctness.md#contracts-as-syntax), [unsafe](../../specs/unsafe.md#safety-contracts) | ✅ |
| Clause order | `-> T from … with … where … requires … ensures …` | [syntax](../../specs/syntax.md#grammar-rules) | ✅ C8 |

## Statements and expressions

| Construct | Example | Decided in | Status |
|---|---|---|---|
| Statement terminators | Rust's rule: `let x = 1;`; a final expression is the block's value | [syntax](../../specs/syntax.md#grammar-rules) | ✅ C3 |
| `let`, `let … else` | `let Some(p) = port else { fail(NoPort) };` | as Rust | ✅ |
| Operators | Rust's set; no `as`; `+% -% *%`; `@` (matrix multiply) | [numerics](../../specs/numerics.md), [syntax](../../specs/syntax.md#deltas-from-rust-and-why) | ✅ |
| Chained comparisons | `0 <= i < len`, `lo < x <= hi` (one direction; `==` doesn't chain) | [types](../../specs/types.md#operators) | ✅ |
| Range membership | `self in 1..` | [syntax](../../specs/syntax.md#deltas-from-rust-and-why) | ✅ |
| Calls, named arguments | `f(a, limit: 32)`, `Request::post("/x", body: …)` | [syntax](../../specs/syntax.md#named-and-default-arguments) | ✅ |
| Turbofish | `x.to::<u8>()`, `matmul::<128, 128, 32>` | as Rust | ✅ |
| Multi-dimensional index | `a[.., 1..3]`, `a[i, ..]` | [accelerators](../../specs/accelerators.md#level-1-tensor-expressions) | ✅ |
| Struct and record literals | `Conn { socket, session }`, `{ min: xs.min(), max: xs.max() }`; parenthesised in `if`/`while`/`match` heads | [syntax](../../specs/syntax.md#grammar-rules) | ✅ C7 |
| Closures | `\|x\| expr`, `\|p, m\| { … }`, `\|\| f()` (no `move`) | [closures](../../specs/closures.md) | ✅ |
| `if`, `match`, loops | `if let … && …`, `while let`, `for &x in xs`, `loop`, `break value` | as Rust | ✅ |
| Comptime conditional | `if const target.os == Os::Linux { … }` | [modules](../../specs/modules.md#conditional-code-is-comptime-if) | ✅ C1 |
| `const` blocks | `const { crc_table(0xEDB88320) }` | [comptime](../../specs/comptime.md) | ✅ |
| Propagation | `x?`, `users.get(id)?.profile?.email` | [error-handling](../../specs/error-handling.md) | ✅ |
| Cannot-fail | `Code::new(s) ?! "built from ALPHABET"` | [error-handling](../../specs/error-handling.md#this-cannot-fail-here) | ✅ |
| Error context | `load(p)? fmt"loading config {p}"` | [error-handling](../../specs/error-handling.md#error-types) | ✅ |
| `try` / `catch` | `try { … } catch { Io(NotFound) => …, e => … }`; `try { a?.b }` alone | [error-handling](../../specs/error-handling.md) | ✅ |
| `fail` | `fail(LinkError::Taken(code))` | [error-handling](../../specs/error-handling.md) | ✅ |
| `escape` | `escape Some(path.clone())` | [closures](../../specs/closures.md#control-flow-out-of-a-lambda) | ✅ |
| Handler blocks | `with Time = clock, Rand = r { … }`, `with Store<User> = users { … }`, as an expression | [effects](../../specs/effects-and-capabilities.md) | ✅ |
| Anonymous impls | `impl Time { fn now(&self) -> Instant { t0 } }` as an expression | [effects](../../specs/effects-and-capabilities.md#defining-handlers) | ✅ |
| `select` | `select { conn = listener.accept() => …, _ = stop_requested() => break, }` | [concurrency](../../specs/concurrency.md) | ✅ |
| `scope` and `spawn` | `scope(\|s\| { s.spawn(\|\| …); }, isolate: true)` (library calls) | [concurrency](../../specs/concurrency.md) | ✅ |
| `gen` blocks, `yield` | `gen { for x in xs { yield f(x) } }`, `let f = yield Action::Shout` | [stdlib](../../specs/stdlib.md#generators) | ✅ |
| `defer`, `errdefer` | `defer { metrics.dec() }`, `errdefer out.abort();` | [resources](../../specs/resources.md) | ✅ |
| `unsafe` blocks | `unsafe("valid: reserve guarantees capacity") { … }` | [unsafe](../../specs/unsafe.md#safety-contracts) | ✅ |
| Raw pointers | `*const T`, `*mut T`, `&mut *p` | [unsafe](../../specs/unsafe.md) | ✅ |
| Key paths | `\Order.customer.email` | [syntax](../../specs/syntax.md#what-swift-has-that-we-lack) | ✅ |
| Declassification | `x.declassify("reason")` (a method call; no new syntax) | [security](../../specs/security.md) | ✅ |

## Patterns

| Construct | Example | Decided in | Status |
|---|---|---|---|
| Rust patterns | literals, `_`, tuples, `Some(x)`, `A \| B`, `1..=9`, `n @ 1..=9`, `[first, .., last]`, guards | as Rust | ✅ |
| Error-union arms | `Io(NotFound)`, `DecodeError(e)`, `LinkError::Taken(c)`, `Trapped(t)` | [error-handling](../../specs/error-handling.md#matching-removes-what-it-handles) | ✅ |
| Refinement patterns | `p: Port => …` | [syntax](../../specs/syntax.md#patterns-and-control-flow) | ✅ |
| Destructuring by name | `let { host, port, .. } = opts` | [syntax](../../specs/syntax.md#patterns-and-control-flow) | ✅ |
| Binding modes | no `ref` / `ref mut` | [syntax](../../specs/syntax.md#patterns-and-control-flow) | ✅ |

## Types

| Construct | Example | Decided in | Status |
|---|---|---|---|
| Paths and generics | `Map<Code, Target>`, `Option<&V>`, `Self::Item` | as Rust | ✅ |
| Borrows, arrays, slices, tuples | `&T`, `&mut T`, `[u8; 32]`, `[f64]`, `(A, B)` | as Rust | ✅ |
| Units on types | `f64[m]`, `i64[ns]`, `Fixed<4>[USD]`, `f64[U^2]`, `Vec<f64[m]>` | [units](../../specs/units.md#why-square-brackets) | ✅ |
| `impl` and `dyn` | `impl Fn(&T) + NoAlloc`, `Box<dyn Fn(Event) with (suspend, fail(E))>`, `&dyn Net` | [types](../../specs/types.md), [closures](../../specs/closures.md) | ✅ |
| Function types | `fn(A) -> B with E` | [types](../../specs/types.md) | ✅ |
| Anonymous records | `-> { min: f64, max: f64 }` | [syntax](../../specs/syntax.md#inference-generic-on-the-fly) | ✅ |
| Never | `!` | as Rust | ✅ |

## Conflicts, resolved

Found by the inventory and decided in [`specs/syntax.md`: Grammar rules](../../specs/syntax.md#grammar-rules):

| # | Conflict | Decision |
|---|---|---|
| C1 | `if comptime` against Rust's `const` spelling | `if const cond { … }` |
| C2 | `mod sqlite3;` against "no `mod` declarations" | `mod name;` only with a generating attribute |
| C3 | Statement terminators | Rust's `;` rule; spec snippets normalised |
| C4 | Juxtaposed unit declarations | Explicit `*` and `/`: `unit km = 1000 * m` |
| C5 | Inline refinements against `where` bounds | Bare only in `type` aliases; parenthesised elsewhere |
| C6 | Integer suffixes | Dropped; annotate instead |
| C7 | Struct literals in condition heads | Parenthesised, as in Rust |
| C8 | Signature clause order | `from`, `with`, `where`, then `requires`/`ensures` |
| C9 | Reserved or contextual keywords | All reserved; keywords may name members and label arguments |
| C10 | Effect lists inside types, where commas already separate things | One bare effect, or several in parentheses: `with (suspend, fail(E))` |
| C11 | Anonymous record shorthand `{ min, max }` vs a block | No shorthand: `{ min: lo, max: hi }` |

Fixed directly, as slips against decided rules: `assert!(…)` in types.md (there are no macros) and
`and` in an unsafe.md contract (Rust's `&&`).

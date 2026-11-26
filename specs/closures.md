# Closures and Lambdas

Follows from [`memory.md`](memory.md), [`concurrency.md`](concurrency.md) and
[`error-handling.md`](error-handling.md).

## Summary

A lambda passed to `visit`, `map`, `for_each`, `sort_by`, `with` and similar APIs:
- can **borrow anything**, including mutable locals and fields;
- can **fail** with `?`, and the error propagates to the caller;
- can **suspend** (do I/O, wait on other tasks);
- needs **no** lifetimes, `move`, `Box`, `Pin`, `Send` bounds, or separate async versions of APIs.

```rust
fn sync_all(index: &Map<Path, Meta>, cache: &mut Cache) -> u64 with fs: Fs, store: Store {
    let mut uploaded = 0;
    index.visit(|path, meta| {
        if meta.dirty {
            let bytes = fs.read(path)?;         // may suspend, may fail
            store.put(path, bytes)?;            // may suspend, may fail
            cache.mark_clean(path);             // mutates a captured borrow
            uploaded += 1;                      // mutates a local
        }
    })?;                                        // the closure can fail, so the call can
    uploaded
}   // inferred in full: with Fs, Store, suspend, fail(Io | Store)
```

## Rust pain points and Moddil's answers

| Rust pain | Cause | Moddil |
|---|---|---|
| `\|x\| async move { … }` cannot borrow its argument or its captures; higher-ranked lifetimes on futures; async closures only stable since 1.85; still no way to say "the returned future is `Send`" | Async means returning a future value, so every async closure returns a state machine with lifetimes | Suspension is an **effect** ([Effects and capabilities as a unifying idea](effects-and-capabilities.md), [`resources.md`: Construction may suspend; drop may not](resources.md#construction-may-suspend-drop-may-not)). A suspending lambda is an ordinary lambda with `suspend` in its effects. There is no future type, so no extra lifetimes. |
| Separate APIs: `for_each`, `try_for_each`, `StreamExt::for_each`, `for_each_concurrent` | APIs are coloured by `Result` and `async` | **One `visit`**, generic over the closure's effects ([`error-handling.md`: Why `Result`-only languages duplicate higher-order APIs](error-handling.md#why-result-only-languages-duplicate-higher-order-apis)) |
| `?` inside a closure exits the closure, not the enclosing function | `Result` is a value | `fail` is an effect, so `?` propagates through `visit` to the caller |
| `'static`, `move` and `Arc::clone` for spawned closures | Tasks can outlive their spawner | Structured concurrency: child closures are non-escaping and borrow freely ([`concurrency.md`: Tasks and borrowing](concurrency.md#tasks-and-borrowing)) |
| `Send` errors across `.await` | `Rc` and `RefCell` are not thread-safe | Every type is sendable ([`concurrency.md`: No `Send` / `Sync`](concurrency.md#no-send--sync)) |
| A closure that touches `self` conflicts with iterating `self.items` | Captures and method calls borrow the whole of `self` | Closures capture individual field paths; methods can declare the fields they touch ([Capturing parts of `self`](#capturing-parts-of-self)) |
| `Fn`/`FnMut`/`FnOnce` bounds tangled with `'static`, `Send` and lifetimes | Closure traits carry lifetime and thread-safety obligations | The same three traits, with the same meaning, but no lifetimes, `Send` or `'static` ([Closure parameters: `Fn`, `FnMut`, `FnOnce`](#closure-parameters-fn-fnmut-fnonce)) |
| `Box<dyn Fn …>`; `impl Trait` not allowed in struct fields | Storing closures needs type erasure | Storing a closure is explicit (`Box<dyn Fn …>`), and its type names its effects ([Stored (escaping) closures](#stored-escaping-closures)) |
| Every async closure becomes its own state machine; future sizes bloat | Stackless async | With stack switching, a suspended lambda's frame sits on the task's stack ([`compiler.md`: WASM concurrency lowering](compiler.md#wasm-concurrency-lowering)) |

## Closure parameters: `Fn`, `FnMut`, `FnOnce`

Moddil keeps Rust's three closure traits, with their usual meaning. What disappears is everything
that made them painful: lifetimes, `Send`, `'static` and `move`.

| Parameter type | The callee may | The closure may |
|---|---|---|
| `impl Fn(…)` | call it any number of times, possibly concurrently | read its captures |
| `impl FnMut(…)` | call it any number of times, one call at a time | read and mutate its captures |
| `impl FnOnce(…)` | call it at most once | consume (move out of) its captures |

`pub` signatures state the trait, and `impl FnMut` is the documented default for callbacks such
as `visit`, `map` and `for_each`; private functions may leave it to inference, which picks the most
permissive trait the body allows. Closures passed as `impl Fn…` parameters are **non-escaping**
([Non-escaping by default](#non-escaping-by-default)). Captures are inferred, so `move` is never
needed.

## Non-escaping by default

- A closure passed as an `impl Fn…` parameter is **non-escaping** (Swift's model): it is
  second-class, like a borrow, and cannot be stored, returned or sent to detached work. In exchange
  it may capture shared and mutable borrows and views
  ([`memory.md`: Rules](memory.md#rules), [`memory.md`: View types](memory.md#view-types)).
- To keep a closure, a function takes `Box<dyn Fn…>` instead ([Stored (escaping) closures](#stored-escaping-closures)). Its captures must then be owned.
- **Zero cost:** no allocation. In release builds the closure is inlined or specialised; in debug
  builds it is passed with a witness table ([`build-times.md`: Language rules that decide build speed](build-times.md#language-rules-that-decide-build-speed)).

## Effects flow through

```rust
fn visit<E>(&self, f: impl FnMut(&K, &V) with E) with E
```

`visit` performs whatever effects `f` performs:
- if `f` is pure, `visit` is pure;
- if `f` can fail, `visit` fails with the same errors;
- if `f` suspends, `visit` suspends.

One implementation serves all cases; there is no `try_visit` or `visit_async`.

- **A call that passes a closure that can fail is marked:**
  `index.visit(|p, m| upload(p)?)?`, like any call that can fail, so every way an error leaves a
  function stays visible ([`error-handling.md`](error-handling.md)). Inside `visit`, a call of
  `f` passes `f`'s errors on without a mark: they are not `visit`'s to handle, and its caller's
  call is marked.
- **A closure's other effects are met where it is written:** a `with` around the closure,
  or the enclosing function's own effects, provides them, not the function that calls it, as
  Effekt's capabilities are lexical ([`effects-and-capabilities.md`](effects-and-capabilities.md)).
  A callee that installs a handler of its own around a call of `f` does not change what `f` uses.

### Concurrency inside a visit

A full worked example, `parallel_map`, is in [`concurrency.md`: Data parallelism: `parallel_map`](concurrency.md#data-parallelism-parallel_map).

```rust
index.visit_concurrent(|path, meta| upload(path)?, limit: 8);
```

- The invocations are children in a structured scope, so they may borrow from the enclosing frame.
- Shared captures are fine. A captured `&mut` borrow cannot be used by concurrent invocations, so
  `visit_concurrent` takes `impl Fn`. The compiler error suggests a sync cell or a channel. This is
  a genuine conflict, not lifetime noise.

## Capturing parts of `self`

```rust
fn refresh(&mut self) {
    self.items.visit(|item| self.total += item.price);   // OK: `items` read, `total` written
}
```

- A closure captures **field paths** (`self.items`, `self.total`), not the whole of `self`, so
  disjoint fields do not conflict.
- A method called from inside the closure normally borrows the whole of `self`. Methods can declare
  the fields they touch, so that `self.add_to_total(x)` can also be called while `self.items` is
  being visited (view types, [`memory.md`: View types](memory.md#view-types)).
- A private method's field footprint is inferred from its body within the mod. A `pub`
  method may declare one, `fn add_to_total(&mut self.{total}, x: Money)`; the declaration is part
  of its API and checked against its body.

## Control flow out of a lambda

- `?` propagates, via effects.
- `return` inside a lambda returns from the lambda, and `break` never crosses a lambda.
- **`escape v` returns `v` from the enclosing function**, crossing any number of
  non-escaping lambdas. There are no labels: its destination is always the enclosing `fn`, and
  closures are not `fn`s. To stop only part of a function early, put that part in its own function.

  ```rust
  fn find_owner(index: &Map<Path, Meta>, user: &User) -> Option<Path> {
      index.parallel_visit(|path, meta| if meta.owner == *user { escape Some(path.clone()) });
      None
  }
  ```

- It is an inferred **escape effect** that `visit` and other higher-order functions pass through,
  like any other effect of their callback. Drops and `errdefer` run on the way out.
- **Inside task scopes, the first `escape` wins.** Its value becomes the result; the other tasks
  are cancelled, as a failing task cancels its siblings
  ([`error-handling.md`: Tasks and concurrency](error-handling.md#tasks-and-concurrency)). It is not
  an error, so nothing is reported or suppressed. This makes a parallel search an ordinary
  function.
- `escape` is not allowed in escaping (stored) closures, which may run after the function returns.

## Stored (escaping) closures

Stored callbacks and event handlers are boxed, owned closures:

```rust
type Handler = Box<dyn Fn(Event) with (suspend, fail(HandlerError))>;

fn on_message(&mut self, h: Handler) { self.handlers.push(h) }
```

- Owned stored closures capture by **move or rc**, never borrows, while one that holds a handler
  is a view (below).
- Their **stored type names their effects**. Effect polymorphism does not leak into stored values
  except through explicit effect variables.
- They are the only place where effects are usually written out by hand.
- **A closure's handler needs are always met where it is written.**
  - A closure that uses a handler captures a shared borrow of it, so it is a
    [view](memory.md#view-types), and so is a `Box<dyn Fn …>` holding it. That borrow is tracked
    like any other a value holds.
    - **A `Box<dyn Fn …>` carries its closure's borrows**: erasing the
      closure's type does not erase them, so the box is a view of each handler it holds and
      cannot outlive or leave the `with` that installed it.
  - An owned stored closure, one kept past its `with`, cannot use handlers. Its type lists only
    `suspend`, `spawn` and `fail`.
  - To keep such a closure longer, move the `with` further out, or pass the dependency as an
    argument.
  - Why: see [`effects-and-capabilities.md`: Effects inside handlers](effects-and-capabilities.md#effects-inside-handlers).
    There is one model for closures, anonymous impls and handlers that depend on other handlers,
    and library code that stores or calls a value never provides effects it was not written for.
- **A function that keeps a closure names its errors.** `Iterator::map`
  keeps `f` in the `Map` it returns and calls it in `next`, so `f`'s errors are `Map`'s, not
  `map`'s: its bound names them with an effect variable it does not list,
  `F: FnMut(Self::Item) -> B with E`, which says they do not leave it.

## Seeing suspension points

There is no `await` keyword; suspension is implicit ([`resources.md`: Construction may suspend; drop may not](resources.md#construction-may-suspend-drop-may-not)).
Reviewers may still want to see where a lambda can suspend, because state in sync cells can change
across a suspension and a lock might be held across it.

**There is no marker in the source.** The language server shows suspension points as inlay hints,
semantic diffs report new ones ([Diffs, code review and version control](semantic-vcs.md)), and a
lint flags a lock held across a suspension.

## Syntax

- Closures use Rust syntax: `|a, b| expr` or `|a, b| { … }`. There is no `move`; captures are
  inferred.
- There are no trailing lambdas. A closure is an ordinary argument and comes before any named
  arguments: `xs.visit_concurrent(|x| …, limit: 8)` ([`syntax.md`](syntax.md)).

## Alternatives considered

- **Meeting a stored closure's handler needs where it is called:** library code that stores or
  calls a value would then provide effects it was not written for, and closures, anonymous impls
  and handlers would not share one model.
- **A suspension marker in the source (an `await`):** inlay hints, semantic diffs and a lint show
  suspension points without one.
- **Labels on `escape`:** its destination is always the enclosing `fn`; a part of a function that
  must stop early goes in its own function.
- **Trailing lambdas:** a closure is an ordinary argument, placed before any named arguments.

## Prior art

- Swift: non-escaping closures by default, `@escaping`, `async` closures, `await` markers.
- Kotlin: inline lambdas with non-local `return`.
- Rust: closure traits, disjoint closure captures (2021 edition), async closures (1.85), labelled
  block `break`, the keyword-generics initiative.
- Koka and Effekt: effect-polymorphic higher-order functions.
- Niko Matsakis's "view types" proposals for partial borrows of `self`.

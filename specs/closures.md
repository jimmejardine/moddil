# Closures and Lambdas

> **Status:** mostly follows from decided direction in [`memory-and-resources.md`](memory-and-resources.md),
> [`concurrency.md`](concurrency.md) and [`error-handling.md`](error-handling.md). Open points are
> marked. Syntax follows [`syntax.md`](syntax.md) (Rust-based).

## 1. Summary

A lambda passed to `visit`, `map`, `for_each`, `sort_by`, `with` and similar APIs:
- can **borrow anything**, including mutable locals and fields;
- can **fail** with `?`, and the error propagates to the caller;
- can **suspend** (do I/O, wait on other tasks);
- needs **no** lifetimes, `move`, `Box`, `Pin`, `Send` bounds, or separate async versions of APIs.

```rust
fn sync_all(index: &Map<Path, Meta>, cache: &mut Cache) -> u64 {
    let mut uploaded = 0;
    index.visit(|path, meta| {
        if meta.dirty {
            let bytes = fs.read(path)?;         // may suspend, may fail
            store.put(path, bytes)?;            // may suspend, may fail
            cache.mark_clean(path);             // mutates a captured borrow
            uploaded += 1;                      // mutates a local
        }
    });
    uploaded
}   // inferred: with suspend, fail(Io | Store), fs, net
```

## 2. Rust pain points and Moddil's answers

| Rust pain | Cause | Moddil |
|---|---|---|
| `\|x\| async move { … }` cannot borrow its argument or its captures; higher-ranked lifetimes on futures; async closures only stable since 1.85; still no way to say "the returned future is `Send`" | Async means returning a future value, so every async closure returns a state machine with lifetimes | Suspension is an **effect** (overview §2.1, [`concurrency.md`](concurrency.md) §8). A suspending lambda is an ordinary lambda with `suspend` in its effects. There is no future type, so no extra lifetimes. |
| Separate APIs: `for_each`, `try_for_each`, `StreamExt::for_each`, `for_each_concurrent` | APIs are coloured by `Result` and `async` | **One `visit`**, generic over the closure's effects ([`error-handling.md`](error-handling.md) §3.2) |
| `?` inside a closure exits the closure, not the enclosing function | `Result` is a value | `fail` is an effect, so `?` propagates through `visit` to the caller |
| `'static`, `move` and `Arc::clone` for spawned closures | Tasks can outlive their spawner | Structured concurrency: child closures are non-escaping and borrow freely ([`concurrency.md`](concurrency.md) §3) |
| `Send` errors across `.await` | `Rc` and `RefCell` are not thread-safe | Every type is sendable ([`concurrency.md`](concurrency.md) §2) |
| A closure that touches `self` conflicts with iterating `self.items` | Captures and method calls borrow the whole of `self` | Closures capture individual field paths; methods can declare the fields they touch (§6) |
| `Fn`/`FnMut`/`FnOnce` bounds tangled with `'static`, `Send` and lifetimes | Closure traits carry lifetime and thread-safety obligations | The same three traits, with the same meaning, but no lifetimes, `Send` or `'static` (§3) |
| `Box<dyn Fn …>`; `impl Trait` not allowed in struct fields | Storing closures needs type erasure | Storing a closure is explicit (`Box<dyn Fn …>`), and its type names its effects (§8) |
| Every async closure becomes its own state machine; future sizes bloat | Stackless async | With stack switching, a suspended lambda's frame sits on the task's stack ([`bootstrap.md`](bootstrap.md) §6) |

## 3. Closure parameters: `Fn`, `FnMut`, `FnOnce`

Moddil keeps Rust's three closure traits, with their usual meaning. What disappears is everything
that made them painful: lifetimes, `Send`, `'static` and `move`.

| Parameter type | The callee may | The closure may |
|---|---|---|
| `impl Fn(…)` | call it any number of times, possibly concurrently | read its captures |
| `impl FnMut(…)` | call it any number of times, one call at a time | read and mutate its captures |
| `impl FnOnce(…)` | call it at most once | consume (move out of) its captures |

**Recommended default** for callbacks such as `visit`, `map` and `for_each`: `impl FnMut`. Closures
passed as `impl Fn…` parameters are **non-escaping** (§4). Captures are inferred, so `move` is never
needed.

## 4. Non-escaping by default

- A closure passed as an `impl Fn…` parameter is **non-escaping** (Swift's model): it is
  second-class, like a borrow, and cannot be stored, returned or sent to detached work. In exchange
  it may capture shared and mutable borrows and views
  ([`memory-and-resources.md`](memory-and-resources.md) §3.2, §4).
- To keep a closure, a function takes `Box<dyn Fn…>` instead (§8). Its captures must then be owned.
- **Zero cost:** no allocation. In release builds the closure is inlined or specialised; in debug
  builds it is passed with a witness table ([`build-times.md`](build-times.md) §3).

## 5. Effects flow through

```rust
fn visit<E>(&self, f: impl FnMut(&K, &V) with E) with E
```

`visit` performs whatever effects `f` performs:
- if `f` is pure, `visit` is pure;
- if `f` can fail, `visit` fails with the same errors;
- if `f` suspends, `visit` suspends.

One implementation serves all cases; there is no `try_visit` or `visit_async`.

### Concurrency inside a visit

A full worked example, `parallel_map`, is in [`concurrency.md`](concurrency.md) §5.1.

```rust
index.visit_concurrent(|path, meta| upload(path)?, limit: 8);
```

- The invocations are children in a structured scope, so they may borrow from the enclosing frame.
- Shared captures are fine. A captured `&mut` borrow cannot be used by concurrent invocations, so
  `visit_concurrent` takes `impl Fn`. The compiler error suggests a sync cell or a channel. This is
  a genuine conflict, not lifetime noise.

## 6. Capturing parts of `self`

```rust
fn refresh(&mut self) {
    self.items.visit(|item| self.total += item.price);   // OK: `items` read, `total` written
}
```

- A closure captures **field paths** (`self.items`, `self.total`), not the whole of `self`, so
  disjoint fields do not conflict.
- A method called from inside the closure normally borrows the whole of `self`. Methods can declare
  the fields they touch, so that `self.add_to_total(x)` can also be called while `self.items` is
  being visited (view types, [`memory-and-resources.md`](memory-and-resources.md) §4).
- *Open:* the exact syntax and rules for methods that declare their fields.

## 7. Control flow out of a lambda

- `?` propagates (decided, via effects).
- *Open:* should a labelled `break` inside a non-escaping lambda leave the **enclosing** block, as
  Kotlin's inline lambdas allow with non-local `return`? In Rust syntax this is a labelled block:

  ```rust
  fn find_owner(index: &Map<Path, Meta>, user: &User) -> Option<Path> {
      'found: {
          index.visit(|path, meta| {
              if meta.owner == *user { break 'found Some(path.clone()) }
          });
          None
      }
  }
  ```

  It is safe because the lambda cannot escape, and it can be implemented as an escape effect that
  `visit` passes through. **Recommended:** allow it, always with an explicit label so it is never
  ambiguous.

## 8. Stored (escaping) closures

Stored callbacks and event handlers are boxed, owned closures:

```rust
type Handler = Box<dyn Fn(Event) with suspend, fail(HandlerError)>;

fn on_message(&mut self, h: Handler) { self.handlers.push(h) }
```

- They capture by **move or rc**, never borrows.
- Their **stored type names their effects**. Effect polymorphism does not leak into stored values
  except through explicit effect variables.
- They are the only place where effects are usually written out by hand.

## 9. Seeing suspension points

There is no `await` keyword; suspension is implicit ([`concurrency.md`](concurrency.md) §8).
Reviewers may still want to see where a lambda can suspend, because state in sync cells can change
across a suspension and a lock might be held across it.

*Open:* how to show them.
- **(a) Recommended:** no marker in the source. The language server shows suspension points as inlay
  hints, and semantic diffs report new ones (overview §2.19).
- (b) A Swift-style `await` marker at call sites, optional or required.

## 10. Syntax

- Closures use Rust syntax: `|a, b| expr` or `|a, b| { … }`. There is no `move`; captures are
  inferred.
- There are no trailing lambdas. A closure is an ordinary argument and comes before any named
  arguments: `xs.visit_concurrent(|x| …, limit: 8)` ([`syntax.md`](syntax.md)).

## 11. Open questions

1. Labelled `break` out of non-escaping lambdas (§7).
2. Default closure trait for callbacks: stated or inferred (§3).
3. Showing suspension points: inlay hints or an `await` marker (§9).
4. Rules and syntax for methods that declare which fields of `self` they touch (§6).

## Prior art

- Swift: non-escaping closures by default, `@escaping`, `async` closures, `await` markers.
- Kotlin: inline lambdas with non-local `return`.
- Rust: closure traits, disjoint closure captures (2021 edition), async closures (1.85), labelled
  block `break`, the keyword-generics initiative.
- Koka and Effekt: effect-polymorphic higher-order functions.
- Niko Matsakis's "view types" proposals for partial borrows of `self`.

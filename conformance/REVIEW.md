# Conformance suite review (November 2026)

A review of every test in [`2027/`](2027) (181 files, slices S0 to S9d) against the reference
paragraphs it cites, so that its shortcomings can be fixed in one sweep. It was done by reading:
nothing was run, so a finding marked **gap?** may hide an implementation bug that a stronger test
would show. **Delete this file when the sweep is done.**

The review began with [`closures/fails-bound.moddil`](2027/closures/fails-bound.moddil): its
`serve` returned `1` without calling `f`, and that hid a bug in the MIR interpreter (see
[Implementation gaps](#implementation-gaps)).

## How to use it

- Fix the findings directory by directory. Each says what is wrong and what to do, with code where
  that helps.
- Where a finding asks for a new case, put a diagnostic case in the directory's error file (a file
  with expected diagnostics is only checked, not run: [`README.md`](README.md#writing-a-test)).
- Run `cargo nextest run -p moddil-conformance` after each directory. A stronger test that fails
  on one interpreter is an implementation bug: fix it, or list it under
  [Implementation gaps](#implementation-gaps) for the slice that owns it.

## Kinds of finding

| Kind | Meaning |
|---|---|
| **claim** | The file does not test the central rule of a paragraph it cites |
| **weak** | Results that cannot tell implementations apart: a derived number (a length, a sum) where the whole value would show more, every case giving the same value, or only the passing path running |
| **missing** | A case the paragraph states, in neither this file nor a sibling: often its error code |
| **cite** | `covers:` names a paragraph the file does not test, or the file tests a rule it does not cite |
| **dead** | Code nothing uses |
| **gap?** | A test shaped around something that may not work yet |

## Summary

| Directory | claim | weak | missing | cite | dead | gap? |
|---|---|---|---|---|---|---|
| arithmetic | | | 1 | | | |
| arrays | | | 2 | | | |
| attributes | | | 1 | | | |
| borrows | | 2 | 1 | | | |
| calls | | | 1 | | | |
| closures | 1 | 2 | 4 | 1 | 1 | 1 |
| collections | 1 | | 4 | 1 | | |
| comptime | | | 1 | | | |
| concurrency | 2 | 2 | 4 | 1 | | |
| contracts | | | 2 | 1 | | |
| derive | | | 1 | | | |
| effects | | | 2 | | | |
| enums, structs | 1 | | | | | |
| errors | | | 3 | 1 | | 1 |
| generics | | | 2 | | | |
| iteration | | 1 | | | | |
| main | | 1 | | | | |
| modules | | | 3 | | | |
| operators | | | 1 | | | |
| ownership | | | 1 | 1 | | |
| prelude | | | 2 | | | |
| refinements | 1 | | | 1 | | |
| syntax, text | | | 3 | | | |
| traits | | | 3 | | | |
| wrappers | 1 | | | | 1 | |

The directories not listed (assert, assignment, bitwise, blocks, comparison, if, items, literals,
logic, loops, match, names, patterns, statements, tuples, types, views) have no finding worth a
change.

## Findings

### arithmetic

- **missing:** `types.moddil` covers `expr.arith.types.1`, which says a compound assignment is
  defined only for `int`s, and for `bool`s with `&=`, `|=` and `^=` (E0205). Neither case is
  tested. Add `let mut b = true; b += 1;` (E0205) to the error file, and `b &= false` with its
  value to `assignment/assignment.moddil`.

### arrays

- **missing:** `expr.array.repeat.1` has three rules no test checks:
  - `x` is evaluated once;
  - `x`'s type must be `Copy` (E0402);
  - a length known only at run time is an error (E0201).

  Add `[{ n += 1; n }; 3]` with `n` observed afterwards, plus the two errors to `errors.moddil`.
  `expr.array.1`'s E0201 (elements of different types) and `expr.index.1`'s E0201 (a non-`int`
  index) are only in `arrays/errors.moddil`'s `a[true]`. Add `[1, true]`.
- **missing:** `move-out.moddil` covers `expr.index.move.1`, which also says:
  - an element behind `Index` cannot be moved out (E0304);
  - an element can be swapped out, replaced or cloned instead.

  The E0304 case is in `collections/borrows.moddil` under another citation (see
  [collections](#collections)). Move it here or cite the paragraph there. Add a running case of
  `replace(&mut a[0], x)`, which no test uses (see [prelude](#prelude)).

### attributes

- **missing:** `track-caller.moddil` covers `attr.track-caller.1`, which says the attribute takes
  no arguments, and that other attributes on functions are not supported yet (I0001). No test
  expects I0001 at all. Add `#[track_caller(x)]` and an unknown attribute to a new
  `attributes/errors.moddil`.

### borrows

- **weak:** `static.moddil` calls `drop(s)` and then uses `g`, as if to show that `g` outlives
  `s`. But `g` came from `greeting()`, which takes no parameter, so it never borrowed `s` and the
  step shows nothing. Drop those lines, or keep the result of `name_or(false, s.as_str())` past
  `drop(s)` as an error case: by its signature it borrows `s`, even when it returns the literal
  (`item.fn.ret-borrow.1`).
- **weak:** In `returning-errors.moddil`, `fn no_source() -> &int { 1 }` is a type mismatch
  (E0201). It tests nothing about returning borrows. Replace it with
  `fn no_source() -> &int { let x = 1; &x }` (E0308), or remove it, since
  `static-errors.moddil` already has that case.
- **missing:** `item.fn.ret-borrow.1` says a `from` parameter may be of a type parameter. Add a
  passing case: `fn keep<T>(a: T, b: &int) -> &int from b { b }`.

### calls

- **missing:** `expr.call.named.1` and `expr.call.default.1` have three cases no test checks:
  - a closure's call names no arguments;
  - a default names no parameter;
  - a default's type names no type parameter.

  Add them to `named_errors.moddil`.

### closures

- **claim, gap?:** In `fails-bound.moddil`, `serve` returns `1` and never calls `f`. Every result
  is `1`, `x` is unused, and `lookup`'s failing paths never run.
  - The comment "here, by its own `try`" describes code that isn't there.
  - `Unhandled` is unused (dead), and no closure literal is passed.
  - The paragraph's main rule, that a call of `f` in the body must handle the errors, isn't
    tested. Replace the body and the test:

    ```rust
    fn serve<F: Fn(int) -> int>(f: F, x: int) -> int where F::Fails: Answer {
        try { f(x)? } catch { e => e.answer() }
    }

    test "a bound on F::Fails takes the closure's errors" {
        serve(lookup, 3)      // => 3
        ;
        serve(lookup, 0)      // => 404
        ;
        serve(lookup, -1)     // => 400
        ;
        serve(plain, 3)       // => 3
        ;
        serve(|x| x * 2, 4)   // => 8
    }
    ```

    With this body the MIR interpreter gives `Missing` and `Bad` instead of 404 and 400 (see
    [Implementation gaps](#implementation-gaps)).
- **weak:** `fails-bound-errors.moddil` has the same placeholder `serve` body and the same
  misleading comment. Use the body above.
- **missing:** `expr.closure.fails.2` says a call of `f` in the body must handle its errors. Add a
  `serve` that calls `f(x)` unmarked or with only `?` to `fails-bound-errors.moddil`; it is an
  error.
- **missing:** Several closure rules have no error case:
  - `expr.closure.1`: a refutable closure parameter (E0209), and `break` or `continue` leaving a
    closure;
  - `expr.closure.capture.1`: a struct field holding a closure (E0309);
  - `expr.closure.call.1`: a wrongly typed argument (E0201).

  Add them to `closure-errors.moddil`.
- **missing:** `expr.closure.fails.1` has no error file. A call passing a failing closure, left
  unmarked, is E0501 (only tested for `spawn`, in `concurrency/diagnostics.moddil`). Add it.
- **missing:** `fn-values.moddil` covers `expr.fn-value.1`, which says `Vec::new` and `Some` can't
  be used as values yet. Add both to an error file.
- **cite:** `closure-errors.moddil` tests E0407 and E0408 for `impl Two { … }`, which are
  `expr.anon-impl.1`'s, but doesn't cite it. Add the citation, and an anonymous method that fails
  with an error its trait's method doesn't list (E0502).
- **weak:** `closures.moddil` cites `prelude.fn.1`, but never names `Output`, `Fails`, `call`,
  `call_mut` or `call_once`. Add a bound that fixes `Output`
  (`F: Fn(int) -> int`, then `F::Output` as a result type), or drop the citation.

### collections

- **claim:** `vec.moddil` cites `coll.buf.1`, but no test in the suite uses `Buf`. Add a
  `collections/buf.moddil` that covers:
  - `with_capacity`, `from_array`, `set_capacity`, `push`, `pop`, `insert`, `remove`, `swap` and
    `as_slice`;
  - the traps: pushing into a full buffer, and an index out of bounds;
  - its elements dropped first to last, with a destructor that records the order.
- **missing:** `coll.vec.1` lists methods no test calls: `with_capacity`, `truncate`, `reserve`,
  `last` and `as_mut_slice`. The growth rule ("at least twice as much") is checked only as
  `capacity() >= 10`. Add a case that pushes into a full `Vec` and checks `capacity()` against the
  old capacity times two.
- **missing:** `expr.index.trait.1` says changing an element through a type without `IndexMut` is
  an error (E0205). Add a type with only `Index`, and assign through it.
- **missing:** `maps.moddil` leaves parts of four paragraphs untested:
  - `coll.set.1`: `remove`, `is_empty` and `iter`;
  - `coll.map.1`: `iter`, which is called nowhere;
  - `coll.map.2`: the `Set` trait;
  - `coll.hash.1`: a hand-written `impl Hash` with a `Hasher`, and the `Hash` impls of `bool`,
    `char`, `Option` and pairs.
- **missing:** `coll.hash.derive.1` says a derive hashes the variant's index, then each field.
  Nothing checks the value. Compare the hashes of `Point { x: 1, y: 2 }` and `Point { x: 2, y: 1 }`,
  which must differ, and of `Key::A(0)` and `Key::B`.
- **cite:** `borrows.moddil`'s `moved_out` tests `expr.index.move.1` (E0304 through `Index`), but
  the file cites only `coll.vec.borrow.1`. Cite it.

### comptime

- **missing:** `comptime.block.1` says a `const { }` block names no local, type parameter or const
  parameter. No test checks this. Add `let n = 1; const { n + 1 }` to `errors.moddil`.

### concurrency

- **claim:** `stop.moddil` claims "a stop reaches the inner scope's tasks first", but observes
  `order.len() + outer.len()`, which is `2` whatever the order. Push into one shared `Vec` and
  observe it:

  ```rust
  let mut order: Vec<int> = [];
  scope(|s| {
      s.spawn(|| { stop_requested(); order.push(2); });
      s.spawn(|| scope(|inner| {
          inner.spawn(|| { stop_requested(); order.push(1); });
      }));
      s.stop();
  });
  order                                       // => [1, 2]
  ```

- **claim:** `cancel.moddil` cites `task.cancel.2`, but never tests it. That paragraph says:
  - while a task unwinds, its cleanup isn't cancelled again;
  - a trap while unwinding ends the program.

  Add a cancelled task whose `defer` waits (`yield_now()`) and then records that it finished, and
  a trap inside a `defer` while unwinding (`//~ trap`). `task.cancel.1` also says "a `join` of a
  cancelled task unwinds the joining task", which isn't tested either.
- **weak:** In `tasks.moddil`, the test "waiting at a join lets the other ready tasks run, in
  order" observes `first.join() + second.join()`, which is `3` whatever the order. Record one
  shared sequence (`log.push(…)`) and observe it, for example `[1, 3, 2]` for "first pushes,
  yields, second runs, first resumes".
- **weak:** `channels.moddil` observes `got.len()` and `got[4]`, but the claim is order. Observe
  `got` (`// => [0, 1, 2, 3, 4]`). The rendezvous (`channel(0)`) is used but not shown to wait. Add
  an order log, showing the sender resumes only after the receiver took the value.
- **missing:** `task.channel.1` and `task.channel.2` have cases no test checks:
  - a negative capacity traps;
  - values left in a channel are dropped.
- **missing:** `task.select.1` and `task.select.2` have four cases no test checks:
  - a value a non-chosen `send` arm would have sent is dropped;
  - a `send` in a `select` doesn't wait for its value to be received;
  - an arm pattern that can fail (E0209);
  - arm values of different types (E0201).
- **missing:** `task.isolate.1` says a trap in a non-isolated scope inside an isolated one fails
  the isolated one. Add it to `isolate.moddil`.
- **missing:** `task.borrow.1` says changing what a task borrows is E0306. Only E0305 and E0308 are
  tested. Add it to `borrows.moddil`.
- **cite:** `select_errors.moddil`'s `#[selectable] fn no_wait` is `task.select.3`'s E0217, but
  the file cites only `task.select.1`. Cite it.

### contracts

- **missing:** `contract.ensures.1` has two cases no test checks:
  - a postcondition is checked when leaving by `?` on `None`;
  - leaving with an error checks none.

  Add a function with a false `ensures` that fails, observed through `try` without a trap.
- **missing:** `contract.1` says a trait's method has no contracts. `contract.2` says a contract
  assigns nothing (E1102). Add both to `errors.moddil`; only calls are tested now.
- **cite:** `errors.moddil`'s `not_bool` is `contract.1`'s E0201, but the file cites only
  `contract.2` and `contract.old.1`. Cite it.

### derive

- **missing:** `derives.moddil` doesn't test `Ord` for an enum (only `PartialOrd`), or a `bool`
  field in a derived ordering (`false` before `true`, `prelude.ordering.1`).

### effects

- **missing:** `eff.infer.1` says effects that grow without end are E0607, which no test expects.
  Add a private generic function that needs `Store<T>` (through a call) and calls itself with
  `Wrap<T>`, so its needs grow at each step.
- **missing:** `eff.control.1` says a control effect is listed once (E0606) and without a name
  (E0601). Only other effects are tested for these codes. Add `with suspend, suspend` and
  `with s: suspend`.

### enums, structs

- **claim:** `structs/structs.moddil`'s test "structs with the same fields are different types"
  observes `s.x + origin().x`, which can't show nominal typing. Add an E0201 case that passes a
  `Size` where a `Point` is expected, to `structs/errors.moddil`, and drop or rename the running
  test.

### errors

- **gap?, missing:** `err.catch.1` says an arm `e` matches any error. No test catches a **generic
  error set** with a binding arm. Only `refinements.moddil` catches `e`, of one concrete type.
  The `fails-bound` finding is the test for it.
- **missing:** `err.catch.1` says the arms have the block's type (E0201). Add
  `try { a()? } catch { A(_) => true }` in an `int` function to `errors.moddil`.
- **missing:** Three rules in `err.question.1` and `err.set.1` have no case:
  - `x? "note"`, a note for the error's trace, which no test writes (`err.question.1`). Add a
    running case where the note changes nothing.
  - "At most one instance of a generic error" (E0508, `err.set.1`): add
    `with fail(Wrap<int> | Wrap<bool>)`.
- **cite:** `errors.moddil`'s `mixed` is `err.try.1`'s E0506, but the file doesn't cite `err.try.1`.
  Cite it.

### generics

- **missing:** `generic.1` says the parameters of one item have distinct names (E0104). Add
  `fn f<T, T>()` to `errors.moddil`.
- **missing:** `generic.infer.1` says `_` stands for a type to infer in an annotation. Add
  `let v: Vec<_> = [1, 2];`, and `generic.bound.1`'s "a bound also gives the trait's
  supertraits" (call a supertrait's method on `T: Sub`).

### iteration

- **weak:** `adapters.moddil` observes `v.len()` and one element. Observe the whole `Vec`
  (`// => [0, 1, 4, 9]`, `// => [0, 3, 6, 9]`) so that order and every item are checked.

### main

- **weak:** `roots/src/lib.moddil`'s test observes `1 // => 1`, which tests nothing. The file's
  real check is that its `main` is accepted. Drop the test, or call a function that takes one of
  the capabilities. `exit.moddil` never observes `ExitCode::Ok.code()` (0, `main.exit.1`).

### modules

- **missing:** `mod.use.1` and `mod.path.1` have untested parts:
  - `pub use` and `pub(mod) use` re-exports (`mod.use.1`): add one to `tree` and use the item
    through the re-exporting module;
  - `self::` as a path start (`mod.path.1`);
  - a path segment that names no type (E0102) or no trait (E0401), also `mod.path.1`.
- **missing:** `mod.tree.1` says a file whose name isn't a name can't be a module (E0704). Add a
  `src/2d.moddil` to a manifest test.
- **missing:** `mod.dep.1` says a mod sees only another mod's `pub` items. This is tested only
  through `use dep::internal`. Add a path use (`dep::internal()`) in `visibility`.

### operators

- **missing:** `traits.moddil` covers `expr.op.trait.1`, but has impls only for `Add` and `Neg`.
  `Sub`, `Mul`, `Div`, `Rem` and `Not` on a type of one's own are called nowhere. "The operands
  are moved in" isn't shown either: use a non-`Copy` left operand again after `+`, as an E0303
  case.

### ownership

- **missing:** `own.copy.1` and `own.copy.derive.1` have untested cases:
  - `&mut T` isn't `Copy` (`own.copy.1`): add the E0303 case of using a `&mut` local after
    `let b = a;` with `a: &mut int`;
  - "a generic type declared `Copy` is `Copy` only for type arguments that keep it within 64
    bytes" (`own.copy.derive.1`): add `Wrap<(int, int, int, int, int, int, int, int, int)>` moved
    twice.
- **cite:** `drop-errors.moddil`'s `#[derive(Copy, Clone)] struct Point` with a destructor is
  `own.copy.derive.1`'s E0411, which the file doesn't cite. Cite it.

### prelude

- **missing:** `prelude.1` names the functions `replace` and `swap`. No test calls either: the
  `swap(` matches are methods and a function of a test's own. Add both to `prelude.moddil`.
- **missing:** `obs.log.1` lists `log.warn` and `log.error`, and "an item called `log` shadows
  it". Only `debug`, `info` and a local are tested.

### refinements

- **claim:** `errors.moddil` cites `refine.new.2` (a base holding a borrow, E0309), but has no
  such case. Add `type Ref = &int where true;`.
- **cite:** `errors.moddil`'s `Bad` (E0402, the base isn't `Debug`) is `refine.new.1`'s, and
  `Calls` (E1001) is `refine.1`'s. Neither paragraph is cited. Cite both.

### syntax, text

- **missing:** `syntax.text.1` says a source file is UTF-8, and gives two errors (E0001) that no
  test expects:
  - an identifier not in NFC;
  - a bidirectional control character outside a literal.

  Add them to `syntax/grammar-error.moddil` or a new error file.
- **missing:** `text.lit.escape.1` lists escapes no test writes: `\r`, `\0`, `\\`, `\'`, `\s`,
  `\xHH`, the line continuation (`\` at the end of a line), and `\u{…}` with `_`. An unknown escape
  (E0001) isn't tested either.
- **missing:** `text.lit.block.1` has untested parts:
  - a raw block (`r"""`);
  - `\s` keeping a trailing space;
  - a line indented less than the closing quotes (E0001);
  - mixed tabs and spaces (E0001).

### traits

- **missing:** `item.trait.super.1` says implementing a subtrait for a type that doesn't
  implement its supertrait is E0402. No test checks this. Add `impl Named for Circle` with no
  `impl Shape for Circle` to `impl-errors.moddil`.
- **missing:** `item.trait.1` and `item.impl.1` say a method declared twice in a trait, or two
  inherent functions of the same name, is E0103. Add both.
- **missing:** `type.impl.ret.1`'s limits have no case: a trait's method with an `impl Trait`
  result, a result type containing one (`Option<impl Shape>`), and a hidden type holding a
  borrow.

### wrappers

- **claim:** `errors.moddil` cites `attr.around.limits.1`, but has no case of it. Add a wrapped
  method, a wrapped `const fn`, and a wrapped function with a pattern parameter (I0001 each).
  `attr.around.body.1`'s "a wrapper adds no error or effect the function does not have"
  (E0501, E0603) is tested only for errors on a `pub` function. Add an effect.
- **dead:** `plain` in `errors.moddil` is never used.

## Implementation gaps

1. **A trait's method called on an error set runs nothing (MIR interpreter).** Found with the
   stronger `closures/fails-bound.moddil` above.
   - In `try { f(x)? } catch { e => e.answer() }` with `F::Fails = Missing | Bad`, the MIR
     interpreter gives the error itself (`Missing`) where the reference interpreter gives `404`.
   - The cause is `Dispatch::target` (`crates/interp/src/dispatch.rs`): a trait method's target
     is found from the types alone. The error set has no impl of its own, so `impl_method` finds
     none, and `None` is taken as "the built-in `Clone`, which copies".
   - To fix it, the call must choose the member of the set by the error's value. A failed lookup
     shouldn't fall back to a copy.

## Patterns across the suite

- **Observe whole values.** Several tests observe a length, a sum or one element where the whole
  value shows more: order, every item, and nothing extra. Prefer `v // => [1, 2, 3]` to `v.len()`
  wherever the value is short.
- **Error codes per paragraph.** Each paragraph that names an error code should have that code
  expected in a file that cites the paragraph. The **cite** and **missing** findings above came
  partly from this check. Run it again after the sweep: list each cited paragraph's codes, and
  the codes the files citing it expect.

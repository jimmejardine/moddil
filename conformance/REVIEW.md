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

## Corrections found while sweeping

- **Conformance doesn't test I0001.** "Not supported yet" belongs to an implementation, not to the
  language, so the runner rejects it. The findings that asked for I0001 cases are dropped: the
  `track_caller` arguments, the wrapper limits, and `Vec::new` and `Some` as values.
- **E0309 for a closure in a field can't be written.** A closure's type can't be named, so a field
  can hold one only through a type parameter, and that makes the struct a view, which is allowed.

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
| comptime | | | 1 | | | |
| contracts | | | 2 | 1 | | |
| derive | | | 1 | | | |
| effects | | | 2 | | | |
| enums, structs | 1 | | | | | |
| generics | | | 2 | | | |
| main | | 1 | | | | |
| modules | | | 3 | | | |
| operators | | | 1 | | | |
| ownership | | | 1 | 1 | | |
| refinements | 1 | | | 1 | | |
| syntax, text | | | 3 | | | |
| traits | | | 3 | | | |
| wrappers | | | 1 | | 1 | |

The directories not listed have no finding left: arithmetic, arrays, attributes, borrows, calls, closures, collections, concurrency, errors, iteration and prelude are done, and
assert, assignment, bitwise, blocks, comparison, if, items, literals, logic, loops, match, names,
patterns, statements, tuples, types and views had none worth a change.

## Findings

### comptime

- **missing:** `comptime.block.1` says a `const { }` block names no local, type parameter or const
  parameter. No test checks this. Add `let n = 1; const { n + 1 }` to `errors.moddil`.

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

### generics

- **missing:** `generic.1` says the parameters of one item have distinct names (E0104). Add
  `fn f<T, T>()` to `errors.moddil`.
- **missing:** `generic.infer.1` says `_` stands for a type to infer in an annotation. Add
  `let v: Vec<_> = [1, 2];`, and `generic.bound.1`'s "a bound also gives the trait's
  supertraits" (call a supertrait's method on `T: Sub`).

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

- **missing:** `attr.around.body.1` says a wrapper adds no error or effect the function doesn't
  have (E0501, E0603). This is tested only for errors on a `pub` function. Add an effect.
- **dead:** `plain` in `errors.moddil` is never used.

## Implementation gaps

None open. Fixed in the sweep so far:

1. **A trait's method called on an error set ran nothing (MIR interpreter).** `calls::plan` now
   picks the error's own type from its value. A method with no impl is now a bug that panics,
   not a silent copy.
2. **A closure parameter's errors that a bound takes could leave the function (sema).** They now
   become the function's inferred errors only if they don't leave. Letting them leave, with `?`
   or unmarked, is E0502, which `expr.closure.fails.2` now says.
3. **A borrow passed for a type parameter came back borrowing nothing (MIR ownership check).** `let r = id(&x); x = 2; *r` was accepted. The call took the argument as lending what it points to, judging by the parameter type after substitution, not as written. Now only a parameter declared a borrow does that. A result of a type parameter may also take `from` (sema), as `item.fn.ret-borrow.1` now says.
4. **A closure's call could name its arguments (sema).** `f(x: 1)` is now E0215.
5. **Two diagnostics had a run of spaces in the middle** (E0308 and E0314 in `crates/mir/src/ownership.rs`), from a line break inside the string.

## Left open

Cases the sweep did not add, each needing more setup than the rule it shows:

- **`task.select.1`:** a `send` in a `select` doesn't wait for its value to be received. It needs a
  rendezvous channel whose receiver isn't waiting yet, and a way to show the sender went on.
- **`task.cancel.1`:** a `join` of a cancelled task unwinds the joining task. It needs a task
  cancelled while another task, in a scope that isn't cancelled, holds its `Task`.

## Patterns across the suite

- **Observe whole values.** Several tests observe a length, a sum or one element where the whole
  value shows more: order, every item, and nothing extra. Prefer `v // => [1, 2, 3]` to `v.len()`
  wherever the value is short.
- **Error codes per paragraph.** Each paragraph that names an error code should have that code
  expected in a file that cites the paragraph. The **cite** and **missing** findings above came
  partly from this check. Run it again after the sweep: list each cited paragraph's codes, and
  the codes the files citing it expect.

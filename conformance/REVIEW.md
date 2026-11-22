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
- **`type.impl.ret.1`'s limits can't be tested here** (a trait's method with an `impl Trait`
  result, a result type containing one): they are I0001 for now.
- **A contract on a trait's method can't be tested here:** it is I0001 for now (`contract.1`).
- **`let b = a;` with `a: &mut int` is a reborrow, not a move** (`borrow.reborrow.1`), so the
  `own.copy.1` finding became "using `a` while `b` is live" (E0306).
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

The directories not listed have no finding left: arithmetic, arrays, attributes, borrows, calls, closures, collections, comptime, concurrency, contracts, derive, effects, enums, errors, generics, iteration, main, modules, operators, ownership, prelude, refinements, structs, syntax, text, traits and wrappers are done, and
assert, assignment, bitwise, blocks, comparison, if, items, literals, logic, loops, match, names,
patterns, statements, tuples, types and views had none worth a change.

## Findings

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
6. **E0607 was reported on every inferred function in the file (sema),** whenever one function's effects grew without end. Now it names only the functions whose effects still grow.
7. **Three files expected diagnostics and values together** (`effects/dyn`, `effects/inference`, `items/test-for`), so their 22 `// =>` lines never ran. Their diagnostics are now in files of their own, and the runner fails such a file.
8. **The lexer rejected escapes the reference allows:** `'\''` (it took the escaped quote for the closing one), `_` between the digits of `\u{…}`, and `\` at the end of a line. It also accepted `\x` above `\x7F`.
9. **A block literal's indentation errors were E0902, at the opening quotes,** while the reference said E0001. Nothing could expect them there, since nothing may follow the opening `"""`. They are now reported at the closing quotes, and the reference says E0902, an invalid literal, as the implementation and the diagnostics table do.

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

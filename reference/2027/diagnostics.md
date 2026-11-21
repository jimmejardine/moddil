# Diagnostics

Diagnostic **codes** are part of conformance; the wording of messages is not. An implementation
reports the code on the line where the problem starts. Codes come in blocks of a hundred by area,
and a code never changes meaning.

| Block | Area |
|---|---|
| E00xx | Syntax |
| E01xx | Names |
| E02xx | Types |
| E03xx | Ownership and mutability |
| E04xx | Traits and generics |
| E05xx | Errors and `fail` |
| E06xx | Effects |

| Code | Meaning | Rule |
|---|---|---|
| E0001 | Syntax error | [syntax.grammar.1](syntax.md) |
| E0002 | A comparison chain in more than one direction | [expr.cmp.chain.2](expressions/comparison.md) |
| E0101 | Unresolved name | [names.resolve.1](names.md) |
| E0102 | Unresolved type | [names.type.1](names.md) |
| E0103 | Duplicate definition, or an item with the name of an import | [names.item.1](names.md), [names.item.2](names.md), [mod.names.1](modules.md) |
| E0104 | Duplicate parameter | [item.fn.params.1](items/functions.md) |
| E0105 | A name not bound in every alternative of `\|` | [pat.or.1](patterns.md) |
| E0106 | A name bound twice in one pattern | [pat.bind.unique.1](patterns.md) |
| E0107 | No such field | [expr.struct.1](expressions/structs.md), [expr.field.1](expressions/fields.md), [pat.struct.1](patterns.md) |
| E0108 | A field given twice | [expr.struct.1](expressions/structs.md), [pat.struct.1](patterns.md) |
| E0109 | `break` or `continue` outside a loop | [expr.break.1](expressions/loops.md) |
| E0110 | Unresolved label | [expr.break.label.1](expressions/loops.md) |
| E0111 | A struct or variant in a form it is not declared with | [expr.struct.form.1](expressions/structs.md), [pat.struct.tuple.1](patterns.md) |
| E0112 | A built-in used as a value | [expr.assert.name.1](expressions/assert.md) |
| E0201 | Mismatched types | [type.match.1](types.md) and the rules that expect a type |
| E0202 | Wrong number of arguments | [expr.call.1](expressions/calls.md), [expr.closure.call.1](expressions/closures.md) |
| E0203 | Not a function | [expr.call.1](expressions/calls.md), [expr.closure.call.1](expressions/closures.md) |
| E0204 | Literal out of range | [expr.lit.int.2](expressions/literals.md) |
| E0205 | Operator not defined for these types | [expr.arith.types.1](expressions/arithmetic.md), [expr.op.trait.1](expressions/arithmetic.md), [expr.cmp.order.1](expressions/comparison.md), [expr.cmp.trait.1](expressions/comparison.md), [generic.bound.1](generics.md) |
| E0206 | Invalid assignment target | [expr.assign.1](expressions/assignment.md) |
| E0207 | Missing fields | [expr.struct.1](expressions/structs.md), [pat.struct.1](patterns.md) |
| E0208 | Non-exhaustive `match` | [expr.match.exhaustive.1](expressions/match.md) |
| E0209 | A pattern that can fail where one that cannot is needed | [pat.irrefutable.1](patterns.md) |
| E0210 | `let … else` whose block can finish normally | [stmt.let.else.1](statements.md) |
| E0211 | A recursive type of infinite size | [item.struct.recursive.1](items/structs.md) |
| E0212 | `break` with a value out of `while` | [expr.break.value.1](expressions/loops.md) |
| E0213 | Wrong number of fields in a pattern | [pat.tuple.1](patterns.md), [pat.struct.tuple.1](patterns.md) |
| E0214 | An empty range pattern | [pat.range.1](patterns.md) |
| E0301 | Mutation of an immutable local (assigning, or borrowing with `&mut`) | [expr.assign.1](expressions/assignment.md), [borrow.mut.1](borrows.md) |
| E0302 | Mutation through a shared borrow | [borrow.mut.1](borrows.md) |
| E0303 | Use of a moved value | [own.move.1](ownership.md), [own.move.partial.1](ownership.md) |
| E0304 | Move out of a borrow | [own.move.borrow.1](ownership.md), [pat.ref.1](patterns.md) |
| E0305 | Conflicting borrows | [borrow.rules.1](borrows.md) |
| E0306 | Assignment to or move of a borrowed value | [borrow.rules.1](borrows.md), [borrow.guard.1](borrows.md) |
| E0307 | Use of a value borrowed with `&mut` | [borrow.rules.1](borrows.md) |
| E0308 | A borrow that outlives its value | [borrow.scope.1](borrows.md), [item.fn.ret-borrow.1](items/functions.md) |
| E0309 | A borrow (or closure) stored in a struct, enum or tuple, a borrow of a borrow, or a borrow as a type argument | [borrow.type.1](borrows.md), [generic.borrow.1](generics.md), [expr.closure.capture.1](expressions/closures.md) |
| E0310 | A returned borrow without a borrowed parameter to come from | [item.fn.ret-borrow.1](items/functions.md) |
| E0311 | A type that cannot be `Copy` | [own.copy.derive.1](ownership.md) |
| E0312 | A `Copy` type over 64 bytes | [own.copy.derive.1](ownership.md) |
| E0313 | `return`, `break` or `continue` out of a `defer` block | [stmt.defer.1](statements.md) |
| E0314 | A field moved out of a value whose type has a destructor | [own.drop.partial.1](ownership.md) |
| E0401 | A name used as a trait that is not one | [item.trait.1](items/traits.md) |
| E0402 | A type that does not implement a trait it must | [generic.bound.1](generics.md), [item.trait.super.1](items/traits.md), [attr.derive.1](attributes.md), [type.dyn.coerce.1](types.md), [eff.with.1](effects.md) |
| E0403 | A method or function that the type does not have | [expr.method.1](expressions/methods.md), [expr.path.assoc.1](expressions/methods.md), [generic.bound.1](generics.md) |
| E0404 | Wrong number of type arguments | [generic.1](generics.md) |
| E0405 | A type that cannot be inferred | [generic.infer.1](generics.md) |
| E0406 | Two impls of a trait for the same type | [item.impl.coherence.1](items/impls.md) |
| E0407 | An impl that leaves out a method or associated type of its trait | [item.impl.trait.1](items/impls.md), [expr.anon-impl.1](expressions/closures.md) |
| E0408 | An impl that defines something its trait does not declare | [item.impl.trait.1](items/impls.md), [expr.anon-impl.1](expressions/closures.md) |
| E0409 | An impl's method whose signature differs from the trait's | [item.impl.trait.1](items/impls.md) |
| E0410 | A method that more than one trait gives the type | [expr.method.ambiguous.1](expressions/methods.md) |
| E0411 | A destructor that is not for the impl's own type and all its type arguments, has bounds, or is for a `Copy` type | [own.drop.destructor.1](ownership.md), [own.copy.derive.1](ownership.md) |
| E0412 | A call of a destructor | [own.drop.destructor.1](ownership.md) |
| E0413 | An associated type or function that does not exist | [generic.assoc.1](generics.md) |
| E0414 | Methods added to a type not declared in the module | [item.impl.1](items/impls.md) |
| E0415 | An impl's type parameter that neither its type nor its trait uses | [item.impl.coherence.1](items/impls.md) |
| E0501 | An error with nowhere to go: an unmarked call that can fail, or an error where nothing can fail | [err.question.1](errors.md), [err.set.1](errors.md), [err.entry.1](errors.md), [stmt.defer.1](statements.md) |
| E0502 | An error that the function's `with fail(…)` does not list | [err.set.1](errors.md) |
| E0503 | A `Result` dropped unused | [err.unused.1](errors.md) |
| E0504 | `?` or `?!` on something that cannot fail | [err.question.1](errors.md) |
| E0505 | A `catch` arm for an error that cannot occur, or a `catch` with nothing to catch | [err.catch.1](errors.md) |
| E0506 | `?` on an `Option` where absence cannot go, or on both `Option`s and errors in one `try` | [err.question.1](errors.md), [err.try.1](errors.md) |
| E0507 | An entry point that declares errors | [err.entry.1](errors.md) |
| E0508 | An error type that is not a struct or enum, or a second instance of a generic one | [err.fail.1](errors.md), [err.set.1](errors.md) |
| E0601 | Something after `with` that is not an effect, or an effect named wrongly | [eff.1](effects.md), [eff.name.1](effects.md) |
| E0602 | An entry point whose call needs an effect nothing installs | [eff.entry.1](effects.md) |
| E0603 | A need that a function whose effects are written out does not list | [eff.infer.1](effects.md), [eff.method.1](effects.md) |
| E0604 | An effect other than `fail` on a trait's method or an impl's | [eff.method.1](effects.md) |
| E0605 | A `dyn` type not behind a borrow, of more than one trait, or of a trait that cannot be used as `dyn` | [type.dyn.1](types.md) |
| E0606 | An effect listed twice, or given two handlers by one `with` | [eff.1](effects.md), [eff.with.1](effects.md) |
| E0607 | Effects inferred without end | [eff.infer.1](effects.md) |
| E0801 | A closure whose kind is weaker than a bound needs | [expr.closure.param.1](expressions/closures.md) |
| E0701 | A path or import naming an item its module cannot see | [mod.vis.1](modules.md), [mod.use.1](modules.md) |
| E0702 | A `use` that names nothing, a glob import, or a declared module | [mod.use.1](modules.md), [mod.tree.1](modules.md) |
| E0703 | An item whose signature names something less visible than itself | [mod.expose.1](modules.md) |
| E0704 | A manifest that is not as a manifest is, or names files that are not there | [mod.1](modules.md), [mod.tree.1](modules.md) |
| E0705 | Mods that depend on each other in a cycle | [mod.dep.1](modules.md) |

**Warnings** have codes of the same form starting with `W`, in the same blocks. They do not stop a
program from running, and conformance tests expect them like errors.

| Code | Meaning | Rule |
|---|---|---|
| W0201 | An unreachable `match` arm | [expr.match.unreachable.1](expressions/match.md) |

Implementations may add codes of their own outside the `E` series. The stage-0 compiler reports
`I0001` for valid Moddil it does not support yet.

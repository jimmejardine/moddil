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
| E0103 | Duplicate definition | [names.item.1](names.md) |
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
| E0202 | Wrong number of arguments | [expr.call.1](expressions/calls.md) |
| E0203 | Not a function | [expr.call.1](expressions/calls.md) |
| E0204 | Literal out of range | [expr.lit.int.2](expressions/literals.md) |
| E0205 | Operator not defined for these types | [expr.arith.types.1](expressions/arithmetic.md), [expr.cmp.order.1](expressions/comparison.md) |
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
| E0309 | A borrow stored in a struct, enum or tuple, or a borrow of a borrow | [borrow.type.1](borrows.md) |
| E0310 | A returned borrow without a borrowed parameter to come from | [item.fn.ret-borrow.1](items/functions.md) |
| E0311 | A type that cannot be `Copy` | [own.copy.derive.1](ownership.md) |
| E0312 | A `Copy` type over 64 bytes | [own.copy.derive.1](ownership.md) |
| E0313 | `return`, `break` or `continue` out of a `defer` block | [stmt.defer.1](statements.md) |

**Warnings** have codes of the same form starting with `W`, in the same blocks. They do not stop a
program from running, and conformance tests expect them like errors.

| Code | Meaning | Rule |
|---|---|---|
| W0201 | An unreachable `match` arm | [expr.match.unreachable.1](expressions/match.md) |

Implementations may add codes of their own outside the `E` series. The stage-0 compiler reports
`I0001` for valid Moddil it does not support yet.

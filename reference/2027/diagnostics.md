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
| E0201 | Mismatched types | [type.match.1](types.md) and the rules that expect a type |
| E0202 | Wrong number of arguments | [expr.call.1](expressions/calls.md) |
| E0203 | Not a function | [expr.call.1](expressions/calls.md) |
| E0204 | Literal out of range | [expr.lit.int.2](expressions/literals.md) |
| E0205 | Operator not defined for these types | [expr.arith.types.1](expressions/arithmetic.md), [expr.cmp.order.1](expressions/comparison.md) |
| E0206 | Invalid assignment target | [expr.assign.1](expressions/assignment.md) |
| E0301 | Assignment to an immutable local | [expr.assign.1](expressions/assignment.md) |

Implementations may add codes of their own outside the `E` series. The stage-0 compiler reports
`I0001` for valid Moddil it does not support yet.

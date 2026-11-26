# Arithmetic

**expr.arith.ops.1** `+`, `-` and `*` are addition, subtraction and multiplication of `int`s. `/`
divides, rounding towards zero; `%` is the remainder, with the sign of the left operand, so that
`a == a / b * b + a % b`.

**expr.arith.neg.1** Unary `-` negates an `int`.

**expr.arith.overflow.1** If the exact result of `+`, `-`, `*`, `/` or unary `-` is not an `int`
(as for `int::MIN / -1`), the program traps. There is no build, mode or region in which it wraps
instead ([Traps](../traps.md)).

**expr.arith.div-zero.1** Division or remainder by zero traps. `int::MIN % -1` is `0`, as its exact
result is.

**expr.arith.types.1** Both operands of an arithmetic operator on `int`s are `int`s: a right
operand of another type is a mismatch (E0201). For a left operand of another type, the operator
calls its trait (**expr.op.trait.1**), and is not defined if the type does not implement it
(E0205). A compound assignment (`a += b`) is defined for the numbers, for `bool`s with `&=`,
`|=` and `^=`, and for a type whose operator trait gives that type again (`+=` through `Add`,
**expr.assign.compound.2**); otherwise it is E0205.

**expr.op.trait.1** On a type other than `int` (and `bool` for `!`), an operator calls a method of
a prelude trait ([Prelude](../prelude.md)): `a + b` is `Add::add(a, b)`, and likewise `-` `Sub::sub`,
`*` `Mul::mul`, `/` `Div::div`, `%` `Rem::rem`; unary `-a` is `Neg::neg(a)` and `!a` `Not::not(a)`.
The binary ones take the right operand's type as `Rhs`, which is `Self` by default
(`trait Add<Rhs = Self>`, **generic.default.1**): `a + b` is `Add<B>`'s for `a`'s type `A`, `B`
being `b`'s type if `A` has an impl for it (`impl Add<Duration> for Instant`), and `A` itself
otherwise, the right operand then having the left operand's type (E0201): so a literal is of the
left operand's type. The operands are moved in, and the result has the trait's `Output` type. An operand that is a borrow of a `Copy` value is read through it
(**borrow.deref.2**); one that borrows a value that is not `Copy` is an error (E0205).

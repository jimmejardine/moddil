# Comparisons

**expr.cmp.eq.1** `==` and `!=` compare two values of the same type (E0201) for equality, and have
type `bool`.

**expr.cmp.order.1** `<`, `<=`, `>` and `>=` compare two values of the same type, and have type
`bool`. On `int`s they compare numbers; on other types they call `PartialOrd`
(**expr.cmp.trait.1**), and are not defined for types that do not implement it (E0205).

**expr.cmp.trait.1** On a type other than `int`, `bool` and `()`, `==` and `!=` call `PartialEq`'s
`eq` and `ne`, and `<`, `<=`, `>` and `>=` call `PartialOrd`'s `lt`, `le`, `gt` and `ge`
([Prelude](../prelude.md)), with shared borrows of the operands, which are not moved. A comparison of
two borrows is not defined (E0205): compare what they point to, with `*`. Tuples do not implement
`PartialEq` or `PartialOrd` in this edition (E0205).

**trait.borrow.eq.1** A shared or `&mut` borrow `&T` implements `PartialEq`, `Eq`, `PartialOrd` and
`Ord` where `T` does, built in: inside another value, a borrow compares as what it points to, so
`v.first() == Some(&1)` holds for a `v` starting with `1`. (Its methods would take a borrow of a
borrow, which no code can write; the comparison of two borrows themselves stays refused, above.)

**expr.cmp.chain.1** A chain `a < b <= c` means `a < b && b <= c`, with `b` evaluated once.
Operands are evaluated left to right, and evaluation stops at the first comparison that is false.

**expr.cmp.chain.2** A chain runs in one direction: every operator is `<` or `<=`, or every one is
`>` or `>=`. `==` and `!=` do not chain (E0002).

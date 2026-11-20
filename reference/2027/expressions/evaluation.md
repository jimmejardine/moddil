# Evaluation

**expr.order.1** The operands of an operator and the arguments of a call are evaluated left to
right, each exactly once, before the operation (except where `&&`, `||` and comparison chains stop
early: [Logic](logic.md), [Comparisons](comparison.md)).

**expr.paren.1** `(e)` has the value and type of `e`.

# Contracts

Contracts are clauses after a function's signature: what must hold when it is called, and what
holds when it returns ([`specs/correctness.md`: Contracts as syntax](../../specs/correctness.md#contracts-as-syntax)).

**contract.1** `requires e` and `ensures e` follow the signature, any number of each, in the
order they are checked. Each `e` is a `bool` (E0201). A trait's method has none, for now.
`requires always e` is a precondition no checking policy may switch off; as no policy switches
any off yet, it is checked as `requires` is.

**contract.2** A contract is pure: it assigns nothing and calls no function with an effect or an
error (E1102).

**contract.requires.1** On entry, after the parameters are bound, each `requires` is evaluated;
one that is `false` traps (a broken precondition), and the trap is reported where the function
was called: a broken precondition is the caller's fault ([Traps](traps.md)).

**contract.ensures.1** When the function returns (at the end of its body, at `return`, or by `?`
on `None`), each `ensures` is evaluated with `result` naming the value returned; one that is
`false` traps at the clause (a broken postcondition). Leaving with an error checks none.

**contract.old.1** In an `ensures`, `old(e)` is the value `e` had on entry: `e` is evaluated then,
and kept as a copy if its type is `Copy`, or else as a clone (`e` itself is not moved), so its
type is `Clone` (E1103).

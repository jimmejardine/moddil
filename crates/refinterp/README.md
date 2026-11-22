# moddil-refinterp

The **reference interpreter**: the executable form of the language reference, running the canonical
AST directly ([`specs/compiler.md`: Two interpreters](../../specs/compiler.md#two-interpreters)).

**Status:** slices S0 to S10, and every slice after: it walks the syntax tree of a checked program
and runs its tests (`run_tests`), on a thread with a large stack so that recursion 10,000 calls
deep traps as the reference says. The conformance runner runs every test on it and on the MIR
interpreter. It reads the prelude's source with the program, implements derives itself, and
tracks moves so that a moved value is not dropped. Handlers are kept on a stack in dynamic scope
and found by the effect's name, rather than passed from caller to callee as the compiler does. A
program of several files and mods is given as their texts and module paths (`run_program`); items
are kept by their module and name, and paths resolved per module. A closure remembers the
variables in scope where it is written, which its body sees as aliases, and the handlers installed
there.

Since it does not infer types, it chooses a trait's method from the kind of value it is called on,
and knows a type parameter (for `T::f()`) only from the values of the arguments given for it or
from type arguments written at the call. Three things it cannot run yet, so the suite avoids them:
a trait's function without `self` called through the trait (`Zero::zero()`), where only the
expected type says which impl; two impls of a trait for different arguments of one generic type
(`impl Tr for Wrap<int>` and `impl Tr for Wrap<bool>`), which it does not tell apart; and, in
generic code, an inherent method that has the same name as the bound trait's method, which it
prefers. Closing these needs type information the compiler has; whether the reference
interpreter should get its own inference is an open question.

## Rules

- **Independent of the compiler.** It depends only on `moddil-syntax` (parser and AST) and
  general-purpose crates, never on `sema`, `mir` or `interp`; CI enforces this. If it shared the
  compiler's semantics, a compiler bug would change the "reference" too and nothing would notice.
- **Written for clarity, not speed.** It covers the safe language and the closed list of undefined
  behaviour.
- **Differential testing:** conformance tests and fuzzed programs run here, on the MIR interpreter and
  on compiled code; any difference is a bug in one of them.

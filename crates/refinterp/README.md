# moddil-refinterp

The **reference interpreter**: the executable form of the language reference, running the canonical
AST directly ([`specs/compiler.md`: Two interpreters](../../specs/compiler.md#two-interpreters)).

**Status:** slices S0 to S2, and every slice after: it walks the syntax tree of a checked program
and runs its tests (`run_tests`), on a thread with a large stack so that recursion 10,000 calls
deep traps as the reference says. The conformance runner runs every test on it and on the MIR
interpreter.

## Rules

- **Independent of the compiler.** It depends only on `moddil-syntax` (parser and AST) and
  general-purpose crates, never on `sema`, `mir` or `interp`; CI enforces this. If it shared the
  compiler's semantics, a compiler bug would change the "reference" too and nothing would notice.
- **Written for clarity, not speed.** It covers the safe language and the closed list of undefined
  behaviour.
- **Differential testing:** conformance tests and fuzzed programs run here, on the MIR interpreter and
  on compiled code; any difference is a bug in one of them.

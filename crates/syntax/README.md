# moddil-syntax

Lexer, parser and AST for Moddil source.

**Status:** not started; grows slice by slice from S0
([M0 slices](../../specs/roadmap.md#m0-the-v0-language-in-slices)).

## Does

- **Lexing** per the reference: UTF-8, UAX #31 identifiers (NFC-normalised), ASCII keywords,
  bidirectional control characters rejected ([`specs/definition.md`: Lexical structure](../../specs/definition.md#lexical-structure)).
- **A hand-written parser** against [`reference/grammar/moddil.ebnf`](../../reference/grammar/README.md),
  for good error recovery and diagnostics; the grammar's generated tests keep the two in step.
- **The canonical AST**: the language is its canonical text form, and the formatter (later) prints
  this AST back to text ([`specs/toolchain.md`: Formatter](../../specs/toolchain.md#formatter)).

## Used by

`moddil-sema` and the rest of the compiler, and **`moddil-refinterp`**, which shares only this crate
with the compiler.

# Grammar

The **one grammar source** for Moddil: `moddil.ebnf`, written in a small EBNF dialect of our own
([`specs/definition.md`: One grammar source](../../specs/definition.md#one-grammar-source)).

**Status:** next to be written. It is the first artefact of M0, and it covers **all decided syntax**,
not only what M0 implements, so ambiguities surface before any parser code exists.

## Rules

- **LL(k) with a small k, and unambiguous.** Contextual keywords are kept to a minimum.
- The canonical formatter's output must parse back to the same AST.
- Units use square brackets (`100[cm]`, `f64[m]`); `<…>` is only for generic arguments
  ([`specs/units.md`](../../specs/units.md#why-square-brackets)).

## What is generated from it

The `crates/grammar` tool checks the grammar and generates:

- the parser's grammar tests (the parser in `crates/syntax` is hand-written against this file);
- the tree-sitter grammar in `editors/`;
- GBNF and Lark grammars for constrained decoding by models
  ([`specs/agent-interface.md`](../../specs/agent-interface.md)).

# moddil-grammar

The tool that owns [`reference/grammar/moddil.ebnf`](../../reference/grammar/README.md).

**Status:** not started; written alongside the grammar, before the parser.

## Does

- Parses the EBNF dialect.
- **Checks** that the grammar is LL(k) with a small k and unambiguous, reporting conflicts with the
  rules and token sequences involved.
- **Generates:**
  - grammar tests for `moddil-syntax` (sentences that must parse, and near-misses that must not);
  - the tree-sitter grammar for `editors/`;
  - GBNF and Lark grammars for constrained decoding
    ([`specs/agent-interface.md`](../../specs/agent-interface.md)).

Runs in CI: a grammar change that breaks LL(k) or desynchronises a generated file fails the build.

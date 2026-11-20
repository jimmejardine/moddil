# moddil-grammar

The tool that owns [`reference/grammar/moddil.ebnf`](../../reference/grammar/README.md).

**Status:** the checker works and gates CI; the generators come next.

```
cargo run -p moddil-grammar                  # checks reference/grammar/moddil.ebnf at k = 3
cargo run -p moddil-grammar -- other.ebnf -k 2
```

## Does

- **Parses the EBNF dialect** (`src/parse.rs`), including flags, guards and greedy optionals
  ([`reference/grammar/README.md`](../../reference/grammar/README.md)).
- **Expands flags** (`src/expand.rs`): every rule reachable from `file` gets one copy per flag set it
  is used with, and groups and quantifiers become helper rules, so each decision is a choice between
  productions.
- **Checks LL(k)** (`src/ll.rs`), which also proves the grammar unambiguous. Each decision's
  productions are explored as a parser would see them (positions plus a stack of return points,
  following every caller at the end of a rule) and compared token by token up to `k`. It reports
  each conflict with the token sequence and the productions involved, unreachable rules, undefined
  rules and left recursion. The approximations only ever add conflicts, so a pass is a proof.
- **The CI gate:** `tests/reference_grammar.rs` fails the build unless the checked-in grammar is
  LL(3), and checks that it is not LL(1), so a checker that accepts everything cannot pass.
- **Will generate:**
  - grammar tests for `moddil-syntax` (sentences that must parse, and near-misses that must not);
  - the tree-sitter grammar for `editors/`;
  - GBNF and Lark grammars for constrained decoding
    ([`specs/agent-interface.md`](../../specs/agent-interface.md)).

Runs in CI: a grammar change that breaks LL(k) or desynchronises a generated file fails the build.

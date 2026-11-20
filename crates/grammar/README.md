# moddil-grammar

The tool that owns [`reference/grammar/moddil.ebnf`](../../reference/grammar/README.md): it checks
the grammar is LL(3) and generates the parser corpus and the grammars other tools use.

**Status:** the checker and every generator work, and CI checks both.

```
cargo run -p moddil-grammar                        # check reference/grammar/moddil.ebnf at k = 3
cargo run -p moddil-grammar -- other.ebnf -k 2     # another file, another k
cargo run --release -p moddil-grammar -- generate  # write the generated files (--check: verify)
```

## Does

- **Parses the EBNF dialect** (`src/parse.rs`), including flags, guards and greedy optionals, and
  the character-level subset of token rules (`src/tokens.rs`)
  ([`reference/grammar/README.md`](../../reference/grammar/README.md)).
- **Expands flags** (`src/expand.rs`): every rule reachable from `file` gets one copy per flag set it
  is used with, and groups and quantifiers become helper rules, so each decision is a choice between
  productions.
- **Checks LL(k)** (`src/ll.rs`), which also proves the grammar unambiguous. Each decision's
  productions are explored as a parser would see them (positions plus a stack of return points,
  following every caller at the end of a rule) and compared token by token up to `k`. It reports
  each conflict with the token sequence and the productions involved, unreachable rules, undefined
  rules and left recursion. The approximations only ever add conflicts, so a pass is a proof.
- **Recognises** (`src/recognise.rs`): a deterministic LL(3) recogniser that chooses productions as
  the hand-written parser must. It is the executable definition of which token sequences the grammar
  accepts.
- **Generates** (`generate`):

  | Module | Writes |
  |---|---|
  | `src/corpus.rs` | `reference/grammar/tests/valid.moddil` (cases that together use every reachable production, judged by the recogniser) and `invalid.moddil` (single-token edits it rejects) |
  | `src/gbnf.rs` | `reference/grammar/generated/moddil.gbnf`, for constrained decoding |
  | `src/lark.rs` | `reference/grammar/generated/moddil.lark`, for Python tooling |
  | `src/treesitter.rs` | `editors/tree-sitter-moddil/grammar.js` (the C scanner beside it is hand-written) |

## Tests and CI

- `tests/reference_grammar.rs`: the checked-in grammar is LL(3), and not LL(1), so a checker that
  accepts everything cannot pass.
- A unit test reads the generated GBNF back and runs every valid case and every sample through an
  Earley recogniser (`src/gbnf_check.rs`).
- `python tools/ci.py grammars` (the `grammars` workflow): the generated files are current
  (`generate --check`), and Lark and tree-sitter accept every valid case and sample
  (`tools/check_grammars.py`).

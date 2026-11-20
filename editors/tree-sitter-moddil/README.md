# tree-sitter-moddil

The tree-sitter grammar for Moddil, for highlighting, folding and structural editing.

| File | Made by |
|---|---|
| `grammar.js` | **Generated** by `cargo run --release -p moddil-grammar -- generate` from [`reference/grammar/moddil.ebnf`](../../reference/grammar/README.md). Do not edit it. |
| `src/scanner.c` | **Hand-written**: the four tokens that are not regular languages (nested block comments, raw strings, `"""` blocks, and tagged literals whose holes nest). Its token order must match `externals` in `grammar.js`. |
| `tree-sitter.json` | Hand-written configuration. |
| `src/parser.c` and the rest of `src/` | Built by `tree-sitter generate`; not committed. |

```
cd editors/tree-sitter-moddil
npx tree-sitter-cli@0.26.13 generate
npx tree-sitter-cli@0.26.13 parse path/to/file.mdl
```

## How it differs from the reference grammar

- It is generated from the rules as written, **without expanding flags**, so it accepts a little
  more than Moddil (as an editor wants: it never rejects code the compiler would explain better).
- tree-sitter allows only the start rule to match the empty string, so a rule `R ::= X?` becomes
  `R ::= X` with every use optional, and other rules that can match nothing are inlined.
- tree-sitter looks one token ahead, where the grammar uses three, so the places it settles with
  more lookahead, a greedy optional or a flag are declared as GLR `conflicts` (14 of them).

CI parses every valid corpus case and the hand-written samples with it (`python tools/ci.py grammars`).

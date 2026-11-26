# Grammar

The **one grammar source** for Moddil: [`moddil.ebnf`](moddil.ebnf), written in a small EBNF dialect
of our own ([`specs/definition.md`: One grammar source](../../specs/definition.md#one-grammar-source)).

**Status:** written for **all decided syntax**, not only what M0 implements, and **checked LL(3)**
by [`moddil-grammar`](../../crates/grammar/README.md) on every CI run: every rule is reachable, and
every decision is settled by at most three tokens of lookahead, by flags, or by a marked greedy
optional. Check it locally with `cargo run -p moddil-grammar`.

## Notation

W3C-style EBNF, in one file with a **Tokens** section and a **Syntax** section:

```
expr_stmt   ::= expr ';'?
call        ::= primary '(' (arg (',' arg)* ','?)? ')'
IDENT       ::= XID_START XID_CONTINUE*        # UPPERCASE names are token rules
```

| Operator | Meaning |
|---|---|
| `a b` | sequence |
| `a \| b` | choice |
| `a?` `a*` `a+` | optional, zero or more, one or more |
| `a??` | greedy optional: taken whenever it can start (Rust's longest reading; used in four places) |
| `( … )` | grouping |
| `'…'` | a terminal |
| `[a-z]`, `A - B` | character class, exclusion (Tokens section only) |

### Flags

One extension expresses Rust's restrictions without writing the expression rules several times:

| Flag | Meaning |
|---|---|
| `ns` | No struct or record literal and no bare block: set in heads that a block follows (`if`, `while`, `match`, `for`, handler values, contracts); on a type, no anonymous record at its start (a `where` clause's, before a body) |
| `stmt` | The expression starts a statement, so it may not begin with a block-like expression |
| `cond` | A `let` condition may appear (`if` and `while` heads) |

| Form | Meaning |
|---|---|
| `name` | No flags; delimiters `( )`, `[ ]`, `{ }` always reset flags |
| `name@` | Inherit every current flag (the leftmost operand) |
| `name@ns` | Inherit only `ns` and `cond` (the other operands) |
| `name+ns` | Start with a flag set (several: `name+ns+cond`) |
| `!ns alt` / `?cond alt` | An alternative removed while a flag is set / present only while it is set |

Flags expand to finite copies of the rules, so the grammar stays checkable as LL(k).

### Tokens

Token rules use a character-level subset, so every generator can derive exact tokens from them:
quoted literals, classes such as `[a-z]` and `[^"\\]`, the built-in classes `XID_START` and
`XID_CONTINUE` (Unicode UAX #31), references to other token rules (fragments), and `A - B` (A,
except the literals B lists). Two markers:

| Marker | Meaning |
|---|---|
| `%skip` | Dropped between syntax tokens: whitespace and comments |
| `%scanner` | Not a regular language: nested block comments, raw strings, `"""` blocks and tagged literals. The exact rule is the comment above it, and each generator implements it natively |

Token rules that syntax rules name are tokens; the others are fragments. A rule that only lists
literals (`KEYWORD`, `PUNCT`) is a choice of terminals.

## How it is written

1. **[`inventory.md`](inventory.md)** lists every decided construct, the spec that decides it and an
   example. It is the grammar's checklist.
2. Conflicts and undecided spellings it finds are settled in
   [`specs/syntax.md`](../../specs/syntax.md) (its open questions, while one is open).
3. Only then does a construct go into `moddil.ebnf`, so the grammar holds decided syntax only.

## Rules

- **LL(k) with a small k, and unambiguous.** Contextual keywords are kept to a minimum.
- The canonical formatter's output must parse back to the same AST.
- Units use square brackets (`100[cm]`, `f64[m]`); `<…>` is only for generic arguments
  ([`specs/units.md`](../../specs/units.md#why-square-brackets)).

## What is generated from it

`cargo run -p moddil-grammar -- generate` writes every file below; CI fails if any is out of date
(`generate --check`) and runs each through its own tool (`python tools/ci.py grammars`).

| File | What | Checked by |
|---|---|---|
| [`tests/valid.moddil`](tests/valid.moddil) | Valid cases that together use **every production** the parser can reach | The LL(3) recogniser in `moddil-grammar`; later `moddil-syntax` |
| [`tests/invalid.moddil`](tests/invalid.moddil) | Near-misses: single-token edits of valid cases that the parser must reject | The recogniser |
| [`generated/moddil.gbnf`](generated/moddil.gbnf) | GBNF (llama.cpp) for constrained decoding, flags exact | An Earley recogniser in `moddil-grammar`'s tests |
| [`generated/moddil.lark`](generated/moddil.lark) | Lark, for Python tooling and constrained decoding, flags exact | Lark 1.2.2 (`tools/check_grammars.py`) |
| [`../../editors/tree-sitter-moddil/grammar.js`](../../editors/tree-sitter-moddil/grammar.js) | tree-sitter, from the rules as written (a superset, as editors want), with a hand-written C scanner | tree-sitter CLI 0.26.13 |

[`tests/samples.moddil`](tests/samples.moddil) is written by hand: the tour's program and lexing edge cases
in real spelling (the generated cases are spaced token by token). Every generated grammar must
accept it too.

Each corpus file is a list of cases, each a whole file after a `// ---` line. The recogniser is the
executable definition of what the grammar accepts: it chooses productions as the hand-written parser
must (three tokens of lookahead, flags, greedy optionals taken).

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
| `ns` | No struct or record literal and no bare block: set in heads that a block follows (`if`, `while`, `match`, `for`, handler values, contracts) |
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

## How it is written

1. **[`inventory.md`](inventory.md)** lists every decided construct, the spec that decides it and an
   example. It is the grammar's checklist.
2. Conflicts and undecided spellings it finds become open questions in
   [`specs/syntax.md`](../../specs/syntax.md#open-questions) and are decided there.
3. Only then does a construct go into `moddil.ebnf`, so the grammar holds decided syntax only.

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

# moddil-syntax

Lexer, parser and syntax tree for Moddil source.

**Status:** the lexer and the parser cover the **whole grammar**; the typed AST covers what the
compiler reads, and grows with it ([roadmap](../../specs/roadmap.md#milestones)).

```rust
let parse = moddil_syntax::parse(text);
parse.errors();     // lexical and syntax errors, in source order
parse.syntax();     // the lossless tree: its text is exactly `text`
parse.tree();       // the typed AST (`ast::SourceFile`)
```

`cargo run -p moddil-syntax --example dump -- file.moddil` prints a file's tree and errors.

## Design

- **Lexer** (`src/lexer.rs`): the Tokens section of
  [`moddil.ebnf`](../../reference/grammar/README.md), by hand, including the four scanner tokens
  (nested block comments, raw strings, `"""` blocks, tagged literals whose holes nest). UTF-8, UAX #31
  identifiers that must be in NFC, bidirectional control characters rejected
  ([`specs/definition.md`: Lexical structure](../../specs/definition.md#lexical-structure)). It never
  fails: bad input becomes an `ERROR` token and an error.
- **Token kinds are generated** from the grammar (`src/generated/kinds.rs`, by `moddil-grammar
  generate`): every keyword and punctuation terminal, the `T![…]` macro, and the table of compound
  punctuation. Node kinds are listed in `src/kinds.rs`.
- **Joint punctuation.** The lexer emits punctuation one character at a time, each marked as joined
  to the next or not. The parser reads compounds (`>>=`, `&&`, `..=`) by maximal munch, as the
  grammar's lexer would, and splits them where the grammar says: `>>`, `>=` and `>>=` closing generic
  arguments, and `&&` as two `&` wherever a `&` begins an expression, type, pattern or effect.
- **Tree** (`src/tree.rs`): our own lossless tree. Immutable **green** nodes (kind, length,
  children, with tokens interned) and **red** cursors (`SyntaxNode`, `SyntaxToken`) that know their
  parent and offset. Comments directly above an item, field, arm or `let` (no blank line between)
  belong to it.
- **Parser** (`src/parser.rs`, `src/grammar/`): recursive descent, one function per grammar rule,
  never more than three tokens of lookahead. The grammar's flags (`ns`, `stmt`, `cond`) are a value
  passed down the expression functions. Greedy optionals (`a??`) are taken when taking them is
  viable within three tokens, as the recogniser does. Parsing emits events that a builder turns into
  the tree, putting trivia back.
- **Recovery:** a missing token is reported and not inserted; an unexpected one is wrapped in an
  `ERROR` node, and lists and blocks resume at `;`, `}`, closing delimiters and item keywords. One
  error per position. Every loop makes progress, so the parser always finishes.
- **Typed AST** (`src/ast.rs`): views over the tree with accessors that return `Option`.

## How the parser is kept in step with the grammar

`tests/corpus.rs`:

- every case in [`valid.moddil`](../../reference/grammar/tests/valid.moddil) (which together use every
  production) and both [samples](../../reference/grammar/tests/samples.moddil) parse without errors;
- every near-miss in [`invalid.moddil`](../../reference/grammar/tests/invalid.moddil) is rejected;
- **differential:** random token edits of the valid corpus, half valid and half invalid, each judged
  by the grammar's own LL(3) recogniser (`moddil_grammar::edited_cases`); the parser must give the
  same verdict on every one. 1,000 in `cargo test`; 10,000 in release in CI's `grammars` step
  (`MODDIL_EDITS` and `MODDIL_SEED` change the count and the edits);
- every tree is lossless, also for every prefix of the samples and for random text.

`tests/snapshots.rs` records the trees of the samples and of focused cases (precedence, flags,
splitting, recovery) with `insta`; review changes with `cargo insta review`.

## Used by

`moddil-sema` and the rest of the compiler, and **`moddil-refinterp`**, which shares only this crate
with the compiler.

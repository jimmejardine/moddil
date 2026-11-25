# moddil-fmt

The canonical formatter for Moddil source ([`specs/toolchain.md`: Formatter](../../specs/toolchain.md#formatter)).

**Status:** the formatter, run by `moddil fmt [--check]` and by the language server's document
formatting (`moddil lsp`). The repository's own Moddil sources are kept in its layout:
`tools/ci.py`'s `fmt` step checks them with `moddil fmt --check`.

```rust
let text = moddil_fmt::format(source)?;   // canonical, with `\n` line endings
```

`cargo run -p moddil-fmt --example fmt -- file.moddil` prints a file formatted.

## The layout

There is **no line width and no setting**: the layout is a function of the syntax tree and its
comments, so the same code always prints the same way. Editors wrap long lines for display.

- **Lines break only where the structure says:**
  - between items, impl and trait members, fields, variants, statements and match arms;
  - inside a block with statements, or with something inside it that breaks;
  - before a function's `where`, `requires` and `ensures` clauses.
- **Function, `test`, `property` and `bench` bodies** always start a new line at `{` (an empty
  one stays `{}`).
- **Signatures:**
  - These stay on the signature's line: name, generics, params, `-> T`, and `resume`, `from`
    and `with`.
  - `where`, `requires` and `ensures` each go on a line of their own, indented, with the
    body's `{` on the next line.
- **Expression blocks** (`if`, `try`, `with`, loops, closures) stay on one line when they hold
  only a value and nothing inside breaks. If one branch of an `if … else` is on lines, every
  branch is.
- **`unsafe("reason") {`** always ends its line.
- **Method chains** go onto lines when two or more of their calls take arguments:
  - The start of the chain, and any argument-less calls right after it, stay on the first
    line.
  - Every later method call goes on a line of its own, indented once.
- **Match arms:**
  - An arm whose body is a block takes no comma after it; any other arm does.
  - The `=>` of arms on consecutive lines of one match line up.
- **Lists** (arguments, params, generics, tuples, arrays, struct literals, use lists) are on
  one line, without a trailing comma.
  - A line comment inside one is how a writer breaks it: the comment ends its line and nothing
    else breaks.
  - The list then opens on its own line, with a trailing comma, and its closer on a line of
    its own. A bare `//` does this where there's nothing to say.
- **Comments:**
  - A trailing comment stays on the line of the code it follows. Those on consecutive lines are
    aligned, two spaces past the longest line.
  - A comment on a line of its own stays on its own line.
  - Doc comments are kept exactly as written.
- **Blank lines:** one is kept wherever the source has any between line-separated things, but
  never at the start or end of a block. The file ends with one `\n`.
- **Spacing within a line** comes from a table of token pairs (`space.rs`); where it has no
  rule, the source's spacing is kept, as one space or none.

## Safety

A file with syntax errors isn't formatted. Before `format` returns, it reparses its output and
checks that every token (but the trailing commas the layout adds or drops) and every comment is
still there, in order. If not, that's a bug in the formatter, reported as `Error::Changed`, and
the source is left alone.

## Tests

- `tests/rules.rs` has a snapshot for each rule of the layout above, on a small input written
  against it.
- `tests/corpus.rs` formats every case of the grammar's corpus (which together use every
  production) and its samples, and every `.moddil` file of the prelude, the mods, the tour, the
  examples and the conformance suite. Each one formats, and formatting it again changes nothing.

## Design

- `print.rs` walks the tree and says where the structure breaks lines and indents.
- `out.rs` builds the lines: the spaces within them, which line a comment ends, the blank lines
  kept, and the aligned columns.
- `space.rs` is the table of token pairs.

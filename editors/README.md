# Editors

Editor support files ([`specs/toolchain.md`: Editor support](../specs/toolchain.md#editor-support)).

**Status:** the tree-sitter grammar exists; editor extensions come later.

## What goes here

- **[`tree-sitter-moddil/`](tree-sitter-moddil/README.md)**: the tree-sitter grammar. `grammar.js` is
  generated from [`reference/grammar/moddil.ebnf`](../reference/grammar/README.md) by
  `crates/grammar` and never edited by hand; the external scanner `src/scanner.c` is hand-written.
- **Editor extensions**, in priority order: VS Code, Zed, Neovim, then JetBrains. All of them talk to
  the `moddil` language server (LSP); the extensions stay thin.

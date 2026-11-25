# Editors

Editor support files ([`specs/toolchain.md`: Editor support](../specs/toolchain.md#editor-support)).

**Status:** the tree-sitter grammar and a VS Code extension exist; the other editors' come later.

## What goes here

- **[`tree-sitter-moddil/`](tree-sitter-moddil/README.md)**: the tree-sitter grammar. `grammar.js` is
  generated from [`reference/grammar/moddil.ebnf`](../reference/grammar/README.md) by
  `crates/grammar` and never edited by hand; the external scanner `src/scanner.c` is hand-written.
- **[`vscode/`](vscode/README.md)**: the VS Code extension, a TextMate grammar for colouring and
  `moddil lsp` for the rest.
- **Editor extensions** still to come, in priority order: Zed, Neovim, then JetBrains. All of them
  talk to the `moddil` language server (LSP); the extensions stay thin.

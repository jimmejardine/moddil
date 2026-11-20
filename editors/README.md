# Editors

Editor support files ([`specs/toolchain.md`: Editor support](../specs/toolchain.md#editor-support)).

**Status:** empty until the grammar exists.

## What goes here

- **The tree-sitter grammar**, generated from [`reference/grammar/moddil.ebnf`](../reference/grammar/README.md)
  by `crates/grammar`: never edited by hand.
- **Editor extensions**, in priority order: VS Code, Zed, Neovim, then JetBrains. All of them talk to
  the `moddil` language server (LSP); the extensions stay thin.

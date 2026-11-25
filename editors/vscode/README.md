# Moddil for VS Code

A thin extension: colouring from a TextMate grammar
([`syntaxes/moddil.tmLanguage.json`](syntaxes/moddil.tmLanguage.json)), and everything else from
the `moddil` language server ([`crates/lsp`](../../crates/lsp/src/lib.rs)), started as
`moddil lsp`: diagnostics as you type, hover, go to definition, inlay hints and completion.

## Using it

1. Build `moddil` and put it on your `PATH` (`cargo install --path crates/moddil`), or set
   `moddil.server.path` to it.
2. In this directory, `npm install`; then open the directory in VS Code and press F5 for a window
   with the extension, or run `npx @vscode/vsce package` for a `.vsix` to install.

It is not published to the marketplace.

## What it holds

- `package.json`: the language (`.moddil`), its grammar and configuration, and the setting
  `moddil.server.path`.
- `language-configuration.json`: comments, brackets and indentation.
- `syntaxes/moddil.tmLanguage.json`: keywords, types, numbers, comments, attributes, and strings,
  with `fmt"…"`'s holes, other tagged literals (`re"…"`, taken as written) and `"""` blocks.
- `extension.js`: starts the language server, and tells it when a manifest or a Moddil file
  changes.

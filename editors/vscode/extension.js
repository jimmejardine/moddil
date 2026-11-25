// The Moddil extension: starts `moddil lsp` (the command `moddil.server.path` names) as the
// language server for `.moddil` files.

const vscode = require("vscode");
const { LanguageClient } = require("vscode-languageclient/node");

let client;

function activate(context) {
  const command = vscode.workspace.getConfiguration("moddil").get("server.path") || "moddil";
  const server = { command, args: ["lsp"] };
  client = new LanguageClient(
    "moddil",
    "Moddil",
    { run: server, debug: server },
    {
      documentSelector: [{ scheme: "file", language: "moddil" }],
      synchronize: {
        // A manifest or a file that comes or goes: the server reads the programs again.
        fileEvents: vscode.workspace.createFileSystemWatcher("**/{moddil.toml,*.moddil}"),
      },
    },
  );
  context.subscriptions.push(client);
  return client.start();
}

function deactivate() {
  return client ? client.stop() : undefined;
}

module.exports = { activate, deactivate };

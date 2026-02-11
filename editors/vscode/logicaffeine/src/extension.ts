import * as path from "path";
import { ExtensionContext, workspace } from "vscode";
import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
  TransportKind,
} from "vscode-languageclient/node";

let client: LanguageClient;

export function activate(context: ExtensionContext) {
  const config = workspace.getConfiguration("logicaffeine");
  const serverPath = config.get<string>("lsp.path", "logicaffeine-lsp");
  const serverArgs = config.get<string[]>("lsp.args", ["--stdio"]);

  const serverOptions: ServerOptions = {
    command: serverPath,
    args: serverArgs,
    transport: TransportKind.stdio,
  };

  const clientOptions: LanguageClientOptions = {
    documentSelector: [{ scheme: "file", language: "logicaffeine" }],
    synchronize: {
      fileEvents: workspace.createFileSystemWatcher("**/*.lg"),
    },
  };

  client = new LanguageClient(
    "logicaffeine",
    "LogicAffeine Language Server",
    serverOptions,
    clientOptions,
  );

  client.start();
}

export function deactivate(): Thenable<void> | undefined {
  if (!client) {
    return undefined;
  }
  return client.stop();
}

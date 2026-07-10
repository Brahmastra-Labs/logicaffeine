import * as fs from "fs";
import {
  commands,
  ExtensionContext,
  languages,
  LanguageStatusItem,
  LanguageStatusSeverity,
  OutputChannel,
  window,
  workspace,
} from "vscode";
import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
  State,
  TransportKind,
} from "vscode-languageclient/node";
import { registerCommands } from "./commands";
import { resolveServer, Resolution } from "./serverResolver";

const LANGUAGE_ID = "logicaffeine";

let client: LanguageClient | undefined;
let statusItem: LanguageStatusItem;

export async function activate(context: ExtensionContext) {
  const outputChannel = window.createOutputChannel("LOGOS Language Server");
  context.subscriptions.push(outputChannel);

  statusItem = languages.createLanguageStatusItem("logicaffeine.server", {
    language: LANGUAGE_ID,
  });
  statusItem.name = "LOGOS Language Server";
  statusItem.command = {
    title: "Show Log",
    command: "logicaffeine.showServerLog",
  };
  context.subscriptions.push(statusItem);

  context.subscriptions.push(
    commands.registerCommand("logicaffeine.showServerLog", () => outputChannel.show()),
    commands.registerCommand("logicaffeine.restartServer", async () => {
      try {
        if (client) {
          await client.restart();
        } else {
          await startServer(context, outputChannel);
        }
      } catch (err) {
        // A bogus `logicaffeine.lsp.path` leaves the client in `startFailed`,
        // where restart()'s internal stop() throws. The extension host must
        // stay alive — swallow it (already surfaced via the server log).
        outputChannel.appendLine(`server restart failed: ${err}`);
      }
    }),
  );
  registerCommands(context);

  await startServer(context, outputChannel);
}

async function startServer(context: ExtensionContext, outputChannel: OutputChannel) {
  const config = workspace.getConfiguration("logicaffeine");

  const resolution = resolveServer({
    platform: process.platform,
    arch: process.arch,
    configuredPath: config.get<string>("lsp.path"),
    extensionRoot: context.extensionPath,
    exists: (p) => fs.existsSync(p),
    canExecute: (p) => {
      try {
        fs.accessSync(p, fs.constants.X_OK);
        return true;
      } catch {
        return false;
      }
    },
  });

  if (resolution.kind === "error") {
    reportServerError(resolution.message);
    return;
  }

  setStatus("starting", resolution);

  const serverOptions: ServerOptions = {
    command: resolution.command,
    args: config.get<string[]>("lsp.args", []),
    transport: TransportKind.stdio,
  };

  const clientOptions: LanguageClientOptions = {
    documentSelector: [{ scheme: "file", language: LANGUAGE_ID }],
    synchronize: {
      fileEvents: workspace.createFileSystemWatcher("**/*.{lg,md}"),
      // Forward logicaffeine.* changes so the server picks up the flycheck
      // toggle (and future settings) live.
      configurationSection: "logicaffeine",
    },
    outputChannel,
  };

  client = new LanguageClient(
    "logicaffeine",
    "LOGOS Language Server",
    serverOptions,
    clientOptions,
  );

  client.onDidChangeState((event) => {
    switch (event.newState) {
      case State.Starting:
        setStatus("starting", resolution);
        break;
      case State.Running:
        setStatus("running", resolution);
        break;
      case State.Stopped:
        setStatus("stopped", resolution);
        break;
    }
  });

  try {
    await client.start();
  } catch (err) {
    const detail = err instanceof Error ? err.message : String(err);
    reportServerError(
      `The LOGOS language server failed to start (${describeSource(resolution)}: ` +
        `'${resolution.command}'): ${detail}`,
    );
  }
}

function reportServerError(message: string) {
  statusItem.severity = LanguageStatusSeverity.Error;
  statusItem.text = "LOGOS: server unavailable";
  statusItem.detail = message;
  window
    .showErrorMessage(message, "Open Settings", "Show Log")
    .then((choice) => {
      if (choice === "Open Settings") {
        commands.executeCommand("workbench.action.openSettings", "logicaffeine.lsp");
      } else if (choice === "Show Log") {
        commands.executeCommand("logicaffeine.showServerLog");
      }
    });
}

function setStatus(
  state: "starting" | "running" | "stopped",
  resolution: Exclude<Resolution, { kind: "error" }>,
) {
  statusItem.busy = state === "starting";
  switch (state) {
    case "starting":
      statusItem.severity = LanguageStatusSeverity.Information;
      statusItem.text = "LOGOS: starting…";
      break;
    case "running":
      statusItem.severity = LanguageStatusSeverity.Information;
      statusItem.text = "LOGOS";
      break;
    case "stopped":
      statusItem.severity = LanguageStatusSeverity.Error;
      statusItem.text = "LOGOS: server stopped";
      break;
  }
  statusItem.detail = `${describeSource(resolution)}: ${resolution.command}`;
}

function describeSource(resolution: Resolution): string {
  switch (resolution.kind) {
    case "settings":
      return "from logicaffeine.lsp.path";
    case "bundled":
      return "bundled server";
    case "path":
      return "from PATH";
    case "error":
      return "unresolved";
  }
}

export function deactivate(): Thenable<void> | undefined {
  return client?.stop();
}

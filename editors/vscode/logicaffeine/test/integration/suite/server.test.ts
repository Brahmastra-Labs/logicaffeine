import * as assert from "assert";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import * as vscode from "vscode";

/**
 * Real-server round trips. CI builds a debug `logicaffeine-lsp` and passes its
 * path via LOGICAFFEINE_LSP_PATH; without it the server suite self-skips
 * (activation and command registration are still covered elsewhere).
 */
const SERVER_PATH = process.env.LOGICAFFEINE_LSP_PATH;

const BROKEN_SOURCE = "## Main\n    Let be.\n";

async function setServerPath(value: string | undefined): Promise<void> {
  await vscode.workspace
    .getConfiguration("logicaffeine")
    .update("lsp.path", value, vscode.ConfigurationTarget.Global);
}

async function pollUntil(condition: () => boolean, timeoutMs: number): Promise<boolean> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (condition()) return true;
    await new Promise((r) => setTimeout(r, 200));
  }
  return condition();
}

describe("commands", () => {
  it("registers every contributed logicaffeine command — no dead-end code lenses", async () => {
    const all = await vscode.commands.getCommands(true);
    // The server's code lenses emit run/verify/prove/checkProof; the rest are
    // editor-side. Every contributed id must be registered.
    for (const id of [
      "logicaffeine.run",
      "logicaffeine.verify",
      "logicaffeine.prove",
      "logicaffeine.checkProof",
      "logicaffeine.setLicense",
      "logicaffeine.restartServer",
      "logicaffeine.showServerLog",
    ]) {
      assert.ok(all.includes(id), `command ${id} must be registered`);
    }
  });

  it("contributed commands match registered commands (package.json ↔ code)", async () => {
    const packageJson = vscode.extensions.getExtension("brahmastra-labs.logicaffeine")!
      .packageJSON as { contributes: { commands: Array<{ command: string }> } };
    const contributed = packageJson.contributes.commands.map((c) => c.command);
    const registered = await vscode.commands.getCommands(true);
    for (const id of contributed) {
      assert.ok(registered.includes(id), `contributed but unregistered: ${id}`);
    }
  });

  it("Run spawns a largo task from the project root", async function () {
    this.timeout(60_000);
    const folders = vscode.workspace.workspaceFolders;
    assert.ok(folders && folders.length > 0, "the fixture workspace must be open");

    const mainLg = vscode.Uri.joinPath(folders![0].uri, "src", "main.lg");
    const started = new Promise<vscode.Task>((resolve) => {
      const listener = vscode.tasks.onDidStartTask((event) => {
        listener.dispose();
        resolve(event.execution.task);
      });
    });

    await vscode.commands.executeCommand("logicaffeine.run", mainLg.toString());

    const task = await Promise.race([
      started,
      new Promise<undefined>((r) => setTimeout(() => r(undefined), 30_000)),
    ]);
    assert.ok(task, "executing logicaffeine.run must start a task");
    assert.strictEqual(task!.definition.type, "largo");
    assert.deepStrictEqual(task!.definition.args, ["run", "--interpret"]);
  });
});

describe("language server", function () {
  this.timeout(120_000);

  before(function () {
    if (!SERVER_PATH) {
      this.skip();
    }
    assert.ok(
      fs.existsSync(SERVER_PATH!),
      `LOGICAFFEINE_LSP_PATH points at '${SERVER_PATH}', which does not exist`,
    );
  });

  after(async () => {
    await setServerPath(undefined);
  });

  it("publishes socratic diagnostics for a broken document", async () => {
    // A real on-disk file (file:// URI) — the realistic path a user's broken
    // source takes; an untitled/in-memory buffer is an edge case the server
    // needn't analyze. Open it BEFORE (re)starting the server so the client
    // syncs it to the freshly-started debug server via `didOpen` on init —
    // opening after the restart races the client's readiness and can be dropped.
    const brokenPath = path.join(
      fs.mkdtempSync(path.join(os.tmpdir(), "lsp-diag-")),
      "broken.lg",
    );
    fs.writeFileSync(brokenPath, BROKEN_SOURCE);
    const document = await vscode.workspace.openTextDocument(vscode.Uri.file(brokenPath));
    await vscode.window.showTextDocument(document);

    await setServerPath(SERVER_PATH);
    await vscode.commands.executeCommand("logicaffeine.restartServer");

    const gotDiagnostics = await pollUntil(
      () => vscode.languages.getDiagnostics(document.uri).length > 0,
      60_000,
    );
    assert.ok(gotDiagnostics, "the server must publish diagnostics for broken source");

    const diagnostics = vscode.languages.getDiagnostics(document.uri);
    for (const diagnostic of diagnostics) {
      assert.strictEqual(diagnostic.source, "logicaffeine");
      assert.ok(diagnostic.message.length > 0, "diagnostics carry explanations");
    }
  });

  it("survives a bogus server path without crashing the extension host", async () => {
    await setServerPath("/nonexistent/logicaffeine-lsp");
    // Must not throw or leave an unhandled rejection behind.
    await vscode.commands.executeCommand("logicaffeine.restartServer");

    const extension = vscode.extensions.getExtension("brahmastra-labs.logicaffeine");
    assert.ok(extension?.isActive, "the extension stays alive after a resolution failure");

    const all = await vscode.commands.getCommands(true);
    assert.ok(
      all.includes("logicaffeine.restartServer"),
      "commands remain registered after a failed start",
    );
  });
});

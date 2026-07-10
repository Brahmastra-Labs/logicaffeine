import * as assert from "assert";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import * as vscode from "vscode";

describe("extension activation", () => {
  it("is installed under the expected id", () => {
    const extension = vscode.extensions.getExtension("brahmastra-labs.logicaffeine");
    assert.ok(extension, "brahmastra-labs.logicaffeine must be present");
  });

  it("activates when a LOGOS document opens", async () => {
    const extension = vscode.extensions.getExtension("brahmastra-labs.logicaffeine");
    assert.ok(extension);

    const document = await vscode.workspace.openTextDocument({
      language: "logicaffeine",
      content: "## Main\n    Let x be 5.\n    Show x.\n",
    });
    await vscode.window.showTextDocument(document);

    // Activation is event-driven; wait for it rather than racing it.
    const deadline = Date.now() + 30_000;
    while (!extension.isActive && Date.now() < deadline) {
      await new Promise((r) => setTimeout(r, 100));
    }
    assert.ok(extension.isActive, "opening a logicaffeine document must activate the extension");
  });

  it("the BUNDLED server answers diagnostics (vsix install gate)", async function () {
    // Only meaningful against an installed VSIX with a staged binary; the
    // dev-host run has no bin/ and covers the server via LOGICAFFEINE_LSP_PATH.
    if (!process.env.VSIX_GATE) {
      this.skip();
    }
    this.timeout(120_000);

    // Open a REAL on-disk file (a `file://` URI) — the realistic path a user's
    // broken source takes. An untitled/in-memory buffer is an edge case the
    // language server needn't analyze; a saved `.lg` is the actual contract.
    const brokenPath = path.join(
      fs.mkdtempSync(path.join(os.tmpdir(), "vsix-gate-")),
      "broken.lg",
    );
    fs.writeFileSync(brokenPath, "## Main\n    Let be.\n");
    const document = await vscode.workspace.openTextDocument(vscode.Uri.file(brokenPath));
    await vscode.window.showTextDocument(document);

    const deadline = Date.now() + 90_000;
    while (
      vscode.languages.getDiagnostics(document.uri).length === 0 &&
      Date.now() < deadline
    ) {
      await new Promise((r) => setTimeout(r, 250));
    }
    const diagnostics = vscode.languages.getDiagnostics(document.uri);
    assert.ok(
      diagnostics.length > 0,
      "the bundled server must publish diagnostics for broken source",
    );
    assert.strictEqual(diagnostics[0].source, "logicaffeine");
  });
});

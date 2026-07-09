import * as path from "path";
import { commands, ExtensionContext, tasks, Uri, window, workspace } from "vscode";
import { findProjectRoot, largoTask, runModeArgs } from "./largo";
import { getLicense, promptAndStoreLicense } from "./license";

/**
 * The code-lens commands the language server emits (`Run` over `## Main`,
 * `Verify`/`Prove` over `## Theorem`, `Check Proof` over `## Proof`), plus the
 * license command backing Verify. Every id here must be contributed in
 * package.json — the integration suite locks registration so a lens can never
 * dead-end again.
 */
export function registerCommands(context: ExtensionContext) {
  context.subscriptions.push(
    commands.registerCommand("logicaffeine.run", (uri?: string) => runProject(uri)),
    commands.registerCommand("logicaffeine.verify", (uri?: string) =>
      verifyProject(context, uri),
    ),
    commands.registerCommand("logicaffeine.prove", (uri?: string) => proveFile(uri)),
    commands.registerCommand("logicaffeine.checkProof", (uri?: string) => proveFile(uri)),
    commands.registerCommand("logicaffeine.setLicense", () => promptAndStoreLicense(context)),
  );
}

/** The file the command targets: the lens argument, else the active editor. */
async function targetFile(uri?: string): Promise<Uri | undefined> {
  if (uri) {
    return Uri.parse(uri);
  }
  const active = window.activeTextEditor;
  if (active?.document.languageId === "logicaffeine") {
    return active.document.uri;
  }
  window.showErrorMessage("Open a LOGOS file first.");
  return undefined;
}

/** Project-scoped commands anchor at the Largo.toml above the file. */
async function requireProjectRoot(file: Uri): Promise<string | undefined> {
  const root = findProjectRoot(path.dirname(file.fsPath));
  if (!root) {
    const choice = await window.showErrorMessage(
      "This file is not inside a LOGOS project (no Largo.toml found above it). " +
        "Create one with 'largo new', or open the project folder.",
      "How do I install largo?",
    );
    if (choice) {
      commands.executeCommand(
        "vscode.open",
        Uri.parse("https://github.com/Brahmastra-Labs/logicaffeine#installation"),
      );
    }
    return undefined;
  }
  return root;
}

async function saveIfDirty(file: Uri): Promise<void> {
  const doc = workspace.textDocuments.find((d) => d.uri.toString() === file.toString());
  if (doc?.isDirty) {
    await doc.save();
  }
}

async function runProject(uri?: string): Promise<void> {
  const file = await targetFile(uri);
  if (!file) return;
  await saveIfDirty(file);
  const root = await requireProjectRoot(file);
  if (!root) return;

  const mode = workspace.getConfiguration("logicaffeine").get<string>("run.mode", "interpret");
  await tasks.executeTask(
    largoTask({ name: "largo run", args: runModeArgs(mode), cwd: root }),
  );
}

async function verifyProject(context: ExtensionContext, uri?: string): Promise<void> {
  const file = await targetFile(uri);
  if (!file) return;
  await saveIfDirty(file);
  const root = await requireProjectRoot(file);
  if (!root) return;

  const license = await getLicense(context);
  if (!license) {
    const choice = await window.showWarningMessage(
      "Verification (preview) needs a license key (Pro and up).",
      "Set License Key",
      "Run Anyway",
    );
    if (choice === "Set License Key") {
      const stored = await promptAndStoreLicense(context);
      if (!stored) return;
      return verifyProject(context, uri);
    }
    if (choice !== "Run Anyway") return;
  }

  await tasks.executeTask(
    largoTask({
      name: "largo verify",
      args: ["verify"],
      cwd: root,
      env: license ? { LOGOS_LICENSE: license } : undefined,
    }),
  );
}

async function proveFile(uri?: string): Promise<void> {
  const file = await targetFile(uri);
  if (!file) return;
  await saveIfDirty(file);

  // `largo prove <file>` is file-scoped; run from the project root when one
  // exists so relative imports resolve, else from the file's directory.
  const root = findProjectRoot(path.dirname(file.fsPath)) ?? path.dirname(file.fsPath);
  const args = ["prove", file.fsPath];
  if (workspace.getConfiguration("logicaffeine").get<boolean>("prove.trace", false)) {
    args.push("--trace");
  }

  await tasks.executeTask(largoTask({ name: "largo prove", args, cwd: root }));
}

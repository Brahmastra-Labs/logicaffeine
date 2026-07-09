import * as fs from "fs";
import { ShellExecution, Task, TaskDefinition, TaskScope, workspace, WorkspaceFolder } from "vscode";
import { findProjectRoot as findProjectRootPure, runModeArgs } from "./largoCore";

export { runModeArgs };

/** `findProjectRoot` against the real filesystem. */
export function findProjectRoot(startDir: string): string | undefined {
  return findProjectRootPure(startDir, fs.existsSync);
}

/** The `largo` binary: the `logicaffeine.largo.path` setting, else PATH. */
export function largoBinary(): string {
  const configured = workspace
    .getConfiguration("logicaffeine")
    .get<string>("largo.path", "largo")
    .trim();
  return configured.length > 0 ? configured : "largo";
}

export interface LargoTaskSpec {
  /** Display name in the terminal panel, e.g. "largo run". */
  name: string;
  args: string[];
  cwd: string;
  env?: Record<string, string>;
}

/** Build a VSCode Task that runs `largo` in a dedicated terminal panel. */
export function largoTask(spec: LargoTaskSpec): Task {
  const definition: TaskDefinition = { type: "largo", args: spec.args };
  const folder: WorkspaceFolder | TaskScope =
    workspace.workspaceFolders?.[0] ?? TaskScope.Workspace;

  const task = new Task(
    definition,
    folder,
    spec.name,
    "largo",
    new ShellExecution(largoBinary(), spec.args, { cwd: spec.cwd, env: spec.env }),
    [],
  );
  task.presentationOptions = { clear: true, showReuseMessage: false };
  return task;
}

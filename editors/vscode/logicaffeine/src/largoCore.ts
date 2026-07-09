import * as path from "path";

/**
 * Pure `largo` decisions — no vscode import, so unit tests run under plain
 * Node. The vscode-coupled task factory lives in largo.ts.
 */

/**
 * Walk upward from `startDir` to the filesystem root looking for `Largo.toml`.
 * `largo run`/`largo verify` are project-scoped, so every command that shells
 * out to them anchors here. Existence checking is injected.
 */
export function findProjectRoot(
  startDir: string,
  exists: (candidate: string) => boolean,
): string | undefined {
  let dir = startDir;
  for (;;) {
    if (exists(path.join(dir, "Largo.toml"))) {
      return dir;
    }
    const parent = path.dirname(dir);
    if (parent === dir) {
      return undefined;
    }
    dir = parent;
  }
}

/** Map the `logicaffeine.run.mode` setting to `largo` arguments. */
export function runModeArgs(mode: string): string[] {
  switch (mode) {
    case "debug":
      return ["run"];
    case "release":
      return ["run", "--release"];
    default:
      return ["run", "--interpret"];
  }
}

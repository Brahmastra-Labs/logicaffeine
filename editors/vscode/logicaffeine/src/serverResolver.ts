import * as path from "path";

/** The bare binary name; also the default value of `logicaffeine.lsp.path`. */
export const DEFAULT_SERVER_NAME = "logicaffeine-lsp";

export interface ResolveInput {
  /** `process.platform` (or a test double). */
  platform: string;
  /** `process.arch` (or a test double). */
  arch: string;
  /** The `logicaffeine.lsp.path` setting, if the user set one. */
  configuredPath: string | undefined;
  /** The extension's install directory (bundled binaries live in `bin/`). */
  extensionRoot: string;
  exists: (p: string) => boolean;
  canExecute: (p: string) => boolean;
}

export type Resolution =
  | { kind: "settings"; command: string }
  | { kind: "bundled"; command: string }
  | { kind: "path"; command: string }
  | { kind: "error"; message: string };

/**
 * Decide which server binary to spawn.
 *
 * Precedence: an explicit `logicaffeine.lsp.path` setting (which must exist —
 * a user's explicit choice is never silently overridden), then the bundled
 * per-platform binary, then the bare name on PATH. Pure function — all I/O is
 * injected — so every platform cell is unit-tested.
 */
export function resolveServer(input: ResolveInput): Resolution {
  const configured = input.configuredPath?.trim();
  if (configured && configured !== DEFAULT_SERVER_NAME) {
    if (!input.exists(configured)) {
      return {
        kind: "error",
        message:
          `logicaffeine.lsp.path is set to '${configured}', but no file exists there. ` +
          `Fix the setting, or remove it to use the bundled server.`,
      };
    }
    return { kind: "settings", command: configured };
  }

  const bundledName = bundledBinaryName(input.platform, input.arch);
  if (bundledName) {
    const bundledPath = path.join(input.extensionRoot, "bin", bundledName);
    if (input.exists(bundledPath)) {
      if (input.platform !== "win32" && !input.canExecute(bundledPath)) {
        return {
          kind: "error",
          message:
            `The bundled language server at '${bundledPath}' is not executable. ` +
            `Reinstall the extension, or run: chmod +x '${bundledPath}'`,
        };
      }
      return { kind: "bundled", command: bundledPath };
    }
  }

  return { kind: "path", command: DEFAULT_SERVER_NAME };
}

/**
 * The staged binary name for a platform/arch cell, mirroring release.yml's
 * `bin/` layout. Windows-on-ARM runs the x64 binary under emulation.
 */
function bundledBinaryName(platform: string, arch: string): string | undefined {
  const key = `${platform}-${arch === "arm64" && platform === "win32" ? "x64" : arch}`;
  switch (key) {
    case "linux-x64":
      return "logicaffeine-lsp-linux-x64";
    case "linux-arm64":
      return "logicaffeine-lsp-linux-arm64";
    case "darwin-x64":
      return "logicaffeine-lsp-darwin-x64";
    case "darwin-arm64":
      return "logicaffeine-lsp-darwin-arm64";
    case "win32-x64":
      return "logicaffeine-lsp-win32-x64.exe";
    default:
      return undefined;
  }
}

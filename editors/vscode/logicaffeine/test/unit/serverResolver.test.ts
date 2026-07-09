import * as assert from "assert";
import * as path from "path";
import { resolveServer, ResolveInput, Resolution } from "../../src/serverResolver";

const ROOT = path.sep === "/" ? "/ext" : "C:\\ext";

function input(overrides: Partial<ResolveInput>): ResolveInput {
  return {
    platform: "linux",
    arch: "x64",
    configuredPath: undefined,
    extensionRoot: ROOT,
    exists: () => false,
    canExecute: () => true,
    ...overrides,
  };
}

function bundled(name: string): string {
  return path.join(ROOT, "bin", name);
}

describe("server resolution", () => {
  describe("explicit setting", () => {
    it("wins over everything when the file exists", () => {
      const configured = path.join(ROOT, "custom", "my-lsp");
      const result = resolveServer(
        input({
          configuredPath: configured,
          exists: (p) => p === configured || p === bundled("logicaffeine-lsp-linux-x64"),
        }),
      );
      assert.deepStrictEqual(result, { kind: "settings", command: configured });
    });

    it("fails loudly when the configured path does not exist — never silently falls through", () => {
      const result = resolveServer(
        input({
          configuredPath: "/nowhere/logicaffeine-lsp",
          // The bundled binary exists, but the user's explicit choice must not
          // be silently overridden by it.
          exists: (p) => p === bundled("logicaffeine-lsp-linux-x64"),
        }),
      );
      assert.strictEqual(result.kind, "error");
      assert.ok(
        (result as Extract<Resolution, { kind: "error" }>).message.includes(
          "/nowhere/logicaffeine-lsp",
        ),
        "the error must name the configured path",
      );
    });

    it("treats the default bare name as no configuration", () => {
      const result = resolveServer(
        input({
          configuredPath: "logicaffeine-lsp",
          exists: (p) => p === bundled("logicaffeine-lsp-linux-x64"),
        }),
      );
      assert.strictEqual(result.kind, "bundled");
    });
  });

  describe("bundled binary per platform/arch", () => {
    const cells: Array<[string, string, string]> = [
      ["linux", "x64", "logicaffeine-lsp-linux-x64"],
      ["linux", "arm64", "logicaffeine-lsp-linux-arm64"],
      ["darwin", "x64", "logicaffeine-lsp-darwin-x64"],
      ["darwin", "arm64", "logicaffeine-lsp-darwin-arm64"],
      ["win32", "x64", "logicaffeine-lsp-win32-x64.exe"],
    ];

    for (const [platform, arch, binary] of cells) {
      it(`${platform}-${arch} → bin/${binary}`, () => {
        const result = resolveServer(
          input({ platform, arch, exists: (p) => p === bundled(binary) }),
        );
        assert.deepStrictEqual(result, { kind: "bundled", command: bundled(binary) });
      });
    }

    it("win32-arm64 falls back to the x64 binary (Windows-on-ARM emulation)", () => {
      const result = resolveServer(
        input({
          platform: "win32",
          arch: "arm64",
          exists: (p) => p === bundled("logicaffeine-lsp-win32-x64.exe"),
        }),
      );
      assert.deepStrictEqual(result, {
        kind: "bundled",
        command: bundled("logicaffeine-lsp-win32-x64.exe"),
      });
    });

    it("a bundled binary that lost its executable bit is a distinct, actionable error", () => {
      const result = resolveServer(
        input({
          exists: (p) => p === bundled("logicaffeine-lsp-linux-x64"),
          canExecute: () => false,
        }),
      );
      assert.strictEqual(result.kind, "error");
      assert.ok(
        (result as Extract<Resolution, { kind: "error" }>).message.includes("executable"),
        "the error must explain the executable bit is missing",
      );
    });

    it("does not run the executable check on win32", () => {
      const result = resolveServer(
        input({
          platform: "win32",
          arch: "x64",
          exists: (p) => p === bundled("logicaffeine-lsp-win32-x64.exe"),
          canExecute: () => false,
        }),
      );
      assert.strictEqual(result.kind, "bundled");
    });
  });

  describe("PATH fallback", () => {
    it("falls back to the bare name when nothing is bundled", () => {
      const result = resolveServer(input({}));
      assert.deepStrictEqual(result, { kind: "path", command: "logicaffeine-lsp" });
    });

    it("uses the .exe-less bare name on every platform (the OS resolves PATH)", () => {
      const result = resolveServer(input({ platform: "win32", arch: "x64" }));
      assert.deepStrictEqual(result, { kind: "path", command: "logicaffeine-lsp" });
    });
  });
});

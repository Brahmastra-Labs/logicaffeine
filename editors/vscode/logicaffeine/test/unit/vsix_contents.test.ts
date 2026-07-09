import * as assert from "assert";
import * as path from "path";
import { execFileSync } from "child_process";

/**
 * Packaging invariants.
 *
 * The extension is bundled by esbuild into dist/extension.js and packaged
 * with `--no-dependencies`. That combination is only correct when BOTH hold:
 * the bundle exists in the package, and nothing in the package needs
 * node_modules at runtime. A `tsc`-only build packaged with
 * `--no-dependencies` ships an extension that cannot resolve
 * `vscode-languageclient` and fails activation — this suite exists so that
 * layout can never ship again.
 */
describe("VSIX contents", function () {
  // vsce spawns npm for prepublish checks; give it room.
  this.timeout(120_000);

  const extensionRoot = path.resolve(__dirname, "..", "..", "..");

  function vsceLs(): string[] {
    const vsce = path.join(
      extensionRoot,
      "node_modules",
      ".bin",
      process.platform === "win32" ? "vsce.cmd" : "vsce",
    );
    const output = execFileSync(vsce, ["ls", "--no-dependencies"], {
      cwd: extensionRoot,
      encoding: "utf8",
    });
    return output
      .split("\n")
      .map((line) => line.trim())
      .filter((line) => line.length > 0);
  }

  it("packages the bundled entry point", function () {
    const files = vsceLs();
    assert.ok(
      files.some((f) => f.endsWith("dist/extension.js")),
      `dist/extension.js must be in the package; got:\n${files.join("\n")}`,
    );
  });

  it("packages every marketplace asset", function () {
    const files = vsceLs();
    for (const required of [
      "icon.png",
      "README.md",
      "CHANGELOG.md",
      "LICENSE.md",
      "snippets/logicaffeine.json",
      "icons/logicaffeine-file.svg",
      "media/walkthrough/install-largo.md",
      "media/walkthrough/new-project.md",
      "media/walkthrough/run-and-prove.md",
      "media/walkthrough/diagnostics.md",
      "syntaxes/logicaffeine.tmLanguage.json",
      "syntaxes/logicaffeine.markdown-injection.json",
      "language-configuration.json",
    ]) {
      assert.ok(
        files.some((f) => f === required || f.endsWith(`/${required}`)),
        `${required} must ship in the VSIX; got:\n${files.join("\n")}`,
      );
    }
  });

  it("needs no node_modules at runtime", function () {
    const files = vsceLs();
    const offenders = files.filter((f) => f.includes("node_modules/"));
    assert.deepStrictEqual(
      offenders,
      [],
      "the bundle must be self-contained; node_modules entries mean --no-dependencies will break activation",
    );
  });

  it("ships no sources, build outputs, or build tooling", function () {
    const files = vsceLs();
    const offenders = files.filter(
      (f) =>
        f.startsWith("src/") ||
        f.startsWith("test/") ||
        f.startsWith("out/") ||
        f.startsWith("out-test/") ||
        f.endsWith(".map") ||
        f === "esbuild.mjs" ||
        f === "tsconfig.json" ||
        f === "tsconfig.test.json",
    );
    assert.deepStrictEqual(offenders, [], "dev files leaked into the package");
  });
});

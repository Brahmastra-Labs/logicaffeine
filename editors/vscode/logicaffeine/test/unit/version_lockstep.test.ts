import * as assert from "assert";
import * as fs from "fs";
import * as path from "path";

/**
 * The workspace releases in lockstep: scripts/bump-version.sh rewrites the
 * root Cargo.toml [workspace.package] version and this extension's
 * package.json version together. This lock fails the moment they drift —
 * including if a future dependency pin ever confuses the bump script's
 * first-match sed.
 */
describe("version lockstep", () => {
  const extensionRoot = path.resolve(__dirname, "..", "..", "..");
  const repoRoot = path.resolve(extensionRoot, "..", "..", "..");

  it("extension version equals the workspace version", () => {
    const packageJson = JSON.parse(
      fs.readFileSync(path.join(extensionRoot, "package.json"), "utf8"),
    );

    const cargoToml = fs.readFileSync(path.join(repoRoot, "Cargo.toml"), "utf8");
    const lines = cargoToml.split("\n");
    const sectionStart = lines.findIndex((l) => l.trim() === "[workspace.package]");
    assert.ok(sectionStart >= 0, "root Cargo.toml must have a [workspace.package] section");

    let workspaceVersion: string | undefined;
    for (const line of lines.slice(sectionStart + 1)) {
      const trimmed = line.trim();
      if (trimmed.startsWith("[")) break;
      const match = trimmed.match(/^version\s*=\s*"([^"]+)"/);
      if (match) {
        workspaceVersion = match[1];
        break;
      }
    }
    assert.ok(workspaceVersion, "[workspace.package] must declare a version");

    assert.strictEqual(
      packageJson.version,
      workspaceVersion,
      "extension package.json version must match the workspace version (run scripts/bump-version.sh)",
    );
  });
});

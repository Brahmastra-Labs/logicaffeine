import * as path from "path";
import { spawnSync } from "child_process";
import {
  downloadAndUnzipVSCode,
  resolveCliArgsFromVSCodeExecutablePath,
  runTests,
} from "@vscode/test-electron";

/**
 * The VSIX install gate: install the REAL packaged artifact into a real
 * VSCode and run the integration suite against it — bundled server binary
 * included. This is the test that catches "the published extension cannot
 * activate" before anything publishes.
 */
async function main() {
  const vsixPath = process.env.VSIX_PATH;
  if (!vsixPath) {
    throw new Error("VSIX_PATH must point at the packaged .vsix to gate");
  }

  const vscodeExecutablePath = await downloadAndUnzipVSCode();
  const [cliPath, ...cliArgs] = resolveCliArgsFromVSCodeExecutablePath(vscodeExecutablePath);

  const install = spawnSync(cliPath, [...cliArgs, "--install-extension", vsixPath], {
    encoding: "utf8",
    shell: process.platform === "win32",
  });
  console.log(install.stdout);
  console.error(install.stderr);
  if (install.status !== 0) {
    throw new Error(`--install-extension failed with status ${install.status}`);
  }

  const extensionRoot = path.resolve(__dirname, "..", "..", "..");
  await runTests({
    vscodeExecutablePath,
    // A stub dev extension: the extension under test is the INSTALLED one,
    // so this run must NOT disable installed extensions.
    extensionDevelopmentPath: path.join(extensionRoot, "test", "fixtures", "stub-ext"),
    extensionTestsPath: path.resolve(__dirname, "suite", "index"),
    launchArgs: [path.join(extensionRoot, "test", "fixtures", "proj")],
    extensionTestsEnv: { VSIX_GATE: "1" },
  });
}

main().catch((err) => {
  console.error("VSIX install gate failed:", err);
  process.exit(1);
});

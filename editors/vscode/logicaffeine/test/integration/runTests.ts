import * as path from "path";
import { runTests } from "@vscode/test-electron";

async function main() {
  // out-test/test/integration → extension root
  const extensionDevelopmentPath = path.resolve(__dirname, "..", "..", "..");
  const extensionTestsPath = path.resolve(__dirname, "suite", "index");
  const fixtureWorkspace = path.resolve(
    extensionDevelopmentPath,
    "test",
    "fixtures",
    "proj",
  );

  const extensionTestsEnv: Record<string, string | undefined> = {};
  if (process.env.LOGICAFFEINE_LSP_PATH) {
    extensionTestsEnv.LOGICAFFEINE_LSP_PATH = process.env.LOGICAFFEINE_LSP_PATH;
  }

  await runTests({
    extensionDevelopmentPath,
    extensionTestsPath,
    launchArgs: [fixtureWorkspace, "--disable-extensions"],
    extensionTestsEnv,
  });
}

main().catch((err) => {
  console.error("integration tests failed:", err);
  process.exit(1);
});

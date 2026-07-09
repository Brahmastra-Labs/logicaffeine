import esbuild from "esbuild";

const production = process.argv.includes("--production");
const watch = process.argv.includes("--watch");

const context = await esbuild.context({
  entryPoints: ["src/extension.ts"],
  bundle: true,
  outfile: "dist/extension.js",
  external: ["vscode"],
  format: "cjs",
  platform: "node",
  // Matches the Electron Node of the VSCode engine floor (^1.75.0).
  target: "node16",
  minify: production,
  sourcemap: !production,
  logLevel: "info",
});

if (watch) {
  await context.watch();
} else {
  await context.rebuild();
  await context.dispose();
}

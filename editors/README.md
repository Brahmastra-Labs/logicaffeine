# editors

Editor integrations for LOGOS. Each one is a client of the language server, `logicaffeine-lsp`
(`crates/logicaffeine_lsp`), which carries the language knowledge: diagnostics, completion, semantic tokens,
hover and the rest. An editor folder holds only what its editor needs around that server, so supporting a new
editor means speaking LSP to it, not teaching the editor LOGOS a second time.

Part of the [Logicaffeine](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) repository.
The extensions here are editor packages, not Cargo crates.

| Folder | What it is |
|---|---|
| `vscode/logicaffeine/` | The Visual Studio Code extension: grammar, snippets, the client, and the bundled server. Its `README.md` is the Marketplace page. |

## Working on the VS Code extension

From `vscode/logicaffeine/`, after `npm ci`:

| Command | What it does |
|---|---|
| `npm run build` | Bundles the client into `dist/`. |
| `npm run lint` / `npm run typecheck` | ESLint over `src/` and `test/`; the TypeScript check. |
| `npm run test:unit` | The client's unit tests. |
| `npm run test:grammar` | The TextMate grammar against the snapshot corpus in `test/grammar/corpus/`. |
| `npm run test:integration` | The extension in a real VS Code against a debug server (`cargo build -p logicaffeine-lsp` first). |
| `npm run package` | A `.vsix`. Release builds stage the server binary for each platform first (`.github/workflows/release.yml`). |
| `npm run test:vsix-gate` | Installs the packaged `.vsix` named by `VSIX_PATH` into a real VS Code and runs the integration suite against it, bundled server included. |

The extension's version moves with the workspace's: `scripts/bump-version.sh` rewrites `package.json` and
`package-lock.json`.

## License

MIT License — see [LICENSE.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LICENSE.md).

---
[Docs index](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/README.md) · [Root README](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) · [Changelog](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/editors/vscode/logicaffeine/CHANGELOG.md)

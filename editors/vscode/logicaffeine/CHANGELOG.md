# Changelog

All notable changes to the LOGOS VSCode extension.

## [Unreleased]

### Added
- Bundled per-platform language server with settings-override and PATH
  fallback; language status item; output channel + `logicaffeine.trace.server`.
- Code-lens commands wired end-to-end: Run (`largo run`, interpreter by
  default), Prove/Check Proof (`largo prove`, optional `--trace`), Verify
  (preview, license via secret storage), plus restart/log commands, context
  menus, and `Ctrl+Alt+R`/`Ctrl+Alt+V` keybindings.
- Rewritten TextMate grammar matching the real language surface (`#`
  comments, decomposed `##` headers, multiword English operators, string
  interpolation with format specs, temporal literals), Markdown injection for
  ```` ```logos ```` fences, and `semanticTokenScopes` so the server's
  resolution-aware tokens render correctly in every theme.
- Snippet library, "Get started with LOGOS" walkthrough, marketplace icon,
  and `.lg` file icon.
- `logicaffeine.flycheck.enable` setting (live — no restart needed) and
  workspace-trust hardening: in untrusted workspaces the bundled server still
  runs, but workspace settings cannot redirect which binaries execute
  (`lsp.path`/`lsp.args`/`largo.path` are restricted); virtual workspaces get
  syntax highlighting only.

### Fixed
- The packaged VSIX previously failed to activate (its runtime dependency was
  excluded); the extension is now esbuild-bundled and self-contained, with
  packaging locks that keep it that way.

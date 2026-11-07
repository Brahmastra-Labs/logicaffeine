# Changelog

All notable changes to the LOGOS VSCode extension.

## [Unreleased]

### Added

- The grammar highlights the quick guide's infinitesimal tests (`is infinitesimal`, `is limited`, `is infinitely close
  to`), `At the limit`, the tensor words `matmul`, `hadamard`, `plus` and `across`, and the constants `pi`/`π` and
  `omega`/`ω`. Locked by the grammar's quick-guide ratchet.

## [0.11.0] - 2026-11-02

### Added
- **Debug a LOGOS file, backwards too.** A `logos` debug type ("LOGOS Debug") runs `largo dap` from `PATH`. A launch configuration takes `program` (default `${file}`), with inline `source` overriding it; an initial configuration and a "LOGOS: Launch file" snippet are provided, and breakpoints are enabled for LOGOS files. Step back and reverse continue come from the adapter, which replays the recorded history (`dap_lock`); breakpoints are acknowledged but not yet mapped to operations.
- **LGX pages highlight as HTML and CSS.** `View:` and `Head:` bodies are embedded HTML with each `{…}` hole highlighted as LOGOS, and `Style:` bodies are embedded CSS; `State:` and `Actions:` read as sections, and a section keyword keeps its highlighting where the embedded grammar is unavailable. The block headers `## Page`, `## Component`, `## Store`, `## Directive`, `## Route`, `## Redirect`, `## Before each request` and `## Error page` are recognized. `vscode_grammar_lock`.
- **The server's `tag` and `attribute` tokens paint as HTML.** Both are declared as semantic token types and mapped to the HTML tag and attribute-name scopes. `lgx_editor_lock::every_semantic_token_type_is_mapped_by_the_vscode_manifest`.
- **LGX snippets.** `page`, `component`, `store`, `route`, `session` and `errorpage`; every view snippet builds for the browser. `vscode_grammar_lock`.
- **Template tags and page statements highlight.** A hole whose first word is one the template reader treats as a tag (`{if …:}`, `{for each …:}`, `{else:}`, `{end}`, `{children}`, `{extends Layout}`, `{block name:}`, `{inspect …:}`, `{when …:}`, `{let … be …}`, `{raw …}`) paints that word as a keyword and a layout or slot name as a name, its body still LOGOS; `Use Cart.`, `Navigate to …` and `Go back.` read as statements. The quickguide ratchet (`npm run test:grammar`) holds every quickguide form to a scope.

### Changed

- Licensed under the MIT License, which replaces the Business Source License 1.1; `package-lock.json` says MIT too,
  where it still said `SEE LICENSE IN LICENSE.md`.

### Removed

- The `LOGOS: Set Verification License Key` command and the prompt before Verify Project. Verify runs `largo verify`
  with nothing added to its environment.

## [0.10.0] - 2026-07-08

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

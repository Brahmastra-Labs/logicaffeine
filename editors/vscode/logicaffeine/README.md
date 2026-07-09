# LOGOS for Visual Studio Code

Language support for **LOGOS** — the English programming language. Programs
are literate Markdown; sentences are statements; the grammar *is* the syntax.
This extension bundles the LOGOS language server for your platform and turns
VSCode into a full IDE for `.lg` files.

```logos
## To double (n: Int) -> Int:
    Return n * 2.

## Main
Let answer be double(21).
Show "The answer is {answer}.".
```

## Highlights

- **English-grammar highlighting.** Verbs color as functions, nouns as types,
  adjectives as modifiers — and resolution upgrades identifiers to what they
  actually are: parameters, fields, enum variants, stdlib calls. Prose inside
  `## Note`/`## Example` blocks fades to comment color, so documentation
  recedes while code speaks.
- **Socratic diagnostics.** Errors explain and suggest instead of scolding:
  "Cannot use 'x' after giving it away … give 'a copy of x' to keep the
  original", with a link to the exact statement that moved the value and a
  quick fix that applies the suggestion.
- **Rust's borrow checker, speaking English.** On save (when cargo is
  installed), your program is compiled through the LOGOS AOT backend and
  `cargo check` runs over it — rustc findings come back translated, on the
  right line of *your* source, under the `logicaffeine (rustc)` source.
- **Run and prove from the editor.** Code lenses run `## Main` (interpreter
  by default — sub-second feedback) and prove `## Theorem` blocks with
  kernel-certified derivations. `Ctrl+Alt+R` runs, `Ctrl+Alt+V` verifies.
- **The full LSP surface.** Completion, hover, go-to-definition (across
  files), find references, rename, workspace symbols, document outline,
  folding, inlay hints (inferred types + ownership states), formatting
  (identical rules to `largo fmt`), semantic tokens with range + delta
  requests, and unused-code fading.

## Getting started

1. Install the extension — the language server is bundled per platform.
2. Install the build tool for running/proving:
   `curl -fsSL https://logicaffeine.com/install.sh | sh`
   (Windows: `irm https://logicaffeine.com/install.ps1 | iex`)
3. `largo new hello && code hello`, open `src/main.lg`, click **Run**.

The "Get started with LOGOS" walkthrough (Help → Get Started) covers the
same steps interactively.

## Settings

| Setting | Default | Purpose |
|---------|---------|---------|
| `logicaffeine.lsp.path` | `logicaffeine-lsp` | Override the bundled server binary |
| `logicaffeine.lsp.args` | `[]` | Extra server arguments |
| `logicaffeine.trace.server` | `off` | LSP traffic tracing (`messages`/`verbose`) |
| `logicaffeine.largo.path` | `largo` | Path to the build tool |
| `logicaffeine.run.mode` | `interpret` | Run lens engine: `interpret`/`debug`/`release` |
| `logicaffeine.prove.trace` | `false` | Render derivation trees under proved theorems |
| `logicaffeine.flycheck.enable` | `true` | On-save rustc analysis (needs a Rust toolchain) |

## Commands

`LOGOS: Run Project`, `Prove Theorems`, `Check Proof`, `Verify Project
(preview)`, `Set Verification License Key`, `Restart Language Server`,
`Show Language Server Log`.

Verification (preview) is license-gated (Pro and up — see
[logicaffeine.com/pricing](https://logicaffeine.com/pricing)); keys are held
in VSCode's secret storage, never in settings files.

## Literate `.md` modules

Fenced ```` ```logos ```` blocks highlight inside any Markdown file. To edit
a literate module with full language support, associate it per workspace:

```json
"files.associations": { "assets/std/*.md": "logicaffeine" }
```

## Troubleshooting

- **"The LOGOS language server failed to start"** — the status item (bottom
  right, over a `.lg` file) names the binary it tried; `LOGOS: Show Language
  Server Log` has the details. An explicit `logicaffeine.lsp.path` must
  exist — it is never silently ignored.
- **No rustc diagnostics on save** — the flycheck needs a Rust toolchain
  (`cargo --version`); without one the extension quietly runs
  interactive-only diagnostics.
- **Run/Prove lens does nothing** — those need `largo` on PATH (or
  `logicaffeine.largo.path`) and a `Largo.toml` project root above the file.

## More

- [LOGOS quick guide](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LOGOS_QUICKGUIDE.md)
- [Repository](https://github.com/Brahmastra-Labs/logicaffeine) ·
  [logicaffeine.com](https://logicaffeine.com)

License: BUSL-1.1 — see LICENSE.md.

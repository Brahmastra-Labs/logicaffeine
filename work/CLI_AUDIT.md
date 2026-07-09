# CLI & LSP Audit

> **Scope.** An honest look at `largo` (the LOGOS CLI), the language server, and how Logos is
> distributed today — plus a roadmap to making the CLI *first-class* (the bar set by cargo,
> wrangler, turso, rustup). This is an understanding-and-planning document. Nothing here is
> implemented yet; the "Roadmap" section is a set of recommendations, not a changelog.
>
> Audited at version **0.9.17**, 2026-06-23.
>
> **STATUS UPDATE (2026-07-07, pre-0.10.0):** the roadmap LANDED. P0 (distribution): release.yml
> builds 10 `largo` artifacts (5 platforms × lean/Z3-static-full) + SHA256SUMS; `install.sh` /
> `install.ps1` ship as web-app static assets at logicaffeine.com, offline-tested by
> `scripts/test-installers.sh` and gated by `installer.yml`; tag pushes run pinned `e2e-install`
> jobs. P1 (polish): global `-q/-v/--color`, styled help with Examples on every command (meta-test
> enforced), `error:`/`help:` typed errors, completions for 5 shells, live-streamed cargo output
> with classified failures. P2 (widen): `repl` (full interactive session), `logic`, `prove`, `sat`
> (shared `satcli` driver), `fmt` (canonical rules shared with the LSP), `emit`, `doc`,
> `add`/`remove`, `clean`; `verify` really wired (`verification`/`verification-static`); `test`
> reserved for the future framework. Found and fixed en route: `## Requires` deps were silently
> dropped (written after `[profile.release]`) / Linux-scoped (after `[target.…]`), and the
> `phase37_cli.rs` suite was dead behind a feature that never existed (now
> `apps/logicaffeine_cli/tests/project.rs`, running). P3 (LSP workspace index) remains open.
>
> **Two adversarial audit rounds** (pre-tag) then fixed: wasm-shim argv/heap collision +
> trap-output loss, `run --emit` fallthrough, REPL double-execution per TTY line, LSP final-line
> range corruption, project-name validation (+ shim JS escaping), manifest-entry confinement,
> `--deep` cache keyed on project path, and a dozen smaller edges — each RED-tested.
>
> **Engine-level stack-overflow: FIXED at the root** (the round-2 deferral, now closed): the
> `ast_depth` gate in the language crate enforces "parsed ⇒ bounded" at the single
> `parse_program` choke point — deep chains, parenthesis towers, and block pyramids all get a
> graceful `AstTooDeep` diagnostic (teaching both fixes: split into `Let`s, or raise
> `LOGOS_MAX_AST_DEPTH` on deep-stack machines) on every surface. Iterative wildcard-free
> walker ratchets future AST variants; limits proven in 2 MiB-thread tests; full corpus
> (language 268 + compile 1045) unaffected.

---

## 1. Executive Summary

Three verdicts, stated plainly:

| Area | Verdict |
|------|---------|
| **CLI (`largo`)** | Real and functional — it genuinely builds, runs, and even has its own package registry. But it is **plain** (no color, progress, completions, or install story) and **narrow** (it surfaces maybe a third of the engine's actual power). |
| **LSP** | The **strongest** corner of this whole story, not a dusty one. 14 real capabilities, 199 tests, multi-platform binaries, a shipped VS Code extension, and — crucially — it calls the *real* compiler so it cannot drift from the language. One genuine limit: single-file only. |
| **Distribution** | This is **the actual problem.** There is no easy way to install `largo`. Release CI ships the *LSP* binary for 5 platforms and a VS Code `.vsix`, but builds **zero CLI binaries.** Today the only ways to get `largo` are `cargo install` (needs a full Rust toolchain) or the web IDE. |

The single most important fact: **the CLI is meant to be how people install and use Logos, and it has no install story of its own.** That is the headline gap.

---

## 2. The CLI Today — `largo`

### Where it lives

```
apps/logicaffeine_cli/
├── Cargo.toml              # binary = "largo", lib = "logicaffeine_cli"
├── src/
│   ├── main.rs             # entry point — installs the JIT unless LOGOS_NO_JIT=1
│   ├── lib.rs              # module re-exports for programmatic use
│   ├── cli.rs              # all clap command definitions + dispatch (run_cli)
│   └── project/
│       ├── manifest.rs     # Largo.toml parser
│       ├── build.rs        # the transpile-then-cargo pipeline
│       ├── registry.rs     # package registry HTTP client (publish/login)
│       └── credentials.rs  # token storage (~/.config/logos/credentials.toml)
```

Binary name: **`largo`**. Library crate: `logicaffeine_cli`. Built on `clap` 4.4 (derive).

### Command inventory (all 9)

| Command | Args / Flags | What it does |
|---------|--------------|--------------|
| `largo new <name>` | — | Scaffolds a new project dir: `Largo.toml`, `src/main.lg` (hello world), `.gitignore`. |
| `largo init` | `--name` | Same, in the current directory; infers name from the folder. |
| `largo build` | `-r/--release`, `--lib`, `--target <triple>`, `--verify`, `--license <key>` | Compiles LOGOS → Rust, synthesizes a Cargo project, runs `cargo build`. `--target wasm` expands to `wasm32-unknown-unknown`; `--lib` emits a `cdylib`. |
| `largo run` | `-r/--release`, `-i/--interpret`, `-- <args>` | Build-then-execute, **or** (with `--interpret`) skip Rust entirely and tree-walk for sub-second feedback. Forwards args + exit code. |
| `largo check` | — | Parse + type-check only; no codegen, no cargo. |
| `largo verify` | `--license <key>` | Z3 static verification. License-gated (Pro+), and behind `#[cfg(feature = "verification")]` — **off in the default build**. |
| `largo publish` | `--registry <url>`, `--dry-run`, `--allow-dirty` | Tarball the project, upload to the registry. Checks git-clean unless `--allow-dirty`. |
| `largo login` | `--registry <url>`, `--token <tok>` | Validate a token against `/auth/me`, store it `0600`. |
| `largo logout` | `--registry <url>` | Remove the stored token. |

### How it wraps cargo (the core mechanism)

`largo` does **not** reimplement a backend — it is a *code generator that drives cargo.* The pipeline
(`src/project/build.rs`):

1. Load `Largo.toml`, resolve the entry point (`.lg` or `.md`).
2. `compile_project()` → Rust source (+ optional C header + a dependency list).
3. Write a **synthetic Cargo project** under `target/{debug,release}/build/`:
   - generated `src/main.rs` (or `lib.rs` with `crate-type = ["cdylib"]` for `--lib`),
   - a `Cargo.toml` injecting the runtime crates (`logicaffeine-data`, `logicaffeine-system` "full", `tokio`),
   - a tuned release profile: `lto = true, opt-level = 3, codegen-units = 1, panic = "abort", strip = true`,
   - `.cargo/config.toml` with `target-cpu=native` (release, non-cross only),
   - auto-injected `wasm-bindgen` when the target is wasm32.
4. Shell out: `cargo build [--release] [--target <triple>]`.
5. Resolve and report the final binary/library path (handles `.exe`, `lib*.so/.dylib`, cross-triples).

```
my_project/                          target/debug/build/         (generated)
├── Largo.toml          ──largo──►   ├── Cargo.toml
└── src/main.lg                      ├── .cargo/config.toml
                                     ├── src/main.rs   ──cargo──► target/debug/<bin>
                                     └── …
```

So "largo wraps cargo in a lot of ways" is exactly right: it's a transpile-then-cargo orchestrator,
plus profile tuning, plus cross-compile/wasm/lib handling.

### The package registry (the ambitious part)

`publish` / `login` / `logout` talk to `registry.logicaffeine.com` — tarball + multipart upload
(`registry.rs`), Bearer-token auth stored in `~/.config/logos/credentials.toml`. This is a real
**cargo/npm-for-Logos**, already built and wired (there's even a `deploy-registry.yml` Cloudflare
Worker + D1 backend). It's one of the most mature pieces of the CLI and a genuine differentiator —
most young languages don't have a package registry at all.

### Dependency model — note there are *two* systems

1. **`Largo.toml [dependencies]`** (`manifest.rs`): Logos packages, specified as `logos:std` URIs,
   `{ path = … }`, or `{ git = … }`. This is the registry-facing dependency graph.
2. **`## Requires` blocks in source** (`build.rs:282`): raw *Rust crate* deps (name + version +
   features) declared inside a `.lg`/`.md` file and injected straight into the generated Cargo.toml.

These are independent, and there is **no `largo add`** — both are hand-edited. Unifying/streamlining
this is a real opportunity (see roadmap P2).

### Environment variables

| Var | Effect |
|-----|--------|
| `LOGOS_NO_JIT` | Skip the JIT; use the bytecode interpreter. |
| `LOGOS_LICENSE` | Pro+ license key for `verify`. |
| `LOGOS_TOKEN` | Registry token (overrides stored credentials). |
| `LOGOS_CREDENTIALS_PATH` | Override credentials file location. |

### Tests

`crates/logicaffeine_tests/tests/phase37_cli.rs` (manifest parsing + the build pipeline, ~10 tests)
plus unit tests in `build.rs`. **Gaps:** registry/auth/publish, interpret mode, cross-compile, lib
mode, and clap-level argument parsing are untested.

---

## 3. Architecture — One Front Door to Many Crates

This is the lens the rest of the audit hangs on: **`largo` is the single entry point to an 18-crate
workspace.** Two questions follow — what does the front door *bundle* (distribution), and what does it
*expose* (capability surfacing)?

### The workspace (18 members, root `Cargo.toml`)

| Layer | Crates |
|-------|--------|
| Foundation | `base`, `lexicon`, `kernel`, `data`, `system` |
| Language pipeline | `language` (NL→FOL/AST), `compile` (codegen + interpreter) |
| Execution tiers | `compile` → `forge` (copy-and-patch JIT) → `jit` (native tier) |
| Verification | `verify` (Z3), `tv` (SMT translation validation) |
| Proof / synthesis | `proof` (backward-chaining prover), `synth` |
| Interfaces | **`cli`**, `lsp`, `web` |
| Internal | `tests`, `wiki_trace` |

### What `largo` actually links (`apps/logicaffeine_cli/Cargo.toml`)

```toml
logicaffeine-base, logicaffeine-kernel, logicaffeine-language, logicaffeine-compile
# native-only:
logicaffeine-jit            # which pulls in forge
```

**That's the whole list.** Which gives two findings:

#### Finding A — the door is narrow (capability surfacing)

Whole capabilities are built in the workspace but are **unreachable from the CLI**:

| Crate / capability | In workspace? | Reachable from `largo`? |
|--------------------|:-------------:|:-----------------------:|
| `compile` / `jit` — build & run | ✅ | ✅ |
| `proof` — theorem proving / proof traces | ✅ | ❌ not linked |
| **logic-mode FOL output** (English → ∀∃, the website headline) | ✅ (in `language`) | ❌ no command |
| `verify` — Z3 static verification | ✅ | ⚠️ feature-gated off by default |
| `synth` — synthesis | ✅ | ❌ not linked |
| `tv` — translation validation | ✅ | ❌ not linked |

The marquee feature of the project — *"Every woman loves a man." → ∀x(Woman(x) → ∃y(Man(y) ∧ Love(x,y)))"* —
is on the front page of the website and is **not a CLI command.** The single front door currently opens
onto roughly a third of the engine.

#### Finding B — one binary is the *good* news (distribution)

Because `largo` statically bundles its capability crates, a user installs **one binary**, never the 14
library crates. That is exactly what makes a clean install story *possible*: ship `largo` (and
optionally `logicaffeine-lsp`) and you're done. The problem in §5 is purely that we don't ship it — not
that it's hard to.

---

## 4. The LSP Today (the part that's already good)

### Where it lives

`crates/logicaffeine_lsp/` — binary **`logicaffeine-lsp`**, built on `tower-lsp` + tokio, ~20 feature
modules (`server.rs`, `pipeline.rs`, one module per capability).

### 14 capabilities — all real (no stubs, no `todo!()`)

diagnostics · hover · completion · semantic tokens · go-to-definition · find references · rename ·
code actions · signature help · inlay hints · folding · document symbols · code lens · formatting.

### Why it's accurate and can't go stale

`pipeline.rs` runs the **real** `logicaffeine_language` lexer/parser and the **real**
`logicaffeine_compile` ownership/escape analysis on every document change. Completions come from the
live type registry; diagnostics come from the actual compiler errors (`socratic_explanation`). There is
**no hardcoded vocabulary** and no shadow parser. When the language changes, the LSP changes with it for
free. This is the single most valuable property an LSP can have, and it's already true here.

### Tests, editors, distribution

- **199 tests**, integration-level (real parsing → real feature output).
- **VS Code extension** (`editors/vscode/logicaffeine`, v0.9.5) with a TextMate grammar; documented
  Neovim / Emacs / Sublime setup.
- **Shipped binaries** for 5 platforms via `release.yml`, bundled into the `.vsix`, also on crates.io.

### Honest limitations

- **Single-file only.** The symbol index is per-document, so go-to-definition, find-references, and
  rename don't cross files, and there's no workspace symbol search.
- **`## To` function blocks** are extracted at the token level rather than full-AST (a documented
  parser gap, with correct name/param/type extraction as a workaround).

Bottom line: this is a real, polished, in-sync LSP. It needs *widening* (workspace awareness), not
rescue.

---

## 5. Distribution & Install — the Real Gap

### What CI actually ships

| Workflow | Trigger | Produces |
|----------|---------|----------|
| `release.yml` | tag `v*` | **LSP** binaries × 5 platforms + VS Code `.vsix`. **No `largo`.** |
| `publish.yml` | tag `v*` | 13 crates → crates.io, in dependency tiers. |
| `deploy-frontend.yml` / `deploy-docs.yml` / `deploy-registry.yml` | post-test on main | web IDE / rustdoc / registry → Cloudflare. |

(Benchmark JSON is regenerated on the bench box and committed — no CI job.)

The `release.yml` matrix builds `logicaffeine-lsp` for `{linux x64/arm64, darwin x64/arm64, win x64}`.
There is **no equivalent job for the CLI binary.**

### How you'd install Logos *today*

| Channel | Status | Notes |
|---------|:------:|-------|
| Web IDE (`logicaffeine.com/guide`) | ✅ | Zero install, runs in-browser (WASM). |
| `cargo install logicaffeine-cli` | ✅ | **Requires a full Rust toolchain.** Binary lands as `largo`. |
| `cargo install logicaffeine-lsp` / GitHub release | ✅ | LSP only. |
| Prebuilt `largo` download | ❌ | Not produced. |
| `curl … | sh` one-liner | ❌ | Doesn't exist. |
| Homebrew | ❌ | No formula/tap. |
| npm wrapper | ❌ | — |
| Docker | ❌ | — |

So the honest answer to "how do I install Logos?" is: *use the website, or install a Rust toolchain and
`cargo install`.* For a tool whose entire job is to be the front door, that's the thing to fix first.

### Papercuts found along the way

- README version badge says **0.8.12** (actual: 0.9.17).
- The website `/guide` has **no install instructions** — only online examples.
- `logicaffeine-nano` is at 0.9.16 while everything else is 0.9.17 (lockstep drift).

---

## 6. What "First-Class" Looks Like

The bar, drawn from cargo / wrangler / turso / rustup:

| Property | Gold standard | Logos today |
|----------|---------------|:-----------:|
| One-line install | `curl … | sh`, Homebrew, prebuilt every platform, `self update` | ❌ (cargo-install only) |
| Polished UX | color, spinners/progress, crisp errors, great `--help` + examples | ⚠️ plain |
| Shell completions | bash/zsh/fish/powershell | ❌ |
| Bare-command overview | `tool` with no args prints help | ⚠️ (errors instead) |
| Complete command set | the front door reaches the whole engine | ⚠️ ~⅓ exposed |
| Package management | `add`/`remove`, a registry | ⚠️ registry ✅, `add` ❌, two dep systems |
| Self-management | version-check nudge, `doctor` | ❌ |
| Editor support | a real LSP | ✅ (the strong point) |

The shape of the work: **install first, then polish, then widen the door, then deepen the LSP.**

---

## 7. Roadmap to First-Class

Prioritized and phased. Each item names the concrete file(s) and reuses machinery that already exists.

### P0 — Install / distribution (the headline fix)

*Confirmed channels: `curl | sh` + prebuilt binaries + Homebrew tap.*

- **Build `largo` in CI.** Add a `build-cli` matrix job to `.github/workflows/release.yml` mirroring
  the existing `build-lsp` job, producing `largo-<platform>.tar.gz` for the same 5 targets. (The LSP
  job is a ready-made template — this is mostly copy-and-adjust.)
- **`install.sh` (`curl | sh`).** Detect OS/arch, download the matching release tarball, drop `largo`
  into `~/.local/bin` (optionally also fetch `logicaffeine-lsp`). Host at a stable URL
  (e.g. `logicaffeine.com/install.sh`).
- **Homebrew tap.** A formula (`brew install logicaffeine/tap/largo`) pointing at the release tarballs
  + sha256. Cheap to maintain once the binaries exist.
- **Keep `cargo install logicaffeine-cli`**; optionally support `cargo binstall`.
- **Fix the front door's signage.** README badge → 0.9.17; add an **Install** section to the README
  *and* the website `/guide`.
- **Stretch:** `largo self update` + a quiet "new version available" nudge.

### P1 — CLI UX polish

- **Shell completions:** `largo completions <bash|zsh|fish|powershell>` via `clap_complete`.
- **Color + progress:** a spinner during the cargo build; colored success/error.
- **Bare `largo` prints help** (`arg_required_else_help`) instead of erroring.
- Tighten help/about text and error messages (clap derive makes this incremental).

### P2 — Widen the front door (surface the engine that's already built)

Each maps to an existing crate capability:

- **`largo fmt`** — reuse the LSP's formatting logic (`crates/logicaffeine_lsp/src/formatting.rs`).
- **`largo emit --fol` / `largo explain`** — surface **logic-mode English→FOL** from
  `logicaffeine_language` (the headline feature, currently CLI-invisible).
- **`largo emit --rust`** — print the generated Rust (`compile_project` already produces it).
- **`largo prove`** — surface the `logicaffeine_proof` engine / proof traces as a first-class verb.
- **Promote `verify`** out of default-off feature-gating into a clear, documented story; consider
  exposing `synth` and `tv`.
- **Round out conventions:** `test`, `doc`, `clean`, and **`add`/`remove`** (which also closes the
  two-dependency-systems gap from §2).

### P3 — LSP cross-file / workspace

Scope this tightly — the LSP is already strong. Add a **workspace-wide symbol index** so
definition/references/rename cross files, plus a workspace symbol search; move `## To` blocks to
full-AST parsing. Nothing else needs touching.

---

## 8. Open Questions / Decisions

- **Binary shape as the engine grows:** keep one bundled `largo` that carries everything, or move to a
  plugin/subcommand model (à la `cargo-*`) so heavy capabilities like `verify`/Z3 stay optional?
- **Where does logic mode live?** Top-level verbs (`largo prove`, `largo fol`) vs. under a single
  `largo emit` with `--fol/--rust/--proof`?
- **npm wrapper** (wrangler-style `npm i -g`) — now, or after curl|sh + Homebrew land?
- **Homebrew** — own tap vs. eventually homebrew-core.

---

## Appendix — Files referenced (none modified by this audit)

- CLI: `apps/logicaffeine_cli/src/{main,lib,cli}.rs`,
  `apps/logicaffeine_cli/src/project/{build,manifest,registry,credentials}.rs`,
  `apps/logicaffeine_cli/Cargo.toml`
- Workspace topology: root `Cargo.toml`
- LSP: `crates/logicaffeine_lsp/src/{server,pipeline,formatting}.rs`, `editors/vscode/logicaffeine/`
- CI: `.github/workflows/{release,publish,benchmark,deploy-frontend,deploy-docs,deploy-registry}.yml`
- Tests: `crates/logicaffeine_tests/tests/phase37_cli.rs`
- Papercut: `README.md` (stale version badge)

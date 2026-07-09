# logicaffeine-docs

Build pipeline that renders the workspace's rustdoc into a static site (served at `docs.logicaffeine.com` via Cloudflare Pages).
A standalone app in the Logicaffeine repo — **not** a workspace member (absent from the root [`Cargo.toml`](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/Cargo.toml) `members`). Part of [Logicaffeine](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md).

## What it is

Not a Rust crate — there is no `Cargo.toml` and no `src/`. The directory is just a build script plus its ignore rule:

```text
apps/logicaffeine_docs/
├── build.sh      # generates the docs site into dist/
├── .gitignore    # ignores dist/
└── README.md
```

`build.sh` is the whole app. It runs `RUSTDOCFLAGS="-D warnings"` by default, so a broken intra-doc link or bare URL in any included README fails the build instead of shipping a broken page. From the repo root it:

1. `rm -rf target/doc`, then runs `cargo doc --no-deps --workspace` (a curated `Z3_SYS_Z3_HEADER` default lets the Z3-backed crates document).
2. Copies every publishable crate's doc tree plus the web app into `apps/logicaffeine_docs/dist/` — the 16 crates `logicaffeine_language`, `logicaffeine_compile`, `logicaffeine_proof`, `logicaffeine_kernel`, `logicaffeine_verify`, `logicaffeine_tv`, `logicaffeine_lexicon`, `logicaffeine_base`, `logicaffeine_data`, `logicaffeine_system`, `logicaffeine_runtime`, `logicaffeine_forge`, `logicaffeine_jit`, `logicaffeine_lsp`, `logicaffeine_cli`, and `logicaffeine_web`. `logicaffeine_tests`, `logicaffeine_synth`, and `logicaffeine_wirebench` are excluded (test/dev-only). A missing doc tree fails the build loudly.
3. Copies rustdoc assets (`static.files/`, `search.index/`, `src-files.js`) and the per-crate `src/` listings.
4. Emits a filtered `crates.js` (`window.ALL_CRATES = [...]`) so search lists only those crates.
5. Writes a real `index.html` landing page: one entry per crate, each blurb pulled from that crate's `Cargo.toml` `description` at build time, so the listing can never drift from the manifests.

Output lands in `dist/` (git-ignored), which Cloudflare Pages serves.

## Build / run

```bash
./apps/logicaffeine_docs/build.sh   # from the repo root, or from anywhere — it cd's to ../.. itself
```

Requires a Rust toolchain (for `cargo doc`). The result is the static `dist/` directory; open `dist/index.html` to preview locally.

## Deployment

`.github/workflows/deploy-docs.yml` runs `build.sh` and deploys `dist/` to the Cloudflare Pages project `logicaffeine-docs` (branch `main`) via `wrangler`. It triggers on a successful completion of the `Tests` workflow on `main` (push events), or on manual `workflow_dispatch`. Needs the `CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID` secrets.

## Dependencies

No Cargo manifest, so no crate dependencies. The tooling it relies on:

- **Rust toolchain / `cargo doc`** — produces the rustdoc HTML.
- **The workspace crates** — the doc content is whatever `cargo doc --no-deps --workspace` emits.
- **`wrangler` + Cloudflare Pages** — deployment only (CI).

## License

Business Source License 1.1 — see [LICENSE.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LICENSE.md).

---
[Docs index](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/README.md) · [Root README](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) · [Changelog](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/CHANGELOG.md)

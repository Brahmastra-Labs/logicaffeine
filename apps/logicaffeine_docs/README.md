# logicaffeine-docs

Build pipeline that renders the workspace's rustdoc into a static site (served at `docs.logicaffeine.com` via Cloudflare Pages).
A standalone app in the Logicaffeine repo — **not** a workspace member (absent from the root [`Cargo.toml`](../../Cargo.toml) `members`). Part of [Logicaffeine](../../NEW_README.md).

## What it is

Not a Rust crate — there is no `Cargo.toml` and no `src/`. The directory is just a build script plus its ignore rule:

```
apps/logicaffeine_docs/
├── build.sh      # generates the docs site into dist/
├── .gitignore    # ignores dist/
└── README.md
```

`build.sh` is the whole app. From the repo root it:

1. `rm -rf target/doc`, then runs `cargo doc --no-deps --workspace`.
2. Copies a fixed curated list of 14 crate doc trees into `apps/logicaffeine_docs/dist/` (`logicaffeine_base`, `logicaffeine_cli`, `logicaffeine_compile`, `logicaffeine_data`, `logicaffeine_forge`, `logicaffeine_kernel`, `logicaffeine_language`, `logicaffeine_lexicon`, `logicaffeine_lsp`, `logicaffeine_proof`, `logicaffeine_system`, `logicaffeine_tv`, `logicaffeine_verify`, `logicaffeine_web`) — `logicaffeine_tests` is deliberately excluded.
3. Copies rustdoc assets (`static.files/`, `search.index/`, `src-files.js`) and the per-crate `src/` listings.
4. Emits a filtered `crates.js` (`window.ALL_CRATES = [...]`) so search lists only those crates.
5. Writes an `index.html` that meta-refreshes to `logicaffeine_language/index.html`, with a `<noscript>` crate index as fallback.

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

Business Source License 1.1 — see [LICENSE.md](../../LICENSE.md).

---
[Docs index](../../new_docs/README.md) · [Root README](../../NEW_README.md) · [Changelog](../../CHANGELOG.md)

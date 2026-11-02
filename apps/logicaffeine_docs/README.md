# logicaffeine-docs

The build that turns the workspace's rustdoc into the static site at `docs.logicaffeine.com`. Crates.io and docs.rs
show one crate at a time; this site shows the whole workspace behind one landing page, with search across every
crate.

Part of [Logicaffeine](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md). It is not a workspace
member and not a Rust crate: there is no `Cargo.toml` and no `src/`, only a build script, the site assembler it hands
off to, the routes it reads, and an ignore rule.

```text
apps/logicaffeine_docs/
├── build.sh      # sets the lint gate and the curated crate list, then runs `site.py build`
├── site.py       # documents the crates, then assembles and gates the site in dist/
├── routes.toml   # the "start here" routes site.py publishes
├── .gitignore    # ignores dist/
└── README.md
```

## What `build.sh` does

`build.sh` sets `RUSTDOCFLAGS="-D rustdoc::broken_intra_doc_links -D rustdoc::private_intra_doc_links -D rustdoc::bare_urls"` unless the caller set it (an explicit `RUSTDOCFLAGS=""` is a fully permissive build), so a dead intra-doc link, a link to a private item (which the published pages cannot reach) or a bare URL in any crate, README included, fails the build instead of shipping a broken page. A default `Z3_SYS_Z3_HEADER` lets the Z3-backed crates document. It curates every publishable crate plus the web app — `logicaffeine_language`, `logicaffeine_compile`, `logicaffeine_proof`, `logicaffeine_kernel`, `logicaffeine_verify`, `logicaffeine_tv`, `logicaffeine_lexicon`, `logicaffeine_base`, `logicaffeine_data`, `logicaffeine_system`, `logicaffeine_runtime`, `logicaffeine_forge`, `logicaffeine_jit`, `logicaffeine_lsp`, `logicaffeine_cli`, and `logicaffeine_web` — and hands them to `site.py build`, which runs its two steps (each also available on its own):

1. **`site.py document`** runs one parallel `cargo check --lib -p …` over the curated crates (building everything rustdoc will need), clears the target directory's `doc/`, creates an empty `doc/<crate>/` for every curated crate, then runs one `cargo doc --no-deps --lib --jobs 1 -p …` over exactly those crates. Documenting only them keeps rustdoc's search index, `crates.js` and implementor scripts to pages the site ships. The rest makes every cross-crate link deterministic: rustdoc links another crate's items only when that crate's doc directory exists as rustdoc starts, and with `--no-deps` cargo neither orders one doc job after another nor keeps a crate's directory while re-documenting it (it deletes it first). Pre-made directories plus one rustdoc at a time mean a directory is only ever missing while no other rustdoc runs. Single invocations keep feature unification across the whole curated set — dependency-ordered invocations would each unify on their own and document a dependency without the features its dependents turn on.
2. **`site.py assemble`** copies rustdoc's output into `dist/` **whole and byte for byte** (only cargo's `.lock` stays behind) and adds the site shell rustdoc does not write:
   - `index.html` — the landing page: one entry per crate, each blurb read from that crate's manifest `description` through `cargo metadata`, so the listing can never drift from the manifests.
   - `404.html` — a real not-found page (`noindex`, root-relative links).
   - `robots.txt` — allows everything and names the sitemap.
   - `sitemap.xml` — the landing page and every crate page that is not a rustdoc redirect stub, at the extensionless address Pages serves it under; past 50,000 URLs it becomes an index over `sitemap-N.xml` parts.

   The site's origin is the workspace's `[workspace.package] documentation` URL.

Nothing is hand-picked out of rustdoc's output because of how Cloudflare Pages treats a missing file. With a top-level `404.html`, an address with no file gets that page and a 404. Without one, Pages assumes a single-page app and answers **every** unknown address with `/index.html` and a 200 — so a file left out of the site turns into a copy of the landing page, silently.

`site.py` refuses the build, and leaves no `dist/` behind, when rustdoc documented a crate that is not listed (or missed one that is), when a shell file would overwrite rustdoc output, when the documentation URL or a crate description is missing, or when any page references a file the site does not ship. Two kinds of reference are rustdoc's to answer for and pass the gate: the `async` implementor script every trait page loads from `trait.impl/` (rustdoc writes it only when another documented crate implements the trait, so it is an honest 404 otherwise), and links inside documentation rustdoc copies from a trait defined outside the site — a dependency's blanket impl, such as `tracing`'s — which are relative to that crate's own docs. Links in our own docs, our own trait impls included, get no such pass.

`crates/logicaffeine_tests/tests/docs_site_lock.rs` documents a fixture workspace with the real toolchain through `site.py`, and checks the result against a model of Pages' routing.

Output lands in `dist/` (git-ignored), which Cloudflare Pages serves. Feature-gate badges ("Available on crate feature
…") render on docs.rs, which builds with nightly rustdoc; this site builds with the stable toolchain, where rustdoc
refuses the `doc_cfg` feature they need.

## Build / run

```bash
./apps/logicaffeine_docs/build.sh   # from the repo root, or from anywhere — it cd's to ../.. itself
```

Requires a Rust toolchain (for `cargo doc`) and Python 3.11+ (for `site.py`). To preview `dist/` with Pages' own routing — extensionless addresses, the 404 page — serve it with `npx wrangler pages dev apps/logicaffeine_docs/dist`; `python3 -m http.server -d apps/logicaffeine_docs/dist` is enough to browse it.

## Deployment

The `docs` job in `.github/workflows/test.yml` runs `build.sh` on every push and pull request, so a broken link fails
review rather than the deploy. `.github/workflows/deploy-docs.yml` runs `build.sh` and deploys `dist/` to the Cloudflare Pages project `logicaffeine-docs` (branch `main`) via `wrangler`. It triggers on a successful completion of the `Tests` workflow on `main` (push events), or on manual `workflow_dispatch`. Needs the `CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID` secrets.

## Dependencies

No Cargo manifest, so no crate dependencies. The tooling it relies on:

- **Rust toolchain / `cargo doc`** — produces the rustdoc HTML.
- **The curated workspace crates** — the doc content is whatever `cargo doc --no-deps --lib` emits for them.
- **Python 3.11+** — runs `site.py` (standard library only).
- **`wrangler` + Cloudflare Pages** — deployment only (CI).

## License

MIT License — see [LICENSE.md](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LICENSE.md).

---
[Docs index](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/docs/README.md) · [Root README](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/README.md) · [Changelog](https://github.com/Brahmastra-Labs/logicaffeine/blob/main/CHANGELOG.md)

# LOGICAFFEINE Docs

Rustdoc documentation site for all logicaffeine crates, deployed to `docs.logicaffeine.com`.

## Building Locally

```bash
./build.sh
```

This generates documentation in `dist/` which is served by Cloudflare Pages.

## Deployment

Docs are automatically deployed via GitHub Actions when tests pass on main.
See `.github/workflows/deploy-docs.yml`.

## Structure

```
dist/
├── index.html              # Redirects to logicaffeine_language
├── logicaffeine_language/  # Core language docs
├── logicaffeine_compile/   # Compiler docs
├── logicaffeine_kernel/    # Runtime kernel docs
├── ...                     # Other crate docs
├── static.files/           # Rustdoc assets
└── src/                    # Source code listings
```

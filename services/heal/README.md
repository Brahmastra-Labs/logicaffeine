# services/heal

The worker behind the Studio's Heal button: it takes `{ mode: "code" | "logic", text }` from an
allowed origin, asks the model named in `wrangler.toml` for the same text as clear, idiomatic LOGOS
or logic sentences, and answers `{ ok: true, healed, model }` or `{ ok: false, error }`. Ten heals a
minute an address, 8 KB a heal, JSON only, no request text in any log. The app's side is
`apps/logicaffeine_web/src/ui/pages/heal.rs`.

## Tests

    npm test

## Run it locally

The model key never enters the repository. For a local worker, put it in `.dev.vars` beside this
file (ignored by git) and start wrangler:

    printf 'NVIDIA_API_KEY=...\n' > .dev.vars
    npx wrangler dev --port 8787

Then build the app with `LOGOS_HEAL_URL=http://127.0.0.1:8787/heal` (the Studio reads it at build
time; the dev server on nas2 is `LOGOS_HEAL_URL=http://<nas2>:8787/heal dx serve ...`) and add that
origin to `ALLOWED_ORIGINS` for the dev run with `--var ALLOWED_ORIGINS:http://<nas2>:8080`.

## Deploy

    npx wrangler secret put NVIDIA_API_KEY
    npx wrangler deploy

`deploy-heal.yml` deploys on push once the secret is set in Cloudflare.

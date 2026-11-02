# Marketing site — four pages, no backend, deployable to a CDN

A content site: a front page, an about page, a writing index and a contact form. No server, no
database and no program: a `## Page` mounts itself, so the project is `pages/`, `static/` and a
`Largo.toml`, which is what `largo new <name> --site` makes. `largo build` turns it into a `dist/`
that any static host will serve (GitHub Pages, Netlify, Cloudflare Pages, S3).

```
examples/marketing_site/
├── Largo.toml          # name, and `site` — the address it is published at
├── src/layout.lg       # `## Component Site` — the shared header, nav and footer
├── pages/app.lg        # the front page, `## Page /`
├── pages/about.lg      # `## Page /about`
├── pages/writing.lg    # `## Page /writing`
├── pages/contact.lg    # `## Page /contact` — a bound input and an action
├── pages/not_found.lg  # `## Error page 404`
├── static/theme.css    # design tokens, linked into every page first
├── static/style.css    # the site's rules
└── README.md
```

```bash
cd examples/marketing_site
largo serve          # http://localhost:8080
largo test           # every page's own ## Test
largo build          # dist/ — upload it
```

## What it shows

**A page owns its URL.** `## Page /about` decides where it serves, in development and in
production, from the one rule both follow.

**A page owns its head.** `Head:` is real head HTML — the title in the tab, the description in a
search result, the card a shared link becomes. Written where the page is, merged over sensible
defaults, so a page that declares nothing still gets a charset, a viewport and a favicon.

**The site's address, written once.** `site = "https://acme.example"` in `Largo.toml` gives every
page its canonical URL (`https://acme.example/about` for `/about`) and the `sitemap.xml` its
`<loc>`s, so no page hard-codes the address it is served at. A page whose `Head:` declares its own
canonical link keeps it.

**Content that ships in the document.** Everything renders at build time, so a crawler, a slow
connection and a reader with scripts off all get the real page. The module loads afterwards and
takes over.

**A 404 that is yours.** `## Error page 404` is published as `404.html`, the file every static host
serves for a URL it cannot map, and `largo serve` answers an unknown URL with the same page.

**Site metadata for free.** `robots.txt` and a `sitemap.xml` of the site's canonical URLs, generated
from the pages themselves — and skipped entirely if you ship your own.

**Both halves in one project.** `/contact` is a reactive page with a bound input and an action; the
other three are static content. They are the same kind of file, and each carries a `## Test` that
`largo test` runs.

**One layout, four pages.** `src/layout.lg` declares `## Component Site` once: a header with a
nav and a footer. Every page starts `Import Layout.` and writes `{extends Site}` +
`{block content:}…{end}`, so the chrome is never duplicated, `/contact`'s own action included. The
active link is a plain expression, `class:active={routePath() is equal to "/about"}`, usable
anywhere an expression is, including inside a layout's own view. Each page is pre-rendered at its
declared path, so `dist/writing/index.html` ships with Writing marked active before the module loads
(`prerender_route_lock`), and every page's own `## Test` asserts that its link is the active one.

## Deploying

`dist/` is plain files. A page at `/about` is written `about/index.html`, which is what every host
serves for that URL, and `_headers` carries the same cache rules the compiled binary would send.

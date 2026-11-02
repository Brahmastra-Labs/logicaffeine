# Multi-page site — layout, shared head, nav, 404

Three pages sharing one layout (`src/layout.lg`'s `## Component Site`), each
with its own `Head:`, a nav that marks its own link active, and a
`## Error page 404` for anything else.

```bash
largo test           # runs the ## Test blocks against the tree-walker and the VM
largo serve          # http://localhost:8080, /about, /contact
largo build --static # dist/, including 404.html
```

## What it shows

**One layout, three pages.** `src/layout.lg` declares `## Component Site`
once; each page starts `Import Layout.` and writes `{extends Site}` +
`{block content:}...{end}`, so the header, nav and footer are never
duplicated.

**The active link is a plain expression, not framework machinery.**
`class:active={routePath() is equal to "/about"}` on each `<a>`. `routePath`
is an ordinary function from `docs/frontend.md`'s Navigation section,
usable anywhere an expression is, including inside a layout's own view. A
page answers its own URL wherever it runs: its `## Test` on the tree-walker
and the VM, Chrome visiting `/about`, and the build, which pre-renders
each page at its declared path, so `dist/about/index.html` ships with About
marked active before the module loads (`prerender_route_lock`). Each page's
own `## Test` asserts that its link, and only its link, is the active one.

**Its own head per page.** Each page's `Head:` sets its `<title>` and
`description`; the shared chrome (charset, viewport, theme-color, favicon)
still merges underneath, tag by tag.

**A real 404.** `pages/not_found.lg`'s `## Error page 404` is served for an
unknown URL, with a real `404` status in development and as `dist/404.html`
after `largo build --static` — the file every static host looks for.

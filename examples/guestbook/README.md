# Guestbook — a site with no JavaScript

The other end of the range from `tic_tac_toe`. Every page is rendered on the server and every
change is an ordinary form POST, so this project ships **no WebAssembly at all**. It works with
scripts switched off, in a text browser, on anything.

```bash
cd examples/guestbook
largo serve    # http://localhost:8080
largo test     # the book's own tests
```

## What it shows

**Routes that answer HTML.** `## Route GET / -> View:` is a template, rendered on the server — the
same template language a browser page uses, with a different destination. `## Route POST /say …`
runs statements and returns a `View` built by an ordinary `## To answer … -> View:` function.

**A form that posts.** No `on-submit`, no handler, no module — `<form method="post">` does what HTML
has always done, and `## Route POST /say (who: Text) and (body: Text)` binds the fields by name.

**CSRF, enforced twice.** A `method="post"` form without a `_csrf` field **does not build** — the
error names the missing field. And because the program declares a `## Session`, the router
**checks** the token before `/say` runs: a POST whose `_csrf` is not the one `csrf()` minted for this
visitor is answered `403` and the book is unchanged. The token is bound to a per-visitor nonce the
session carries, so one visitor's token is worthless to anyone else. Safety you cannot forget to
add, because the build and the router will not let you.

**A session.** `## Session` declares what a visitor carries between requests — here the name they
last signed with, which the form offers back. `the session's name` reads and writes it like any
other field; the router loads it from a signed cookie and saves it only when a request changed it.

**A private storage format.** The book is `who|body` lines on disk. That is the server's own shape,
never the wire — it can change without touching anything a visitor sees.

**Records with ordinary English field names.** `a public who, which is Text` — `who` is the word you
reach for on a record about a person, and it is just a field name.

## Honest limits

This demo's POST answers with a page rather than redirecting. That is a choice about how small the
example stays, not a limit: `## Route … -> Response` lowers, so a route can carry its own status and
headers, and `redirectTo` / `permanentRedirectTo` build one — POST/redirect/GET is available if you
want it.

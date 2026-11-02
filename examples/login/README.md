# Login — session, CSRF, a protected route

A signed-in session (`## Session`), a front page at `/` that says who is signed
in, a CSRF-checked sign-in form, a protected
`/dashboard` that redirects an anonymous visitor to `/login`, and the
POST/redirect/GET pattern throughout, exactly as `docs/web.md`'s Sessions
section teaches. One demo account (`ada` / `correct horse`), checked in
LOGOS rather than a database.

```bash
largo test    # runs the ## Test blocks against the route table
largo run     # http://localhost:8080/login
```

## What it shows

**A protected route.** `## Route GET /dashboard` checks `the session's user`
itself and answers `redirectTo("/login")` for an anonymous visitor —
`unauthorized()`'s cousin used the documented way: a redirect, not a bare
401, because the visitor is meant to sign in and continue.

**CSRF, automatically.** The form carries `csrf()` in a hidden `_csrf`
field; the router checks it before `POST /login` ever runs, so a forged or
missing token is a `403` the route itself never has to think about.

**POST/redirect/GET.** `POST /login` never renders a page itself: success
redirects to `/dashboard`, failure redirects back to `/login?failed=1` —
reloading either page is always a plain `GET`, never a resubmission.

## Limits

**`docs/web.md` shows no way to test a CSRF-protected `POST` route end to
end.** `route`, `routeStatus`, `routeContentType`, `routeHeader` and their
`…With` forms (the only testing functions the guide documents) each build a
`Request` with no headers, so every call arrives as a new anonymous visitor
carrying no cookie. A token minted by one call can never satisfy a later one,
and no documented call fetches a token and then submits it as the same
visitor. The repository's own CSRF suite drives `__lgx_route_handler`, an
internal entry point `docs/web.md` never names, with a hand-built `Request`
carrying the first response's `set-cookie` (see
`crates/logicaffeine_tests/tests/csrf_automatic_lock.rs`'s `MK`/`exchange`
helpers). This app's own tests therefore check each route's documented
behaviour independently (the form carries a token, an anonymous visit is
redirected, a tokenless POST is refused) rather than a full
sign-in-then-visit flow.

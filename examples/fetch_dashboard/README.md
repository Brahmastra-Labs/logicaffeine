# Fetch dashboard — loading, error, retry

A typed `Get`, a loading state, an error banner that says what went wrong with
a retry button, and a page-wide failed-fetch counter that is independent of how
the specific call handles its own error.

```
examples/fetch_dashboard/
├── Largo.toml          # a frontend-only project: no backend entry
├── pages/app.lg        # the page, the Metric record, and its tests
├── static/style.css    # served at the site root as /style.css
├── static/theme.css    # the design tokens the page's styles read
└── README.md
```

```bash
largo test           # runs the ## Test blocks on the tree-walker and the VM
largo check          # includes the WebAssembly build a browser runs
largo serve          # http://localhost:8080
largo build --static # dist/ - upload it anywhere
```

The project has no backend, so the tests stub `/api/metrics` with
`Given that GET "/api/metrics" answers …`; point the URL at a real service to
use it for real.

## What it shows

**A typed answer, not a status code to branch on.** `Get "/api/metrics" as
Seq of Metric, then loaded.` hands `loaded` a
`Result of Seq of Metric and HttpError`: `Inspect`'s `Ok`/`Err` arms are the
whole of the error handling for that one call. The `Ok` list is decoded through
the JSON codec derived from `## A Metric has:`; nothing parses the body by hand.

**The error says what went wrong.** `e's message` is the status and the body
the server answered (`500: server exploded`), or what could not be decoded, or
why the request never connected. The banner shows it verbatim, as an `alert`.

**Two places react to a failure.** `loaded`'s own `Err` arm sets the page's
`status`/`lastError` for this one call; `When a fetch fails (url: Text) and
(error: Text):` separately counts every failed request the page ever makes,
whichever action started it. The two are independent: the last test makes two
failed calls and sees two counts, even though `loaded` runs both times too.

**Retry re-issues the identical typed request.** `retry` and `load` are the
same two statements; the test stubs a fresh `Given that` before clicking
retry, and a later stub for the same method and URL is the one a later
request sees.

**The count reads as English.** `{failedFetches | plural "failed fetch" "failed
fetches"}` is the `plural` filter from the standard `ui.lg`: `1 failed fetch`,
`2 failed fetches`.

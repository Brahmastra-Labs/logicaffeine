# Fetch dashboard — loading, error, retry

A typed `Get`, a loading state, an error banner with a retry button, and a
page-wide failed-fetch counter that is independent of how the specific call
handles its own error.

```bash
largo test           # runs the ## Test blocks against the tree-walker and the VM
largo serve          # http://localhost:8080
largo build --static # dist/ - upload it anywhere
```

## What it shows

**A typed answer, not a status code to branch on.** `Get "/api/metrics" as
Seq of Metric, then loaded.` hands `loaded` a
`Result of Seq of Metric and HttpError` — `Inspect`'s `Ok`/`Err` arms are the
whole of the error handling for that one call.

**Two places react to a failure.** `loaded`'s own `Err` arm sets the page's
`status`/`lastError` for THIS call; `When a fetch fails (url: Text) and
(error: Text):` separately counts every failed request the page ever makes,
whichever action started it — the two are independent (the last test proves
it: two failed calls, two counts, even though `loaded` runs both times too).

**Retry re-issues the identical typed request.** `retry` and `load` are the
same two statements; the test stubs a fresh `Given that` before clicking
retry, and a later stub for the same method and URL is the one a later
request sees.

## Limits

**Not yet landed in `examples/`.** `browser_tier_lock` refuses all 5 scenarios: the compiled module
is invalid WebAssembly (`local.set[0] expected type i32, found local.get of type i64` in function
#19) — the same class of backend bug as `examples/signup_form`'s G17, a different site. Finding
**G17b**; RED test
`gauntlet_findings_lock::fetch_dashboards_compiled_module_is_valid_wasm_a_real_engine_can_load`,
owner heap3. Stays in `work/lgx-ease/gauntlet-scratch/` until it lands.

**The error banner does not show the server's own message.** `e's message`
— the documented way to read what went wrong with a typed fetch
(`docs/frontend.md`'s Fetching section: "a status outside 200-299 ... are
all `Err`, with `e's message` saying which") — crashes the WASM AOT backend
with "field access with no static slot", for any `Ok` type (reproduced with
a plain `Int`, no record or list involved). The tree-walker and the lowered
bytecode VM both handle it correctly; only the browser build fails. This app
shows a fixed message instead. Finding **G10** in
`work/lgx-ease/gauntlet.md`; RED test in
`crates/logicaffeine_tests/tests/gauntlet_findings_lock.rs`
(`list_and_variant_lowering_bugs::reading_a_typed_fetch_errors_message_compiles_for_the_browser`).

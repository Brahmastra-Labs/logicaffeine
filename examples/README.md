# Examples

Complete LOGOS projects. Each web example is a project with a `Largo.toml`: `cd` into it and run `largo serve`,
and `largo test` to run its `## Test` blocks. The pages use LGX, the frontend syntax taught in
[docs/frontend.md](../docs/frontend.md); the backends use the routes, sessions and requests of
[docs/web.md](../docs/web.md). The hardware example is a set of standalone programs for `largo emit`.

## A page on its own

| Example | What it shows |
|---------|---------------|
| [counter](counter/) | The smallest LGX page: one `## Page`, one `State:` field, three `Actions:`, one `View:` |
| [tic_tac_toe](tic_tac_toe/) | A whole game as one `## Page` compiled to WebAssembly and served as static files — no backend |
| [stopwatch](stopwatch/) | Two timers on one pattern: a component-owned `Every 1 second as tick:` guarded by a `running` field |
| [todo](todo/) | Add, remove, toggle, filter, a live count, and a list that survives a reload |
| [data_table](data_table/) | Sort, filter and paginate, with `keyed by` so a row's own element follows its data |
| [modal_toast](modal_toast/) | A modal dialog, toasts that leave on their own, and keyboard handling, all driven by `## Test`s |
| [signup_form](signup_form/) | `forms.lg` validation, an error summary, `.prevent`, and a submit button disabled while invalid |
| [file_drop_upload](file_drop_upload/) | A `(file: File)` drop handler reading the dropped file's name, size and type |
| [task_board](task_board/) | A page assembled from components that wrap other components |
| [shopping_cart](shopping_cart/) | A persisted `## Store` of cart lines shared by the page and a panel component that takes its `remove` action as a prop |

## Pages that talk to a server

| Example | What it shows |
|---------|---------------|
| [fetch_dashboard](fetch_dashboard/) | A typed `Get` with a loading state, an error banner with retry, and a page-wide failure counter |
| [infinite_scroll](infinite_scroll/) | A feed that loads the next page each time the visitor reaches the bottom |
| [chat](chat/) | `Publish` / `When a message arrives` delivering a message to every page on a topic, plus a shared `## Store` |
| [login](login/) | A `## Session`, a CSRF-checked sign-in form, and a protected route that redirects an anonymous visitor |
| [fullstack_todo](fullstack_todo/) | Backend and frontend both in LOGOS: four `## Route`s and one `## Page /` compiled to WebAssembly |
| [guestbook](guestbook/) | Server-rendered pages and ordinary form POSTs — a site that ships no WebAssembly at all |

## Sites

| Example | What it shows |
|---------|---------------|
| [marketing_site](marketing_site/) | Four pages, no backend: what `largo new <name> --site` makes, built by `largo build` into a `dist/` for any CDN |
| [multi_page_site](multi_page_site/) | Three pages sharing one layout component, per-page `Head:`, an active nav link, and a `## Error page 404` |

## Hardware

| Example | What it shows |
|---------|---------------|
| [rdu_demo](rdu_demo/) | LOGOS programs lowered for a SambaNova RDU with `largo emit rdu`: a certified placement per program, on a modeled chip, nothing run on hardware |

---
[Root README](../README.md) · [Docs index](../docs/README.md)

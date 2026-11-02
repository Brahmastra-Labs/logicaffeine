# Fullstack Todo — a pure-LOGOS web app

Backend and frontend are both written in LOGOS. The backend is four `## Route`
declarations answering typed values; the frontend is one `## Page /` compiled to
WebAssembly. State lives in one file the backend owns.

```
examples/fullstack_todo/
├── Largo.toml          # entry = src/server.lg
├── src/server.lg       # backend: four ## Route declarations + the todo store
├── src/shared.lg       # the Todo record both halves import
├── pages/app.lg        # frontend: the page, its row component, and its tests
├── static/style.css    # served at the site root as /style.css
└── README.md
```

## Run it

```bash
cd examples/fullstack_todo
largo serve
```

(From a checkout without `largo` installed, `cargo run -p logicaffeine-cli -- serve` in the
same directory runs the same command.)

`largo serve` owns the public port, compiles `pages/app.lg` to WASM, serves `static/` at the
site root, and hands every request the page and the assets do not answer (the `/api/*`
routes) to the backend's `Serve` program. Open **http://127.0.0.1:8080**, type a todo, press
**Add**, tick it, delete it. Each action calls the backend and redraws the list from the
server's answer.

`largo serve` hot-reloads: edit `pages/app.lg` or `src/server.lg` and the page reloads (a page
edit rebuilds the WASM in memory; a backend edit respawns the `Serve` child).

## The wire format: JSON, derived from the answer type

Every route answers `Seq of Todo`, and a route's answer type is what decides the wire format —
so the JSON is derived from the `Todo` record rather than assembled by hand. Nothing in
`server.lg` writes a `{`, and the page reads the answer with `Get "/api/todos" as Seq of Todo`,
through the decoder derived from the same record.

```json
[{"id":1,"text":"buy milk","done":false}]
```

The store on disk is a different thing and deliberately so: `todos.json` holds one todo per line
as `id|text|done`, a format the server owns privately and never shows anyone. The API contract is
the record; the file layout is an implementation detail that could change without the page
noticing.

## State: one file the backend owns

`storePath()` answers `"todos.json"` and that is the whole configuration — no environment
variable, no database, nothing to set up before the demo runs. Each request reads the file,
transforms it, and writes it back, so state survives across a stateless `Serve` with no external
dependency.

That is a demo's store, not a production one: two requests arriving together both
read-modify-write the same file and the second wins. Making it durable is the point at which you
would reach for `postgres.lg`, which is a real pure-LOGOS client proven against PostgreSQL 16 in
`crates/logicaffeine_tests/tests/postgres_live.rs` — this example simply does not use it.

## The API

| Method | Path                        | Parameters      | Answers        |
|--------|-----------------------------|-----------------|----------------|
| GET    | `/api/todos`                | —               | `Seq of Todo`  |
| POST   | `/api/todos`                | `text: Text`    | `Seq of Todo`  |
| POST   | `/api/todos/toggle/:id`     | `id: Int`       | `Seq of Todo`  |
| POST   | `/api/todos/delete/:id`     | `id: Int`       | `Seq of Todo`  |

Each is a `## Route` declaration; `## Main` is one line, `Serve on port 8080.` The `:id` routes
take their parameter from the path itself, so there is no body to parse and no id to pull out of
one by hand.

Everything else (the page, `static/style.css`) is served by `largo serve` under the routing
rule described in the [Web guide](../../docs/web.md): an exact match against a built page or a
static asset wins, then a declared `## Route`. The backend declares no catch-all of its own.

## Shipping it

```bash
largo build --release
```

packs the page, its WASM and `static/` into the backend's binary, `target/release/fullstack_todo`:
one file that serves the whole app, assets first and then the routes, with nothing beside it on
disk. `crates/logicaffeine_tests/tests/todo_deploy_e2e.rs` builds that binary and drives the whole
create/toggle/delete loop through it over real HTTP.

## Known behavior / honest gaps

- **The store is last-writer-wins.** Two requests arriving together both read the file, both
  transform their own copy, and the second write erases the first. This is a demo's store; the
  concurrency story is the reason a real app reaches for a database rather than a file.

## How it is tested

- **`pages/app.lg`'s own `## Test`s** — run by `largo test`. They type into the form, submit
  it, tick and delete a row, answering each request the page sends with `Given that POST … answers
  …`, and check the document after every turn. `examples_lock` requires every action, handler and
  `bind` on the page to be driven by one of them.
- **`crates/logicaffeine_tests/tests/fullstack_todo_lock.rs`** — compiles `pages/app.lg` to WASM,
  drives it under a mock DOM, and for every fetch the page issues calls the backend's store logic
  (on the tree-walker, over an in-memory store) and feeds the response back, proving the two `.lg`
  files interoperate, with no sockets.
- **`crates/logicaffeine_tests/tests/todo_deploy_e2e.rs`** — the release binary, end to end.

Two more tests cover machinery this README mentions but the example does not itself use:
`http_static_lock.rs` (the static-serving helpers and their traversal guard) and
`postgres_live.rs` (the `postgres.lg` client against a real PostgreSQL 16).

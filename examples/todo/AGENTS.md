# AGENTS.md — writing LOGOS

This is a LOGOS project. LOGOS is the English programming language: every
statement is an English sentence that ends with a period, and the compiler
holds you to what the sentence means.

## The loop

Check first, run second. Both speak JSON for you:

```text
largo check --json
largo run --interpret --json
largo run --interpret --max-steps 5000000 --timeout 10s --seed 7
largo fmt --check --json
largo test
largo serve
```

- `largo check --json` prints one `logos-check/1` envelope: `ok` plus
  `diagnostics`, each with a stable kebab-case `code`, a byte+line/col
  `span`, a one-line `message`, and a full Socratic `explanation` that
  teaches the fix.
- `largo run --interpret` executes on the interpreter with no Rust build.
  `largo run` builds a native binary and runs it. Your own generated code is
  untrusted: bound it with `--max-steps N` (deterministic step budget — exit
  122 at the identical op every time, partial output delivered) and
  `--timeout 10s` (wall clock — exit 124). `--seed N` reproduces concurrent
  runs byte-for-byte; `--json` prints a `logos-run/1` envelope.
- `largo test` runs the project's `## Test` blocks deterministically.
- `largo serve` runs the project over HTTP, and what it runs is read from the
  project itself: `pages/` or `static/` is a site (with a `Serve` backend
  beside it, a full-stack site), and a `Serve` program alone is a backend. It
  prints the bound address, honors `LOGOS_PORT`/`PORT`, and drains gracefully
  on SIGTERM. `largo build` on a site exports `dist/` for any static host.
- Look up any diagnostic code or construct offline:

```text
largo explain zero-index
largo explain Let
largo explain use-after-move --json
```

## The five things every agent trips on

1. **Indices start at 1.** `item 1 of xs` is the first element; `item 0`
   is an error (`largo explain zero-index`).
2. **Compare with `is equal to`.** `x is equal to y`, `x equals y` and
   `x is not equal to y` compare any two values. A bare `is` before a number
   (`If n is 5:`) compares too, but `If name is "Ada":` does not parse —
   `is equal to` always does.
3. **`Let` binds, `Set` changes.** `Let x be 5.` then `Set x to 6.` is fine;
   `Let mutable x be 5.` says you mean to change it. In a page's `State:` write
   plain `Let` — every field is changed by `Set` in an action, and `mutable`
   is refused there.
4. **Every statement ends with a period.** Including the last one in a
   block.
5. **Giving moves ownership.** After `Give xs to f.`, `xs` is gone; show it
   (`Show xs.`) to lend, or pass `a copy of xs` to keep it.

## Program shape

```logos
# Main

## Main

Let greeting be "Hello".
Show greeting.
```

Blocks are `##` headers: `## Main` is the entry point, `## To <verb> …:`
defines a function, `## A <Noun> has:` defines a struct, `## Test: <name>`
defines a test.

```logos
## To double (n: Int) -> Int:
    Return n * 2.

## Test: doubling doubles

Expect that double(21) is equal to 42.
```

## The web (projects with a `pages/` directory)

A page IS a component that owns a URL. `## Page /path` says where it serves and
mounts itself — there is no `## Main` and no `Mount` to write. `State:` is what
it knows, `Actions:` what it does, `View:` real HTML with `{…}` holes, `Style:`
real CSS scoped to it. One event is one turn: the action runs, the page paints
once, and if the action fails the state goes back to what it started as. A
`## Test` in the page file drives it and checks the document.

```logos-dom
## Page /

State:
    Let clicks be 0.

Actions:
    To bump:
        Set clicks to clicks + 1.

View:
    <button id="more" on-click={bump}>Click me</button>
    <p id="count">Clicks so far: {clicks}</p>

## Test: the button counts

Click "#more".
Expect that the text of "#count" is equal to "Clicks so far: 1".
```

A `## Component <Name>` is a piece a page places (`<Row todo={t}/>`); it is
mounted by whoever renders it, never by a `Mount` of its own.

Holes are escaped everywhere, including inside attributes — you never call an
escaper. `{if …:}…{else:}…{end}` and `{for each x in xs, keyed by x's id:}` are
the conditionals and loops. `bind={draft}` is two-way on an input.

The server half is `## Route` and `Serve on port 8080.`, and its JSON is
derived from the record — you do not write a serializer, and the compiler
REFUSES a page whose `Get … as T` disagrees with the route that answers it:

```logos
## A Todo has:
    an id, which is Int.
    a text, which is Text.

## To loadTodos () -> Seq of Todo:
    Return a new Seq of Todo.

## Route GET /api/todos -> Seq of Todo:
    Return loadTodos().

## Main

Serve on port 8080.
```

Start one with `largo new mysite --site` (pages only) or `largo new myapp --web`
(pages and a backend). Run it with `largo serve` (rebuilds and reloads the
browser on every save; add `--host 0.0.0.0` to open it on your phone).
`largo check` compiles your pages too and points at `file:line:col`, so a page
that does not build fails the check rather than the deploy, and `largo test`
runs every page's `## Test`. `largo build` exports a site to `dist/`;
`largo build --release` packs a full-stack site's pages, wasm and assets into
ONE binary.

## MCP

The whole toolchain is also an MCP server — structured check, sandboxed
deterministic runs, English→FOL, kernel-certified proofs, and the full
learning guide as resources:

```text
claude mcp add logos -- largo mcp
```

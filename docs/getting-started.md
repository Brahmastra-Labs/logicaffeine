# Getting started

You already describe programs in English every day: in tickets, in code review, in the comment above the tricky
loop. LOGOS lets that description *be* the program, and hands you a compiler that holds you to it. This page takes you
from nothing installed to a tested program, a tested web page, and an English sentence turned into logic. Budget
about fifteen minutes.

None of it needs a Rust toolchain. `largo`, the LOGOS build tool, runs everything in-process until you ask it for a
native binary.

## Install

One line on Linux and macOS (x64 and arm64):

```bash
curl -fsSL https://logicaffeine.com/install.sh | sh
```

Windows:

```powershell
irm https://logicaffeine.com/install.ps1 | iex
```

That puts a single `largo` binary in `~/.local/bin` (Windows: `%LOCALAPPDATA%\Programs\largo`), checked against the
release's `SHA256SUMS`. It never asks for sudo and never edits your shell config; if the directory isn't on your
`PATH`, it prints the line to add. Already have Rust? `cargo install logicaffeine-cli` works too.

## Your first program

```console
$ largo new hello
Created LOGOS project 'hello'
$ cd hello
```

`largo new` wrote a `Largo.toml` manifest and one source file, `src/main.lg`:

<!-- getting-started: generated hello/src/main.lg -->
```logos
# Main

A simple LOGOS program.

## Main

Show "Hello, world!".
```

```output
Hello, world!
```

LOGOS source is Markdown. A `##` header starts a definition (`## Main` is where the program begins), and everything
else is prose the compiler reads past, so a program is its own documentation. Run it:

```console
$ largo run --interpret
Hello, world!
```

`--interpret` runs the program inside `largo`, on the bytecode VM with its JIT turning hot code native, in a
fraction of a second. Plain `largo run` (the command `largo new` suggests) compiles LOGOS to Rust and hands the result
to cargo for a native binary, so that one needs a Rust toolchain. Without one it stops and says so, and `largo doctor` lists exactly what you
have and what each missing piece would unlock.

## A function, and a test for it

Replace `src/main.lg` with this:

<!-- getting-started: write hello/src/main.lg -->
```logos
# Main

A first program with a test.

## To greet (name: Text) -> Text:
    Return "Hello, " + name + "!".

## Main

Show greet("world").

## Test: greet puts the name in the middle

Expect that greet("Ada") is equal to "Hello, Ada!".
```

```output
Hello, world!
```

`## To greet (name: Text) -> Text:` declares a function; the indented lines are its body. The `## Test` block sits
right beside the code it checks, and its name is the sentence it proves.

```console
$ largo run --interpret
Hello, world!
$ largo test
PASS  greet puts the name in the middle
1 passed · 0 failed · 0 skipped · seed 0
```

A test runs in a sealed world: a clock frozen at 2020-01-01, an in-memory filesystem, an empty environment, seeded
randomness and no network, unless the test says in a sentence that it may use the real thing. So a test that passes on
your laptop passes in CI, and a failure replays from its seed. The report ends with a rank; add a `For all` property
and watch it climb.

## A web page

LGX is the LOGOS web framework: a page is one file with its state, its actions in English, its HTML and its CSS, and
it compiles to WebAssembly for the browser.

```console
$ cd ..
$ largo new site --web
Created LOGOS project 'site'
$ cd site
```

A `--web` project has a backend (`src/main.lg`, a set of `## Route`s), a `pages/` directory and a `static/` directory.
Open `pages/app.lg`. Below the prose at the top, you'll find a component, the front page, and a test that clicks the
button:

<!-- getting-started: excerpt site/pages/app.lg -->
```logos-dom
## Component Card (title: Text)

View:
    <section class="card">
      <h1>{title}</h1>
      {children}
    </section>

## Page /

State:
    Let clicks be 0.

Actions:
    To bump:
        Set clicks to clicks + 1.

View:
    <Card title="site is alive">
      <p>This page is LOGOS compiled to WASM.</p>
      <button id="more" on-click={bump}>Click me</button>
      <p id="count">Clicks so far: {clicks}</p>
    </Card>

## Test: the button counts clicks

Click "#more".
Click "#more".
Expect that the text of "#count" is equal to "Clicks so far: 2".
```

One click is one turn: the action runs, the page paints once, and if the action fails the state rolls back. The test
drives the page in-process, no browser required:

```console
$ largo test
PASS  pages/app.lg › the button counts clicks
1 passed · 0 failed · 0 skipped · seed 0
```

Now serve it:

```console
$ largo serve
Serving site on http://127.0.0.1:8080 …
```

Open <http://localhost:8080> and click the button. Save `pages/app.lg` and the browser reloads with your change; a
page that stops compiling shows its error, with the line, in place of the page. Anything the pages and `static/` don't
answer goes to the backend. In a second terminal:

```console
$ curl http://localhost:8080/api/hello
Hello from site!
```

## English into logic

The other half of LOGOS reads English and writes First-Order Logic.

```console
$ largo logic "Every man is mortal."
∀x((Man(x) → Mortal(x)))
$ largo logic --all-readings "Every woman loves a man."
1. ∀x((Woman(x) → ∃y((Man(y) ∧ ∃e(Love(e) ∧ Agent(e, x) ∧ Theme(e, y))))))
2. ∃y((Man(y) ∧ ∀x((Woman(x) → ∃e(Love(e) ∧ Agent(e, x) ∧ Theme(e, y))))))
```

The second sentence really is ambiguous. Reading 1: each woman loves some man, possibly a different one each. Reading
2: there is one man every woman loves. Ask a language model to formalize it and it picks one for you; LOGOS parses, so
it hands you both and lets you choose. The `e` is an event: "loves" becomes a loving event with an agent and a theme,
which is what lets adverbs, tense and "yesterday" attach later without rewriting the formula.

## No install at all

The [Studio](https://logicaffeine.com/studio) runs all of the above in your browser. The parser, the interpreter, the
proof engine and the code generators compile to WebAssembly and run client-side, with Logic, Code, Math, Hardware and
Web modes side by side.

## Where to go next

- **The language:** [Imperative mode](imperative-mode.md): types, collections, ownership, contracts, modules.
- **Web apps:** [Frontend](frontend.md) builds a page up piece by piece, then [Web](web.md) covers the server half.
- **English to logic:** [Logic mode](logic-mode.md): quantifiers, tense, modals, and the long tail.
- **Proofs:** [Proof & verification](proof-and-verification.md): the kernel and what it trusts.
- **Every construct on one page:** the [quick guide](../LOGOS_QUICKGUIDE.md).
- **Every `largo` command:** [The `largo` CLI](cli.md).
- **Working with an agent:** `claude mcp add logos -- largo mcp` gives an AI agent checking, sandboxed runs, English
  to logic and certified proofs as tools; every `largo new` project also ships an `AGENTS.md`.

## Where this is tested

Every command on this page is replayed by
[`getting_started_lock`](../crates/logicaffeine_tests/tests/getting_started_lock.rs) with only `largo` on the `PATH`
(no cargo, no rustc, no node), and each output shown must appear in what `largo` prints. The programs run under
[`doc_examples`](../crates/logicaffeine_tests/tests/doc_examples.rs), and the page runs under
[`canonical_teaching_lock`](../crates/logicaffeine_tests/tests/canonical_teaching_lock.rs) on the tree-walker and
the bytecode VM, compiled for the browser, with its test.

---
[Docs index](README.md) · [Root README](../README.md) · [Changelog](../CHANGELOG.md)

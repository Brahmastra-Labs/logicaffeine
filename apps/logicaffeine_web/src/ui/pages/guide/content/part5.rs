//! Part V: Projects and Tooling — sections: modules, cli-largo, stdlib, building-web-apps,
//! execution-tiers, part-5-checkpoint.

use super::{
    CodeExample, ExampleMode, Exercise, ExerciseKind, RecapQuestion, Runnability, Section,
};

pub(super) const PART_5: &[Section] = &[
    Section {
        id: "modules",
        number: 27,
        title: "Modules",
        part: "Part V: Projects and Tooling",
        content: r#"
Organize large programs across multiple files using the module system.

### Importing Modules

Use `Use` to import a module.

### Qualified Access

Access module contents with the possessive `'s`.

### Creating Modules

Each `.md` file is a module. The filename becomes the module name.

### Visibility

By default, all definitions are public. Mark fields private with no `public` modifier.

The playground runs single-file programs, so the example below shows the module SHAPE
in one file — definitions above, `## Main` below. Multi-file `Use` imports are a
largo-project feature.
"#,
        hook: "One file is fine for a script; a real project wants its vocabulary split by topic — geometry here, billing there. Each file is a module, and the filename is the name.",
        objectives: &[
            "explain how files become modules",
            "name what the playground can and cannot import",
        ],
        misconceptions: &[],
        examples: &[
            CodeExample {
                id: "module-import",
                label: "The Module Shape, in One File",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("5 squared = 25"),
                predict_first: false,
                code: r#"## To square (n: Int) -> Int:
    Return n * n.

## Main
Let x be 5.
Let result be square(x).
Show "5 squared = " + result."#,
            },
        ],
        exercises: &[],
        recap: &[
            RecapQuestion {
                id: "rq-mod-file",
                question: "What makes a module in LOGOS?",
                answer: "A file — the filename becomes the module name.",
            },
            RecapQuestion {
                id: "rq-mod-playground",
                question: "Why does the playground not run `Use` imports?",
                answer: "It runs one file at a time; multi-file imports need a largo project on disk.",
            },
        ],
    },
    Section {
        id: "cli-largo",
        number: 28,
        title: "The CLI: largo",
        part: "Part V: Projects and Tooling",
        content: r#"
LOGOS projects are built with `largo`, the LOGOS build tool.

### Installing

```bash
curl -fsSL https://logicaffeine.com/install.sh | sh
```

Windows, in PowerShell: `irm https://logicaffeine.com/install.ps1 | iex`.
Prebuilt for Linux/macOS (x64 + arm64) and Windows x64, SHA-256-verified, installed to
`~/.local/bin` with no sudo. `--full` bundles Z3 static verification.

### Creating a Project

| Command | Description |
|---------|-------------|
| `largo new <name>` | Create a new project in a new directory |
| `largo init` | Initialize a project in the current directory |

This creates a `Largo.toml` manifest and `src/main.lg` entry point.

### Build Commands

| Command | Description |
|---------|-------------|
| `largo build` | Compile the project to a native binary |
| `largo build --release` | Compile with optimizations |
| `largo run` | Build and run |
| `largo run --interpret` | Run on the interpreter—no Rust build, sub-second feedback |
| `largo run --release` | Build and run with optimizations |
| `largo check` | Type-check without compiling |
| `largo verify` | Run Z3 static verification (the full build) |
| `largo build --verify` | Build with verification |
| `largo build --target wasm` | Cross-compile to WebAssembly |
| `largo opts <file>` | Report which optimizations actually fire |

### The Wider Verbs

| Command | Description |
|---------|-------------|
| `largo repl` | Interactive session: imperative statements + English→FOL logic mode |
| `largo logic "<sentence>"` | English → First-Order Logic (`--all-readings`, `--format latex`) |
| `largo prove [file]` | Kernel-certified theorem proving (`## Theory` / `## Theorem` blocks) |
| `largo sat <file.cnf>` | The certified SAT solver (DIMACS in, DRAT proofs out) |
| `largo fmt [--check]` | Format sources (the LSP's rules) |
| `largo emit <rust\|c\|wasm>` | Print or write the generated code |
| `largo doc` | Generate markdown docs from a project's `##` blocks |
| `largo add / remove <dep>` | Edit `Largo.toml` dependencies (format-preserving) |
| `largo clean` | Remove build artifacts |
| `largo completions <shell>` | Shell tab-completion scripts |

### Package Registry

Publish and manage packages on the LOGOS registry:

| Command | Description |
|---------|-------------|
| `largo login` | Authenticate with the registry |
| `largo publish` | Publish your package |
| `largo publish --dry-run` | Validate without publishing |
| `largo logout` | Log out from the registry |

### Project Manifest

The `Largo.toml` file defines package metadata and dependencies:

```toml
[package]
name = "myproject"
version = "0.1.0"
entry = "src/main.lg"

[dependencies]
```
"#,
        hook: "",
        objectives: &[
            "choose the right largo subcommand for check, run, build, and prove workflows",
            "run a program on the interpreter for sub-second feedback",
        ],
        misconceptions: &[],
        examples: &[],
        exercises: &[],
        recap: &[
            RecapQuestion {
                id: "rq-largo-check",
                question: "Which command type-checks without compiling?",
                answer: "`largo check`.",
            },
            RecapQuestion {
                id: "rq-largo-interp",
                question: "Which flag runs a program with no Rust build at all?",
                answer: "`largo run --interpret` — sub-second feedback on the interpreter.",
            },
            RecapQuestion {
                id: "rq-largo-prove",
                question: "Which command certifies Theorem blocks?",
                answer: "`largo prove` — kernel-certified, not merely claimed.",
            },
        ],
    },
    Section {
        id: "stdlib",
        number: 29,
        title: "Standard Library",
        part: "Part V: Projects and Tooling",
        content: r#"
LOGOS provides built-in functions for common operations.

### Currently Available

These built-ins work in both the playground and compiled programs:

- `Show x.` — Output values to the console
- `length of x` — Get the length of a list or text
- `format(x)` — Convert any value to text
- `abs(n)` — Absolute value of a number
- `min(a, b)` — Minimum of two integers
- `max(a, b)` — Maximum of two integers

### Standard Library Modules

These modules import themselves the moment you call them — no `Use` line needed:

| Module | Functions |
|--------|-----------|
| **file** | `read(path) -> Result of Text and Text`, `write(path, content) -> Result of Unit and Text` |
| **time** | `now() -> Nat` (Unix milliseconds), `sleep(ms)` |
| **random** | `randomInt(min, max) -> Int`, `randomFloat() -> Real` |
| **env** | `get(key) -> Option of Text`, `args() -> Seq of Text` |
| **crypto** | Post-quantum ML-KEM-768, ChaCha20, SHA-3/Keccak — written in LOGOS itself |
| **uuid** | RFC 9562 UUIDs, all versions |

Every stdlib definition carries literate `## Note` documentation — hover over a call in the
editor, or run `largo doc`, to read it.

File, environment, and network access follow the host program: compiled programs get the real
operating system, while the browser playground is sandboxed, so those calls are limited here.
"#,
        hook: "Before reaching for a library: absolute value, min and max, lengths, formatting — the everyday verbs are already here, and the bigger modules (file, time, random, crypto) import themselves the moment you call them.",
        objectives: &[
            "use the core builtins without any imports",
            "name which stdlib modules auto-import on first call",
            "distinguish playground-safe calls from host-dependent ones",
        ],
        misconceptions: &[],
        examples: &[
            CodeExample {
                id: "stdlib-example",
                label: "Standard Library",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("Built-in functions:\nlength of nums = 5\nlength of text = 5\nabs(-42) = 42\nmin(10, 3) = 3\nmax(10, 3) = 10"),
                predict_first: false,
                code: r#"## Main
Let nums be [5, -3, 8, -1, 4].
Let text be "Hello".

Show "Built-in functions:".
Show "length of nums = " + format(length of nums).
Show "length of text = " + format(length of text).
Show "abs(-42) = " + format(abs(-42)).
Show "min(10, 3) = " + format(min(10, 3)).
Show "max(10, 3) = " + format(max(10, 3))."#,
            },
            CodeExample {
                id: "stdlib-file-read",
                label: "Reading a File (Result-Typed)",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: None,
                predict_first: false,
                code: r#"## Main
Let r be read("notes.txt").
Inspect r:
    When Ok (content):
        Show content.
    When Err (e):
        Show "could not read: " + e."#,
            },
            CodeExample {
                id: "stdlib-sha3",
                label: "SHA3-256 (Written in LOGOS)",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("32"),
                predict_first: false,
                code: r#"## Main
Let digest be sha3_256(text_bytes("abc")).
Show length of digest."#,
            },
        ],
        exercises: &[
            Exercise {
                id: "ex-stdlib-hole",
                kind: ExerciseKind::Completion,
                concepts: &["core builtins"],
                prompt: "Fill the hole with the builtin that turns -42's minimum into 42.",
                mode: ExampleMode::Imperative,
                expected: "42",
                starter: "## Main\nShow ???(min(-42, 7)).",
                hints: &[
                    "min picks -42; which builtin removes the sign?",
                    "Three letters, no import.",
                ],
                reference: "## Main\nShow abs(min(-42, 7)).",
            },
            Exercise {
                id: "ex-stdlib-clamp",
                kind: ExerciseKind::Practice,
                concepts: &["core builtins"],
                prompt: "Print abs(min(-42, 7)); then clamp 15 into the range 1..10 by composing max and min.",
                mode: ExampleMode::Imperative,
                expected: "42\n10",
                starter: "## Main\n",
                hints: &[
                    "min picks the smaller — which of -42 and 7 is that?",
                    "Clamping composes: the max of (the min of x and the ceiling) and the floor.",
                ],
                reference: "## Main\nShow abs(min(-42, 7)).\nShow max(min(15, 10), 1).",
            },
        ],
        recap: &[
            RecapQuestion {
                id: "rq-stdlib-core",
                question: "Which everyday builtins need no import at all?",
                answer: "Show, length of, format, abs, min, max — always in scope.",
            },
            RecapQuestion {
                id: "rq-stdlib-auto",
                question: "How do the file/time/random/crypto modules get imported?",
                answer: "They import themselves the moment you call them — no Use line.",
            },
            RecapQuestion {
                id: "rq-stdlib-host",
                question: "Why is `read(path)` compiled-only?",
                answer: "The playground has no host filesystem; compiled programs get the real operating system.",
            },
        ],
    },
    Section {
        id: "building-web-apps",
        number: 30,
        title: "Building Web Apps",
        part: "Part V: Projects and Tooling",
        content: r##"
A LOGOS web page is one file. `## Page /` declares the page and the path it serves at, and the page
mounts itself: no `## Main`, no `Mount`. `State:` is what it knows, `Actions:` says what it does,
`View:` is real HTML with `{…}` holes, and `Style:` is CSS scoped to it. Press **Open page** on an
example below: it compiles to WebAssembly right here and runs in the frame beneath it, the same
preview the Studio's Web tab shows.

### A page, and the turn

One event is one **turn**: the action runs, derived fields recompute, and the page repaints once.
A turn is a transaction — if an action fails partway, every field it touched goes back to where the
turn started, and the page stays live. `on-click={bump}` names the action that handles the click;
nothing in the markup changes state directly.

Each example ends with a `## Test`. A test drives the page the way a visitor does — `Click`,
`Type … into`, `Submit` — and asserts on what is on screen. The page is already mounted when the
test starts. `largo test` runs every test in a project, and the guide's own suite runs these.

### Lists and identity

`{for each t in todos, keyed by t's id:}` … `{end}` repeats its body per item, with an `{else:}`
arm for an empty list. The key gives each row an identity, so when an item is removed the rows
after it keep their elements — their focus and anything typed into them — instead of being rewritten
in place. `Let left always be …` is a **derived** field: it recomputes whenever what it reads changes.

`bind={draft}` connects an input to a field both ways, and `on-submit.prevent={add}` handles a form
without the browser's own page load.

### Components

`## Component Card (title: Text)` declares a part a page places with `<Card title="…">…</Card>`. Its
header lists its **props**, which are read-only inside it; `{children}` marks where the markup
written between its tags lands. A component that keeps state of its own declares `State:` and
`Actions:` exactly as a page does, and each placement is its own instance.

### Running a site

`largo new mysite --web` scaffolds a project: pages in `pages/`, static files in `static/`, and a
backend of `## Route`s in `src/main.lg`. `largo serve` builds every page, serves the site, and
reloads the browser on every save; `largo build --static` writes a `dist/` any static host serves,
with each page pre-rendered so it reads before any script runs. A view also renders to text on a
server: `html of Greeting` is a component's markup, every hole escaped.
"##,
        hook: "A button that counts clicks needs a number, a handler, and a repaint — and in most frameworks a fourth piece: the bookkeeping that keeps the other three in step. One missed update and the page shows a count that is no longer true.",
        objectives: &[
            "write a page with State:, Actions: and a View: of real HTML",
            "trace one click from its handler to the single repaint that ends the turn",
            "build a list that keeps each row's identity with keyed by",
            "write a ## Test that drives a page and checks what it shows",
            "run a site with largo serve and publish it with largo build --static",
        ],
        misconceptions: &[],
        examples: &[
            CodeExample {
                id: "web-counter-page",
                label: "A page that counts",
                mode: ExampleMode::Web,
                runnability: Runnability::Playground,
                expected_output: None,
                predict_first: false,
                code: "## Page /\n\nState:\n    Let clicks be 0.\n    Let label always be \"clicked {clicks} times\".\n\nActions:\n    To bump:\n        Set clicks to clicks + 1.\n\n    To reset:\n        Set clicks to 0.\n\nView:\n    <button id=\"more\" on-click={bump}>+1</button>\n    <button id=\"reset\" on-click={reset}>reset</button>\n    <p id=\"count\">{label}</p>\n\n## Test: each click is one turn\n\nClick \"#more\".\nClick \"#more\".\nExpect that the text of \"#count\" is equal to \"clicked 2 times\".\nClick \"#reset\".\nExpect that the text of \"#count\" is equal to \"clicked 0 times\".\n",
            },
            CodeExample {
                id: "web-keyed-list",
                label: "A list you add to and remove from",
                mode: ExampleMode::Web,
                runnability: Runnability::Playground,
                expected_output: None,
                predict_first: false,
                code: "## A Task has:\n    an id, which is Int.\n    a text, which is Text.\n\n## Page /\n\nState:\n    Let tasks be a new Seq of Task.\n    Let draft be \"\".\n    Let nextId be 1.\n    Let left always be length of tasks.\n\nActions:\n    To add:\n        If draft is equal to \"\":\n            Return.\n        Push (a new Task with id nextId and text draft) to tasks.\n        Set nextId to nextId + 1.\n        Set draft to \"\".\n\n    To remove (id: Int):\n        Let mutable kept be a new Seq of Task.\n        Repeat for t in tasks:\n            If t's id is not equal to id:\n                Push t to kept.\n        Set tasks to kept.\n\nView:\n    <form id=\"add\" on-submit.prevent={add}>\n      <input id=\"draft\" type=\"text\" aria-label=\"New item\" bind={draft}>\n      <button type=\"submit\">Add</button>\n    </form>\n    <ul>\n      {for each t in tasks, keyed by t's id:}\n        <li class=\"task\">{t's text} <button id=\"remove-{t's id}\" on-click={remove(t's id)}>remove</button></li>\n      {else:}\n        <li class=\"empty\">Nothing to do.</li>\n      {end}\n    </ul>\n    <p id=\"left\">{left} left</p>\n\n## Test: tasks are added and removed\n\nType \"milk\" into \"#draft\".\nSubmit \"#add\".\nType \"bread\" into \"#draft\".\nSubmit \"#add\".\nExpect that the count of \".task\" is equal to 2.\nClick \"#remove-1\".\nExpect that the count of \".task\" is equal to 1.\nExpect that the text of \"#left\" is equal to \"1 left\".\n",
            },
            CodeExample {
                id: "web-component-props",
                label: "A component with props and children",
                mode: ExampleMode::Web,
                runnability: Runnability::Playground,
                expected_output: None,
                predict_first: false,
                code: "## Component Card (title: Text)\n\nView:\n    <section class=\"card\"><h2>{title}</h2>{children}</section>\n\nStyle:\n    .card { border: 1px solid #d0d0d0; border-radius: 8px; padding: 1rem; }\n\n## Page /\n\nState:\n    Let unread be 3.\n\nView:\n    <Card title=\"Inbox\"><p id=\"unread\">{unread} unread</p></Card>\n\n## Test: the caller's markup lands inside the card\n\nExpect that the text of \".card h2\" is equal to \"Inbox\".\nExpect that the text of \"#unread\" is equal to \"3 unread\".\n",
            },
        ],
        exercises: &[
            Exercise {
                id: "ex-web-reset",
                kind: ExerciseKind::Completion,
                concepts: &["an action as an event handler"],
                prompt: "Wire the reset button so its test passes: a click on it must run the action that sets the count back to zero.",
                mode: ExampleMode::Web,
                expected: "",
                starter: "## Page /\n\nState:\n    Let clicks be 0.\n\nActions:\n    To bump:\n        Set clicks to clicks + 1.\n\n    To reset:\n        Set clicks to 0.\n\nView:\n    <button id=\"more\" on-click={bump}>+1</button>\n    <button id=\"reset\" on-click={???}>reset</button>\n    <p id=\"count\">{clicks}</p>\n\n## Test: reset brings the count back to zero\n\nClick \"#more\".\nClick \"#more\".\nClick \"#reset\".\nExpect that the text of \"#count\" is equal to \"0\".\n",
                hints: &[
                    "A handler names an action declared under `Actions:`.",
                    "Two actions are declared; one of them already handles the other button.",
                ],
                reference: "## Page /\n\nState:\n    Let clicks be 0.\n\nActions:\n    To bump:\n        Set clicks to clicks + 1.\n\n    To reset:\n        Set clicks to 0.\n\nView:\n    <button id=\"more\" on-click={bump}>+1</button>\n    <button id=\"reset\" on-click={reset}>reset</button>\n    <p id=\"count\">{clicks}</p>\n\n## Test: reset brings the count back to zero\n\nClick \"#more\".\nClick \"#more\".\nClick \"#reset\".\nExpect that the text of \"#count\" is equal to \"0\".\n",
            },
        ],
        recap: &[
            RecapQuestion {
                id: "rq-web-mount",
                question: "What does a `## Page` file need in place of `## Main` and `Mount`?",
                answer: "Nothing: `## Page /path` mounts itself at its path. `Mount` belongs in a `## Test` that renders one component on its own.",
            },
            RecapQuestion {
                id: "rq-web-turn",
                question: "An action sets two fields and then fails. What does the page show afterwards?",
                answer: "Both fields as they were before the turn: a turn is a transaction, rolled back whole, and the failure goes to `When an action fails`.",
            },
            RecapQuestion {
                id: "rq-web-key",
                question: "What does `keyed by t's id` keep that an unkeyed list loses?",
                answer: "Each row's identity: removing or reordering items moves each row's element, with its focus and typed text, along with its item.",
            },
        ],
    },
    Section {
        id: "execution-tiers",
        number: 31,
        title: "How LOGOS Runs: The Five Tiers",
        part: "Part V: Projects and Tooling",
        content: r#"
One program, five ways to execute it — you choose the tradeoff, the semantics never change:

| Tier | What | When |
|------|------|------|
| Tree-walking interpreter | runs the AST directly | this playground, `largo run --interpret` |
| Register bytecode VM | compact bytecode, `Int` fast paths | live sync, the browser |
| EXODIA JIT | copy-and-patch native x86-64 | hot functions tier up automatically |
| AOT Rust | `largo build` — full native binary | production (the 11-language benchmark winner) |
| Direct WASM | `largo build --emit wasm`, no rustc | milliseconds to a `.wasm` module |

Translation validation proves the compiled tiers faithful: the emitted Rust is checked
equivalent to the source semantics with an SMT solver — you do not have to trust the
compiler, you can check it.

### Steering the optimizer

`## No <Name>` decorators disable a named optimization for the program (forty exist —
`Memo`, `Tco`, `Inline`, `Unroll`, …), and `## Tier opt` pins the VM's tier-up policy.
Both parse and run everywhere; they *matter* to the compiled tiers. `largo opts <file>`
reports which optimizations actually fired.

> **Mental model:** One recipe, five kitchens: a tasting counter (interpreter), a prep
> line (VM), a flash grill (JIT), a full restaurant build (AOT Rust), and a food truck
> (direct WASM). Same dish every time — translation validation is the taste test that
> proves the plating identical. Only the speed changes; the menu never does.

*Try it: both examples below print the same 42 — the decorators change how it runs, never
what it means.*
"#,
        hook: "The same program runs five ways — interpreted in this page, JIT-compiled mid-loop, or built to a native binary. You choose the tradeoff; the meaning is proven identical, so the choice is never a risk.",
        objectives: &[
            "name the five execution tiers and when each fires",
            "use ## No and ## Tier decorators without changing meaning",
            "explain what translation validation proves",
        ],
        misconceptions: &[],
        examples: &[
            CodeExample {
                id: "tier-decorator",
                label: "Pinning the Tier Policy",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("42"),
                predict_first: false,
                code: r#"## Tier opt eager

## Main
Show 6 * 7."#,
            },
            CodeExample {
                id: "no-opt-decorator",
                label: "Disabling One Optimization",
                mode: ExampleMode::Imperative,
                runnability: Runnability::Playground,
                expected_output: Some("42"),
                predict_first: false,
                code: r#"## No Memo

## Main
Show 6 * 7."#,
            },
        ],
        exercises: &[],
        recap: &[
            RecapQuestion {
                id: "rq-tiers-five",
                question: "What are the five ways one LOGOS program can execute?",
                answer: "Tree-walking interpreter, register VM, EXODIA JIT, AOT Rust, and direct WASM.",
            },
            RecapQuestion {
                id: "rq-tiers-decorators",
                question: "What do ## No and ## Tier decorators change — and what do they never change?",
                answer: "How the program runs (optimizations, tier-up policy); never what it means.",
            },
            RecapQuestion {
                id: "rq-tiers-tv",
                question: "What does translation validation prove?",
                answer: "That the compiled output is semantically equivalent to the source — checked with an SMT solver, not trusted.",
            },
        ],
    },
    Section {
        id: "part-5-checkpoint",
        number: 32,
        title: "Checkpoint: Projects and Tooling",
        part: "Part V: Projects and Tooling",
        content: r#"
Modules, largo, the stdlib, and the five tiers — the working-programmer's toolkit.
Three challenges blend them with earlier parts, then the recall cards collect.
"#,
        hook: "",
        objectives: &[
            "combine builtins, decorators, and earlier-part constructs in working programs",
            "diagnose which Part V section to reread when a challenge stumps you",
        ],
        misconceptions: &[],
        examples: &[],
        exercises: &[
            Exercise {
                id: "cp5-tier-len",
                kind: ExerciseKind::Challenge,
                concepts: &["execution tiers + opt decorators", "core builtins"],
                prompt: "Disable the Memo optimization with a decorator, then print the length of \"LOGOS\" — same answer, by design.",
                mode: ExampleMode::Imperative,
                expected: "5",
                starter: "## Main\n",
                hints: &[
                    "Decorators sit above ## Main: `## No <Name>`.",
                    "`length of` counts characters of Text too.",
                ],
                reference: "## No Memo\n\n## Main\nShow length of \"LOGOS\".",
            },
            Exercise {
                id: "cp5-clamp-fn",
                kind: ExerciseKind::Challenge,
                concepts: &["core builtins", "function definition"],
                prompt: "Write clamp(x, lo, hi) from min and max; show clamp(12, 1, 10) then clamp(0, 1, 10).",
                mode: ExampleMode::Imperative,
                expected: "10\n1",
                starter: "## Main\nShow clamp(12, 1, 10).\nShow clamp(0, 1, 10).",
                hints: &[
                    "One Return: the min of (the max of x and lo) and hi.",
                    "12 exceeds the ceiling; 0 undercuts the floor.",
                ],
                reference: "## To clamp (x: Int) and (lo: Int) and (hi: Int) -> Int:\n    Return min(max(x, lo), hi).\n\n## Main\nShow clamp(12, 1, 10).\nShow clamp(0, 1, 10).",
            },
            Exercise {
                id: "cp5-len-report",
                kind: ExerciseKind::Challenge,
                concepts: &["core builtins", "string interpolation"],
                prompt: "Compute the length of \"tiers\" into a variable, then print `len: 5` by interpolating it.",
                mode: ExampleMode::Imperative,
                expected: "len: 5",
                starter: "## Main\n",
                hints: &[
                    "`length of` phrases don't parse inside braces — compute first.",
                    "Then interpolate the name: {n}.",
                ],
                reference: "## Main\nLet n be length of \"tiers\".\nShow \"len: {n}\".",
            },
        ],
        recap: &[
            RecapQuestion {
                id: "cq5-check",
                question: "Fast feedback loop: which two largo invocations skip rustc entirely?",
                answer: "`largo check` (type-check only) and `largo run --interpret`.",
            },
            RecapQuestion {
                id: "cq5-auto",
                question: "What triggers a stdlib module's import?",
                answer: "The first call into it — demand-imported, no Use line.",
            },
            RecapQuestion {
                id: "cq5-tiers",
                question: "Which tier runs in this page, and which produces a native binary?",
                answer: "The tree-walking interpreter here; AOT Rust via largo build.",
            },
            RecapQuestion {
                id: "cq5-tv",
                question: "Why can you trust largo build without trusting the compiler?",
                answer: "Translation validation checks the emitted code equivalent to the source with an SMT solver.",
            },
            RecapQuestion {
                id: "cq5-module",
                question: "What is the unit of modularity in a largo project?",
                answer: "The file — one file, one module, named by its filename.",
            },
            RecapQuestion {
                id: "cq5-decorator",
                question: "Can a ## No decorator change a program's output?",
                answer: "No — decorators steer how it runs, never what it means (and the suite locks that).",
            },
        ],
    },
];

//! News article data and content.

/// News article structure
#[derive(Clone, Debug)]
pub struct Article {
    pub slug: &'static str,
    pub title: &'static str,
    pub date: &'static str,
    pub summary: &'static str,
    pub content: &'static str,
    pub tags: &'static [&'static str],
    pub author: &'static str,
}

/// Format a tag for display (e.g., "formal-logic" -> "Formal Logic")
pub fn format_tag(tag: &str) -> String {
    tag.split('-')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect(),
                None => String::new(),
            }
        })
        .collect::<Vec<String>>()
        .join(" ")
}

/// Get all articles sorted by date (newest first)
pub fn get_articles() -> Vec<&'static Article> {
    let mut articles: Vec<&'static Article> = ARTICLES.iter().collect();
    articles.sort_by(|a, b| b.date.cmp(a.date));
    articles
}

/// Get a single article by slug
pub fn get_article_by_slug(slug: &str) -> Option<&'static Article> {
    ARTICLES.iter().find(|a| a.slug == slug)
}

/// Get all unique tags from articles
pub fn get_all_tags() -> Vec<&'static str> {
    let mut tags: Vec<&'static str> = ARTICLES
        .iter()
        .flat_map(|a| a.tags.iter().copied())
        .collect();
    tags.sort();
    tags.dedup();
    tags
}

/// Get articles filtered by tag
pub fn get_articles_by_tag(tag: &str) -> Vec<&'static Article> {
    let mut articles: Vec<&'static Article> =
        ARTICLES.iter().filter(|a| a.tags.contains(&tag)).collect();
    articles.sort_by(|a, b| b.date.cmp(a.date));
    articles
}

/// All news articles
static ARTICLES: &[Article] = &[
    Article {
        slug: "release-0-11-0-lgx-full-stack-web-apps",
        title: "v0.11.0 — LGX: Full-Stack Web Apps in LOGOS, Tested on Every Tier",
        date: "2026-11-02",
        summary: "LGX arrives: full-stack web applications written in LOGOS, with pages, components, stores, typed routes, signed sessions, CSRF enforcement and a web standard library, lowered to one implementation that the tree-walker, the bytecode VM and the browser's WebAssembly all run and that largo test checks on each, headless Chrome included. Also new since 0.10.1: tests, observation, replay and reverse debugging in the language; a standard library written in LOGOS; multi-file programs and a stricter checker; C as a general backend beside new RISC-V and CUDA tiers, sharing one scheduler; a kernel whose integers are Lean's, and Lean 4 import; an agent-facing toolchain; native HTTP and TLS; compilation to Tenstorrent, FPGAs and sky130, a language model's decode step on two FPGA boards and a decode engine for the Intel PAC D5005; and infinitesimals, hyperreals and supertasks computed exactly.",
        content: r#"
I have built web apps the usual way for a long time: a front end in one language, a back end in another, a build pipeline between them, and a test suite that mostly checks the seams. This release is the version of that I wanted. A page and its server are both LOGOS, the page your browser runs is the same program the interpreter and the VM run, and `largo test` checks it on all three, so a test that passes has passed on the thing you ship. The rest of the cycle filled in what a real application leans on: tests and replay in the language, a standard library written in itself, and backends from C to CUDA.

v0.11.0 follows 0.10.1 (2026-07-11). Its headline is LGX: a LOGOS program can now be a web application, pages and server both, and the page a browser runs is the same lowered LOGOS the tree-walker and the bytecode VM run, checked against both by `largo test`. Around it, the language gains tests, observation and replay as statements, a standard library written in itself, multi-file programs with the type checker run on every join; C becomes a general backend beside new RISC-V and CUDA tiers; the kernel's integers become Lean's and Lean 4 developments import; and the toolchain grows interfaces built for agents. The compiler also lowers a language model's decode step to FPGAs, from a tiny int8 transformer generating on two Artix-7 boards to a Qwen3.8-27B decode engine for the Intel PAC D5005 held bit for bit from its arithmetic to its Verilog, and exact numbers reach past the reals: infinitesimals, hyperreals and supertasks that end in their exact state. The release's section of `CHANGELOG.md` is the complete record, with a lock for every claim; the breaking changes are summarized at the end.

## LGX: full-stack web applications in LOGOS

A `## Page /path` file is a page, a `## Route` is a typed HTTP endpoint, and one project serves itself in development, exports a static site and builds into a single binary; none of this existed in 0.10.1. A page lowers to ordinary LOGOS, and that lowered program is the only implementation: the tree-walker runs it as the reference, the VM runs it, and the browser runs the WebAssembly it compiles to, with a small host (`crates/logicaffeine_compile/assets/web/runtime.js`) that applies the page's patch stream and routes events. `docs/frontend.md` teaches the framework in the order an app is built, `docs/web.md` teaches the server half, and `docs/lgx-reference.md` lists every construct.

### Pages and components

`## Page /todos/:id (id: Int)` binds typed path segments, and a page file with no header is routed by its name. A page is sections of one file: `State:` is a record of `Let` fields, `Actions:` holds the only code that changes it, `View:` is strict HTML, and `Style:` is CSS scoped to its component by a real tokenizer. Templates have filters (`{x | f}`), conditionals, keyed loops (`{for each x in xs, keyed by k, at index i:}`), `{inspect}`, layouts (`{extends Site}`, `{block}`) and `{children}`; every element closes, `<script>` and `<style>` are refused, holes are escaped, and another template language's spelling (`elif`, `endfor`) is refused with the LGX one. `Head:` merges a page's head over the defaults, `## Directive` gives an attribute behavior on the real element, and `## Error page 404` answers an unmatched URL. `## Component Name (props)` is placed as `<Name prop={x}/>`: props are read-only, a `fn(T) -> Unit` prop runs the parent's action as the parent's turn, every placement is its own instance, and a missing, extra or mistyped prop is refused (`component_model_lock`, `lgx_template_lock`).

### Turns, stores, triggers and events

One event is one turn: the action runs, derived fields recompute, and the page repaints once. A turn is a transaction: a failing action rolls back every field it touched and reaches the page's `When an action fails` handler, naming the component it came from, and the page keeps answering (`turn_rollback_fuzz`). `## Store Cart` shares state and actions across components; `preserved across reloads` keeps it for the tab and `persisted as "cart"` in local storage. `## Store S shared on "topic"` replicates a store to every page and tab on the topic through the relay, its `Tally`, `ConvergentCount`, `SharedSet` and `SharedSequence` fields CRDTs their author reads as plain values (`shared_store_crdt_lock`), and `Publish m on "topic".` reaches every `When a message arrives on "topic"` handler once. `When <Store>'s <field> changes:` wakes each component that uses the store once per commit, and a chain of wakes past 16 is refused by name; a component placing itself is refused past a depth of 32; `When this component appears:` runs after the paint that drew it; and timers run on the page's own clock against absolute due times. Event modifiers (`.prevent`, `.stop`, `.once`, `.capture`, `.passive`) behave the same on every tier, and an unknown one is refused with the nearest spelling.

### Data, forms and navigation

`Get "/api/todos" as Seq of Todo, then loaded.`, and its `Post`, `Put`, `Patch`, `Delete` and multipart `Upload` siblings, hand the action a `Result of T and HttpError`: a status outside 200–299, a body that does not decode and a refused connection are each an `Err`, and a typed fetch completes in-process on the tree-walker and the VM as it does in a browser. `bind={field}` connects a form control to a field both ways and keeps focus and caret across repaints, and `required`, `pattern` and `min`/`max` block a submit on every tier as the browser does. `Navigate to` a path the page's own route serves stays in the page, Back included (`typed_fetch_lock`, `forms_lock`).

### Typed routes, sessions and CSRF

`## Route GET /posts/:id (id: Int) -> Text:` declares one path, and the answer type picks the content type: `Text`, `View` as HTML, a record as JSON through a derived codec, or `Response` for status, headers and cookies. A segment typed `Int` that is not one is answered `400`. `Serve on port 8080.` answers every request concurrently from the route table; `route`, `routeStatus` and `routeHeader` answer a request with no socket, so a server is tested with `Expect`; and `## Before each request` and `## Redirect /old -> /new.` cover guards and moved URLs. `## Session` declares the visitor's fields, carried in one HMAC-SHA-256-signed cookie. With a session declared, a state-changing request without the visitor's token is answered `403` before the route runs, a state-changing `<form>` without `_csrf` does not compile, and a page's own scripted requests carry the token, which `largo serve` and a built binary place in each page document. A `## To` function that reads or writes a `## Main` binding builds on AOT-Rust and C, the bindings becoming one record lent to it, so a session site ships as a binary (`session_lock`, `csrf_lock`, `aot_program_record_lock`).

### The web standard library

The server half is LOGOS source under `crates/logicaffeine_compile/assets/std/`: `http.lg` builds responses and cookies (`HttpOnly; SameSite=Lax; Secure` by default, refusing a value that would split the header); `web.lg` escapes and reads queries, form bodies and multipart uploads; `static.lg` is the one MIME table `largo serve` and `largo build` both read, and `staticUnder` refuses any `..` segment before touching disk; `session.lg`, `forms.lg` and `ui.lg` hold signed sessions, validation and pagination (`docs/lgx-stdlib.md`); and `json.lg` parses RFC 8259 with exact decimals. `tcp.lg` opens sockets and certificate-verified TLS connections, and `postgres.lg` speaks the PostgreSQL v3 protocol with SCRAM-SHA-256 authentication, parameterized and prepared statements, transactions, cancellation and `pgConnectSecure`, which is TLS or a refusal (`postgres_protocol` pins every frame byte for byte).

### One implementation, one renderer

`crates/logicaffeine_compile/assets/std/vdom.lg` is the reconciler, written in LOGOS. A commit diffs the new tape against the screen with keyed matching and minimal moves (reversing five keyed rows is four moves), and a paint that fails half-drawn sends nothing. A pre-rendered page is adopted rather than redrawn, so focus, caret and text typed before the module loaded survive, and markup a browser's parser would rewrite (`<tr>` outside `<tbody>`) is linted at build time. A server draws with the same walk onto the same tape, so the markup a server sends and the page a browser paints come from one renderer; `one_lgx_implementation_lock` reads the compiler's source for any second implementation's entry points.

A page's module carries only the reconciler paths it reaches: the one-button counter page ships 29,439 bytes gzipped (`vdom_size_lock`). Collection and text accesses compile to calls into shared helpers outside loops and keep their inline fast path inside them, which made one program's gzipped module 42% smaller at the same hot-loop speed. A turn's garbage is freed at the safepoint after the turn, and a repainting page costs 0 bytes per turn at 2,000 turns and at 20,000 (`wasm_memory_bound_lock`); a page that genuinely runs out of memory fails with a named `OutOfMemory` rather than corrupting.

### Tested where it runs, a real browser included

A page's `## Test` starts from a freshly mounted page and fails if it asserts nothing. It clicks, types, submits, presses keys, reloads, advances the page clock, answers fetches (`Given that GET "/api/todos" answers 200 with "…".`) or serves them from the program's own routes, and asks `the text of`, `the count of`, `is visible`, `is focused` and `the URL`; a selector the engine does not evaluate is refused by name instead of matching nothing. `largo test` drives every page test through one driver (`page_host`) on the tree-walker and the VM and reports a divergence as the first statement at which the documents differ; `largo test --browser` adds headless Chrome, driven over its DevTools pipe with a paused virtual clock and trusted input. Known divergences sit in a shrink-only ledger: an unlisted divergence fails, and so does a listed one that has started to agree (`browser_tier_lock`). Every name a view uses is resolved where it is written, with a did-you-mean; `largo check` adds accessibility warnings (an image without `alt`, a nameless button, an unlabelled control); and every refusal, lint and runtime failure carries `file:line:col` to the terminal, the dev overlay, `largo check --json` and the language server.

### Shipping a site

`largo new <name> --web` scaffolds routes, `pages/` and `static/`, and `--site` a site with no backend. `largo serve` serves a site, a full-stack site or a backend as the project requires, rebuilds and reloads on save, and shows a page's compile error at its line in place of the page. `largo build --static` writes a `dist/` of pre-rendered pages, content-addressed modules, static assets, `robots.txt`, `sitemap.xml`, `_headers`, `_redirects` and `404.html` for any static host; beside a `Serve` entry, `largo build` embeds the pages and assets in the binary, so development and production answer the same bytes (`dev_prod_parity_lock`). Modules are cached immutably by content hash, assets are compressed once at build time and answered `304` on a matching `ETag`, and `real_website_lock` serves a four-page site both as static files and as one binary over a non-loopback address.

### Eighteen example projects

`examples/` is new: `counter`, `todo`, `tic_tac_toe`, `task_board`, `signup_form`, `fetch_dashboard`, `multi_page_site`, `modal_toast`, `stopwatch`, `shopping_cart`, `chat`, `data_table`, `file_drop_upload`, `infinite_scroll`, `marketing_site`, `login`, `guestbook` (server-rendered, no JavaScript) and `fullstack_todo`, each with a README stating its limits. `examples_lock` requires every project to test each action, handler, trigger and binding it declares, to pass on every tier, to export its site and to build into a binary, and every browser snippet in the guides is mounted, compiled and tested, so a guide cannot teach a second way of writing an app (`canonical_teaching_lock`).

## Tests, observation, replay and reverse debugging

`## Test: <prose>` names a test by its sentence. Its body is ordinary LOGOS plus `Expect that <condition>.`, which records a failure and continues, rendering the condition back to English with the value of each named part; `Expect this to fail`, `Expect output:` snapshots, `Advance time by <duration>.` and `Given a file … containing …` round it out. A test declares the real resources it may use and its step budget (`This test may run up to N steps.`, so a runaway fails at the identical op every run), and otherwise runs against a virtual clock, an in-memory filesystem, seeded randomness and no network (`hermeticity_redteam`). `For each of these cases:` over a pipe table runs one sub-test per row, and `For all integers a and b where …:` samples 256 seeded cases and shrinks a counterexample.

`largo test`, a reserved stub in 0.10.1, discovers every test in the entry's import closure and in `pages/`, reports byte-identically across runs, and fails on a failure, on zero executed tests, on a filter that matches nothing or on more skips than the project's budget. `--prove` exhausts finite domains, `--tiers all` re-runs each passing test on the VM and compares byte for byte, and `--json` prints a claim inventory. A run ends in an earned rank: UNTRIED, FAILING, GREEN, HARDENED (a `For all` property held) or PROVEN (a property proved by exhaustion) (`test_runner_lock`, `rank_lock`).

`Note that …`, `Warn that …` and `Record that …` emit structured developer, operator and audit events on a channel separate from program output, with deterministic per-site sampling, and `During <gerund>:` opens a span that closes on every exit path. The tree-walker, the VM, compiled Rust and WebAssembly emit the same stream, serialized as `.logosobs` (`obs_tier_differential`), and a release build erases every observation statement, so the compiled program is byte-identical to the program without them (`obs_zero_cost_codegen_lock`).

A failing `largo run --interpret` writes a self-contained `.replay` bundle (source, scheduler seed and decision trace, arguments, and the failure's kind, message and cursor), and `largo replay <bundle> --verify` reports the failure reproduced byte-identically, no longer firing, or diverged. `largo dap` speaks the Debug Adapter Protocol with `stepBack` and `reverseContinue`, and the VS Code extension adds a `logos` debug type that runs it. Reverse steps rest on the bytecode VM recording only what each op destroys onto a history tape and replaying it backward to the exact prior state, tasks, pipes and `Select` included (`replay_lock`, `dap_lock`, `reverse_op_lock`).

## The language: its library, its modules, its checker

### A standard library written in LOGOS

Thirty-five new modules are written in LOGOS and auto-imported when a program uses them: text (`strings`, `regex`, `encoding`, `json`, `unicode`), numbers (`math` with 157 functions, `statistics`, a seeded `rng` bit-identical on every tier), collections (`iter`, `heap`, `sort`, `collections`), linear algebra (`blas`, and `lapack` with 53 routines from Cholesky to SVD and matrix functions), `calendar`, `compress`, `vision` and the web modules above. `crypto.lg` adds SHA-256, HMAC and PBKDF2 pinned to their FIPS and RFC vectors and ChaCha20, and SHA-3 and SHAKE moved from a Rust binding into LOGOS, byte-identical. `benchmarks/parity/PARITY_LEDGER.md` holds the library to a deduplicated superset of the C, Python, Rust, Go and Java standard libraries: 548 of 548 in-scope capabilities correct on every claimed tier and 38 of 520 performance-gated capabilities benchmarked faster, both ratcheted by `parity_catalog_lock`. Every position a module takes or returns is 1-based. A program's own `## To trim` wins at the program's call sites while the library keeps calling its own, and the compiled tiers carry only the library functions a program reaches.

### Programs span files

The `Import` sentence is new (`Import Geometry.`, `Import G from "shapes/geometry.lg".`, `Import G as Geo.`, `Import double as dbl from Util.`), and 0.10.1's Markdown link still counts. A name resolves beside the importer, as a directory module, under `src/`, at the `Largo.toml` root or in a declared `path` or `git` dependency. The modules assemble into one source every tier compiles, so imported functions reach every backend, where 0.10.1 carried only imported types; functions are callable bare or as `Util::double(…)`, each file sees only what it imports, and `## To private f` stays in its module. A rustc error in generated code is reported at the LOGOS `file:line:col` with a caret (`project_module_lock`, `sourcemap_remap`).

### The type checker runs on every program

`check_program` checks every statement kind with no wildcard arm. A call to a name defined nowhere is refused at check time with the runtime's words (`Unknown function: <name>`), even inside a branch that never runs; so are a wrong arity or argument type, a mistyped field or element write, and a type nothing declares. Generic records and enums carry their arguments, so `Box of Int` and `Box of Text` are different types. A type error names the function its author wrote, or the construct a compiler-written function came from, such as the `add` action of `## Component Counter`. Of the 32 programs in the ill-typed corpus, 18 are refused, and the 14 still accepted are an exact-set ledger (`type_checker_rejects_lock`).

### New statements and values

`Fail with <reason>.` stops a program with its reason, and `Assert`, `Require` and the new `Assume` take `, otherwise <reason>`, reading identically on the tree-walker, the VM, AOT-Rust, C and WebAssembly; range contracts carry units (`Assume v is between 0.2 volts and 0.7 volts.`). Also new: `Swap x and y.`; `Reversibly:`, whose in-place additive and xor updates lower to reversible VM opcodes; `Close ch.`; `Await h into r.`; `Converge remote into local.`, the idempotent CRDT join; tuples and named records (`Let (q, r) be divide(a, b).`); `Let open be every t in todos where not t's done.` and `Remove every t from todos where t's done.`; and `is approximately X within eps`. Every word-keyword is contextual, so `Let at be 1. Show at.` runs. New values and builtins: a floating complex, `Word8`, compression that picks the smallest of deflate, lz4 and zstd, authenticated sealing (compress, then ChaCha20-Poly1305, the tag verified before any byte is returned) and erasure coding over the new `logicaffeine_base::gf256`.

## Numbers past the reals

`Let dt be an infinitesimal.` declares the program's infinitesimal, with what is known of its reciprocal (`whose reciprocal is even`, `divisible by 3`, …), and `Let dx be an infinitesimal much smaller than dt.` declares another below every value made from `dt`, so a program holds several in the order it states. Their values are exact. The hyperreals compute in their field and take roots: `sqrt(dt)` is `√dt`, and the square root of any positive hyperreal is an algebraic Puiseux series in `dt`. A function of a hyperreal is known by its series with exact coefficients, so `exp(dt)` is `1 + dt + O(dt^2)` and `ln(1 + dt)` is `dt - (1/2)·dt^2 + O(dt^3)`. `the standard part of x`, `x is infinitesimal`, `x is limited` and `x is infinitely close to y` read them, and every comparison between them is decided (`hyperreal_closure`, `hyperreal_functions`, `hyperreal_scales`).

A loop whose clock steps by `dt` is a supertask: it runs `⌈1/dt⌉` times and ends in its exact state. An affine update ends in its closed form (Thomson's lamp at `0 when ⌈1/dt⌉ is even, 1 when it is odd`); a polynomial map whose monomials close under it ends exactly, so `Σ t²·dt` is `1/3 - (1/2)·dt + (1/6)·dt^2`; an Euler step of a differential equation ends infinitely close to the solution, which is its standard part (`e` for `Set x to x + x * dt.`, certified digits for a polynomial field); and a bouncing ball's Zeno tail ends at the sum of its series. A supertask may run inside each pass of another, nested to any depth. Imamura's Lemma 1 runs as he wrote it, a `While` that counts toward a value that may be infinite ending on each residue class of `⌈1/dt⌉` its guard reads, and so do his Propositions 3 and 6: `the real number whose digits in base k are f` is an `ExactReal` that calls the program's `f` only as far as a comparison, a floor or a printed digit asks (`supertask_strategies`, `supertask_hybrid`, `supertask_imamura_paper`, `defined_reals`).

Where a program declares `Rational`, `Algebraic`, `ExactReal` or `Hyperreal`, `sqrt`, `exp`, `ln`, `pi` and `e` are exact: `Let r: Algebraic be sqrt(2).` is `√2`, and `r * r` is `2`. Code that declares none of them keeps its meaning (`exact_by_context`). Every value the extension reaches is bit-identical on the tree-walker, the VM, AOT-Rust, both C backends, linked WebAssembly and RISC-V, where the compiled backends link one value runtime built from the Rust the interpreter runs; a device tier refuses such a program by name.

## Execution tiers

### C is a general backend

In 0.10.1 `largo emit c` was a benchmark subset with no concurrency, big integers, exact or dated values, bounds checks or `Show` for collections. `compile_to_c` now splices the standard library, keeps only the functions a program reaches, monomorphizes generics, and carries the exact numeric tower (an `Int` promotes to a big integer on provable overflow, and `7 / 2` stays the exact `7/2`), money, quantities, UUIDs, Words, every temporal value, tuples, nested collections, insertion-ordered maps and sets, closures, CRDT counters and policy checks. Values render as the interpreter renders them, and an out-of-range index, an absent map key, an unmatched `Inspect` and a zero divisor fail with the interpreter's words and exit status 1. A value C cannot type is refused by name; where the interpreter promotes an `Int` past 64 bits on operands C cannot prove bounded, C still wraps, a divergence ledgered in `crates/logicaffeine_tests/tests/gauntlet/c_known_divergences.txt`. `compile_to_c_via_bytecode` also lowers the optimized bytecode the VM runs to C (`c_numeric_tower_lock`, `type_tier_coverage_lock`).

### One scheduler for the C family

Host C, RISC-V, the GPU and the bytecode-to-C walker run one C port of `logicaffeine_runtime`'s `process_step` (`crates/logicaffeine_compile/src/codegen_c/sched_core.c`): host C and RISC-V run tasks as coroutines, and the stepped tier runs them as step functions over heap frames, the lowering the CUDA tier and the walker take. Every concurrency operation is a scheduling point; `Sleep`, closed pipes, timer ties, `Stop` and `Await` behave as on the interpreter; a failing task fails alone; and each binary draws its choices from `LOGOS_SEED`. `c_family_scheduler_conformance_lock` holds 36 programs at six seeds on all of them against the tree-walker and the VM. Deterministic Rust builds (`compile_to_rust_deterministic`) now replay the interpreter's whole schedule on a port of the same scheduler, where 0.10.1 fixed only how a `Select` picked its winner (`replay_executor_port_lock`).

### RISC-V and CUDA

`largo emit riscv` lowers the register bytecode to RV64GC assembly with a C runtime and prints the gcc and qemu command. It lowers 115 of the 117 ISA ops (`Tcp` and `Serve` need host sockets and are refused by name), and `riscv_aot_lock` runs the corpus, the concurrency and network oracles and the benchmark programs under `qemu-riscv64` against the VM. `largo emit cuda` writes one `.cu` that runs a whole program on one GPU thread, with a host launcher, a `Makefile` and the C tier's oracle; the runtime slice a program reaches is lifted to the device, with a float formatter byte-identical to the host's and a failure path that flushes device `printf` and traps. It lifts 112 of the 117 ops, and concurrent programs schedule through the stepped tier (`cuda_aot_lock`).

### WebAssembly and AOT-Rust

`compile_to_wasm`, behind `largo emit wasm` and every page module, monomorphizes generics as the C tier does, so the generics corpus runs apart from one program. Values display as the VM displays them, records, maps and nested collections lower, sets and maps keep insertion order through removals, Text appends in amortized time, and a function with more than 64 basic blocks no longer traps (`wasm_large_function_lock`); the `$` currency format spec is still refused there. AOT-Rust builds what the interpreter runs: generics stay Rust generics, tuples may hold records, a field may be named with any Rust keyword, a binding keeps the exact value type its right-hand side produces, format specs give every kind the interpreter's text, and a value past 64 bits reaches every statement kind. A compiled runtime error prints its LOGOS message alone and exits 1 (`semantic_probes_lock`, `format_spec_all_tiers`). An `Int` that outgrows a machine word is exact on WebAssembly as on every other tier: the module carries a tagged word, the integer while it fits and a handle to the host's integer when not, record fields and `mutable` parameters keep it exact, and a program whose integers all fit a word compiles to the module it did before (`exact_int_every_tier_lock`). A Real shows on both WebAssembly hosts as `fmt_f64` defines, the shortest digits that read back to the same bits, where 0.10.1 printed six fixed decimals and `1.0 / 3.0` read `0.333333` (`wasm_float_display_lock`). On AOT-Rust, `Simultaneously:` binds every task's value where 0.10.1 threw away the results of three or more tasks (`e2e_concurrency`), and an `Int` only literals decide is an `i64` where rustc defaulted it to `i32` (`int_representation_lock`).

A shown line costs no system call. 0.10.1's compiled Rust and `largo run --interpret` made one `write(2)` per `Show`, because Rust's standard output is line-buffered whatever it is connected to; both now buffer 64 KiB on a pipe or a file and write a line at a time to a terminal, as the C tier's stdio does, and output keeps its place before a failure's reason, before input is read and at exit (`show_buffering_lock`).

### The native tier

The JIT compiles 27 builtins to native code where 0.10.1 lowered `sqrt` alone: `abs`, `min` and `max` on `Int`, the bit counts, `floor`, `ceil` and `round` to an `Int`, `pow` with a constant exponent up to 32 (unrolled checked multiplies that deopt to the big-integer path), the `word32`/`word64`/`word16` family with its logical operations, shifts and rotations, and `chr`. Words ride unboxed in native registers, so a SHA-256 round and FNV-1a run natively, byte-identical to the tree-walker, and `forge_numeric_perf_lock` requires a hot native region to beat the VM by at least 3× on x86_64. An op whose instruction the CPU lacks declines to the VM, and `forge_builtin_compliance_ratchet` classifies all 234 builtins. List typing in the native tier is flow-sensitive per instruction, every reachable instruction has a deopt row, and a store into a list that may be a caller's copies it first, as the VM does (`jit_forge_random_fuzz`). Swaps of scalars and word rotates run natively too (`jit_scalar_swap_lock`, `jit_native_rotate_lock`).

## An optimizer a program can hold to account

`## Require <opt>` fails compilation, naming the optimization, when it did not fire; `largo build`, `largo emit` and the verified path honor it through one decorator seam, which also stops `largo build` and `largo emit` dropping a file's `## No` (`decorator_through_largo`). The registry grows from 40 to 65 optimizations, each switchable by keyword. New in both pipelines: sparse conditional constant propagation, integer induction strength reduction, register promotion, loop interchange, run-path memoization and comptime table-ization; AOT-Rust adds overflow-guard elision for bounded accumulators, helper specialization, div-mod fusion, an AVX2 reduction of transcendental accumulator loops and native bit intrinsics, and closed forms extend to sums of squares and cubes. Against compiled C, Rust and Go in the parity catalog, `math.abs` moves from 0.41× to 1.05× and `math.divmod` from 0.73× to 1.05×. RISC-V, WebAssembly and the fabric lowering first run `vm::peephole`, which shrinks 25 of the 35 benchmarks and which `peephole_soundness_lock` holds outcome-preserving over the corpus, every benchmark and a 400-program fuzzer. The passes that move code now read one exhaustive write model and one capture module, closing a family of wrong answers such as a binding a called function assigns being folded to its pre-call value (`optimizer_write_effects_lock`, `optimizer_name_capture_lock`).

## The kernel is Lean's

`Int` in the kernel is Lean's two-constructor inductive (`Int_ofNat | Int_negSucc`), and `add`, `sub`, `mul`, `div`, `mod` and `le` are checked definitions over it that bottom out in checked `Nat` arithmetic. The integer ring and order laws 0.10.1 assumed as axioms are proof terms both kernels check with empty axiom closures, and `dne`, also an axiom in 0.10.1, is a checked definition. The prelude gains `HEq`, `Setoid` and `Quotient` with Lean's universe-polymorphic quotient primitives, and defined `Option`, `Prod`, `Sum`, `Sigma`, `Subtype`, `Fin`, `List`, `Array`, `Char`, `String` and `Decidable` instances. The trusted base is inventoried, assumption by assumption, in `crates/logicaffeine_kernel/TCB.md`, held to the live prelude by `tcb_ledger_lock`, and `#print axioms` is kernel API (`Print Axioms name.` in the vernacular).

Conversion follows Lean's algorithm in both kernels (lazy δ-reduction by definitional height, K-like ι-reduction, structure η, proof irrelevance), and literal fast paths compute only after the kernel proves the program's defining equations, so a rebound constant loses the acceleration instead of computing a wrong value. Registration refuses what 0.10.1 inserted: an unchecked `Context::add_definition`, a redefinition under a dependent theorem, a constructor added after exhaustive elimination and a definitional alias around strict positivity each produced a closed proof of `False`, and each is refused now (`registration_soundness`, `stable_meaning`). The kernel vernacular holds a Lean development: theorems, opaque constants, mutual and universe-polymorphic inductive blocks, quotients, structures, namespaces, notation, hygienic macros, Lean's binder kinds and implicit, named, default and auto arguments, with Lean's auxiliaries (`casesOn`, `noConfusion`, `brecOn`, `decEq`) derived as checked definitions and a printer that is the parser's exact inverse up to α (`vernacular_roundtrip`). A kernel term can carry how it was written (a binder kind, a source position, a Lean `mdata` entry) without changing what it means: every checking door erases the annotation before it measures or checks anything, and certificates carry none (`term_annotation_transparency_lock`).

### Lean 4 proofs import into LOGOS

`largo lean import --project DIR --target Name --out DIR` compiles a Lean project with the pinned Lean 4.14.0 and lean4export and checks every declaration of the target's closure twice: with the LOGOS kernel under Lean's exact, non-cumulative conversion, and with the independent re-checker. A reference translator that shares no code with the importer binds the statement to the Lean source, assumptions are held to Lean's `propext`, `Quot.sound` and `Classical.choice`, and the closure is written as a literate `.lg` document that re-parses to the identical declarations. The package (a manifest with SHA-256 digests and a trusted-base disclosure, the document, an environment certificate and the source export) is replayed by a fresh process before it lands in `--out`, and `largo lean replay DIR` re-verifies it from its bytes alone. Refusals carry typed diagnostic codes and exit statuses, and `unsafe` and `partial` declarations are quarantined (`lean_package_replay`, `lean_cli_e2e`).

### A Lean Kernel Arena checker

`logicaffeine_kernel::dag` checks Lean 4 export streams directly over a hash-consed DAG with two independently written engines, a complete decision procedure for universe levels, kernel-derived recursors (an export's own are admitted only when identical), and `Nat` and `String` literal extensions enabled only after the environment's definitions are validated against Lean-elaborated reference terms, which regenerate byte-identically under Lean 4.29.1 and 4.34.1. `arena_corpus_lock` holds it to the expected verdict on each of the 221 cases of the Arena corpus at lean-kernel-arena `66c354d6`, whose tests are exported with Lean v4.34.1; a decline never counts as a pass, and the heaviest inputs are in the ignored set. A value-engine probe spends its budget on every δ-unfolding it takes, so the `navier-stokes-euler` test no longer unfolds without limit inside abandoned probes (`dag_sem_probe`), and checking a ladder of redexes infers each body once (`dag_sem_bounds`). The `logos-kernel` and `logos-dual` entries in `crates/logicaffeine_kernel/arena/` are pinned to that revision (`arena_entry_lock`), prepared, and not yet submitted.

### LOGOS's calculus on the Arena core

The hash-consed core that checks Lean exports now also decides LOGOS's own calculus (`Theory::Logos`). Indexed `match` with motive validation and coverage over the constructors the indices allow, `fix` and mutual `fix` under the termination guard, cumulative inductives, the K-like rule, the `Int` literal bridge and the elimination restrictions run beside Lean's rules on one engine, and `Theory::Lean` keeps every Arena verdict. The core admits every declaration of the standard library and refuses each paradox of `kernel_paradox_corpus` by the rule the `Term` kernel refuses it by (`theory_logos_parity`). A `Context` keeps an admission log, and `dag::mirror` brings a core up to any context, admitting only what is new. Under the `shadow` feature the context's mirrored core decides every checked door's verdict a second time, and a disagreement panics with both verdicts (`dag_mirror`).

Observational equality (`observational::register`) adds `Obs A a b : SProp`, which computes on the type it compares at. At a function type it compares pointwise, so function extensionality holds by conversion. At a proposition it is logical equivalence, so propositional extensionality holds too. At an inductive family it compares by constructors, and at a quotient by the equivalence closure, so `Quot_sound` is a theorem. `Obs_cast` carries a value along an equality of types and computes, so a closed numeral it carries is a numeral. Both kernels and the core compute these rules, and only for the certified constants (`kernel_observational`, `dag_observational`). Every certified quotient computes in the core, including the one a kernel installs for Lean content, and both kernels admit Lean's `funext`, proved through it.

A proof can be a tactic script over the kernel's own goals. `GoalState` holds each goal as a metavariable with its hypotheses and target. Every tactic builds a proof term: `intro`, `exact`, `apply`, `refine`, `rewrite`, `induction`, `cases` with index unification, `subst`, `injection`, `calc`, `cc`, `decide`, `simp` and the rest. `qed` hands the closed term to the kernel, so no tactic is trusted (`kernel_tactics`). The vernacular takes a script as a theorem's proof (`Theorem t : P := by intro x; exact h.`). `StandardLibrary::context()` registers the standard library once per process. A context of one's own is a copy-on-write clone of it, and the certificates take their trusted base from it (`prelude_registered_once_lock`).

We closed every soundness hole the paradox corpus found after 0.10.1 (`kernel_paradox_corpus`):
- a universe-polymorphic family that may be a proposition no longer eliminates into data;
- a bare reference to a polymorphic declaration stands at that declaration's own levels, where an implicit level had let a proposition and its data copy be used apart;
- a universe level is never `SProp`;
- the termination guard's refutation refinement needs a closed decision;
- a declared type, and an axiom's type, are checked to be types;
- a `Nat` literal reads as `Zero` or `Succ` only while those are `Nat`'s constructors.
Each admitted a closed proof of `False`, a definition that never terminates, or an ill-typed registration; each is refused now. The reflection algebra's seven operations are checked structural recursions instead of assumptions (`tcb_ledger_lock`).

## The proof surface

A `Proof:` line takes Lean-style steps with real term arguments (`exact (hpq hp)`, `use (n + 1)`, `refine` with `?_` holes, `have`, `apply`, `intro`, `cases … with`, positional `rewrite … at`, `simp only`, `calc`, `by_contra`, `ring`, `linarith`, `induction`, `all_goals`, `focus`), and every step still ends in one kernel-certified derivation. A `Proof:` line is now a sketch: the obligations it leaves are discharged by the certified automation and reported, and a step that fails is never swept. Arithmetic can be stated in English: `Prove: For all n, n plus 0 equals n.` certifies through the same kernel path as the formal surface, and `Proof: By induction on n.` inducts. The ring oracle keeps big-integer coefficients and rests on no axiom now that the ring laws are proved in the prelude, and integer literals past `i64` are numbers in every proof layer. `VerifiedProof::axiom_footprint()` is `#print axioms` for a certified proof, `largo prove --certificate` prints an independently re-checkable kernel certificate for every kernel-proved `## Theorem` (`proof_certificate_lock`), and `lapack_provable_lock` proves the standard library's `gesv` solve with exact linear algebra over ℚ.

## A toolchain for people and agents

`largo serve`, `replay`, `explain`, `expand`, `mcp`, `dap` and `lean` are new commands, and `largo test` runs tests. Every command reads one project model: `project::shape` classifies a project as a site, a full-stack site, a backend or a program; every command compiles the same import closure; `path` and `rev`-pinned `git` dependencies link everywhere; and a build's binary is its own even under a shared `CARGO_TARGET_DIR` (`project_shape_lock`, `build_identity_lock`). `largo emit` reaches eleven targets, from `rust`, `c` and `wasm` to `riscv`, `cuda`, `tenstorrent`, `rdu` and `analog`, every target but `rust` compiling the full closure and reporting a refusal at `file:line:col`. `largo expand <file>` prints the ordinary LOGOS a page or route lowers to, and `largo explain <topic>` answers a diagnostic code, construct or type offline.

`largo mcp` serves the toolchain over the Model Context Protocol: six tools (`logos_check`, `logos_run`, `logos_logic`, `logos_prove`, `logos_emit`, `logos_opts`), a `write-logos` prompt and sixteen resources embedded in the binary. `logos_run` never executes inside the server: it re-spawns a worker under a wall clock and an optional step budget, and a program the requested engine cannot run is a typed refusal, never a silent run on another engine. Every project `largo new` writes carries an `AGENTS.md` whose every `largo` line re-parses against the real CLI. `largo check --json` prints `logos-diagnostic/1` objects with a stable code, span, explanation and quickfix, every parse error's code is catalogued in `docs/ERROR_CATALOG.md`, and `largo run` gains `--max-steps N` (a runaway stops at the identical op and exits 122 with its partial output), `--timeout` (exit 124), `--seed N` and `--json` (`diagnostic_schema_lock`, `mcp_e2e`).

`largo verify` now verifies. In 0.10.1 it asked Z3 whether `true` is valid and reported success; it now runs the Z3 static-verification pass, escape analysis and the type checker over the assembled project, and `largo build --verify` takes the same path (`feature_forwarding_lock`). An installed `largo` also builds native binaries outside a repository checkout, taking the runtime crates from crates.io at its own exact version.

The language server understands LGX: go-to-definition, hover, rename and references inside `View:`; completion of the page's own names in `View:`, `Head:` and `Style:`; templates painted as HTML; and template lints published with stable codes locked equal to those of `largo check --json` and `largo mcp`. Across files, an unresolved name a project module exports gets an `Import <Module>.` quickfix. The VS Code extension highlights `View:` as HTML and `Style:` as CSS, reads a template tag's first word (`{if …:}`, `{for each …:}`, `{children}`, `{extends Layout}`) as a keyword and `Use`, `Navigate to` and `Go back` as statements, ships LGX snippets and adds the `logos` debug type (`lgx_editor_lock`, `vscode_grammar_lock`). In the Studio, a file that declares a `## Page` opens in a fifth mode, Web, which compiles it to WebAssembly in the browser and renders it live in a sandboxed frame; the Studio ships 160 examples (89 in 0.10.1), and the Syntax Guide grows from 25 to 43 sections with graded exercises, predict-first examples, misconception callouts and recall cards that `/review` schedules alongside Learn's (`guide_pedagogy`).

## logicaffeine-system serves HTTP and opens verified TLS

The new `http` feature carries an HTTP/1.1 core (keep-alive and pipelining, an 8 MiB body cap and a 64 KiB header cap, chunked bodies refused, no `Date` header so a response depends only on its handler) whose `400`, `413` and `500` answers are byte-identical on the tree-walker, the VM and both native engines; a handler panic answers `500` and the server keeps serving. A production lifecycle reaches every tier through the environment (`LOGOS_MAX_INFLIGHT`, `LOGOS_REQUEST_TIMEOUT_MS`, `LOGOS_DRAIN_MS`, readiness and liveness paths, and more) with no change to code generation, and `SIGTERM` or `SIGINT` stops advertising readiness, finishes in-flight work within the drain deadline and releases the port (`http_lifecycle_parity`). With the opt-in `io-uring` feature on Linux, a thread-per-core io_uring engine is the default wherever a probe ring initializes, falling back to tokio and naming why. TLS verifies against the bundled Mozilla roots plus a PEM named by `LOGOS_TLS_EXTRA_ROOT`, and every native build links it (`tcp_tls_lock`).

## Silicon and accelerators

### Tenstorrent, and one spatial IR for every dataflow chip

`largo emit tenstorrent` writes a TT-Metalium bundle that runs a program's compute body on one Tensix core, with `Show` routed to DPRINT and the C tier's oracle beside it; channels, tasks and sockets are refused by name against a shrink-only ledger, and a recognized GEMM, elementwise, pipe, fan-out or select program lowers to multi-core Tensix kernels. Tensor programs lower once into one spatial IR (`spatial::ir::SpatialProgram`: stages over bounded buffers joined by typed handshakes) on a described mesh (Wormhole, Blackhole, SN40L, SN50, RTX 3060), and every verified program carries a `ChoreographyCertificate` (liveness, buffer bounds, the numerics claim) whose `recheck` trusts only the PR/RUP checker and integer arithmetic. From that one IR come Tensix kernels, a persistent CUDA kernel, synthesizable Verilog with a matching cycle model, and an RDU placement with Spatial source. At 640³ on Tenstorrent's Wormhole simulator the compiler's own plan moves 4,096,440 bytes against `ttnn.matmul`'s 4,097,000 and spends no more NOC transactions or Tensix instructions (`benchmarks/silicon/tenstorrent_gemm.json`; simulator counts, not silicon). The planner prices every candidate at what the compiler emits: its closed form equals the built choreography count for count on all six presets (`spatial_work_model_lock`), and each matrix engine states the sub-block API it runs, so on the sky130 fabric's cycle-exact twin it picks the fastest plan it is offered at every shape the twin times (`spatial_fabric_plan_lock`). Every target states its roofs, each cited to the vendor document it is read from, and `roofline_invariant_lock` holds every metered and every planned GEMM to them. An exact GEMM on Wormhole or Blackhole whose partial sums are provably exact integers splits its reduction and finishes in the output's own page, reading each operand once: on the simulator a 2048 × 2048 × 1024 GEMM takes 5,966 device cycles where holding the reduction in one block takes 22,895 (`spatial_exact_in_place_lock`).

### Transformers written in LOGOS, and the GPU kernels under them

Tensor sugar (`Let C be A matmul B.`, `softmax of X`, `rmsnorm of X with weight W`, `attention of Q over K and V with H heads G groups up to P`, `moe of X with router … choosing K of E`) is shape-checked at parse time and desugars to ordinary loops every tier runs; `tensor_recognize` lifts it back into typed operations the CUDA, Tenstorrent and spatial front ends share. Qwen3.8-27B's hybrid forward (48 Gated DeltaNet and 16 gated-attention layers) is generated as a LOGOS program and lowered to CUDA, and its precise lane reproduces llama.cpp's greedy continuation token for token on the CPU reference and on an A100, recorded at 41.29 tok/s against llama.cpp's 42.17 (`benchmarks/results/qwen38_27b.json`). Recognized matmuls lower to DP4A int8, WMMA, `cp.async` or `mma.sync` kernels chosen from a legality-checked tiling lattice for the RTX 3060 or the B200, and the persistent CUDA GEMM proves before printing that no shared ring overfills, running at 17,994 GOPS at 2048³ bf16 on an RTX 3060 (`benchmarks/results/cuda_spatial_persistent.json`).

### RDU and Plasticine lanes, modeled

`largo emit rdu` lowers a program onto the SN40L or SN50 descriptor and writes a certified placement, its certificate and a modeled report, and an SDK bridge refuses any artifact whose certificate does not recheck; `--model qwen38` lowers the whole model into decode, batched-decode, tensor-parallel and prefill bundles whose layer-by-layer execution emits the VM's token. Every chip number is labeled published or modeled, and nothing ran on an RDU. `largo emit plasticine` turns a counted loop or a tiled fp32 GEMM into Spatial source; under the public Plasticine toolchain the compiler's emitted loops calibrated the PCU model to within 5.2% across the simulated apps, and every report row says `modeled`.

### FPGAs: a soft core, programs as circuits, a whole-program engine

`largo build --emit isa` prints a program as the Silicon ISA listing, and `vm::binisa` frames a compiled program as a checksummed `LGB1` stream. A soft-core interpreter executes the whole ISA, and programs stream over a UART to it on a Nexys A7-100T built with the open yosys and nextpnr-xilinx flow, every `Show` compared with the VM. All 35 programs of the benchmark suite, `strings` included, run on the soft core's width-64 model at a fixed input with every `Show` equal to the VM's (`soft_core_width_parity`). The last to join was `text_bytes`, chip opcode 237: its UTF-8 range encoder equals the standard library's on every Unicode scalar value, and `soft_core.v` decodes it, held to the model under iverilog. The board hosts run unchanged from macOS. `scripts/lib/ftdi_ports.py` finds each board's FT2232 channels by FTDI serial and USB interface on Linux and macOS alike, and runs the loader under `sudo` only where the OS needs it. `logos-chip.py kit` exports a board kit — the host scripts, the images and their build recipes, each program's stream with its VM golden, and the protocol table as `chip_host --table` prints it — that drives both boards with no Rust toolchain (`board_host_portability_lock`, `logos_chip_lock`). `codegen_sva::fabric_rtl` lowers a program to a fetch-free dataflow circuit computing the VM's result modulo 2^64, with 118 of its 120 ops realized, each compute op proven equal to the VM by a witness or a corpus program that exercises it (`fabric_coverage_lock`). The whole-program engine is one universal netlist that receives any fitting program over a UART and runs recursion, heap collections, binary64 floats, text and `Word32`; the core algorithms of all 35 benchmarks run identically to the VM on its model, and `scripts/logos-chip.py run` streams a `## Main` program to it on a Nexys A7-100T or Nexys Video and compares every `Show` with the VM. Coprocessor and memory RTL (a big-integer datapath, Keccak-f[1600], the ML-KEM NTT butterfly, a weight-stationary GEMM tile, a DRAM command scheduler with an independent legality checker) is held to its software oracle under iverilog.

### A language model decodes on two FPGA boards

`spatial::int8_model` writes a tiny int8 transformer's decode step as a LOGOS program, its requantizing shifts, saturating clamps and table-driven attention (`lut attention of Q over K and V …`) forms of the language that desugar to loops every tier runs. The spatial fabric lowering prints it for the Nexys Video and the Nexys A7 as a board image: a host port on the serial line, the autoregressive loop on the chip, and a core clock at the fastest period the routed design's timing allows. Each board generates the golden trace token for token, each step in the cycles the twin predicts, at 27.7 µs a token on the Nexys Video and 43.3 µs on the Nexys A7; split tensor-parallel across both boards, each chip takes under 20 µs a step on chip and the two still generate the golden trace (`decode_images_lock`, `decode_split_images_lock`). The fabric engine under it banks its memories, runs a vector unit beside the matrix array, pipelines every body a register a node, and narrows every word to the width an interval analysis proves; `fabric_postsynth_lock` simulates the synthesized netlist at gate level. The same program runs on the GPU from the same IR. The flow is the open one, yosys and nextpnr-xilinx patched where the router wrote LUT-RAM contents as zero and left the hard cells untimed, and every recorded image re-derives byte for byte from the repository (`xc7_image_flow_lock`).

### A Qwen3.8-27B decode engine for the Intel PAC D5005

A decode engine for Qwen3.8-27B on one Intel PAC D5005, a Stratix 10 SX 2800 with four DDR4-2400 channels, is written down layer by layer, each layer held bit for bit to the one above it. `spatial::serving` prices a pass that streams the weights once through every sequence the card holds, over its DDR4, its PCIe link, its integer array, its vector units and its block-scale lanes, from the checkpoints' own headers: Q4_K_M with every trick reaches 562 tokens/s on 2k contexts (`benchmarks/silicon/d5005/roofline.json`). The engine's arithmetic is every number it computes, as exact integer and IEEE f32 operations quantized the way llama.cpp's CPU path quantizes, and on Qwen3.5-0.8B its fastest point parts from the f32 run within two standard errors of llama.cpp's own build. A checkpoint's step is a program of unit commands the model interprets; the machine lays every byte of a step in the card's memory; and the engine in Verilog runs a whole step of a tiny model under Verilator, leaving every logit, state and cache exactly as the machine does (`d5005_numerics_lock`, `d5005_program_lock`, `d5005_machine_lock`, `d5005_engine_rtl_lock`). Each DDR4 channel is modeled from the shell's own EMIF configuration and held to JESD79-4, and the weight image lays a checkpoint over the four channels in the order a step reads it: Qwen3.8-27B Q4_K_M is 17.2 GB over 522,132 tiles, each ending in its CRC-32C. The OFS build tree, the stream engine, the loader and the bring-up receipt run unchanged against the RTL in co-simulation (`d5005_flow_lock`, `d5005_image_stream_lock`).

### sky130 and Caravel

A process is a value in the compiler (`codegen_sva::pdk_target`: `SKY130A`, `GF180MCUC`), and a block's OpenLane configuration is derived with the measurement behind each number. Hardened blocks, among them soft cores, fabric SoCs, systolic tiles, a SIMT engine and a coprocessor core, are recorded signoff-clean with every sky130 corner closed in the `benchmarks/silicon/*signoff*.json` ledgers. The certified DRAM command player is a Caravel user macro inside the unmodified `user_project_wrapper`; macro and wrapper are signoff-clean, and the assembled submission passes all 19 `mpw_precheck` checks (`benchmarks/silicon/mpw_precheck.json`). The whole-program engine is prepared as a second Caravel user project, with its heap, call-frame stack and configuration image in an off-die QSPI PSRAM, trapping `0xE1` on a call past the frame stack and `0xE2` on a heap access past its region; 24 of the 35 suite programs run on its shipped profile, and its macro and its wrapper harden signoff-clean at every corner, slew included, and pass the foundry's golden DRC deck; its assembled submission passes all 19 `mpw_precheck` checks, LVS included (`benchmarks/silicon/caravel_macro_signoff.json`, `benchmarks/silicon/caravel_signoff_wpe.json`, `benchmarks/silicon/mpw_precheck_wpe.json`). Every setting `scripts/openlane-run.sh` accepts reaches the flow or stops the run with its reason, and a block's power-strap pitch and hold-repair margin are compiler levers (`ClosureSettings::pdn_vpitch_um`, `hold_slack_margin_ns`). The antenna repair reaches an upper-layer run beside a diode site with a stacked jog. The jog router refuses a bridge that would join its own net through a corner or a short edge, the shape the shuttle's maximal-rule deck flags and Magic does not. Clock-tree synthesis, a diode-aware split of dioded nets and the post-route diode ECO each work to the SDC's fanout limit with every antenna diode counted as a load, and the post-diode upsize re-checks slew on the route the flow makes after antenna repair. The OpenLane every block is hardened with is reproducible from the repository (`scripts/openlane-overlay/`).

Hardened blocks are re-signed at the period every corner meets under delay-oriented synthesis, each built under the strategy its period was measured with: the DRAM command player at 5.0 ns, the HFT decision fabric at 4.4 ns, the CGRA at 6.4 ns, the memory subsystem at 8.1 ns and the GEMM array at 9.4 ns. The whole-program engine runs on `user_clock2` at 21 ns (47.62 MHz), the period its slow corner meets, with its float units staged a carry chain or a shift a stage and its stall taken from registers. Global routing's effort is a compiler lever, and clock-tree synthesis takes the largest cost exponent a core's size allows, so a large core builds its clock tree in minutes.

### A comparative-synthesis bench

Seven kernels, a trading decision, a Horner polynomial and a SIMT shader compiled from LOGOS, and a GEMM tile, Keccak-f[1600], an NTT butterfly and a DDR2 controller from LOGOS's RTL generators, are measured against Bambu, Calyx, Gemmini, Vortex, Spatial and LiteDRAM on one bench: a shared functional harness, Artix-7 place-and-route over five seeds, sky130 signoff at each candidate's minimum period and at the kernel's iso period, gate-level power, and a DDR2 traffic gate held to JEDEC. Every printed number is re-derived from its tool reports (`comparative_synthesis_lock`).

### Analog, with the tolerance as a compiler output

`largo emit analog` lowers a float function whose inputs carry `Assume x is between A and B.` ranges onto analog tiles and writes the tile netlist, a gain schedule, a range certificate that `logicaffeine_proof::interval_replay` replays, an itemized error budget whose `recheck` re-derives its total, and a behavioral ngspice netlist, so the tolerance of an analog result is a compiler output an independent checker replays. An integer region is claimed exact only when its budget proves the error below half a grid step, and `codegen_mx::partition` binds a function to the analog plane only when its certificate admits it. Every physical constant is sourced or labeled modeled, and no analog silicon exists (`analog_range_certificate_lock`, `analog_error_budget_lock`).

## Packaging

Every published crate builds from its own crates.io tarball: each manifest carries an anchored `include` allowlist, every file a crate embeds lives in that crate, and `scripts/check-packaging.sh` builds every tarball against its siblings and each docs.rs feature set and holds each `.crate` under 8 MiB, per push and as the release preflight (`packaging_lock`). `rust-version` said 1.75 while published code needed 1.82 and the locked dependency graph 1.88; it is now 1.88, and a CI `msrv` job checks it. The released binaries build with a whole-program-LTO `dist` profile, which on x86_64 Linux takes `largo` from 53.5 MB to 37.2 MB and `logicaffeine-lsp` from 25.5 MB to 16.3 MB (`dist_profile_lock`). Every test now lives in the `logicaffeine-tests` crate, so no published crate ships test code.

## Also in this release

- `ConvergentCount` and `Tally` merge per replica and idempotently, so a redelivered state never double-counts, and `SharedMap from K to V`, which compiled to Rust that did not build in 0.10.1, builds, runs and converges on the tree-walker, the VM and a compiled binary (`shared_map_all_tiers`).
- A `mutable` parameter is the caller's variable on both C tiers and on WebAssembly, a lent element or field is stored back through the keys it was lent with, C evaluates operands in the order LOGOS does, and a function that changes a program variable is pure on no path, so memoization and compile-time evaluation leave it alone on every backend (`in_out_lock`, `c_operand_order_lock`, `purity_lock`).
- ML-KEM and ML-DSA run on the tree-walker and the VM, where 0.10.1 returned `Nothing`, from one table of native kernels shared with AOT-Rust (`native_kernel_lock`).
- A launched task's failure fails the run on the tree-walker as on the VM, `Launch a task to f with x and y` passes two arguments, and a `Select` naming one pipe in two arms no longer takes that pipe's next value.
- The VM runs file statements against the tree-walker's sandboxed filesystem, the tree-walker recurses as deep as the VM, and nested writes (`Push 5 to a's b's c`) land where the sentence says on the tree-walker and the VM.
- Every decompressor stops at 256 MiB, and an untrusted field count no longer preallocates beyond the remaining input (`never_panic_lock`).
- `analytics`, `vm` and `sha256` join the cross-language benchmark suite in all eleven languages (32 programs to 35), and `benchmarks/PROFILE.md` profiles every benchmark for LOGOS and C.
- Every legal-entity surface names Datacenter.Dev as the licensor; the GitHub URLs are unchanged.
- A rational is the VM's exact number on every backend that compiles a program, C, C bytecode, RISC-V, linked WebAssembly and Rust, through lists, records, parameters and results (`rational_number_lock`).
- A linked WebAssembly module carries none of its runtime's debug information, so a program of eight functions links to 272 KB (`emit_wasm`).
- A PostgreSQL cancel request waits for the postmaster to close its connection, as libpq's `PQcancel` does, so it never lands on the statement after it (`postgres_protocol`).

## Breaking changes

Some programs and invocations that 0.10.1 accepted are refused or behave differently. The Breaking changes section of `CHANGELOG.md` gives each item's lock and full migration, and every Rust API change.

- **New header words.** `Test`, `Component`, `Page`, `Route`, `Store`, `Directive` and `Redirect` head code blocks: one without its shape (`## Page layout`) is a parse error, and a prose heading within two edits of one (`## Tests`, `## Usage`, `## Next steps`) is refused as a probable typo. Start such a heading with a word three or more edits from every header word (`## Notes on pages`).
- **The checker refuses more.** A wrong arity or argument type, a call to a function defined nowhere, a mistyped field or element write, a type nothing declares and a bare variant name two enums declare are refused. Fix the call, declare or `Import` the type, and name the variant's enum (`a new Failure from Outcome`).
- **Lending is exclusive.** `h(a, a)` and `h(a, popLast(a))`, where `h` takes a `mutable` parameter, are refused; evaluate the other use first, or pass `copy of a`.
- **A declared function wins over a statement word.** `show(3).` calls a program's own `show`; write `Show 3.` where the statement was meant.
- **Modules are scoped per file, and loading is fail-closed.** A name only a transitively linked module defines is refused, as is a module outside the project, `Largo.toml` and dependency roots; import it directly, or declare it as a dependency.
- **Interpolation holes are read whole, and raw strings are verbatim.** A hole that does not fully parse is refused instead of printing its prefix, and `r"{x}"` shows `{x}`; write literal braces as `{{` and `}}`.
- **`the weekday of` is ISO 8601.** Sunday is 7, not 0; `(the weekday of d) mod 7` gives the old numbering.
- **More than 16 nested parentheses is `AstTooDeep`.** Raise `LOGOS_MAX_AST_DEPTH` or flatten the expression.
- **A compiled binary exits 1 with the LOGOS message** on a runtime error, where 0.10.1 printed a Rust panic report and exited 101, and AOT `text_from_bytes` refuses invalid UTF-8 as the interpreters do.
- **`Option` is a tagged value, `word16` is a `Word16`, and `args()` names the program.** `some 0` differs from `none`, Words of different widths do not compare, and `args()` starts with the program's name on an in-process engine.
- **`largo emit c` output follows the interpreter.** A record shows as `Point { x: 10, y: 20 }`, and an absent map key, an out-of-range index, an unmatched `Inspect` and a zero divisor exit 1.
- **A project with `pages/` or `static/` is a site.** If its entry never `Serve`s, a plain `largo build` exports `dist/`; `largo run` still runs the entry, and `--target`, `--lib`, `--emit` or `--native-functions` take the program path.
- **Dependencies are resolved, not ignored.** A `git` dependency needs a 40-hex `rev`, and a `path` that does not exist is refused.
- **The built binary moved** to the project's `target/<mode>/<name>`.
- **`largo verify`, `check`, `fmt --check`, `prove`, `test` and `run --interpret` refuse more,** and `largo verify` no longer passes unconditionally.
- **Proof certificates from 0.10.1 are refused.** Re-run `largo prove --certificate`.
- **Kernel semantics follow Lean.** Integer division is Euclidean (`div (-7) 3` is `-3`), `Prop` lives in `Type 0`, and instances resolve by priority, then the later declaration.
- **Kernel registration refuses what it used to insert.** An ill-typed `Context::add_definition` body is recorded as refused, a name something depends on cannot be redeclared with a different meaning, and `Repl::execute` refuses trailing input.
- **`linarith` no longer means `omega`.** It reasons over the rationals; write `omega` for integer goals.
- **A theorem block is read as written.** A `Given:` below `Prove:`, and two premises citable by one name, are refused.
- **Seven research modules left `logicaffeine-proof`:** `category_collapse`, `census`, `cofactor`, `res_width`, `orbit_stability`, `polycalc_gfp` and `polycalc_zm`. Pin `logicaffeine-proof = "=0.10.1"` for them.
- **Rust API.** Public enums gained variants and none is non-exhaustive, some structs gained fields, and some signatures changed; `codegen::with_seeded_select` is `codegen::with_deterministic_replay`.
- **The lexicon JSON moved** to `crates/logicaffeine_lexicon/assets/lexicon.json`, exported as `logicaffeine_lexicon::LEXICON_JSON`, content unchanged.
- **Published tarballs carry no tests, examples or fixtures.** Run them from a checkout.
- **Web app links moved.** The Syntax Guide's advanced-features anchor is gone, and the Studio's Code examples moved into topic folders.

## Install it

```bash
curl -fsSL https://logicaffeine.com/install.sh | sh
```

On Windows:

```powershell
irm https://logicaffeine.com/install.ps1 | iex
```

Prebuilt `largo` binaries for Linux and macOS (x64 and arm64) and Windows x64, SHA-256-verified, with no toolchain and no sudo. Piping to `sh -s -- --full` instead installs the flavor with Z3 static verification bundled.
"#,
        tags: &["release", "web", "language", "testing", "compiler", "kernel", "proof", "tools", "hardware"],
        author: "Tristen Harr",
    },
    Article {
        slug: "write-it-in-english-ship-it-anywhere",
        title: "Write It in English. Ship It Anywhere.",
        date: "2026-09-25",
        summary: "Since v0.10: websites written in LOGOS can go live on any host, three new examples show you how, largo runs your code the moment it's installed, and the Learn page, Syntax Guide and Studio all got friendlier.",
        content: r#"
We've been busy since v0.10 landed in July. Here's the short version.

## Your English-language website, live

LOGOS could always build a website. Now it can ship one.

- **Deploy anywhere.** `largo build --static` produces a folder of ready-to-serve pages for any static host. Need a backend? Your project builds into a single binary that runs in a container or on Fly, Render or Kubernetes.
- **Production-ready serving.** Graceful shutdown, health checks, request timeouts and overload protection, configured with environment variables instead of code.
- **Faster first visits.** Your site's app now downloads compressed.
- **Pages that look good when shared.** Each page sets its own title and social preview card, and your 404 page ships with the site.

## Three new examples to learn from

- **Tic-tac-toe:** a complete game with no server at all.
- **Guestbook:** a site with zero JavaScript that rejects forged form submissions for you.
- **Marketing site:** four pages, ready for a CDN, sitemap included.

## Install and go

`largo build` and `largo run` now work the moment you install, at full native speed. Before, the native build needed a copy of our source code.

```bash
curl -fsSL https://logicaffeine.com/install.sh | sh
largo new hello && cd hello && largo run
```

The Linux build with Z3 verification bundled now runs on standard Linux distributions too.

## Learning, smoother

- **[Learn Logic](/learn), rebuilt from the inside.** Lessons load reliably, exercises come in order, tests hold back answers until you submit, and your progress survives a refresh. It fits your phone, too.
- **A [Syntax Guide](/guide) that talks back.** Every example sits in the same editor as the Studio: hover a word to see what it means, and mistakes are underlined as you type.
- **A calmer [Studio](/studio).** Open a file and it picks the right mode on its own, and panels drag and resize smoothly.

## Under the hood

- Leaner code for WebAssembly, RISC-V and hardware targets, with fewer instructions in hot loops.
- Large functions now run in the browser instead of crashing.
- `who`, `what`, `whom` and `item` work as ordinary names, and putting a number where a page element belongs is caught before you run.
- For hardware folks: `largo emit plasticine` turns a LOGOS loop into code for reconfigurable dataflow chips.

Jump into the [Studio](/studio) and try it.
"#,
        tags: &["announcement", "web", "education", "studio", "tools"],
        author: "LOGICAFFEINE Team",
    },
    Article {
        slug: "solver-vs-the-field-pigeonhole",
        title: "Pigeonhole vs the Field: Z3, Kissat, and SaDiCaL",
        date: "2026-06-28",
        summary: "A new Solver-vs-the-field section on the benchmarks page pits our certified prover against Z3 (SMT), Kissat (the CDCL world champion) and SaDiCaL (the reference PR/SDCL solver) on a byte-identical formula. On pigeonhole, Z3 and Kissat hit the resolution wall while we certify it in milliseconds; against SaDiCaL — our own proof class — we are several× faster with a kilobyte certificate against its megabytes. The same prover runs in the Studio, in WASM, no Z3.",
        content: r#"
## The resolution wall, measured three ways

The pigeonhole principle — `n` pigeons into `n-1` holes — needs a resolution refutation of size 2^Ω(n) (Haken 1985), and every CDCL solver inherits that wall. We measured it on a byte-identical formula (i9-14900K, release); external solvers run as subprocesses emitting a clausal proof (solve + certify, like ours):

- PHP(16): ours **14.5 ms** (120-step proof) · Z3 timeout · Kissat timeout · SaDiCaL 53 ms (242 KB proof)
- PHP(28): ours **238 ms** (378-step) · Z3 timeout · Kissat timeout · SaDiCaL 977 ms (4.2 MB)
- PHP(40): ours **1.6 s** (780-step) · Z3 timeout · Kissat timeout · SaDiCaL 7.4 s (25 MB)

Z3 (10 s cap) and Kissat (15 s cap) — the CDCL world champion — cannot even finish PHP(16). SaDiCaL, the reference symmetry/PR solver, *does* complete (no timeout inflation) — and there our certified SR (substitution-redundancy) proof is several× faster, with a certificate of exactly n(n-1)/2 steps against SaDiCaL's growing megabytes.

## A different proof system, not a faster engine

Resolution (RUP/DRAT) ⊊ PR ⊊ SR. Kissat lives at resolution — exponential on pigeonhole. We live at SR — polynomial. Each SR step deletes a whole symmetry orbit with one certified clause whose witness *is* the symmetry, collapsing the exponential search into a polynomial proof. The certificate is exactly quadratic, every time — you can count it.

Two more families round out the section, each a different collapse:

- **Mutilated chessboard** (matching): a colour-count Hall witness in microseconds on sparse, irregular adjacency. By 18×18 *all three* — Z3, Kissat and SaDiCaL — hit the wall; ours stays at 0.24 ms.
- **Tseitin parity on an expander** (linear algebra): Gaussian elimination over GF(2), flat at ~10 µs. At n=90 Z3 walls and Kissat/SaDiCaL grind to seconds with multi-megabyte proofs.

## SDCL: discover the proof with zero hints

The apex: handed only raw clauses — no symmetry hint, no construction — our SDCL *discovers* the certified proof itself. PHP(5) → 40 self-discovered PR clauses, PHP(6) → 60, PHP(7) → 84, each driving the search to zero conflicts.

## Honest boundaries

Random 3-SAT is the control: no structure to exploit, ours tracks Kissat within milliseconds, and SaDiCaL — tuned for structure — fares poorly; we claim no win there. Sub-millisecond comparisons carry external process-startup overhead, so the headline rests on the cases where a competitor takes seconds or hits the wall. Proofs are emitted in DRAT/DPR and accepted by the community checkers (drat-trim / dpr-trim).

## It runs in your browser

The proof engine is pure Rust and compiles to WASM unchanged — Z3, Kissat and SaDiCaL are only the comparison oracles in the native benchmark. The Studio's Hardware mode ships a live pigeonhole demo: type `pigeons: 6`, watch the last pigeon find no home, and read the certified verdict computed in the browser. The benchmark and the demo are the same prover.
"#,
        tags: &["benchmarks", "solver", "verification", "proof", "studio"],
        author: "LOGICAFFEINE Team",
    },
    Article {
        slug: "release-0-10-1-full-binaries-on-stock-glibc",
        title: "v0.10.1 — The Full Linux Binaries Run on Stock glibc",
        date: "2026-07-11",
        summary: "A patch release for one defect: v0.10.0's Z3-bundled `largo-full` Linux binaries exited at startup on stock Ubuntu 22.04 and older with `GLIBCXX_3.4.31 not found`. They now carry libstdc++ statically and start wherever the standard binaries do.",
        content: r#"
## What broke

The `--full` flavor of `largo` bundles Z3 for static verification. Z3 4.16 needs C++20's
`std::format`, so the full binaries are built with gcc-13 — and gcc-13's libstdc++ tags its
symbols `GLIBCXX_3.4.31`, a version stock Ubuntu 22.04 and older do not ship. v0.10.0's
`largo-full-linux-x64` and `largo-full-linux-arm64` linked libstdc++ dynamically, asked the
system for that version, and died at startup with `GLIBCXX_3.4.31 not found`.

The standard binaries, which carry no Z3, were not affected.

## The fix

The full Linux builds now link libstdc++ statically. `z3-sys` emits a plain `-lstdc++`; the
build hands the linker a library-search directory that holds only `libstdc++.a`, so that
flag resolves to the static archive and the binary no longer depends on the system's
libstdc++ at all.

## Get it

```bash
curl -fsSL https://logicaffeine.com/install.sh | sh -s -- --full
```

Or, without Z3:

```bash
curl -fsSL https://logicaffeine.com/install.sh | sh
```
"#,
        tags: &["release", "tools"],
        author: "LOGICAFFEINE Team",
    },
    Article {
        slug: "release-0-10-0-copy-and-patch-jit",
        title: "v0.10.0 — EXODIA: Native Execution Tier",
        date: "2026-07-08",
        summary: "The language gains a register bytecode VM and a copy-and-patch JIT with a register-allocating x86-64 backend (EXODIA), reaching geomean parity with V8/Node on the interpreter benchmark suite. Five new crates join the workspace — forge, jit, synth, runtime, and tv — alongside prebuilt largo binaries with one-line installers, a language server grown to 20 providers with a socratic teaching layer, a VSCode extension on the marketplaces, verified-arithmetic proof work, and strict whole-input parsing.",
        content: r#"
## Install it

```bash
curl -fsSL https://logicaffeine.com/install.sh | sh
```

Prebuilt `largo` binaries for Linux/macOS (x64 + arm64) and Windows x64
(`irm https://logicaffeine.com/install.ps1 | iex`), SHA-256-verified, no toolchain, no sudo.
Add `--full` for the flavor with Z3 static verification bundled. This release also widens
the CLI itself: `largo repl` (interactive imperative + English→FOL sessions), `largo logic`,
`largo prove`, `largo sat`, `largo fmt`, `largo emit`, `largo doc`, `largo add/remove`,
shell completions, and live cargo output during builds.

## The editor teaches you to code

The language server grew from 14 to 20 providers and became genuinely socratic. Hover,
completion documentation, and the REPL's new `:explain` all render one lesson table
(`logicaffeine_language::teach`): a plain-sentence explanation, a runnable example, and a
guiding question for every taught keyword, every `##` block type, and the built-in types.
All 39 parse-error explanations follow the contract *what happened → why → a guiding
question → the next step*, and all 158 stdlib prelude definitions carry literate `## Note`
docs that surface in hover, completion, and signature help — while the runtime prelude
strips them byte-exactly. Diagnostics deepened the same way: the typechecker, ownership,
and escape checkers report every failing statement on its own span with cause links
("was given away here" points at the exact `Give`), and on save the generated Rust runs
through `cargo check` with every rustc finding translated back to English on user-source
spans — English with a borrow checker. Semantic highlighting is resolution-aware,
references and rename cross files, and the workspace indexes every `.lg`/`.md` for
symbols and goto-definition.

The VSCode extension ships with this release as per-platform VSIXs attached to the GitHub
release (marketplace listings come later): each bundles one server binary, a TextMate
grammar rewritten against the real surface and locked to the quickguide by ratchet, and
run/verify/prove code lenses wired to `largo` — behind a CI install gate that installs the
actual packaged VSIX into a real VSCode on three OSes and asserts diagnostics round-trip
before anything releases.

## EXODIA — the native execution tier

The English-like language no longer runs only on a tree-walking interpreter. It now executes on a register bytecode VM with a copy-and-patch JIT on top, and the JIT carries a contiguous register-allocating x86-64 backend (EXODIA). On the interpreter benchmark suite the VM+JIT geomean moves from 3.34x to roughly 1.0x against Node/V8, with 12 of 31 programs faster than V8.

### The bytecode VM

A register bytecode VM in `logicaffeine-compile` is the live engine for both the synchronous native path and WASM. It is corpus-certified against the tree-walking interpreter as a shadow oracle, and its `Int` fast paths are bit-identical to the kernel by the wrapping-`i64` spec, pinned by an edge-grid differential. Release-bench on a representative program: tree-walker 70ms, VM 16.6ms (4.2x), tiered 6.0ms (11.7x).

### The tier-up seam

Hot **functions** compile per call with argument guards and kind-inference (parameters `Int`, comparisons `Bool`). Hot **Main loops** region-tier through an OSR-lite path with incoming-dead analysis and per-entry guards, so loops that never reach a function-level JIT still run native. Anything outside the supported integer/float subset fails closed back to bytecode — the deopt contract — so correctness never depends on the JIT accepting a given program.

### logicaffeine-forge — the executable-memory layer and EXODIA backend

`logicaffeine-forge` is the foundation the copy-and-patch JIT builds on. `JitPage` allocates page-aligned memory, copies machine code into it, flips the page to executable, and hands back a callable function pointer. At runtime, compiling a function is `memcpy(stencil bytes)` followed by patching relocations. On top of the stencil runtime sits the J1 micro-op compiler and the EXODIA contiguous register-allocating region/function x86-64 codegen tier (`regalloc.rs`, `x64asm.rs`), which replaces per-stencil-piece dispatch with allocated registers and contiguous code.

### Platform-correct W^X

Write-xor-execute is handled per platform: on macOS/aarch64 (Apple Silicon) via `mmap(MAP_JIT)` with per-thread `pthread_jit_write_protect_np` toggling and a mandatory `sys_icache_invalidate`; on other Unix via `mmap(RW)` then `mprotect(RX)` with an I-cache flush on aarch64 Linux; on Windows via `VirtualAlloc` then `VirtualProtect` then `FlushInstructionCache`. The JIT is native-only; the browser continues to run the bytecode VM.

### logicaffeine-jit — the native tier

`logicaffeine-jit` is the seam between the VM and the forge. `ForgeTier` translates VM bytecode into the forge's `MicroOp` subset for both whole functions (`ChainFn`) and hot loop regions (`RegionChain`), and `install()` makes the tier process-wide — `largo` installs it at startup. The crate is `#![cfg(not(target_arch = "wasm32"))]`, so WASM builds it to nothing.

### logicaffeine-synth — offline proof tooling for the stencils

`logicaffeine-synth` (EXODIA Phase 2) proves the JIT's integer micro-operations correct offline. It carries Z3 specifications for the micro-ops over 64-bit bitvectors, satisfiability and algebraic-property gates, and a three-way witness harness that runs Z3-chosen inputs through the real compiled stencil. It runs at development and CI time and never on the production runtime path.

### logicaffeine-runtime — deterministic concurrency

`logicaffeine-runtime` is the operational semantics of concurrency for the interpreter and VM: the task scheduler, FIFO channels, `Select`, a logical-clock timer wheel, and the seed/trace machinery. A run is a deterministic function of `(program, seed)` and replays bit-for-bit from `(program, trace)` through a single `Chooser::decide` choke point. It is pure `std`, WASM-safe, tokio-free, and by charter never linked into AOT-compiled binaries — the compiled path uses `logicaffeine-system` instead.

### logicaffeine-tv — translation validation

`logicaffeine-tv` proves that the Rust emitted by the compiler is observationally equivalent to its LOGOS source, per compile, by symbolically executing both sides into the shared `logicaffeine-verify` domain and discharging the equivalence with Z3. `check_encoder_sound` cross-validates the LOGOS encoder against the tree-walking interpreter — the trust anchor that catches a buggy encoder rather than letting it prove two wrong things equal. This is translation validation at rung 3–4: the trust boundary is the encoders, Z3, and rustc.

### Verified arithmetic and proof work

The kernel gains proof-producing arithmetic (`arith.rs`) with certificates, the proof engine gains modal translation and independent verification, and the solver stack adds a CDCL core with an incremental grid solver, grounding, and trust-tiers.

### Strict whole-input parsing

`compile()` now rejects parses that strand tokens (`TrailingTokens`) instead of silently dropping meaning — for a hardware spec, a dropped `until AWREADY` clause is a wrong assertion, not a style issue. The parser gains the coverage to match: noun-noun compound heads, possessive heads over ambiguous noun/verb words, trailing temporal operators inside `If`-consequents, postposed `when`-clauses, and quantified or cardinal objects under modals and under `never`.

### Also in this release

The general CDCL solver dropped its per-conflict allocations (~30% faster on random 3-SAT,
byte-identical search) and decides binary clauses straight from the watch lists; the
`/benchmarks` solver section now shows our certified proof size beside every winning family
and adds CaDiCaL and CryptoMiniSat to the field. The site itself is prerendered (real
per-route HTML with per-page metadata for crawlers and link unfurlers) and code-split — a
first visit downloads ~683 KB gzipped instead of 3.9 MB. And no LOGOS program can
stack-overflow the compiler anymore: a 5,000-term expression chain gets a graceful
diagnostic teaching both fixes, on every surface from the CLI to the web Studio.

### Published

`logicaffeine-forge`, `logicaffeine-jit` and `logicaffeine-runtime` join the lockstep version line and the crates.io publish pipeline, shipping and versioning alongside the rest of the workspace.
"#,
        tags: &["release", "jit", "vm", "compiler", "benchmarks", "translation-validation", "verification", "language", "tools"],
        author: "LOGICAFFEINE Team",
    },
    Article {
        slug: "release-0-9-16-ieee-1800-2023-sva-upgrade",
        title: "v0.9.16 — IEEE 1800-2023 SVA Upgrade",
        date: "2026-04-06",
        summary: "Full IEEE 1800-2023 SVA compliance: ArrayMap, TypeThis, RealConst expression variants, rand real checker variables, triple-quoted strings, Z3 Real sort across the verification pipeline, 95 new tests, hardware parser gap fixes, and CI pipeline optimization.",
        content: r#"
## IEEE 1800-2023 SVA Upgrade

This release upgrades the SVA formal verification pipeline from IEEE 1800-2017 to IEEE 1800-2023 compliance across 4 sprints (22-25), adding 95 new tests with full backwards compatibility.

### New Expression Variants

Three new `SvaExpr` variants for 2023-specific constructs: `ArrayMap` for IEEE 7.12 array `.map()` methods with iterator and index arguments, `TypeThis` for IEEE 6.23 `type(this)` parameterization, and `RealConst` for IEEE 5.7.2 real literal constants in assertion expressions.

### Real-Valued Checker Variables

`RandVarType` enum replaces the old `width: u32` field on `RandVar`, supporting both `BitVec(u32)` and `Real` discriminants. This enables `rand real` and `rand const real` checker variables as specified in IEEE 17.7 (2023). `BoundedSort::Real` and `VerifyType::Real` thread the Z3 Real sort through the entire verification pipeline.

### Triple-Quoted Strings & New System Tasks

IEEE 5.9 triple-quoted string support (`"""..."""`) in assertion action blocks, with proper escape sequence handling and string-aware `else` keyword parsing. Three new system tasks recognized: `$timeunit`, `$timeprecision` (IEEE 20.4.1), and `$stacktrace` (IEEE 20.17).

### Hardware Parser Gap Fixes

Fixes for `shall` modal gate, `always`/`never` after copula, `after`/`when` subordinators, `and`-conjunction in conditionals, `bit` as noun, `request`/`grant` disambiguation, counting quantifiers, and HAB SVA synthesis patterns.

### CI Pipeline Optimization

Benchmark result commits now use `[skip ci]` to prevent cascading workflow re-triggers, reducing total workflow executions from 12 to 7 per release.
"#,
        tags: &["release", "hardware-verification", "sva", "formal-verification", "ieee-1800"],
        author: "LOGICAFFEINE Team",
    },
    Article {
        slug: "release-0-9-15-ieee-1800-sva-coverage",
        title: "v0.9.15 — IEEE 1800-2017 SVA Coverage Expansion",
        date: "2026-04-04",
        summary: "40+ new SvaExpr variants for full IEEE 1800-2017 coverage: property connectives, LTL temporal operators, sequence composition, abort operators, assertion directives, local variables, bitwise operators, vacuity analysis, and benchmark specifications for FVEval, VERT, and AssertionBench.",
        content: r#"
## IEEE 1800-2017 SVA Coverage Expansion

This release dramatically expands SystemVerilog Assertion coverage toward full IEEE 1800-2017 compliance, adding 40+ new expression variants and a complete vacuity analysis module.

### Property Connectives & LTL Operators

Full property-level temporal logic: `not`, `implies`, `iff` for property connectives, plus `always`, `s_always`, `eventually`, `s_eventually`, and `until` (weak/strong, overlapping/non-overlapping) for LTL temporal reasoning. Bounded and unbounded variants with proper `$` semantics.

### Sequence Composition & Abort Operators

Sequence-level `and`/`or` with correct endpoint semantics, `accept_on`/`reject_on` abort operators with synchronous variants, `followed_by` with overlapping/non-overlapping modes, and `strong`/`weak` sequence qualifiers.

### System Functions & Bitwise Operators

IEEE system functions: `$onehot0`, `$onehot`, `$countones`, `$isunknown`, `$sampled`, `$bits`, `$clog2`, `$countbits`, `$isunbounded`. Full bitwise and reduction operators, bit/part selects, and concatenation for hardware-level signal manipulation.

### Vacuity Analysis

New IEEE 16.14.8 compliant vacuity checking module with 33 rules for nonvacuous evaluation tracking. Detects dead assertions that can never be exercised, preventing false confidence from vacuously true properties.

### Assertion Directives & Advanced Features

Concurrent and immediate assertion directives with deferred timing modes. Local variables in sequences, endpoint methods (`triggered`/`matched`), complex data types with struct field access and enum literals, let declarations, checker declarations, and multi-clock annotations.

### Benchmark Targeting

Engineering specifications for three industry benchmarks: FVEval NL2SVA (300 cases), VERT (20,000 cases), and AssertionBench (101 designs). Comprehensive gap analysis documenting the path to 100% IEEE 1800-2017 SVA coverage across 21 planned sprints.
"#,
        tags: &["release", "hardware-verification", "sva", "formal-verification", "ieee-1800"],
        author: "LOGICAFFEINE Team",
    },
    Article {
        slug: "release-0-9-14-production-verification-engine",
        title: "v0.9.14 — Production Verification Engine",
        date: "2026-04-03",
        summary: "Full IC3/PDR model checking, k-induction with invariant strengthening, Craig interpolation, CEGAR abstraction refinement, liveness-to-safety reduction, multi-clock verification, compositional assume-guarantee reasoning, and expanded synthesis oracle.",
        content: r#"
## Production Verification Engine

This release transforms the verification modules from proof-of-concept implementations into production-grade engines with full algorithmic depth.

### IC3/PDR Model Checking

Complete IC3 implementation with frame management, counterexample-guided generalization, and inductive invariant extraction. Proves unbounded safety properties without requiring a bound parameter.

### K-Induction & Interpolation

K-induction with auxiliary invariant strengthening for properties that require inductive proofs. Craig interpolation extracts predicates from refutation proofs for automatic abstraction refinement.

### CEGAR Abstraction Refinement

Full CEGAR loop: predicate abstraction builds an abstract model, model checking finds abstract counterexamples, feasibility checking detects spurious traces, and interpolation discovers new predicates to refine the abstraction.

### Liveness & Multi-Clock

Liveness checking with fairness constraints and ranking functions, including liveness-to-safety reduction for tool interoperability. Multi-clock verification handles synchronizer correctness, metastability windows, and clock domain interaction proofs.

### Compositional Verification

Assume-guarantee reasoning with contract decomposition, circular compositional proofs, and interface refinement checking. Enables verification of large designs by decomposing into independently verifiable components.

### Synthesis & Security

Expanded reactive synthesis oracle with realizability checking. Security property verification with information flow analysis and non-interference checking for hardware designs.
"#,
        tags: &["release", "hardware-verification", "model-checking", "formal-verification", "ic3"],
        author: "LOGICAFFEINE Team",
    },
    Article {
        slug: "release-0-9-13-hardware-kernel-integration",
        title: "v0.9.13 — Hardware Kernel Integration",
        date: "2026-04-03",
        summary: "First-class hardware types in the formal kernel, 20+ new verification modules including IC3, k-induction, CDC, power, and security analysis, Verilog extraction via Curry-Howard, and 40+ new test suites.",
        content: r#"
## Hardware Kernel Integration

This release bridges the formal kernel with the hardware verification pipeline, enabling kernel-level reasoning about hardware designs.

### Hardware Types in the Kernel

First-class `Bit`, `BitVec`, and `Circuit` types are now part of the kernel prelude. Bitvector decision procedures in the reduction engine allow the kernel to evaluate hardware-level operations directly.

### 20+ New Verification Modules

A major expansion of the verification infrastructure:

- **Model checking** — IC3, k-induction, and Craig interpolation for unbounded property verification
- **Compositional verification** — assume-guarantee reasoning with contract-based decomposition
- **Domain-specific analysis** — clock domain crossing (CDC), power isolation, and security property verification
- **Multi-clock domains** — formal reasoning about synchronization across clock boundaries
- **Synthesis oracle** — Z3-guided synthesis from properties to circuit implementations
- **Liveness & fairness** — liveness checking with ranking functions and fairness constraints
- **Abstraction refinement** — CEGAR-style abstraction with automatic predicate discovery
- **Certificate generation** — independently checkable verification certificates
- **Automata-based reasoning** — Buchi and omega-automata for temporal properties

### Verilog Extraction

Synthesis of Verilog modules from kernel proof terms via Curry-Howard correspondence — the proof of correctness IS the circuit.

### RISC-V Protocol Support

Pre-verified SVA property templates for RISC-V bus protocols, alongside existing AXI4, APB, UART, SPI, and I2C templates.

### 40+ New Test Suites

Comprehensive test coverage for every new verification domain, from CDC analysis to compiler verification to parameterized systems.
"#,
        tags: &["release", "hardware-verification", "kernel", "formal-verification", "synthesis"],
        author: "LOGICAFFEINE Team",
    },
    Article {
        slug: "release-0-9-12-full-verification-pipeline",
        title: "v0.9.12 — Full Hardware Verification Pipeline",
        date: "2026-04-02",
        summary: "FOL-to-SVA synthesis, coverage and sufficiency analysis, CEGAR refinement, RTL extraction and knowledge graph linking, protocol templates, waveform rendering, invariant discovery, and consistency checking.",
        content: r#"
## Full Hardware Verification Pipeline

This release completes the hardware verification pipeline from English specifications through RTL linking to formal equivalence checking.

### FOL-to-SVA Synthesis

Pattern-matching translation from first-order logic to SystemVerilog Assertions via Kripke-lowered structures, mapping quantified temporal patterns directly to SVA temporal operators.

### Specification Analysis

Three new analysis modules assess specification quality before verification: **coverage analysis** measures how well SVA properties cover the spec knowledge graph, **sufficiency analysis** detects lonely signals and missing handshake patterns, and **spec health checking** catches contradictions and vacuity through the Z3 pipeline.

### RTL Integration

A Verilog declaration parser extracts module structure (ports, signals, parameters, clock detection), and the RTL knowledge graph links spec-level and RTL-level signal names for automated property binding.

### CEGAR Refinement & Decomposition

Counterexample-guided refinement classifies SVA divergence and suggests transformations. Hierarchical property decomposition enables independent verification of sub-properties.

### Protocol Templates & Invariant Discovery

Pre-verified parameterizable SVA properties for AXI4, APB, and handshake protocols. Automatic candidate invariant generation from knowledge graph structure with Z3 verification.

### Waveform & Consistency

Z3 counterexamples render to VCD format for waveform viewer inspection. Multi-property consistency checking with minimal unsatisfiable subset extraction.
"#,
        tags: &["release", "hardware-verification", "sva", "formal-verification"],
        author: "LOGICAFFEINE Team",
    },
    Article {
        slug: "release-0-9-11-bitvector-bmc",
        title: "v0.9.11 — Bitvector Theory & Bounded Model Checking",
        date: "2026-03-31",
        summary: "Bitvector and array types in the verification IR, bounded model checking for temporal properties, equivalence checking module, and expanded SVA model with seven new temporal operators.",
        content: r#"
## Bitvector Theory & Bounded Model Checking

This release deepens the hardware verification pipeline with bitvector-level reasoning and temporal verification.

### Bitvector & Array Types

The verification IR now supports `BitVector(n)` and `Array(idx, elem)` types with a full `BitVecOp` enum covering bitwise, shift, arithmetic, and comparison operations — all directly encodable to Z3's bitvector theory.

### Bounded Model Checking

A new `verify_temporal()` method unrolls transition relations over bounded steps, checking temporal properties at each state and producing counterexamples on violation.

### SVA Model Expansion

Seven new SVA expression variants — `Repetition`, `SAlways`, `Stable`, `Changed`, `DisableIff`, `Nexttime`, and `IfElse` — with parsing and emission support for richer assertion coverage.

### Equivalence Checking

A new `equivalence.rs` module provides structural and semantic equivalence checking for verification expressions.
"#,
        tags: &["release", "hardware-verification", "bitvector", "model-checking"],
        author: "LOGICAFFEINE Team",
    },
    Article {
        slug: "release-0-9-10-hardware-verification",
        title: "v0.9.10 — Hardware Verification Pipeline",
        date: "2026-03-30",
        summary: "New SVA codegen module for generating SystemVerilog Assertions from FOL specifications, knowledge graph semantics, and 16 new hardware verification test suites.",
        content: r#"
## Hardware Verification via Futamura Projections

This release introduces the hardware verification pipeline — generating SystemVerilog Assertions (SVA) directly from natural language specifications through the existing FOL transpilation and Futamura projection infrastructure.

### SVA Codegen

The new `codegen_sva` module translates first-order logic specifications into SystemVerilog Assertions, enabling formal verification of hardware designs from natural language requirements.

### Knowledge Graph Semantics

A new `knowledge_graph.rs` module extracts structured knowledge graphs from parsed specifications, providing semantic analysis and equivalence checking capabilities for hardware verification workflows.

### Test Coverage

16 new test files cover the full pipeline: lexicon integration, SVA translation, temporal logic, roundtrip verification, Z3 equivalence checking, and end-to-end hardware verification through Futamura projections.
"#,
        tags: &["release", "hardware-verification", "sva", "knowledge-graph"],
        author: "LOGICAFFEINE Team",
    },
    Article {
        slug: "release-0-9-9-futamura-validated",
        title: "v0.9.9 — All 3 Futamura Projections Validated",
        date: "2026-03-29",
        summary: "PE Map/CCopy support across all PE variants, expression embedding analysis for smarter memoization, extended key generation, and full validation of all three Futamura Projections with 6,035 tests passing.",
        content: r#"
## Futamura Projections — Fully Validated

All three Futamura Projections are now verified with comprehensive cross-projection equivalence testing:

- **P1 (Specialization)** — 25-pair program equivalence, Jones optimality confirmed, zero interpretive overhead in residual code.
- **P2 (Compiler Generation)** — no PE dispatch names or BTA artifacts leak into generated compilers, comprehensive P1 matching verified.
- **P3 (Compiler-Generator Generation)** — triple equivalence across 10 programs, cross-projection byte-identical output, cogen matches P2 compiler, 20 surface-level language feature tests covering structs, maps, sequences, options, variants, control flow, recursion, and copy semantics.

512 Futamura-specific tests pass. 6,035 total tests across the full suite with 0 failures.

## PE Map & CCopy Support

All three PE variants (pe_source, pe_bti, pe_mini) now handle Map types and copy expressions through partial evaluation. `exprToVal` processes `CNew` with Map typename and `CCopy` targets; `valToExpr` reconstructs Map values for residual code emission.

## Expression Embedding Analysis

New `exprEmbeds()` and `argsStrictlyEmbed()` predicates compare expression structure to determine when one expression is subsumed by another — enabling more precise memoization decisions during specialization. Covers CInt, CBool, CText, CFloat, CVar, CBinOp, CCall, and CList structures.

## Extended Key Generation

pe_mini now generates unique memoization keys for 15 additional CExpr variants, preventing key collisions in the specialization cache for complex programs involving collections, maps, field access, ranges, slices, copies, and set operations.
"#,
        tags: &["release", "compiler", "futamura-projections", "partial-evaluation"],
        author: "LOGICAFFEINE Team",
    },
    Article {
        slug: "release-0-9-6-studio-fix-local-vec",
        title: "v0.9.6 — Studio Fix & Local Vec Optimization",
        date: "2026-03-19",
        summary: "Fixes the Studio file browser broken since 0.9.0, adds escape-analysis-driven local Vec optimization for zero-overhead collection indexing, and borrow parameter emission for readonly/mutable-only Seq params.",
        content: r#"
## Studio File Browser Fix

The Studio sidebar has been broken since v0.9.0 when Dioxus was upgraded from 0.6 to 0.7. The root cause: Dioxus 0.7 no longer serves files from the `assets/` directory directly. The OPFS web worker (`opfs-worker.js`) and stylesheet (`style.css`) were silently 404ing, which broke the file browser UI.

The fix moves these files to `public/assets/` where Dioxus 0.7 serves them at the same URLs the app expects. Additionally, `robots.txt`, `sitemap.xml`, and `_redirects` move from `assets/` to `public/`, eliminating the manual copy step that was patched into `deploy-frontend.yml`.

## Local Vec Optimization

A new escape analysis pass (`collect_escaping_collection_vars`) identifies collection variables that never leave the function boundary — they're not returned, not passed to other functions, and not stored in structs. These "local" collections are now stored as plain `Vec<T>` instead of `LogosSeq<T>` (the `Rc<RefCell<Vec<T>>>` wrapper), providing zero-overhead indexing without the indirection and runtime borrow checking of `RefCell`.

## Borrow Parameter Optimization

Readonly `Seq<T>` parameters now emit `&[T]` borrows instead of cloning the entire collection. Mutable-only parameters emit `&mut [T]`. The borrow type propagates through aliases — if a parameter is aliased to a local variable, the local inherits the borrow type. This pairs with the existing call-graph-based readonly analysis to eliminate unnecessary allocations at function boundaries.

## Peephole Pattern Updates

The peephole optimizer's slice, push, extend, and drain patterns have been updated to handle both `Vec<T>` and `LogosSeq<T>` source types, ensuring optimizations fire regardless of whether a collection was determined to be local or escaping.
"#,
        tags: &["release", "compiler", "performance", "studio"],
        author: "LOGICAFFEINE Team",
    },
    Article {
        slug: "release-0-9-4-pe-infrastructure",
        title: "v0.9.4 — PE Infrastructure, Self-Application & Reference Semantics",
        date: "2026-03-16",
        summary: "PE BTI and mini source programs for Futamura Projections 2 and 3, genuine self-application verified, Rc-based reference semantics with LogosSeq/LogosMap, and ~5,000 new tests.",
        content: r#"
## PE Infrastructure

Three new LOGOS source programs power the Futamura projection pipeline:

- **PE BTI source** (`pe_bti_source.logos`, 1215 LOC) — a binding-time improved partial evaluator. Renames internal predicates (`isStatic` → `checkStatic`, `specResults` → `memoCache`) and uses B-suffixed entry points (`peExprB`, `peBlockB`) to avoid name collisions during self-application. This is what Projection 2 produces as a compiler.

- **PE mini source** (`pe_mini_source.logos`, 785 LOC) — a clean-room minimal partial evaluator with its own `PEMiniState` type (no `specResults`/`onStack` fields). Uses M-suffixed entry points (`peExprM`, `peBlockM`). This is what Projection 3 produces as a compiler generator.

- **Decompile source** (`decompile_source.logos`, 645 LOC) — a source-level decompiler that converts PE output back to readable LOGOS.

## Genuine Self-Application

The partial evaluator can now specialize itself. Key results verified by tests:

- `PE(pe_source, nano-PE(CInt(42)))` produces `"42"` — The Trick demonstrated
- `PE(pe_source, pe_mini(5+3))` produces `"8"` — a real PE specializing a real PE
- Projection 2 with dynamic target: PE produces residual code containing the dynamic target and no source PE function definitions

Critical fixes that enabled this: `CMapGet` folding for `CNewVariant`/`CNew` expressions, `staticEnv` seeding in all-static `CCall`, and `isLiteral` → `isStaticValue` promotion in `CLet`/`CSet`.

## Reference Semantics

Collections now use Rc-based reference semantics throughout:

- **`LogosSeq<T>`** wraps `Rc<RefCell<Vec<T>>>` — `.clone()` is O(1) shared reference, `.deep_clone()` copies data
- **`LogosMap<K,V>`** wraps `Rc<RefCell<FxHashMap<K,V>>>` — same semantics
- Interior mutability via `RefCell::borrow_mut()` — methods take `&self`
- `copy of` in LOGOS source triggers `.deep_clone()` for value semantics where needed

## PE Engine Improvements

- **specResults memoization** activated with deep-cloned body results
- **makeKey collision fix** — `exprToKeyPart` covers all `CExpr` variants with structural keys
- **MSG wiring** — `interner` threaded through drive/generalize, `msg()` activated on whistle
- **BTA SCC wiring** — `analyze_with_sccs()` called in optimize paths
- **isStatic expansion** — `CNew`, `CRange`, `CCopy` recognized as static
- **Partially-static data** — `CLen` folds `CTuple`, `exprToVal` handles `CNewVariant`/`CNew`
- **extractReturn sentinel** — changed fallback from `CInt(0)` to `CVar("__no_return__")` to prevent silent miscompilation

## Testing

~5,000 new Futamura projection tests across Sprints A–J, plus 191 lines of reference semantics E2E tests. All 4,513 tests pass with zero regressions.
"#,
        tags: &["release", "compiler", "partial-evaluation", "futamura"],
        author: "LOGICAFFEINE Team",
    },
    Article {
        slug: "release-0-9-0-type-checker",
        title: "v0.9.0 — Type Checker, Codegen Refactor & Performance Push",
        date: "2026-02-27",
        summary: "Bidirectional type checker with Robinson unification, codegen split into 13 modules, 15 performance optimizations, 6 new language features, and ~392 new tests.",
        content: r#"
## Bidirectional Type Checker

The compiler now includes a full bidirectional type checker built on Robinson unification. It eliminates `Unknown` types across the board — field access, empty collections, option literals, pipe receives, inspect arm bindings, and closure calls all get concrete types. The checker runs between analysis and codegen, producing a `TypeEnv` that downstream optimization passes use for better decisions.

## Codegen Architecture Refactor

The monolithic `codegen.rs` (8,300 lines) has been split into 13 focused modules: `context`, `detection`, `expr`, `stmt`, `peephole`, `program`, `ffi`, `marshal`, `policy`, `bindings`, `tce`, and `types`. The public API is preserved via re-exports — no downstream breakage. The C backend (`codegen_c.rs`, 2,000 lines) was similarly split into `emit`, `runtime`, and `types`.

## 15 Codegen Optimizations

This release adds a battery of performance improvements:

- **Last-use clone elimination** — skips `.clone()` when a value's last use is detected
- **Liveness-based move** — backward dataflow analysis identifies variables that can be moved instead of cloned
- **Sentinel exit detection** — eliminates redundant loop condition checks
- **Dead post-loop counter elimination** — removes counter variables unused after loop exit
- **HashMap `.get()` for comparisons** — avoids cloning map values just to compare them
- **String byte comparison** — uses `.as_bytes()` for ASCII string equality
- **Self-append via `write!`** — eliminates temporary allocations in string concatenation
- **Flattened string concatenation** — chains of `+` on strings collapse into a single `format!`
- **Read-only `&[T]` borrows** — call graph analysis identifies parameters that are never mutated
- **`Vec::with_capacity`** — pre-allocates vectors when the size is known
- **`assert_unchecked` for proven bounds** — removes redundant bounds checks
- **Raised inline threshold** — more aggressive inlining for small functions
- **Power-of-2 modulo strength reduction** — `x % 2^n` becomes `x & (2^n - 1)`
- **`target-cpu=native`** — generated projects compile for the host CPU architecture
- **FxHashMap/FxHashSet** — `rustc-hash` for faster integer-key hashing

## New Language Features

- **Bitwise operators**: `x xor y`, `x shifted left by y`, `x shifted right by y`
- **Break statement**: `Break.` exits the innermost while loop
- **Unary NOT**: `not x` for logical and bitwise negation
- **Generic function type parameters**: polymorphic type variable declarations
- **Triple-quote strings**: `"""multi-line"""` with automatic indentation stripping
- **Scientific notation**: `4.84e+00`, `2.5e-2` in numeric literals

## Analysis Infrastructure

Three new analysis passes power the optimization pipeline:

- **Call graph analysis** (`callgraph.rs`) — whole-program call graph with Kosaraju SCC detection
- **Liveness analysis** (`liveness.rs`) — backward dataflow for per-statement live-after sets
- **Read-only parameter inference** (`readonly.rs`) — fixed-point iteration over the call graph

## Testing

~392 new tests across 8 new test files covering the type checker, bitwise operations, break statements, codegen optimization, math builtins, string interpolation, and optimizer features. The benchmark suite expanded from ~6 to 30+ programs with multi-language implementations and correctness verification.
"#,
        tags: &["release", "compiler", "performance", "type-system"],
        author: "LOGICAFFEINE Team",
    },
    Article {
        slug: "release-0-8-19-benchmark-fixes",
        title: "v0.8.19 — Benchmark Reliability",
        date: "2026-02-15",
        summary: "Fixes missing Zig benchmark data, increases benchmark runs for lower variance, and adds a swap pattern regression test.",
        content: r#"
## Benchmark Variance

Geometric mean speedup numbers were bouncing between releases due to measurement noise. With only 3 warm-ups and 10 runs, individual outliers could shift results by several percent.

v0.8.19 increases runtime benchmarks to **5 warm-ups and 20 runs**. Doubling the sample count reduces standard error by ~30%, giving more stable comparisons across versions.

## Zig 0.15 Upgrade

All 6 Zig benchmark programs were stuck on Zig 0.13 APIs. Two of them (Collection Operations, String Assembly) additionally used 0.14+ field init syntax that didn't work on either version — causing missing data on CI.

Rather than patching for compatibility with an old version, all programs have been upgraded to Zig 0.15 and CI now installs Zig 0.15.2. Key API changes:

- `std.io.getStdOut().writer()` replaced by `std.fs.File.stdout().writer(&buf)` with explicit buffering and flush
- `std.ArrayList(T)` is now unmanaged — allocator passed to each method call instead of stored at init
- `std.AutoHashMap` remains managed (no change)

## Swap Pattern Regression Guard

Investigation of a bubble_sort performance dip in v0.8.18 benchmarks confirmed the swap optimization fires correctly — the CI result was environmental noise. A new regression test locks this down: it compiles a bubble_sort kernel and asserts the generated Rust contains `.swap()`.
"#,
        tags: &["release", "benchmarks"],
        author: "LOGICAFFEINE Team",
    },
    Article {
        slug: "release-0-8-18-propagation-fix",
        title: "v0.8.18 — Constant Propagation Safety Fix",
        date: "2026-02-15",
        summary: "Fixes two constant propagation bugs that hid use-after-move and zone escape errors from rustc.",
        content: r#"
## The Problem

v0.8.17 introduced a constant propagation optimizer pass that substitutes immutable variables with their literal values. Two edge cases broke safety checks that LOGOS relies on rustc to enforce.

### String Propagation (E0382)

String is non-Copy in Rust. When the propagator substituted a string variable with its literal value, each use site got an independent `String::from(...)` allocation instead of a move. This meant rustc could no longer detect use-after-move errors:

```
Let s be "hello".
Let a be s.
Let b be s.
```

Before the fix, `s` was substituted away — both `a` and `b` got their own `String::from("hello")`, and the program compiled. After the fix, `s` remains as an identifier, rustc sees the double move, and reports E0382.

### Zone Escape (E0597)

Zone-scoped variables were being propagated outside their zone. The escape checker treats `Expr::Literal` as always safe, so substituting a zone-scoped variable with its literal value hid the escape violation:

```
Let mutable leak be 0.
Zone "test" with capacity 100:
    Let p be 42.
    Set leak to p.
```

Before the fix, `p` was propagated to `42`, and `Set leak to 42` passed escape analysis. After the fix, zone-scoped bindings are not registered in the propagation environment, so `p` remains as an identifier and the escape checker correctly reports E0597.

## The Fix

Two targeted changes in `optimize/propagate.rs`:

1. **`is_propagatable_literal`** — replaces `is_literal`, excluding `Literal::Text` since String is non-Copy in Rust
2. **`propagate_zone_block`** — processes zone bodies using the outer environment for substitution but does not register zone-scoped `Let` bindings, preventing them from leaking outward
"#,
        tags: &["release", "compiler"],
        author: "LOGICAFFEINE Team",
    },
    Article {
        slug: "release-0-8-17-c-backend",
        title: "v0.8.17 — C Backend & Test Expansion",
        date: "2026-02-15",
        summary: "A self-contained C codegen backend, constant propagation optimizer, and 334 new E2E tests across 20 files — plus 5 bug fixes with zero regressions.",
        content: r#"
## C Codegen Backend

LOGOS can now compile to C. `compile_to_c()` produces a single self-contained `.c` file with an embedded runtime — no external dependencies, compiles with `gcc -O2`.

The C runtime includes:
- `Seq_i64`, `Seq_bool`, `Seq_str` — dynamic arrays with 1-based indexing
- `Map_i64_i64` — open-addressing hash map
- String helpers — `logos_concat`, `logos_substr`, `logos_parseInt`
- IO — `printf`-based `Show` implementation

C keyword escaping handles collisions: variables named `int`, `double`, `char`, `void`, etc. get a `logos_` prefix via `escape_c_ident()`. The `CContext::resolve()` method applies escaping at a single point for all name lookups.

Supported: integers, floats, booleans, strings, sequences, maps, sets, control flow, functions, recursion.
Not supported: CRDTs, async, zones, networking, polymorphism, closures, structs, enums, Escape blocks.

## Constant Propagation

A new optimizer pass (`optimize/propagate.rs`) performs forward substitution of immutable constants. Given:

```
Let x be 5.
Let y be x + 3.
Show y.
```

After constant propagation, `x` is substituted into the expression for `y`, enabling further folding by the existing constant folder. The pipeline order is: fold → propagate → dce.

Safety: the pass skips substitution inside `Index` and `Slice` expressions to preserve swap and vec-fill pattern detection in the codegen.

## 334 New E2E Tests

20 new test files covering both Rust codegen and interpreter paths:

- **16 Rust codegen mirror files** (181 tests) — every interpreter-only feature now also tested through the Rust codegen pipeline: primitives, variables, expressions, comparisons, logical operators, control flow, iteration, functions, collections, maps, sets, tuples, types, structs, enums, edge cases
- **`e2e_codegen_gaps.rs`** (64 tests) — floats, modulo, options, nothing, collection type combos, struct/enum patterns, control flow, functions, escape blocks, strings
- **`e2e_codegen_optimization.rs`** (15 tests) — TCO, constant propagation, DCE, vec-fill, swap, fold, index simplification
- **`e2e_interpreter_gaps.rs`** (60 tests) — interpreter counterparts for gap coverage
- **`e2e_interpreter_optimization.rs`** (14 tests) — interpreter counterparts for optimization correctness

## Bug Fixes

1. **For-range guard for complex expressions** — `While i is at most length of items` previously produced `_` in generated Rust because `codegen_expr_simple` couldn't handle the `length of` expression. Added `is_simple_expr()` guard to bail out when the limit is too complex.

2. **For-range post-loop value for empty loops** — `While x < 5` with `x = 10` previously set `x = 5` after the empty loop. Now uses `max(start, limit)` for exclusive and `max(start, limit + 1)` for inclusive bounds.

3. **Vec-fill pattern relaxed mutability** — `Let items be a new Seq of Bool` (without explicit `mutable`) now matches the vec-fill optimization. The pattern check was too strict about mutability annotations.

4. **C codegen missing Set variants** — `SetI64` and `SetStr` were not handled in `c_type_str()`, causing a match failure.

5. **Interpreter float comparison** — `apply_comparison` now handles Float-Float, Int-Float, and Float-Int comparisons instead of falling through to a default case.
"#,
        tags: &["release", "compiler", "testing"],
        author: "LOGICAFFEINE Team",
    },
    Article {
        slug: "release-0-8-16-range-fix",
        title: "v0.8.16 — RangeInclusive Fix",
        date: "2026-02-15",
        summary: "Fixes a 41.4% bubble sort regression caused by Rust's RangeInclusive overhead. All for-range loops now emit exclusive ranges.",
        content: r#"
## The Problem

v0.8.15 introduced for-range loop emission — converting `While i is at most n` to `for i in 1..=n`. Benchmarks revealed bubble sort regressed by 41.4% at N=2000.

Root cause: Rust's `RangeInclusive` (`..=`) carries per-iteration overhead. The internal `ExactSizeIterator` implementation needs bookkeeping to handle the edge case where the range includes `T::MAX`. In an O(n^2) inner loop, this overhead compounds.

The test that caught it (`tier1a_simple_counting_loop` in `phase_optimize.rs`):

```rust
let rust = compile_to_rust(source).unwrap();
assert!(rust.contains("for i in 1..6"));
```

The assertion checks that the generated Rust uses an exclusive range (`1..6`) rather than an inclusive one (`1..=5`).

## The Fix

All inclusive ranges now emit the exclusive form with `limit + 1`:

| Before (v0.8.15) | After (v0.8.16) |
|---|---|
| `for i in 1..=5` | `for i in 1..6` |
| `for i in 1..=n` | `for i in 1..(n + 1)` |
| `for j in 1..=((n - 1) - i)` | `for j in 1..(((n - 1) - i) + 1)` |

For literal limits, the addition is computed at compile time — `1..6` not `1..(5 + 1)`.

Exclusive `<` bounds are unchanged: `for i in 0..5` was already correct.

## Bubble Sort Codegen (After Fix)

The inner loop of the bubble sort benchmark now generates:

```rust
for j in 1..(((n - 1) - i) + 1) {
    if arr[(j - 1) as usize] > arr[((j + 1) - 1) as usize] {
        arr.swap((j - 1) as usize, ((j + 1) - 1) as usize);
    }
}
```

No `..=` anywhere in generated code. `Range` (exclusive) uses a simple counter increment with no bookkeeping.
"#,
        tags: &["release", "performance", "compiler"],
        author: "LOGICAFFEINE Team",
    },
    Article {
        slug: "release-0-8-15-direct-strike",
        title: "v0.8.15 — Direct Strike",
        date: "2026-02-15",
        summary: "For-range loop emission, iterator-based loops, list literal type inference, exclusive-bound vec fill, and equality swap patterns — five codegen optimizations that close the gap on array-heavy benchmarks.",
        content: r#"
## For-Range Loop Emission

The highest-impact optimization in this release. The codegen now detects counting loop patterns and emits Rust `for` ranges instead of `while` loops. This gives LLVM trip count information, enabling unrolling and vectorization.

```
Let i be 1.
While i is at most 5:
    Show i.
    Set i to i + 1.
```

Previously generated:

```rust
let mut i = 1;
while (i <= 5) {
    println!("{}", i);
    i = (i + 1);
}
```

Now generates:

```rust
for i in 1..6 {
    println!("{}", i);
}
let mut i = 6;
```

The post-loop `let mut i = 6` preserves counter semantics — code after the loop that reads `i` gets the correct value. LLVM eliminates it if unused.

Both inclusive (`is at most` → `..` with `limit + 1`) and exclusive (`is less than` → `..`) bounds are supported. Variable limits work: `While i is at most n` generates `for i in 1..(n + 1)`.

A `body_modifies_var` guard prevents the transformation when the counter is modified inside the loop body (other than the final increment), and step sizes other than 1 correctly fall back to `while`:

```
Let i be 0.
While i is at most 10:
    Show i.
    Set i to i + 2.
```

This stays as `while` — the step is 2, not 1.

The pattern is integrated at all 7 peephole call sites in the codegen, including inside TCE, accumulator, memoization, and C export paths.

## Iterator-Based Loops

`Repeat for x in items` previously always emitted `for x in items.clone()`, copying the entire collection before iterating. For a `Vec<i64>` with a million elements, that's 8MB of unnecessary allocation.

When the element type is `Copy` (`i64`, `f64`, `bool`) and the loop body doesn't mutate the collection, the codegen now emits `.iter().copied()`:

```
Let items: Seq of Int be [1, 2, 3].
Repeat for x in items:
    Show x.
```

Generates `for x in items.iter().copied()` instead of `for x in items.clone()`.

A recursive `body_mutates_collection` helper walks through nested `If`, `While`, `Repeat`, and `Zone` blocks to detect `Push`, `Pop`, `Remove`, `SetIndex`, or `Set` operations targeting the collection. If any mutation is found, the safe `.clone()` path is preserved:

```
Repeat for x in items:
    Push x to items.
```

This keeps `.clone()` — the body mutates `items`.

Non-Copy types like `Text` also keep `.clone()`, since `.iter().copied()` requires `Copy`.

## Direct Array Indexing for List Literals

List literals like `[10, 20, 30]` now register their element type in the codegen's type tracking. Previously, only explicitly annotated variables (`Let items: Seq of Int`) or `Expr::New` constructors got type registration. List literals fell through to the `LogosIndex` trait dispatch path, adding bounds check overhead and an index conversion function call per access.

Now `[10, 20, 30]` infers `Vec<i64>` from the first literal element, enabling direct `arr[(idx-1) as usize]` indexing without trait dispatch. Combined with Copy-type detection, integer list indexing also skips `.clone()`:

```
Let items be [10, 20, 30].
Let x be item 2 of items.
Show x.
```

Generates direct `items[(2 - 1) as usize]` instead of `LogosIndex::logos_get(&items, 2).clone()`.

## Vec Fill Enhancement

The vec-fill peephole pattern — converting push loops to `vec![val; count]` — previously only matched `While i is at most n` (inclusive `<=` bound). It now also matches `While i is less than n` (exclusive `<` bound):

```
Let mut items be a new Seq of Int.
Let mut i be 0.
While i is less than 5:
    Push 0 to items.
    Set i to i + 1.
```

Generates `vec![0i64; 5 as usize]` instead of a push loop. The count calculation adjusts for exclusive vs inclusive bounds: `i < 5` starting at 0 produces 5 elements; `i <= 5` starting at 0 produces 6.

## Swap Pattern Enhancement

The swap peephole — converting adjacent-index comparison + cross-assignment into `arr.swap()` — previously only matched `>`, `<`, `>=`, `<=` comparisons. It now also matches `equals` and `is not`:

```
If a equals b:
    Set item j of items to b.
    Set item (j + 1) of items to a.
```

Generates `items.swap(...)` instead of two separate assignments through a temporary. This covers partition-style algorithms that swap on equality conditions.
"#,
        tags: &["release", "performance", "compiler"],
        author: "LOGICAFFEINE Team",
    },
    Article {
        slug: "release-0-8-14-bedrock",
        title: "v0.8.14 — Bedrock and Maybe",
        date: "2026-02-15",
        summary: "Deep expression folding across all 26 AST variants, unreachable-after-return elimination, algebraic identity simplification for integers and floats, and Maybe as dual syntax for Option.",
        content: r#"
## Deep Expression Recursion

The constant folder previously handled top-level binary expressions but left sub-expressions inside function calls, list literals, struct constructors, index expressions, `Option`/`Maybe` `Some` wrappers, `Contains` checks, and closures untouched. A catch-all `_ => expr` arm silently passed through 20+ `Expr` variants without recursing.

v0.8.14 replaces the catch-all with exhaustive matching across all 26 variants. Every sub-expression site — `Call` args, `List` elements, `New` field initializers, `Index` expressions, `OptionSome` values, `Closure` bodies — now gets folded. Given:

```
## To double (n: Int) -> Int:
    Return n * 2.

## Main
Show double(2 + 3).
```

The `2 + 3` inside the call arguments folds to `5` before codegen, producing `double(5)` instead of `double(2 + 3)`.

## Unreachable-After-Return DCE

Statements following a `Return` in the same block are dead code. The dead code elimination pass now truncates blocks after the first `Return`:

```
## To f () -> Int:
    Return 42.
    Show "unreachable".
    Return 99.
```

The `Show` and second `Return` are eliminated. This also catches returns inlined from constant-folded `if true` branches. Returns inside nested `If` blocks do not incorrectly truncate the outer block.

## Algebraic Simplification

A new `try_simplify_algebraic` pass fires when one operand of a binary expression is a known identity or annihilator:

| Pattern | Result |
|---------|--------|
| `x + 0`, `0 + x` | `x` |
| `x * 1`, `1 * x` | `x` |
| `x * 0`, `0 * x` | `0` |
| `x - 0` | `x` |
| `x / 1` | `x` |

These rules apply to both integers and floats. The simplification returns the surviving operand directly — no arena allocation needed. Combined with deep expression recursion, `double(5 + 0)` first recurses into the call argument, then simplifies `5 + 0` to `5`, producing `double(5)`.

## Maybe Syntax

`Maybe` is now a first-class alias for `Option`. Both forms work interchangeably:

```
Let x: Option of Int be nothing.
Let y: Maybe of Int be nothing.
Let z: Maybe Int be nothing.
```

The direct form — `Maybe Int` without the `of` preposition — follows Haskell convention. It works in all positions: variable annotations, function return types, and nested generics (`Maybe List of Int` → `Option<Vec<i64>>`). All seven codegen paths that handle `Option` now accept `Maybe` identically.
"#,
        tags: &["release", "performance", "compiler"],
        author: "LOGICAFFEINE Team",
    },
    Article {
        slug: "release-0-8-13-optimizer",
        title: "v0.8.10–0.8.13 — The Optimizer",
        date: "2026-02-14",
        summary: "Accumulator introduction, automatic memoization, mutual tail call optimization, purity analysis, peephole patterns, and copy-type elision — recursive functions now compile to loops, and generated code approaches hand-written Rust.",
        content: r#"
## Accumulator Introduction

The centerpiece of this release cluster is **accumulator introduction**. The optimizer detects recursive patterns like `f(n-1) + k` (additive) and `n * f(n-1)` (multiplicative) and rewrites them into zero-overhead loops. Given this LOGOS source:

```
## To factorial (n: Int) -> Int:
    If n is at most 1:
        Return 1.
    Return n * factorial(n - 1).
```

The codegen emits:

```rust
fn factorial(mut n: i64) -> i64 {
    let mut __acc: i64 = 1;
    loop {
        if n <= 1 {
            return __acc * 1;
        }
        {
            let __acc_expr = n;
            __acc = __acc * __acc_expr;
            let __tce_0 = n - 1;
            n = __tce_0;
            continue;
        }
    }
}
```

The identity element (`1` for multiplication, `0` for addition) initializes the accumulator. Each recursive return becomes an accumulator update and a `continue`. Stack frames are eliminated entirely.

## Mutual TCO and Memoization

**Mutual tail call optimization** handles function pairs like `isEven`/`isOdd` that call each other in tail position. The optimizer merges them into a single `__mutual_isEven_isOdd` function with a `__tag: u8` parameter and a `loop { match __tag { 0 => { ... }, 1 => { ... } } }` dispatch. The original functions become `#[inline]` wrappers that call the merged function with the appropriate tag.

**Automatic memoization** targets pure functions with multiple recursive calls. A fixed-point **purity analysis** identifies side-effect-free functions (no `Show`, file I/O, CRDT operations, etc.). Pure multi-branch functions like Fibonacci receive a `thread_local!` `RefCell<HashMap>` cache — the function checks the cache before executing, stores results after, and drops complexity from O(2^n) to O(n) with zero source changes.

## Peephole Patterns and Build Profile

Lower-level optimizations target generated Rust quality:

- **Vec fill**: a push loop with constant value becomes `vec![val; count]`
- **Swap pattern**: adjacent-index comparisons with temp-variable swaps become `arr.swap(i, j)`
- **Copy-type elision**: `.clone()` on `Vec`/`HashMap` indexing dropped for `Copy` types (`i64`, `f64`, `bool`)
- **Direct collection indexing**: known `Vec`/`HashMap` types use direct indexing instead of trait dispatch
- **HashMap equality**: `map.get()` replaces `map[key].clone()` in comparisons

Release builds use `opt-level = 3`, `codegen-units = 1`, `panic = "abort"`, `strip = true`, and LTO. Hot paths carry `#[inline]`; all `Showable`, `LogosContains`, and `LogosIndex` trait impls use `#[inline(always)]`.
"#,
        tags: &["release", "performance", "compiler"],
        author: "LOGICAFFEINE Team",
    },
    Article {
        slug: "release-0-8-6-benchmarks",
        title: "v0.8.6 — Ten-Language Benchmark Suite",
        date: "2026-02-13",
        summary: "A benchmark suite comparing LOGOS against C, C++, Rust, Go, Zig, Nim, Python, Ruby, JavaScript, and Java — with CI automation and an interactive results page.",
        content: r#"
## Why Benchmark

Performance claims without numbers are marketing. We implemented 6 benchmark programs in 10 languages and LOGOS, measured wall-clock time with `hyperfine` (3 warmup runs, 10 measured runs), and published the results on every release.

## The Programs

Six programs exercise different computational patterns:

- **fib** — naive recursive Fibonacci (function call overhead)
- **sieve** — Sieve of Eratosthenes (array mutation, tight loops)
- **collect** — hash map insert/lookup (allocation pressure)
- **strings** — string concatenation (allocator throughput)
- **bubble_sort** — O(n²) sort (nested loops, array mutation, swaps)
- **ackermann** — Ackermann function (extreme recursion depth, stack frame overhead)

Each program is implemented idiomatically in C, C++, Rust, Go, Zig, Nim, Python, Ruby, JavaScript, and Java. No artificial handicaps or advantages.

## CI Integration

A GitHub Actions workflow runs the full benchmark suite on every tagged release. The runner compiles all implementations, verifies correctness against expected output files, benchmarks runtime with `hyperfine`, measures compilation time separately, and assembles the results into `benchmarks/results/latest.json`. Results are committed back to the repository, and the frontend deploy triggers after benchmarks land.

Versioned results are archived in `benchmarks/results/history/`, so performance trends are recoverable across releases.

## The Results Page

The `/benchmarks` page embeds `latest.json` at compile time. Each benchmark tab shows grouped bar charts split by language tier (systems, managed, transpiled, interpreted), with LOGOS highlighted. Collapsible sections show full statistics (mean, median, stddev, min, max, coefficient of variation), source code for all implementations side-by-side, and compilation time comparisons.

A cross-benchmark summary computes geometric mean speedup versus C across all 6 programs, providing a single aggregate performance number.

## What It Measures

The benchmark suite tests the full compilation path: LOGOS source → parser → codegen → Rust output → `rustc` → binary. Interpreter mode runs separately at smaller input sizes. This means the numbers reflect codegen quality — the optimizer's job — and `rustc` optimization of the generated code. When LOGOS gets faster, it's because the generated Rust got better.
"#,
        tags: &["release", "performance", "benchmarks"],
        author: "LOGICAFFEINE Team",
    },
    Article {
        slug: "release-0-8-2-interpreter-optimizer",
        title: "v0.8.2 — Interpreter Mode and Optimizer Infrastructure",
        date: "2026-02-12",
        summary: "An interpreter for sub-second feedback during development, the first optimizer passes (constant folding, dead code elimination), and map insertion syntax.",
        content: r#"
## Interpreter Mode

Before v0.8.2, every code change required a full compilation cycle: LOGOS source → Rust codegen → `cargo build` → run binary. For large projects, that's minutes of waiting per iteration.

`largo run --interpret` bypasses codegen entirely. The interpreter walks the AST directly, evaluating expressions and executing statements without generating Rust. Startup is sub-second.

The interpreter supports the full language surface: variables, functions, control flow, collections, structs, enums, pattern matching, string operations, and arithmetic. It handles 1-based indexing, type coercion, and the standard library. The goal is behavioral equivalence with compiled output — if it works in the interpreter, it works when compiled.

Interpreter mode is for development. It doesn't optimize, doesn't type-check at the Rust level, and runs slower than compiled code by orders of magnitude. But it turns the edit-run cycle from minutes to milliseconds.

## Optimizer Infrastructure

This release lays the foundation for all subsequent optimization work (v0.8.10–0.8.13). Two passes ship in v0.8.2:

**Constant folding** evaluates compile-time-known expressions during codegen. `3 + 4` becomes `7` in the generated Rust, not a runtime addition. This propagates through variable assignments when the optimizer can prove the value is constant.

**Dead code elimination** removes unreachable branches. An `if false { ... }` block (after constant folding resolves the condition) is dropped entirely from the output. This keeps generated code clean and reduces what `rustc` has to process.

Both passes operate on the AST before Rust emission — they improve codegen quality without changing language semantics.

## Map Insertion Syntax

A new syntax form for map mutations:

```
Set scores at "alice" to 95.
```

This generates `scores.insert("alice".to_string(), 95)` in Rust. Previously, map insertion required method-call syntax that didn't match the natural-language feel of the rest of the language.

## FFI Safety Hardening

The C export system introduced in v0.8.0 received safety improvements: thread-local error caching (so FFI errors don't cross thread boundaries), panic boundaries at every exported function (so a LOGOS panic doesn't unwind into C), null handle validation, and a dynamic `logos_version()` function for ABI compatibility checking.

`LogosHandle` changed from `*const c_void` to `*mut c_void` to correctly represent mutable state. Text/String types are excluded from the C ABI value types — they require explicit conversion through the string API.
"#,
        tags: &["release", "feature", "performance"],
        author: "LOGICAFFEINE Team",
    },
    Article {
        slug: "release-0-8-0-lsp-ffi",
        title: "v0.8.0 — LSP Server and FFI System",
        date: "2026-02-10",
        summary: "A Language Server Protocol implementation with full language intelligence, a VSCode extension for 5 platforms, and a C FFI export system for cross-language interop.",
        content: r#"
## Language Server

LOGOS ships an LSP server with 14 features across four categories. The server runs as `largo lsp` over stdio, maintaining per-document state with full re-analysis on every keystroke.

**Code intelligence**: diagnostics (real-time parse and analysis errors), hover (type info, verb class, tense, aspect), and semantic tokens (keyword, variable, function, struct, string, number classifications with delta encoding).

**Navigation**: go to definition, find references, document symbols (outline view), and code lens (inline reference counts above definitions).

**Refactoring**: rename (with prepare-rename validation) and code actions (extract to function, dead code elimination).

**Editing**: completions (triggered on `.`, `:`, `'`), signature help (triggered on `with`, `,`), inlay hints (inferred types, parameter names), folding ranges, and formatting.

Token resolution handles the full range of name-bearing token types: `Identifier`, `ProperName`, `Noun`, `Adjective`, and `Verb`. A variable named `x` might be lexed as `Adjective(Symbol)` rather than `Identifier` — the server resolves this uniformly through a centralized `resolve_token_name()` function.

## VSCode Extension

The extension bundles pre-compiled LSP binaries for 5 platform targets: `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `x86_64-apple-darwin`, `aarch64-apple-darwin`, and `x86_64-pc-windows-msvc`. Installation is zero-configuration. TextMate grammars handle syntax highlighting; semantic token overrides from the LSP provide meaning-aware coloring.

## FFI / C Export System

Functions marked `is exported` get C-linkage wrappers for consumption by C, C++, Python (via ctypes/cffi), and any language with a C FFI:

```
## To add (a: Int) and (b: Int) -> Int is exported:
    Return a + b.
```

The codegen emits `#[export_name = "logos_add"] pub extern "C" fn` with `std::panic::catch_unwind` boundaries. Integer and float parameters pass directly; text parameters marshal through `*const c_char` input / `*mut *mut c_char` output with null-pointer validation. A `LogosStatus` enum (`Ok`, `NullPointer`, `ThreadPanic`, etc.) communicates errors. Collections and structs cross the boundary as opaque `LogosHandle` pointers with typed accessor functions (`logos_seq_i64_push`, `logos_person_age`, etc.).

## CI/CD

Three GitHub Actions workflows ship with v0.8.0:

- **Release**: builds platform binaries and the VSCode extension `.vsix` on tag push
- **Publish**: publishes all workspace crates to crates.io in dependency order
- **Deploy**: builds the WASM frontend and deploys to production

The publish workflow handles the lockstep versioning scheme — all 11 crates share a single version number and publish atomically.
"#,
        tags: &["release", "tools", "feature"],
        author: "LOGICAFFEINE Team",
    },
    Article {
        slug: "stablecoins-treasury-future-of-money",
        title: "Stablecoins, Treasury Bills, and the Future of American Money",
        date: "2026-02-02",
        summary: "The GENIUS Act became law in 2025, and the CLARITY Act is advancing through Congress. From Ripple's RLUSD to USDC on Solana, Treasury-backed stablecoins are becoming infrastructure for the digital dollar.",
        content: r#"
## The 40,000-Foot View: A New Monetary Architecture

The [GENIUS Act](https://www.congress.gov/bill/119th-congress/senate-bill/1582) became law in July 2025, establishing the first federal framework for stablecoins. The [CLARITY Act](https://www.congress.gov/bill/119th-congress/house-bill/3633), which would establish broader digital asset market structure, passed the House and awaits Senate action. Combined with executive orders establishing a Strategic Bitcoin Reserve and prohibiting a government CBDC, the architecture of American digital money is taking shape.

The policy direction is clear: private stablecoins backed by Treasury debt, running on regulated blockchain infrastructure, with defined jurisdictional boundaries between agencies. The government provides the backing; the private sector provides the innovation.

### The GENIUS Act: Stablecoin Law

President Trump signed the GENIUS Act (Guiding and Establishing National Innovation for U.S. Stablecoins) into law on July 18, 2025, after bipartisan passage — 68-30 in the Senate, 308-122 in the House.

The reserve requirements channel capital directly into US government debt:

- **100% reserve backing** required with liquid assets
- Permitted reserves: US dollars, short-term Treasury bills (93 days or less), Treasury-backed reverse repos, government money market funds
- **Monthly public disclosures** of reserve composition mandatory
- Issuers under $10 billion may opt for state regulation if "substantially similar"

Payment stablecoins are explicitly **not securities or commodities** under the Act. Foreign issuers are permitted subject to Treasury Department determination of comparable home-country regulations.

### The CLARITY Act: Pending Market Structure Legislation

The [Digital Asset Market Clarity Act](https://www.congress.gov/bill/119th-congress/house-bill/3633) (H.R. 3633), introduced in May 2025, passed the House on July 17, 2025 with a 294-134 bipartisan vote. As of January 2026, the bill is pending in the Senate Banking Committee, which released a 278-page amended draft on January 12, 2026.

Key provisions in the House-passed version:

- **CFTC would receive "exclusive jurisdiction"** over "digital commodity" spot markets
- **SEC would retain jurisdiction** over investment contract assets
- "Digital commodity" defined as assets whose value is "intrinsically linked" to blockchain use
- **Stablecoins explicitly excluded** from the "digital commodity" definition (covered by GENIUS Act)
- Digital commodity exchanges, brokers, and dealers would register with CFTC
- Contains what authors describe as the "strongest illicit-finance framework Congress has considered for digital assets"

If enacted, GENIUS would handle stablecoins while CLARITY handles market structure. The regulatory fog that plagued the industry since 2017 is beginning to lift.

### The Ripple Precedent: XRP Is Not a Security

The regulatory clarity owes much to [Ripple Labs' five-year legal battle](https://www.ccn.com/education/crypto/ripple-vs-sec-timeline-and-outcomes/) with the SEC.

**July 13, 2023**: Judge Analisa Torres issued a landmark ruling — **XRP is not a security when sold on public exchanges**. Institutional sales ($728 million) were unregistered securities offerings, but secondary market trading was vindicated.

**May 8, 2025**: After five years of litigation, Ripple and the SEC reached a **$50 million settlement** — down from the original $125 million penalty. Executives Brad Garlinghouse and Chris Larsen were fully cleared.

**August 2025**: Both parties withdrew all appeals, officially ending the case.

The precedent matters: distinguishing between institutional token sales and secondary market trading created legal clarity that enabled everything that followed, including multiple spot XRP ETFs approved in November 2025.

### RLUSD: The Gold Standard for Regulated Stablecoins

[Ripple's RLUSD](https://ripple.com/solutions/stablecoin/) launched December 17, 2024 after receiving NYDFS (New York Department of Financial Services) approval — one of the world's strictest regulatory regimes.

**Reserve Structure:**
- 100% backed 1:1 by US dollars
- Reserves held in: US dollar deposits, US government bonds, cash equivalents
- Monthly third-party reserve attestations published publicly
- Issued by Standard Custody & Trust Company under New York trust charter

**Market Position (late 2025):**
- Market cap: $1.26 billion
- Third-largest US-regulated stablecoin
- 80% deployed on Ethereum ($1.01 billion)
- 20% deployed on XRP Ledger ($225 million)

RLUSD demonstrates what GENIUS Act compliance looks like in practice: transparent reserves, regulatory oversight, multi-chain deployment.

### The XRP Ledger: Purpose-Built for Payments

The [XRP Ledger](https://xrpl.org/) operates differently from general-purpose smart contract platforms. Its Federated Byzantine Agreement consensus achieves transaction finality in 3-5 seconds at approximately 1,500 TPS — without mining or staking.

**Native DeFi features built into the protocol:**
- Central Limit Order Book (CLOB) DEX
- Automated Market Maker (AMM) — voted into protocol March 2024
- Permissioned DEX for institutional participants (launched June 2025)
- Clawback feature for compliance (enabled January 2025)

The June 2025 launch of an **XRPL EVM Sidechain** enables Ethereum smart contract compatibility while maintaining the main chain's payment-focused architecture.

Multiple stablecoins now operate on XRPL: Circle's USDC, Braza Group's USDB, Schuman Financial's EUROP, StratsX's XSGD.

### USDC: Multi-Chain Treasury-Backed Liquidity

[Circle's USDC](https://www.circle.com/) — at $60+ billion market cap — represents the transparent, audited model of Treasury-backed stablecoins.

**Reserve Structure:**
- 1:1 backing by cash and short-term US Treasuries in segregated accounts
- Circle Reserve Fund (USDXX): SEC-registered government money market fund
- Monthly attestations by Big Four auditors
- Daily reporting available through BlackRock

**Multi-Chain Deployment:**
- 20+ blockchains: Ethereum, Solana, Base, Optimism, Arbitrum, XRP Ledger
- 35+ million users globally
- Solana USDC transfer volume surpassed Ethereum on December 29, 2025

### Solana: Speed and Scale

[Solana](https://solana.com/) has emerged as a leading stablecoin settlement layer, with $15+ billion in stablecoin market cap — 3x growth from end of 2024.

**Technical advantages:**
- 400-millisecond block time
- Transaction costs: fractions of a cent
- 1,000-4,000 TPS

**Institutional adoption:**
- **Visa**: Using Solana for USDC settlement
- **Stripe**: Added support for Solana-based USDC payments
- **PayPal**: PYUSD launched on Solana May 2024, leveraging Token Extensions for built-in compliance features

PayPal's integration is notable: PYUSD users see a unified balance across PayPal/Venmo wallets regardless of underlying blockchain. The infrastructure becomes invisible to consumers.

### Ethereum: Smart Contract Foundation

[Ethereum](https://ethereum.org/) still hosts over 80% of global stablecoin supply, with smart contracts enabling:

- **DeFi integration**: Lending protocols (Aave, Compound), DEXs (Uniswap), yield farming
- **Automated treasury management**: Smart contracts rebalancing reserves
- **Payroll automation**: Enterprises using smart contracts for compliant global payments
- **MetaMask Stablecoin Earn**: Launched July 2025 for passive yield on stablecoins

DAI — the decentralized, crypto-collateralized stablecoin from MakerDAO — demonstrates an alternative model: algorithmic stability backed by over-collateralized positions in ETH, USDC, and approved tokens, governed by MKR token holders.

### The Strategic Bitcoin Reserve

The stablecoin framework exists alongside the [Strategic Bitcoin Reserve](https://www.whitehouse.gov/fact-sheets/2025/03/fact-sheet-president-donald-j-trump-establishes-the-strategic-bitcoin-reserve-and-u-s-digital-asset-stockpile/) established March 6, 2025.

The government holds an estimated 207,000 Bitcoin (approximately $17 billion at March 2025 prices) from criminal and civil forfeitures. The policy: no selling. A "digital Fort Knox."

A separate Digital Asset Stockpile holds Ether, XRP, Solana, and Cardano from forfeitures. The government is now a holder of the assets it regulates.

### No CBDC — A Deliberate Choice

[Executive Order 14178](https://www.federalreserve.gov/central-bank-digital-currency.htm) (January 2025) prohibits federal agencies from establishing, issuing, or promoting a Central Bank Digital Currency. The House passed the Anti-CBDC Surveillance State Act in July 2025.

While 134 jurisdictions (98% of global GDP) pursue CBDCs, the US chose a different path: let regulated private stablecoins — backed by Treasury debt, running on multiple blockchains — serve the function of digital dollars. The government gets demand for its debt instruments; the private sector innovates.

### Formal Verification: The Missing Layer

When stablecoin smart contracts control billions in Treasury-backed reserves, software correctness becomes a matter of financial system stability.

[Formal verification tools](https://www.certora.com/) like Certora — securing over $100 billion in DeFi total value locked — provide mathematical proof that contract code behaves as specified. The same techniques that verify aerospace software can verify financial infrastructure.

LOGOS contributes to this stack: translating natural language regulatory requirements into [first-order logic](https://plato.stanford.edu/entries/logic-classical/) that can be checked against implementations. When the GENIUS Act says reserves must be "100% backed by liquid assets," that requirement can be formally specified and verified.

### The Architecture Taking Shape

The emerging system:

1. **Treasury-backed stablecoins** (RLUSD, USDC) provide dollar liquidity
2. **Multiple blockchains** (XRP Ledger, Solana, Ethereum) provide settlement infrastructure
3. **Federal law** (GENIUS Act) regulates stablecoins; **pending legislation** (CLARITY Act) would clarify broader market structure
4. **Strategic reserves** (Bitcoin, digital assets) diversify government holdings
5. **Formal verification** ensures contract correctness
6. **No CBDC** — private innovation with public backing

Much of this is already operational. The stablecoins are trading; the reserves are published; the GENIUS Act is law. The CLARITY Act's passage would complete the regulatory picture. Whether this architecture defines digital money for the next decade depends on execution — and on the quality of the software underlying it all.

### Further Reading

- [GENIUS Act](https://www.congress.gov/bill/119th-congress/senate-bill/1582)
- [CLARITY Act](https://www.congress.gov/bill/119th-congress/house-bill/3633)
- [Ripple RLUSD](https://ripple.com/solutions/stablecoin/)
- [Circle USDC Transparency](https://www.circle.com/transparency)
- [XRP Ledger Documentation](https://xrpl.org/)
- [Strategic Bitcoin Reserve](https://www.whitehouse.gov/fact-sheets/2025/03/fact-sheet-president-donald-j-trump-establishes-the-strategic-bitcoin-reserve-and-u-s-digital-asset-stockpile/)
"#,
        tags: &["blockchain", "finance", "regulation", "stablecoins", "xrp"],
        author: "LOGICAFFEINE Team",
    },
    Article {
        slug: "formal-verification-space-satellites",
        title: "Proving Code Correct in Orbit: Formal Verification for Space Systems",
        date: "2026-01-30",
        summary: "NASA's Artemis program, the seL4 verified microkernel, and static analyzers like Astrée demonstrate how mathematical proof techniques protect spacecraft from software failures 250,000 miles from the nearest debugger.",
        content: r#"
## When Reboot Isn't an Option

The Artemis II flight software — approximately 50,000 lines of code controlling human lives on the journey to lunar orbit — was "flown" more than 100,000 simulated times before the actual launch. But simulation alone can't guarantee correctness. For that, NASA increasingly relies on [formal methods](https://shemesh.larc.nasa.gov/nfm2025/): mathematical techniques that prove software properties hold for all possible inputs, not just the ones you thought to test.

### The NASA Formal Methods Program

The [NASA Formal Methods Symposium](https://shemesh.larc.nasa.gov/nfm2025/), now in its 17th year, brings together researchers working on "formal techniques for software and system assurance in space, aviation, and robotics." The 2025 symposium at William & Mary focused on verification challenges for the next generation of space systems.

NASA's formal methods infrastructure includes:

- **NASA-STD-8739.8**: Software Assurance and Safety Standard
- **NASA-STD-8739.9**: Software Formal Inspections Standard
- **Independent Verification and Validation (IV&V)**: Applied to all human spaceflight programs

Projects currently under formal verification scrutiny include the Roman Space Telescope, Europa Clipper, Regenerative Fuel Cell systems, and all Artemis program elements — SLS, Orion, and Gateway.

### The seL4 Verified Microkernel

[seL4](https://sel4.systems/) represents the gold standard in formally verified operating systems. This microkernel — 8,700 lines of C and 600 lines of assembler — comes with a machine-checked proof from abstract specification down to the actual C implementation.

The proof guarantees: no code injection attacks, no buffer overflows, no null pointer dereferences. These aren't claims based on testing; they're mathematical certainties derived from the code itself.

**Space Applications of seL4:**

NASA's core Flight Software (cFS), which runs on the Artemis program and Roman Space Telescope missions, has been ported to seL4. This led to the development of Magnetite, a real-time operating system built on verified foundations.

**Defense Applications:**

DARPA's PROVERS and INSPECTA programs (started 2024) fund continued seL4 development. The CASE project with Collins Aerospace and Galois explores verified software for military aviation systems. [DornerWorks](https://dornerworks.com/) has deployed seL4 for aerospace, defense, medical, and automotive customers since 2015.

### DO-178C and the Formal Methods Supplement

Commercial and military aviation software follows [DO-178C](https://en.wikipedia.org/wiki/DO-178C), released in December 2011 and recognized by the FAA, EASA, and Transport Canada. The standard defines Design Assurance Levels (DAL) A through E based on failure severity, with Level A — "catastrophic failure" — requiring the most rigorous verification.

The formal methods supplement, [DO-333](https://en.wikipedia.org/wiki/DO-178C#Formal_methods_supplement), provides specific guidance for applying theorem proving, model checking, and abstract interpretation to certification. Crucially, formal methods can complement or replace dynamic testing — you can prove properties instead of testing for them.

Additional supplements cover Model-Based Development (DO-331), Object-Oriented Technology (DO-332), and Tool Qualification (DO-330).

### Astrée: Static Analysis at Scale

[Astrée](https://www.absint.com/astree/index.htm) is a commercial static analyzer built on abstract interpretation theory. It doesn't test code; it mathematically analyzes all possible execution paths to prove the absence of runtime errors.

**Aerospace Deployments:**

- **Airbus (since 2003)**: Safety-critical software for the A380 and subsequent aircraft
- **European Space Agency (2008)**: Proved absence of runtime errors in the Jules Verne ATV automatic docking software — the system that autonomously docked supply vessels with the International Space Station

**Other Safety-Critical Industries:**

- **Bosch Automotive Steering (2018)**: Replaced legacy analysis tools, acquired worldwide license
- **Framatome**: TELEPERM XS platform for nuclear reactor control systems

Astrée is certified by NIST as satisfying criteria for sound static code analysis (2020) and complies with ISO 26262 (automotive), DO-178B/C (aerospace), IEC-61508 (functional safety), and EN-50128 (railway).

**Technical Capabilities:**

The analyzer detects divisions by zero, buffer overflows, null pointer dereferences, data races, and deadlocks. It also estimates Worst-Case Execution Time (WCET) — critical for real-time systems where missing a deadline can be as dangerous as computing the wrong answer.

### SpaceX: A Different Philosophy

[SpaceX](https://www.spacex.com/) takes a somewhat different approach to software verification. Falcon 9 runs three dual-core x86 processors executing Linux, with code written in C/C++. The architecture relies on triplex redundancy — three independent computers that vote on decisions — to tolerate both hardware failures and potential software bugs.

As SpaceX engineers have noted: "Writing the software is some small percentage of what actually goes into getting it ready to fly." The verification process involves extensive simulation, defensive programming, and fault-tolerant architecture.

For NASA's Commercial Crew Demo-1 mission, the requirement was explicit: software must be tolerant to any two simultaneous faults. Redundancy compensates for uncertainty.

### The Verification Spectrum

These approaches represent different points on a verification spectrum:

| Approach | Assurance Level | Cost | Applicability |
|----------|-----------------|------|---------------|
| **Formal proof** (seL4) | Mathematical certainty | Very high | Critical kernels, security foundations |
| **Static analysis** (Astrée) | Absence of specific bug classes | High | Safety-critical applications |
| **Model checking** | Exhaustive state exploration | Medium-high | Protocol verification, concurrent systems |
| **Redundancy** (SpaceX) | Fault tolerance | Medium | Systems where verification cost exceeds redundancy cost |
| **Testing** | Confidence, not proof | Variable | Everything (necessary but not sufficient) |

### LOGOS and Space Systems

Natural language requirements documents are standard in aerospace. "The system shall not permit commanded thrust during crew egress" is a requirement that must be translated into verified software behavior.

LOGOS enables this translation explicitly. A requirement stated in controlled English can be parsed into [first-order logic](https://plato.stanford.edu/entries/logic-classical/), which can then be checked against formal specifications of the software. The gap between what the requirement document says and what the code does becomes formally verifiable.

Combined with tools like [Z3](https://github.com/Z3Prover/z3) for satisfiability checking, this pipeline could reduce the manual effort in aerospace verification while increasing confidence in the translation from requirements to implementation.

### The Stakes

When software fails in space, there's no patch deployment. The Artemis astronauts will be 250,000 miles from the nearest software engineer. Every line of code, every state transition, every interrupt handler must work correctly the first time and every time after.

Formal verification doesn't make this easy. But it makes it possible to know — with mathematical certainty — that critical properties hold. In an environment where "pretty sure" isn't good enough, proof matters.

### Further Reading

- [NASA Formal Methods Symposium 2025](https://shemesh.larc.nasa.gov/nfm2025/)
- [seL4 Foundation](https://sel4.systems/)
- [DO-178C Overview](https://en.wikipedia.org/wiki/DO-178C)
- [Astrée Static Analyzer](https://www.absint.com/astree/index.htm)
- [NASA IV&V Program](https://www.nasa.gov/ivv-services/)
"#,
        tags: &["aerospace", "verification", "safety", "space"],
        author: "LOGICAFFEINE Team",
    },
    Article {
        slug: "smart-contract-verification-defi-security",
        title: "Smart Contract Verification: Formal Methods Meet DeFi Security",
        date: "2026-01-28",
        summary: "With $77 billion lost to smart contract exploits, formal verification tools like Certora are becoming essential infrastructure. The same mathematical techniques that prove aerospace software correct can prove financial contracts behave as specified.",
        content: r#"
## The $77 Billion Problem

Between 2023 and 2025, approximately $77.1 billion was lost to smart contract exploits. In 2024 alone, $1.42 billion disappeared across 310 security incidents. Access control flaws caused 59% of 2025 losses; 34.6% of exploits stemmed from faulty input validation.

These aren't theoretical risks. In February 2025, [Bybit lost $1.5 billion](https://www.chainalysis.com/) in under 15 minutes. In May 2025, Cetus DEX lost $223 million due to a missing overflow check. A single missing validation in a single function cost a quarter billion dollars.

Traditional software testing — even extensive testing — cannot guarantee the absence of such bugs. [Formal verification](https://www.certora.com/) can.

### The DAO Hack: Where It Started

The case for smart contract verification crystallized in 2016 with [The DAO hack](https://www.gemini.com/cryptopedia/the-dao-hack-makerdao). Approximately $150 million in ETH was stolen through a reentrancy attack — a bug class where a contract calls back into itself before updating its state, allowing repeated withdrawals.

The vulnerability had been identified before the attack. A fix was pending. The attacker moved faster.

The aftermath split Ethereum into two chains (Ethereum and Ethereum Classic) and established a precedent: smart contract bugs have existential consequences, and traditional development practices aren't sufficient.

### Verification Tools: The Current Landscape

**[Certora](https://www.certora.com/)** leads the formal verification space for smart contracts. The platform:

- Secures over $100 billion in total value locked (TVL)
- Serves Aave, MakerDAO, Uniswap, Lido, EigenLayer, and the Solana Foundation
- Has written over 70,000 verification rules
- Claims to "secure 70% of top protocols"

A notable finding: Certora discovered a fundamental flaw in MakerDAO's core DAI equation — a bug that had been present since 2018, undetected through years of audits. The formal verifier found it in 23 seconds. Fuzz testing had failed to find it after 125 million iterations.

Certora also discovered a bug in SushiSwap's Trident pools before deployment and open-sourced the Certora Prover in February 2025.

**[Trail of Bits](https://www.trailofbits.com/)** developed several essential tools:

- **Echidna**: Property-based fuzzer that generates randomized test cases
- **Slither**: Static analyzer that identifies vulnerability patterns
- **Manticore**: Symbolic execution engine for deep analysis

**[OpenZeppelin](https://www.openzeppelin.com/)** provides:

- Auditing services protecting over $50 billion in assets
- Trusted Solidity and Cairo libraries used across the ecosystem
- Zero-knowledge proof verification services

### How Formal Verification Works

Formal verification for smart contracts follows a process similar to aerospace verification:

**1. Specification**: Define what the contract should do in formal logic. "The total supply of tokens must equal the sum of all balances" becomes a mathematical invariant.

**2. Analysis**: The verifier explores all possible execution paths, proving the specification holds in every case — not just tested cases.

**3. Counterexamples**: If verification fails, the tool provides a concrete counterexample: specific inputs that violate the specification, enabling targeted debugging.

This differs fundamentally from testing. Testing shows the presence of bugs; formal verification proves their absence (for specified properties).

### Case Studies: What Verification Could Have Prevented

**Wormhole Bridge (February 2022)** — $320+ million stolen. The attacker bypassed signature verification by injecting a fake system account. A formal specification stating "all transfers require valid signatures from authorized accounts" would have caught the missing check.

**Euler Finance (March 2023)** — $197 million stolen via flash loan attack. A missing check on liquidity status in the DonateToReserve function allowed manipulation. An invariant specifying "donations cannot create undercollateralized positions" would have identified the vulnerability.

**Ronin Bridge (March 2022)** — $625 million stolen by the North Korean Lazarus Group. This was a key compromise, not a code bug — 5 of 9 validator keys were obtained through social engineering. Formal verification of code wouldn't have helped; operational security failed.

The distinction matters: formal verification proves code correctness, not operational security.

### The Economics of Verification

Formal verification isn't cheap. Comprehensive verification of a complex protocol can exceed $200,000. Audits from top firms (Trail of Bits, OpenZeppelin, Certora) command premium prices.

But the economics increasingly favor verification:

- A $200,000 verification cost is trivial compared to a $200 million exploit
- Only 30% of DeFi developers had integrated formal verification as of Q3 2025
- Pilot programs show formal verification can reduce vulnerabilities by up to 70%

The industry is converging on a layered approach:

1. **Static analysis** (Slither) catches common patterns quickly
2. **Fuzzing** (Echidna) explores edge cases through randomized testing
3. **Formal verification** (Certora) proves critical invariants
4. **Bug bounties** (Immunefi, HackerOne) incentivize external review
5. **Continuous monitoring** (Forta, Tenderly) detects anomalies in production

### The LOGOS Connection

Smart contract specifications are often written in natural language, then manually translated to formal properties. This translation is error-prone — the specification might not capture what the developer intended, or the formal property might not match the English description.

LOGOS addresses this gap. A specification like:

> "The total supply of tokens must always equal the sum of all account balances"

Can be parsed into [first-order logic](https://plato.stanford.edu/entries/logic-classical/):

```
∀t(TotalSupply(t) = Σ(Balance(a, t)) for all accounts a)
```

This formal representation can then be fed to verification tools, checked against implementations, and traced back to the original requirement. The translation becomes explicit and verifiable rather than implicit and error-prone.

### What Changes

As stablecoins become regulated financial infrastructure under the [GENIUS Act](https://www.congress.gov/bill/119th-congress/senate-bill/1582) and DeFi protocols manage billions in Treasury-backed assets, the stakes for smart contract correctness approach those of aerospace or medical devices.

The same [Z3 theorem prover](https://github.com/Z3Prover/z3) used to verify flight control software can verify stablecoin reserve management. The same formal methods that prove absence of buffer overflows can prove absence of reentrancy vulnerabilities.

The tools exist. The economic incentive exists. The remaining challenge is adoption — integrating formal verification into development workflows as a standard practice rather than an expensive add-on.

### Further Reading

- [Certora](https://www.certora.com/)
- [Trail of Bits](https://www.trailofbits.com/)
- [Halborn DeFi Hacks Report 2025](https://www.halborn.com/reports/top-100-defi-hacks-2025)
- [The DAO Hack Explained](https://www.gemini.com/cryptopedia/the-dao-hack-makerdao)
- [Aave Continuous Formal Verification](https://governance.aave.com/t/security-and-agility-of-aave-smart-contracts-via-continuous-formal-verification/10181)
"#,
        tags: &["blockchain", "verification", "security", "defi"],
        author: "LOGICAFFEINE Team",
    },
    Article {
        slug: "medical-device-software-verification",
        title: "Medical Device Software: When Bugs Kill Patients",
        date: "2026-01-27",
        summary: "The Therac-25 accidents killed patients through software race conditions. Modern medical devices contain hundreds of thousands of lines of code. IEC 62304 and formal verification techniques are how we prevent the next tragedy.",
        content: r#"
## The Therac-25 Lesson

Between 1985 and 1987, the [Therac-25](https://en.wikipedia.org/wiki/Therac-25) radiation therapy machine caused at least six accidents where patients received approximately 100 times the intended radiation dose. Multiple patients died. It remains "the worst series of radiation accidents in the 35-year history of medical accelerators."

The cause was software. Specifically: race conditions in concurrent programming and the removal of hardware interlocks in favor of software-only safety checks.

The manufacturer initially dismissed user complaints as "impossible" — the software couldn't produce those readings. The software could. It did. Patients paid with their lives.

### The Complexity Problem

Modern medical devices are orders of magnitude more complex than the Therac-25:

- Contemporary pacemakers: up to **80,000 lines of code**
- Infusion pumps: more than **170,000 lines of code**
- A 200,000-line device can have more than **10^12 individual execution paths**

Manual analysis of all paths in a 200,000-line device would require 20,000 developers working for over 100 years. Testing all paths isn't just impractical; it's mathematically impossible within any reasonable timeframe.

### IEC 62304: The Software Lifecycle Standard

[IEC 62304](https://en.wikipedia.org/wiki/IEC_62304) is the international standard for medical device software lifecycle processes, recognized by the FDA, European regulators, and health authorities worldwide.

The standard classifies software by risk:

| Class | Risk Level | Example |
|-------|------------|---------|
| **A** | No injury possible | Data display, documentation |
| **B** | Non-serious injury possible | Monitoring alerts, dosage calculations with manual override |
| **C** | Death or serious injury possible | Closed-loop drug delivery, radiation therapy control |

Class C software — where bugs can kill — requires the most rigorous verification. Changes must be verified through testing, regression testing is mandatory, and integration with [ISO 14971](https://en.wikipedia.org/wiki/ISO_14971) risk management is required throughout development.

The FDA recognizes IEC 62304 conformity, which can significantly reduce 510(k) submission documentation requirements.

### Why Testing Isn't Enough

The fundamental limitation of testing is coverage. A test demonstrates that specific inputs produce correct outputs. It says nothing about untested inputs.

For the Therac-25, the race condition that caused overdoses occurred only under specific timing conditions — when operators entered commands at a particular speed in a particular sequence. Normal testing didn't trigger it. The condition existed in the code, invisible to testers.

Formal verification inverts this relationship. Instead of checking specific inputs, formal methods prove that properties hold for **all possible inputs**. "The radiation dose shall never exceed the prescribed maximum" becomes a mathematical statement verified against the code itself.

### Formal Methods in Medical Device Verification

Research applications of formal verification in medical devices:

**Insulin Infusion Pumps**: Researchers have used [Event-B](https://en.wikipedia.org/wiki/Event-B) modeling language and Rodin proof tools to verify safety properties of insulin pump software. The Generic Infusion Pump (GIP) Project created a reference implementation verifiable against safety requirements.

**Pacemakers**: Pacemaker software has been formalized using Event-B, with Abstraction Trees enabling closed-loop model checking — verifying not just the device software but its interaction with models of cardiac physiology.

**Static Analysis**: [Formal methods-based static analysis](https://www.embedded.com/a-formal-methods-based-verification-approach-to-medical-device-software-analysis/) can exhaustively explore software behavior in minutes, proving absence of runtime errors like null pointer dereferences, buffer overflows, and arithmetic exceptions.

### The Regulatory Trajectory

The FDA has signaled increasing interest in formal methods for pre-market review. Guidance documents acknowledge that traditional testing alone may be insufficient for high-complexity, safety-critical software.

Current trends in FDA regulation emphasize:

- **Software as a Medical Device (SaMD)**: Software that itself qualifies as a medical device, regardless of hardware
- **Cybersecurity**: Connected devices introduce vulnerabilities beyond functional correctness
- **Machine Learning**: AI/ML-based devices require new verification paradigms

[IEC 81001-5](https://www.iso.org/standard/76098.html), now recognized by the FDA, addresses health software security lifecycle — verification must extend beyond functional correctness to include security properties.

### Verification Techniques Applicable to Medical Devices

The same tools used in aerospace apply to medical devices:

**Abstract Interpretation** ([Astrée](https://www.absint.com/astree/index.htm)): Proves absence of runtime errors including divisions by zero, buffer overflows, null pointer dereferences. Already used in nuclear reactor control systems.

**Model Checking**: Exhaustively explores finite state spaces. Effective for protocol verification and concurrent systems — exactly the kind of code that killed Therac-25 patients.

**Theorem Proving** ([Z3](https://github.com/Z3Prover/z3), [Coq](https://coq.inria.fr/)): Generates mathematical proofs of program properties. Higher assurance than other methods; higher cost.

**Formal Specification Languages**: [Event-B](https://www.event-b.org/), [TLA+](https://lamport.azurewebsites.net/tla/tla.html), [Alloy](https://alloytools.org/) enable precise specification of system behavior for analysis.

### LOGOS for Medical Requirements

Medical device requirements documents are written in natural language: "The pump shall cease infusion if the pressure exceeds the safety threshold." These requirements must be translated into software specifications, then into code, then verified.

LOGOS provides a formal semantics for this translation. The English requirement parses to [first-order logic](https://plato.stanford.edu/entries/logic-classical/):

```
∀t∀p(Pressure(t) > SafetyThreshold ∧ Infusing(t) → Cease(t+1))
```

This formal specification can be:
- Checked for internal consistency (does it contradict other requirements?)
- Traced to implementation (does the code satisfy this property?)
- Verified against the final system (does the compiled software maintain this invariant?)

The gap between what regulators read in requirements documents and what verification tools analyze becomes explicit and auditable.

### The Stakes

A modern pacemaker makes life-or-death decisions millions of times over its operational lifetime. An insulin pump calculates drug doses that could cause hypoglycemic shock if wrong. A radiation therapy system delivers energy that heals at correct doses and kills at incorrect ones.

These devices cannot be recalled like smartphones. They cannot be patched over the air. They must work correctly from deployment through end of life, in patients whose physiological states the developers couldn't anticipate.

Formal verification doesn't make this easy. But it provides mathematical guarantees that testing cannot. For software that decides who lives and who dies, those guarantees matter.

### Further Reading

- [Therac-25 Case Study](https://en.wikipedia.org/wiki/Therac-25)
- [IEC 62304 Standard](https://en.wikipedia.org/wiki/IEC_62304)
- [ISO 14971 Risk Management](https://en.wikipedia.org/wiki/ISO_14971)
- [FDA Software as Medical Device](https://www.fda.gov/medical-devices/digital-health-center-excellence/software-medical-device-samd)
- [Event-B Formal Method](https://www.event-b.org/)
- [Formal Methods in Medical Devices Paper](https://link.springer.com/chapter/10.1007/978-3-319-21070-4_39)
"#,
        tags: &["medical", "verification", "safety", "regulation"],
        author: "LOGICAFFEINE Team",
    },
    Article {
        slug: "logos-on-grokipedia",
        title: "LOGOS Recognized on Grokipedia: A Milestone for Natural Language Programming",
        date: "2026-01-26",
        summary: "LOGOS now has its own page on Grokipedia, xAI's community-driven encyclopedia. This independent recognition validates the growing interest in natural language as a programming interface.",
        content: r#"
## LOGOS Joins the Encyclopedia

LOGOS has earned its own entry on [Grokipedia](https://grokipedia.com/page/LOGOS_programming_language), the AI-generated encyclopedia launched by [xAI](https://x.ai) in October 2025. For a domain-specific language focused on translating English to formal logic, independent documentation matters — it means the ideas are resonating beyond our own channels.

### What xAI's Grok Documented

The Grokipedia article, generated by [Grok](https://en.wikipedia.org/wiki/Grok_(chatbot)), provides a technical overview of LOGOS as a language that compiles natural English into either executable [Rust](https://www.rust-lang.org/) code or [first-order logic](https://plato.stanford.edu/entries/logic-classical/) representations. The coverage includes:

- **Dual-mode compilation** — the same English input produces either imperative code or formal logic suitable for [automated theorem proving](https://en.wikipedia.org/wiki/Automated_theorem_proving)
- **Distributed systems primitives** — native [CRDT](https://en.wikipedia.org/wiki/Conflict-free_replicated_data_type) support and peer-to-peer networking via libp2p
- **Formal verification** — integration with the [Z3 theorem prover](https://github.com/Z3Prover/z3), the SMT solver developed by Leonardo de Moura and Nikolaj Bjørner that received the 2015 ACM SIGPLAN Programming Languages Software Award
- **Semantic parsing** — [Montague-style](https://plato.stanford.edu/entries/montague-semantics/) compositional semantics and Neo-Davidsonian event representations

The article also documents current limitations honestly, which we appreciate. Transparency about what LOGOS can and cannot do is important.

### Why Independent Documentation Matters

The history of programming language adoption shows that external validation accelerates credibility. When [Gottlob Frege](https://plato.stanford.edu/entries/frege/) published his *Begriffsschrift* in 1879, establishing the foundations of modern predicate logic, it took decades for mathematicians to recognize its significance. Today, discoverability happens faster — but it still requires independent sources confirming that a project solves real problems.

Having documentation on Grokipedia means researchers, developers, and students exploring natural language programming can find LOGOS through xAI's infrastructure, not just through our own marketing.

### The Broader Context

LOGOS sits at the intersection of several active research areas: [semantic parsing](https://en.wikipedia.org/wiki/Semantic_parsing) (mapping natural language to logical forms), [formal verification](https://en.wikipedia.org/wiki/Formal_verification) (mathematical proofs of program correctness), and the growing interest in AI systems that can reason formally. Companies like [Anthropic](https://www.anthropic.com/) are exploring how large language models can assist with logical reasoning, while tools like Z3 provide the backend verification infrastructure.

We're building LOGOS because we believe natural language shouldn't be a barrier to formal thinking — and independent recognition suggests others agree.

### Try It Yourself

Read the full entry at [grokipedia.com/page/LOGOS_programming_language](https://grokipedia.com/page/LOGOS_programming_language), or experience the language directly in the [Studio](/studio).
"#,
        tags: &["milestone", "community", "announcement"],
        author: "LOGICAFFEINE Team",
    },
    Article {
        slug: "introducing-logicaffeine",
        title: "Introducing LOGICAFFEINE: From Natural Language to Formal Logic",
        date: "2026-01-15",
        summary: "LOGICAFFEINE translates everyday English into rigorous First-Order Logic, making formal reasoning accessible without requiring expertise in symbolic notation.",
        content: r#"
## The Gap Between How We Think and How We Prove

Natural language is imprecise by design. When we say "every student passed an exam," we might mean each student passed at least one exam (possibly different exams), or that there exists a single exam everyone passed. In conversation, context resolves these ambiguities. In formal reasoning, they become logical errors.

[First-order logic](https://plato.stanford.edu/entries/logic-classical/) (FOL) eliminates this ambiguity. Developed by [Gottlob Frege](https://plato.stanford.edu/entries/frege/) in 1879 and independently by [Charles Sanders Peirce](https://plato.stanford.edu/entries/peirce-logic/) in 1885, FOL provides a precise language for expressing statements about objects, their properties, and their relationships. It's the foundation of modern mathematics, database query languages, and [automated theorem proving](https://en.wikipedia.org/wiki/Automated_theorem_proving).

The problem: learning FOL notation is a barrier. LOGICAFFEINE removes that barrier.

### What LOGICAFFEINE Does

LOGICAFFEINE is an English-to-FOL transpiler built on LOGOS. You write sentences in plain English:

> "Every philosopher who teaches logic influences some student."

LOGICAFFEINE parses the sentence using [Montague semantics](https://plato.stanford.edu/entries/montague-semantics/) — a framework developed by mathematician Richard Montague that treats natural language with the same rigor as formal languages — and outputs the corresponding logical formula:

```
∀x((Philosopher(x) ∧ TeachesLogic(x)) → ∃y(Student(y) ∧ Influences(x, y)))
```

The [universal quantifier](https://en.wikipedia.org/wiki/Universal_quantification) (∀) was introduced by Gerhard Gentzen in 1935, derived from a rotated "A" for "all." The [existential quantifier](https://en.wikipedia.org/wiki/Existential_quantification) (∃) was introduced by Giuseppe Peano in 1896, derived from a rotated "E" for "exists." LOGICAFFEINE outputs both Unicode symbols and LaTeX notation.

### Why This Matters Now

The rise of AI systems that can engage in complex reasoning — like [Anthropic's Claude](https://www.anthropic.com/claude), which uses [Constitutional AI](https://www.anthropic.com/research/constitutional-ai-harmlessness-from-ai-feedback) to reason about ethical constraints — has renewed interest in formal logic as a verification layer. If an AI system claims to have proven something, how do we verify the proof?

Formal verification tools like the [Z3 theorem prover](https://github.com/Z3Prover/z3) can check logical validity, but they require input in formal notation. LOGICAFFEINE bridges this gap: you express your reasoning in English, and we produce verifiable formal logic.

This matters for:

- **Critical thinking education** — see exactly what your arguments claim, no symbolic notation required
- **AI alignment research** — express constraints in natural language, verify them formally
- **Software specification** — describe requirements in English, generate testable formal specifications
- **Legal and policy analysis** — identify logical ambiguities in contract language or regulations

### The Technology

LOGICAFFEINE uses a parsing architecture inspired by research in [computational linguistics](https://en.wikipedia.org/wiki/Computational_linguistics). The lexer classifies words using a curated vocabulary database. The parser builds [abstract syntax trees](https://en.wikipedia.org/wiki/Abstract_syntax_tree) using techniques from the [Open Logic Project](https://openlogicproject.org/), a collaborative effort to create open-source logic education materials. The transpiler generates FOL following the notation conventions established in [forall x](https://forallx.openlogicproject.org/), a free textbook used in university logic courses worldwide.

### Get Started

Visit the [Learn page](/learn) for an interactive curriculum starting from first principles, or jump directly into the [Studio](/studio) to experiment with your own sentences.

Formal logic has been the foundation of rigorous reasoning since Aristotle. LOGICAFFEINE makes it accessible to anyone who can write a sentence.
"#,
        tags: &["release", "announcement", "formal-logic"],
        author: "LOGICAFFEINE Team",
    },
    Article {
        slug: "getting-started-with-fol",
        title: "First-Order Logic: A Practical Introduction",
        date: "2026-01-18",
        summary: "First-order logic has been the foundation of mathematical reasoning for over a century. Here's how it works and why LOGICAFFEINE makes it accessible.",
        content: r#"
## What First-Order Logic Actually Is

[First-order logic](https://plato.stanford.edu/entries/logic-classical/) (also called predicate logic or quantificational logic) is a formal system for making precise statements about objects and their relationships. The [Stanford Encyclopedia of Philosophy](https://plato.stanford.edu/) describes it as the standard framework for formalizing mathematical theories and reasoning about computational systems.

The "first-order" designation distinguishes it from propositional logic (which only handles true/false statements) and higher-order logics (which can quantify over predicates themselves). As philosopher [W.V.O. Quine](https://en.wikipedia.org/wiki/Willard_Van_Orman_Quine) noted, first-order logic hits a sweet spot: expressive enough for most mathematical reasoning, constrained enough to have [complete proof systems](https://en.wikipedia.org/wiki/G%C3%B6del%27s_completeness_theorem).

### The Core Components

**Variables** represent arbitrary objects in your domain of discourse. In the formula `∀x(Human(x) → Mortal(x))`, the variable `x` ranges over all objects.

**Quantifiers** specify scope:
- The [universal quantifier](https://en.wikipedia.org/wiki/Universal_quantification) **∀** (introduced by Gentzen in 1935) means "for all"
- The [existential quantifier](https://en.wikipedia.org/wiki/Existential_quantification) **∃** (introduced by Peano in 1896) means "there exists"

**Predicates** express properties and relations. `Human(x)` is a unary predicate (one argument); `Loves(x, y)` is binary (two arguments).

**Connectives** combine statements:
- `∧` (conjunction): "and"
- `∨` (disjunction): "or"
- `→` (implication): "if...then"
- `¬` (negation): "not"

### A Historical Example

The classic syllogism "All humans are mortal; Socrates is human; therefore Socrates is mortal" becomes:

```
Premise 1: ∀x(Human(x) → Mortal(x))
Premise 2: Human(socrates)
Conclusion: Mortal(socrates)
```

This is valid by [universal instantiation](https://en.wikipedia.org/wiki/Universal_instantiation) — substituting a specific constant (`socrates`) for the universally quantified variable. [Aristotle](https://plato.stanford.edu/entries/aristotle-logic/) analyzed this pattern 2,400 years ago; FOL provides the modern formal notation.

### Why Precision Matters

Consider: "Every student admires some professor."

This sentence has two valid interpretations:
1. `∀x(Student(x) → ∃y(Professor(y) ∧ Admires(x, y)))` — each student admires at least one professor (possibly different professors)
2. `∃y(Professor(y) ∧ ∀x(Student(x) → Admires(x, y)))` — there's one professor everyone admires

The difference is [quantifier scope](https://plato.stanford.edu/entries/quantification/). In natural language, both readings are valid. In formal logic, you must choose. This precision is why FOL underlies [database query languages](https://en.wikipedia.org/wiki/Relational_algebra), [automated theorem provers](https://en.wikipedia.org/wiki/Automated_theorem_proving), and formal software specifications.

### How LOGICAFFEINE Helps

Traditional FOL education requires memorizing symbols and manipulation rules — the approach used in university courses and textbooks like [forall x](https://forallx.openlogicproject.org/) from the [Open Logic Project](https://openlogicproject.org/).

LOGICAFFEINE inverts this: you write natural English, and we show the formal translation. This builds intuition for what logical structure underlies everyday statements. When you see that "No cats are dogs" becomes `¬∃x(Cat(x) ∧ Dog(x))`, you understand negation scope experientially rather than abstractly.

### Practical Applications

FOL isn't just academic. It's embedded in:

- **Databases**: SQL's `WHERE` clauses implement FOL predicates; `EXISTS` and `NOT EXISTS` are quantifiers
- **Formal verification**: Tools like the [Z3 theorem prover](https://github.com/Z3Prover/z3) check logical satisfiability for software correctness proofs
- **AI systems**: Knowledge representation in expert systems and semantic web technologies like [OWL](https://en.wikipedia.org/wiki/Web_Ontology_Language) builds on description logics derived from FOL
- **Legal reasoning**: Contract analysis tools identify ambiguous clauses by detecting scope ambiguities

### Next Steps

The [Learn page](/learn) offers an interactive curriculum progressing from basic predicates through quantifier nesting and scope ambiguity. The [Studio](/studio) provides a sandbox for experimenting with arbitrary sentences.

First-order logic has been the backbone of precise reasoning since [Frege](https://plato.stanford.edu/entries/frege/) and [Peirce](https://plato.stanford.edu/entries/peirce-logic/) independently invented it in the 1870s-1880s. LOGICAFFEINE makes that precision accessible without the notation barrier.
"#,
        tags: &["tutorial", "beginner", "formal-logic", "education"],
        author: "LOGICAFFEINE Team",
    },
    Article {
        slug: "studio-mode-playground",
        title: "The LOGICAFFEINE Studio: Interactive Logic Exploration",
        date: "2026-01-20",
        summary: "The Studio provides real-time English-to-FOL translation, multiple output formats, syntax visualization, and AST inspection for exploring how natural language maps to formal logic.",
        content: r#"
## A Workbench for Formal Reasoning

The LOGICAFFEINE Studio is an interactive environment for exploring how natural language maps to [first-order logic](https://plato.stanford.edu/entries/logic-classical/). Unlike static textbooks or one-way compilers, the Studio provides immediate feedback as you type, helping you build intuition for logical structure.

### Real-Time Translation

Type any English sentence and watch the FOL translation update live. The parser processes your input through several stages:

1. **Lexical analysis** — words are classified (nouns, verbs, quantifiers, connectives) using a vocabulary database informed by [computational linguistics](https://en.wikipedia.org/wiki/Computational_linguistics) research
2. **Syntactic parsing** — the sentence structure is analyzed following patterns from [Montague grammar](https://plato.stanford.edu/entries/montague-semantics/)
3. **Semantic composition** — meanings combine according to the [principle of compositionality](https://en.wikipedia.org/wiki/Principle_of_compositionality): the meaning of the whole derives from the meanings of parts and their syntactic combination
4. **FOL generation** — the final logical formula is produced

This pipeline mirrors how theoretical linguists analyze meaning, but made interactive.

### Output Formats

The Studio supports multiple notation systems:

**Unicode symbols** — the modern standard used in academic papers and digital communication:
```
∀x(Human(x) → ∃y(Heart(y) ∧ Has(x, y)))
```

**LaTeX notation** — for integration with academic typesetting:
```
\forall x (Human(x) \rightarrow \exists y (Heart(y) \land Has(x, y)))
```

The [universal quantifier symbol](https://en.wikipedia.org/wiki/Universal_quantification) (∀) was introduced by [Gerhard Gentzen](https://en.wikipedia.org/wiki/Gerhard_Gentzen) in 1935 — a rotated "A" for "all." Gentzen also developed [natural deduction](https://en.wikipedia.org/wiki/Natural_deduction), the proof system underlying many modern theorem provers.

### Syntax Highlighting

The Studio color-codes your input to reveal logical structure:

- **Quantifier words** (every, all, some, no) highlighted in purple
- **Nouns and noun phrases** in blue — these become predicates or constants
- **Verbs** in green — these become predicates relating arguments
- **Connectives** (and, or, if, not) highlighted to show logical structure

This visualization helps you see how English grammar maps to logical form before you even look at the output.

### Abstract Syntax Tree Inspection

For deeper analysis, the Studio displays the [abstract syntax tree](https://en.wikipedia.org/wiki/Abstract_syntax_tree) (AST) of your sentence. The AST reveals:

- How quantifier scope is resolved (which quantifier binds which variable)
- How relative clauses attach to their head nouns
- How coordination (and/or) groups constituents
- Where ambiguity might arise in the parsing

This feature is particularly useful for understanding sentences with multiple valid interpretations — you can see exactly how LOGICAFFEINE resolved the ambiguity.

### Exploration Strategies

**Start with classic examples:**
- "All humans are mortal" — basic universal quantification
- "Some philosophers are wise" — existential quantification
- "No cats are dogs" — negated existentials

**Explore quantifier interaction:**
- "Every student read some book" — compare `∀∃` vs `∃∀` readings
- "Most students who take logic pass some exam" — restricted quantification

**Test ambiguous constructions:**
- "The professor saw the student with the telescope" — attachment ambiguity
- "Flying planes can be dangerous" — structural ambiguity

**Examine complex sentences:**
- Build up from simple predicates to nested quantifiers
- Compare how small wording changes affect logical structure

### Technical Foundation

The Studio is built on LOGOS, which compiles to [Rust](https://www.rust-lang.org/) for the web interface via WebAssembly. The parsing architecture uses techniques documented in the [Open Logic Project](https://openlogicproject.org/) materials, adapted for real-time interactive use.

For verified outputs, LOGICAFFEINE can connect to the [Z3 theorem prover](https://github.com/Z3Prover/z3) to check satisfiability — confirming that your logical formula is consistent (or identifying contradictions).

### Open the Studio

The [Studio](/studio) is freely available. No account required — just start typing and explore how your natural language intuitions map to formal logical structure.

As linguist Barbara Partee [famously said](https://plato.stanford.edu/entries/montague-semantics/) about lambda calculus in semantics: "Lambdas changed my life." The Studio aims to provide similar revelations about the hidden logical structure of everyday language.
"#,
        tags: &["feature", "tutorial", "tools"],
        author: "LOGICAFFEINE Team",
    },
    Article {
        slug: "formal-verification-matters",
        title: "Why Formal Verification Matters: From Aviation to AI",
        date: "2026-01-22",
        summary: "Formal verification uses mathematical proof to guarantee software correctness. Here's why industries from aerospace to AI are adopting these techniques.",
        content: r#"
## Beyond Testing: Mathematical Proof of Correctness

Testing can show the presence of bugs, but never their absence. [Formal verification](https://en.wikipedia.org/wiki/Formal_verification) takes a different approach: mathematically proving that software behaves correctly for all possible inputs, not just the ones you thought to test.

This distinction matters when failure is catastrophic. In aviation, medical devices, and financial systems, "works in testing" isn't good enough.

### The Aerospace Standard

The [DO-178C](https://en.wikipedia.org/wiki/DO-178C) standard, recognized by the FAA in 2013, defines software certification requirements for airborne systems. It specifies five criticality levels, with Level A (catastrophic failure) requiring the most rigorous verification.

The [DO-333 supplement](https://en.wikipedia.org/wiki/DO-178C) specifically addresses formal methods as a complement to traditional testing. Since 2001, [Airbus](https://en.wikipedia.org/wiki/Airbus) has deployed formal verification tools across its avionics development. Model checking alone has found 26 errors in flight control systems that traditional testing missed — errors that could have caused mode confusion in critical flight phases.

### How It Works

Formal verification typically involves three components:

**Specification** — a precise description of what the software should do, expressed in formal logic. This is where [first-order logic](https://plato.stanford.edu/entries/logic-classical/) and its extensions become essential.

**Implementation** — the actual code.

**Proof** — mathematical evidence that the implementation satisfies the specification for all possible executions.

The [Z3 theorem prover](https://github.com/Z3Prover/z3), developed by Leonardo de Moura and Nikolaj Bjørner, is the most widely-used tool for automated verification. It won the 2015 ACM SIGPLAN Programming Languages Software Award and the 2018 ETAPS Test of Time Award. Z3 handles [satisfiability modulo theories](https://en.wikipedia.org/wiki/Satisfiability_modulo_theories) (SMT) — checking whether logical formulas involving arithmetic, arrays, and other data structures have solutions.

### Beyond Aviation

**Automotive**: [ISO 26262](https://en.wikipedia.org/wiki/ISO_26262) mandates formal methods for safety-critical automotive software. Braking systems, steering controls, and autonomous driving components increasingly require verified implementations.

**Finance**: High-frequency trading systems and smart contracts use formal verification to prevent costly errors. A single bug in a trading algorithm can cause millions in losses before humans can intervene.

**Cryptography**: Security-critical code is increasingly verified against formal specifications. The [CompCert](https://en.wikipedia.org/wiki/CompCert) verified C compiler and [seL4](https://en.wikipedia.org/wiki/SEL4) verified microkernel demonstrate that even foundational system software can be formally verified.

**AI Systems**: As AI takes on consequential decisions, formal verification provides accountability. [Anthropic](https://www.anthropic.com/), the company behind Claude, researches [Constitutional AI](https://www.anthropic.com/research/constitutional-ai-harmlessness-from-ai-feedback) — using formal constraints to guide AI behavior. Verification tools can check whether AI systems respect specified constraints across all possible inputs.

### The Natural Language Gap

Traditional formal verification requires specifications in mathematical notation. Engineers must translate requirements documents — written in English — into formal specifications. This translation is error-prone and creates a barrier to adoption.

LOGICAFFEINE addresses this gap. By parsing natural language directly into [first-order logic](https://plato.stanford.edu/entries/logic-classical/), we enable:

- **Requirements traceability** — natural language requirements map directly to formal specifications
- **Stakeholder communication** — non-technical stakeholders can review specifications in English while engineers work with formal versions
- **Rapid prototyping** — explore logical implications of requirements before committing to implementations

### The LOGOS Verification Pipeline

LOGOS, the engine behind LOGICAFFEINE, includes optional Z3 integration for static verification. When enabled, you can:

1. Express properties in natural English
2. Automatically translate to FOL
3. Check satisfiability and validity via Z3
4. Receive results with explanations

This brings formal verification closer to the accessibility threshold needed for broader adoption.

### Getting Started

The [Studio](/studio) provides interactive exploration of logical translations. The [Learn page](/learn) covers the fundamentals of first-order logic needed to understand verification results.

Formal verification has proven its value in the most demanding industries. LOGICAFFEINE works to make that rigor accessible beyond specialists.
"#,
        tags: &["verification", "formal-logic", "engineering", "safety"],
        author: "LOGICAFFEINE Team",
    },
    Article {
        slug: "montague-semantics-nlp",
        title: "Montague Semantics: The Mathematics Behind Natural Language",
        date: "2026-01-24",
        summary: "Richard Montague proved that natural language could be analyzed with the same mathematical rigor as formal logic. His framework powers modern semantic parsing, including LOGICAFFEINE.",
        content: r#"
## "There Is No Important Theoretical Difference"

In 1970, mathematician [Richard Montague](https://en.wikipedia.org/wiki/Richard_Montague) made a bold claim: "There is no important theoretical difference between natural languages and the artificial languages of logicians."

This was radical. Linguists had long treated natural language as fundamentally different from formal systems — messy, ambiguous, context-dependent. Montague argued the opposite: natural language could be given a fully rigorous [model-theoretic semantics](https://en.wikipedia.org/wiki/Semantics_of_logic), just like first-order logic.

He proved it by constructing one.

### The Montague Framework

[Montague semantics](https://plato.stanford.edu/entries/montague-semantics/), documented in three papers published between 1970 and 1973, treats natural language interpretation as mathematical function composition.

The core principle is **compositionality**: the meaning of a complex expression is determined by the meanings of its parts and the way they're syntactically combined. This mirrors how formal logic builds complex formulas from atomic predicates using connectives and quantifiers.

Montague used [typed lambda calculus](https://en.wikipedia.org/wiki/Typed_lambda_calculus) as the "glue" for composition. As linguist Barbara Partee [later wrote](https://plato.stanford.edu/entries/montague-semantics/): "Lambdas changed my life." Lambda expressions let you represent functions that combine word meanings into phrase meanings, phrase meanings into sentence meanings, and ultimately into [first-order logic](https://plato.stanford.edu/entries/logic-classical/) formulas.

### How Composition Works

Consider "Every student reads."

In Montague's framework:
- "student" denotes a predicate: `λx.Student(x)`
- "reads" denotes a predicate: `λx.Reads(x)`
- "every" is a function that takes a predicate and returns a quantified expression: `λP.λQ.∀x(P(x) → Q(x))`

Combining these:
1. "every student" = `λQ.∀x(Student(x) → Q(x))`
2. "every student reads" = `∀x(Student(x) → Reads(x))`

The meaning assembles compositionally, just like building a complex number from simpler operations.

### Intensional Logic

Montague extended [first-order logic](https://plato.stanford.edu/entries/logic-classical/) to handle intensional contexts — cases where substituting equivalent expressions changes meaning.

"John believes the morning star is bright" and "John believes the evening star is bright" can have different truth values even though the morning star *is* the evening star (both are Venus). Montague's [intensional logic](https://en.wikipedia.org/wiki/Intensional_logic) captures this by distinguishing between the extension (the actual referent) and the intension (the concept or sense).

This matters for AI systems that reason about beliefs, knowledge, and possibility — all intensional notions.

### From Theory to NLP

Montague's work influenced the development of [semantic parsing](https://en.wikipedia.org/wiki/Semantic_parsing): computational systems that map natural language to logical forms.

Modern approaches include:
- **Rule-based systems** using explicit Montague-style grammars
- **Neural semantic parsers** trained on (sentence, logical form) pairs
- **Hybrid systems** combining learned representations with compositional structure

Recent research at institutions like [Stanford NLP](https://nlp.stanford.edu/) and [Google Research](https://research.google/) explores how large language models can learn compositional semantic representations — essentially rediscovering Montague's insights through statistical learning.

### LOGICAFFEINE's Approach

LOGICAFFEINE implements a Montague-inspired pipeline:

1. **Lexical lookup** — words are assigned semantic types and lambda expressions
2. **Type-driven parsing** — syntactic analysis follows type-compatibility constraints
3. **Lambda reduction** — complex meanings are computed through function application
4. **FOL output** — final formulas use notation from the [Open Logic Project](https://openlogicproject.org/) conventions

This architecture handles:
- Quantifier scope ambiguity (multiple valid readings)
- Relative clauses ("students who study")
- Coordination ("reads and writes")
- Negation scope ("not every student reads" vs "every student doesn't read")

### The Legacy

Montague died in 1971, at 40, before seeing the full impact of his work. Today, his framework underlies:

- Formal semantics curricula at universities worldwide
- Computational linguistics and NLP research
- Knowledge representation in AI systems
- Semantic web technologies

The [Stanford Encyclopedia of Philosophy](https://plato.stanford.edu/entries/montague-semantics/) maintains a comprehensive entry on Montague semantics, documenting both the original theory and subsequent developments.

### Try It Yourself

The [Studio](/studio) lets you see Montague-style composition in action. Type a sentence and examine the AST to see how meanings combine. The [Learn page](/learn) covers the underlying logical concepts.

Montague proved that natural language has mathematical structure. LOGICAFFEINE makes that structure visible.
"#,
        tags: &["linguistics", "formal-logic", "research", "education"],
        author: "LOGICAFFEINE Team",
    },
];

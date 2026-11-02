//! ★ LOCK — every server surface COMPILES ON THE TIER A SERVER SHIPS AS. ★
//!
//! `largo build` produces an AOT-Rust binary, and **that tier is the only one that runs the static
//! type checker.** The tree-walker and the VM infer loosely; every server lock in this campaign
//! drives those two. `route_aot_lock` was the first to compile on the deploy tier and it found a
//! break immediately — a route answering a record was proven to WORK on the tiers the tests use and
//! to not COMPILE on the tier a server ships as. That is the worst split available: the suite is
//! green and the deploy is broken.
//!
//! At the time this file was written, **seventeen locks named a server construct and not one of
//! them called `compile_to_rust`** — `ssr_lock`, `ssr_child_component_lock`, `server_surface_lock`,
//! `derived_json_lock`, `from_json_lock`, `typed_fetch_lock`, `csrf_lock`, `serve_fetches_lock`,
//! `progressive_form_lock`, `wire_contract_lock` and the rest. Each proved its own subject
//! honestly, on tiers where the question of whether it builds is never asked.
//!
//! ★ AND `Ok` PROVES NOTHING. `compile_to_rust` returns `Ok` for a program it has REFUSED — the
//! refusal is a `compile_error!` embedded in the returned Rust. So an `is_ok()` ratchet across
//! these subjects would be green while proving neither that they build nor that they load, which is
//! worse than no ratchet because it looks like coverage. Every assertion here reads the emitted
//! Rust for `compile_error!`.
//!
//! Two halves:
//!
//!   1. **A corpus** — one representative program per server-surface subject, each required to
//!      compile clean on AOT-Rust. This is the coverage.
//!   2. **A catalogue** — every `*_lock.rs` that names a server construct must either assert on the
//!      deploy tier ITSELF or be listed here as covered by the corpus. A new server lock that does
//!      neither turns this red. That is the half that stops the gap reopening: the seventeen were
//!      not an oversight anybody made, they were a question nobody was asked.

use logicaffeine_compile::compile::compile_to_rust;

/// Compile on the DEPLOY tier and read the emitted Rust for refusals.
fn builds(subject: &str, src: &str) -> String {
    let rust = compile_to_rust(src)
        .unwrap_or_else(|e| panic!("`{subject}` must compile to AOT-Rust: {e:?}\n{src}"));
    let refusals: Vec<&str> = rust
        .lines()
        .filter(|l| l.contains("compile_error!"))
        .collect();
    assert!(
        refusals.is_empty(),
        "`{subject}` emits {} `compile_error!` — the binary would not build:\n{}\n{src}",
        refusals.len(),
        refusals.join("\n")
    );
    rust
}

const TODO: &str = "## A Todo has:\n    an id, which is Int.\n    a text, which is Text.\n\n";

/// One representative program per server-surface subject, named for the lock it stands in for.
fn corpus() -> Vec<(&'static str, String)> {
    vec![
        (
            "ssr_lock",
            "## Component Greeting\nState:\n    Let name be \"ada\".\nView:\n    <p>Hi {name}</p>\n\n\
             ## Main\nShow html of Greeting.\n"
                .to_string(),
        ),
        (
            "ssr_child_component_lock",
            "## Component Row (label: Text)\nView:\n    <li>{label}</li>\n\n\
             ## Component List\nState:\n    Let items be [\"a\", \"b\"].\n\
             View:\n    <ul>{for each i in items:}<Row label={i}/>{end}</ul>\n\n\
             ## Main\nShow html of List.\n"
                .to_string(),
        ),
        (
            "server_surface_lock",
            "## Route GET /report -> View:\n    <h1>Report</h1>\n\n\
             ## Main\nShow route(\"GET\", \"/report\").\n"
                .to_string(),
        ),
        (
            "route_aot_lock",
            format!("{TODO}## Route GET /api/todos -> Seq of Todo:\n\
                \x20   Return [a new Todo with id 1 and text \"a\"].\n\n\
                 ## Main\nShow route(\"GET\", \"/api/todos\").\n"),
        ),
        (
            "derived_json_lock",
            format!("{TODO}## Main\nShow toJson_Todo(a new Todo with id 1 and text \"a\").\n"),
        ),
        (
            "from_json_lock",
            format!("{TODO}## Main\n\
                 Inspect fromJson_Todo(toJson_Todo(a new Todo with id 2 and text \"b\")):\n\
                \x20   When Ok (t):\n\
                \x20       Show t's text.\n\
                \x20   When Err (e):\n\
                \x20       Show e.\n"),
        ),
        (
            "csrf_lock",
            "## Main\nLet mine be csrfToken(\"secret\", \"ada\").\nShow csrfValid(\"secret\", \"ada\", mine).\n"
                .to_string(),
        ),
        (
            "wire_contract_lock",
            format!("{TODO}## Route GET /api/todos -> Seq of Todo:\n\
                \x20   Return a new Seq of Todo.\n\n\
                 ## Main\nShow routeStatus(\"GET\", \"/api/todos\").\n"),
        ),
        (
            "state_field_type_lock",
            "## Component App\nState:\n    Let n be 0.\n    Let doubled always be n * 2.\n\
             View:\n    <p>{doubled}</p>\n\n## Main\nShow html of App.\n"
                .to_string(),
        ),
        (
            "progressive_form_lock",
            "## Route POST /subscribe -> Text:\n    Return \"thanks\".\n\n\
             ## Main\nShow route(\"POST\", \"/subscribe\").\n"
                .to_string(),
        ),
        (
            // Every kind of attribute the server walk folds at compile time or decides at run
            // time — literal, interpolated, boolean, bound, a merged class and a scope class.
            "ssr_attribute_order_lock",
            "## Component App\nState:\n    Let t be \"tip\".\n    Let u be \"/next\".\n    Let k be \"row\".\n\
            \x20   Let on be true.\n    Let off be false.\n\
             View:\n    <div>\n\
            \x20     <img alt={t} class=\"card\" title={t}>\n\
            \x20     <a href={u} class=\"row\" class:done={on} data-x=\"1\" aria-label={t}>x</a>\n\
            \x20     <li title={t} class=\"{k} item\" class:hot={off} data-k={k}>x</li>\n\
            \x20     <button type=\"button\" disabled={on} class=\"b\" aria-label={t}>go</button>\n\
            \x20     <input type=\"text\" bind={t} class=\"f\" aria-label={t} name=\"q\">\n\
            \x20     <input type=\"checkbox\" bind={on} width=\"3\" aria-label=\"c\">\n\
            \x20   </div>\n\
             Style:\n    p { color: red; }\n\n\
             ## Main\nShow html of App.\n"
                .to_string(),
        ),
    ]
}

#[test]
fn every_server_surface_subject_compiles_on_the_deploy_tier() {
    for (subject, src) in corpus() {
        builds(subject, &src);
    }
}

#[test]
fn the_corpus_actually_exercises_the_server_lowering() {
    // ★ A CORPUS THAT COMPILED NOTHING WOULD PASS. Each program must reach the code this file
    // exists to cover — the router, the derived codec, or the server-side view walk — or the
    // assertion above is green for the wrong reason.
    for (subject, src) in corpus() {
        let rust = builds(subject, &src);
        assert!(
            rust.contains("__route_")
                || rust.contains("__html_")
                || rust.contains("toJson_")
                || rust.contains("fromJson_")
                || rust.contains("csrf"),
            "`{subject}` compiles but reaches none of the server lowering — it is not covering \
             what it claims to:\n{src}"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// ★ THE CATALOGUE — a new server lock cannot skip the deploy tier without saying so.
// ---------------------------------------------------------------------------------------------

/// Locks that name a server construct but assert nothing on the deploy tier THEMSELVES, each with
/// the reason that is acceptable.
///
/// `CoveredByCorpus` means this file compiles its shape above. `BrowserSubject` means the lock's
/// subject genuinely only exists in a browser, so there is nothing to build server-side.
const EXEMPT: &[(&str, Why)] = &[
    ("ssr_lock.rs", Why::CoveredByCorpus),
    ("ssr_child_component_lock.rs", Why::CoveredByCorpus),
    ("server_surface_lock.rs", Why::CoveredByCorpus),
    ("derived_json_lock.rs", Why::CoveredByCorpus),
    ("from_json_lock.rs", Why::CoveredByCorpus),
    ("csrf_lock.rs", Why::CoveredByCorpus),
    ("wire_contract_lock.rs", Why::CoveredByCorpus),
    ("state_field_type_lock.rs", Why::CoveredByCorpus),
    ("progressive_form_lock.rs", Why::CoveredByCorpus),
    ("ssr_attribute_order_lock.rs", Why::CoveredByCorpus),
    // A page's own side of the wire, and the browser tier's rendering rules. These run in a
    // document; there is no server binary for them to be built into.
    //
    // `typed_fetch_lock.rs` and `fullstack_todo_lock.rs` were here and are NOT any more: with the
    // heuristic reading code instead of prose (see `server_surface_locks`), neither names a server
    // construct outside a comment, so they are not server-surface locks at all and an exemption for
    // them would be a stale entry — which `the_catalogue_names_only_locks_that_exist` fails on, by
    // design. An exemption is a statement that something WOULD be caught; it has to keep being true.
    ("serve_fetches_lock.rs", Why::BrowserSubject),
    ("headless_runtime_lock.rs", Why::BrowserSubject),
    ("loop_else_lock.rs", Why::BrowserSubject),
    ("void_element_serialize_lock.rs", Why::BrowserSubject),
    ("xss_corpus_lock.rs", Why::BrowserSubject),
    ("ssr_bench_lock.rs", Why::BrowserSubject),
    // The page-coverage model: which `## Test` step drives which action, handler, trigger and
    // `bind` of a page. Its one `## Route` answers a page's boot fetch.
    ("page_coverage_lock.rs", Why::BrowserSubject),
    // Not web locks at all. Each compiles a one-line `Serve on port` program as a CARRIER for the
    // thing it really measures (an op-coverage census), so there is nothing about the server tier
    // for it to assert. Prose-only mentions no longer reach this list at all — `server_surface_locks`
    // reads code, not comments.
    ("analog_op_coverage_lock.rs", Why::NamesOneInPassing),
    ("tenstorrent_op_coverage_lock.rs", Why::NamesOneInPassing),
    ("soft_core_op_support_lock.rs", Why::NamesOneInPassing),
    ("wprog_op_reachability_lock.rs", Why::NamesOneInPassing),
    // A server program as the INPUT to a decision about something else: which tests `largo test`
    // runs beside a `Serve` entry (`closure_lock.rs`), what a project is (`project_shape_lock.rs`,
    // and `shape_agreement_lock.rs`, whose `Serve` sentence sits inside a `Show` string to prove
    // it misroutes nothing), what `largo expand` prints (`largo_expand_lock.rs`), how fast the
    // interpreter's own `Serve` loop answers (`http_serve_bench_lock.rs`), and whether `largo check`
    // passes a backend the deploy tier refuses (`check_deploy_refusal_lock.rs`).
    ("closure_lock.rs", Why::NamesOneInPassing),
    ("check_deploy_refusal_lock.rs", Why::NamesOneInPassing),
    ("project_shape_lock.rs", Why::NamesOneInPassing),
    ("shape_agreement_lock.rs", Why::NamesOneInPassing),
    ("largo_expand_lock.rs", Why::NamesOneInPassing),
    ("http_serve_bench_lock.rs", Why::NamesOneInPassing),
    // A `## Route` as grammar: the AST a header parses into (`component_def_lock.rs`), the lexer's
    // header reading (`lgx_headers_lock.rs`), its optional trailing colon
    // (`header_trailing_colon_lock.rs`), and where a name in a route template resolves
    // (`view_resolution_lock.rs`).
    ("component_def_lock.rs", Why::NamesOneInPassing),
    ("lgx_headers_lock.rs", Why::NamesOneInPassing),
    ("header_trailing_colon_lock.rs", Why::NamesOneInPassing),
    ("view_resolution_lock.rs", Why::NamesOneInPassing),
    // A route or an `html of` written to be refused, so the lock can read where the refusal points.
    ("type_error_location_lock.rs", Why::NamesOneInPassing),
    ("wave3_refusal_location_lock.rs", Why::NamesOneInPassing),
    // A `## Route` is bait for something else entirely: a typed-fetch failure
    // (`return_control_flow_lock.rs`, proving `Return.` leaves the fetch-failure handler), a
    // prelude-splice trigger (`ui_bridge_seam_lock.rs`, proving a parse error's span survives the
    // splice), and a refusal placed on a known line (`diagnostic_location_lock.rs`, proving WHERE a
    // route's refusal points on every surface). None asserts anything about the deploy tier.
    ("return_control_flow_lock.rs", Why::NamesOneInPassing),
    ("ui_bridge_seam_lock.rs", Why::NamesOneInPassing),
    ("diagnostic_location_lock.rs", Why::NamesOneInPassing),
    // A `## Route` as one row of a backend's parity corpus: the direct C emitter's, the bytecode C walker's and
    // the RISC-V tier's in-process `route(…)` dispatch held to the VM's answer, with no server binary in it.
    ("c_ast_vm_parity_lock.rs", Why::NamesOneInPassing),
    ("c_bytecode_lock.rs", Why::NamesOneInPassing),
    ("riscv_aot_lock.rs", Why::NamesOneInPassing),
    // The compiled tier's server is the subject, built and run through `largo build` / `largo run` as a user
    // builds one, which this ratchet's source scan cannot see.
    ("served_address_lock.rs", Why::BuildsThroughTheCli),
    // A served program's release build is the subject: the lock writes the project the compiler generates and
    // builds it with cargo, as `largo build --release` does, then serves from the binary.
    ("failure_boundary_lock.rs", Why::BuildsItsGeneratedProject),
];

#[derive(PartialEq, Debug, Clone, Copy)]
enum Why {
    CoveredByCorpus,
    BrowserSubject,
    /// The file names a server construct in prose or in a list of examples, and compiles no LOGOS
    /// program at all — a source-scanning ratchet, or a lock about something else entirely.
    ///
    /// ★ SPLIT OUT FROM `BrowserSubject`, which two entries were already borrowing while their own
    /// comment said "not web locks at all". An exemption's whole job is to say WHY, so one wearing
    /// a reason that is not its reason is the same defect as a stale comment — and this campaign
    /// has spent real time on those. `BrowserSubject` now means only what it says: the subject runs
    /// in a document, so there is no server binary for it to be built into.
    NamesOneInPassing,
    /// The lock builds the server on the deploy tier itself, through the `largo` binary (`largo build`,
    /// `largo run`), and asserts on the program that build produces.
    BuildsThroughTheCli,
    /// The lock writes the Rust project the compiler generates (`write_cargo_project`) and builds it with cargo,
    /// the deploy tier's own build, which fails on any `compile_error!`, and asserts on the binary it produces.
    BuildsItsGeneratedProject,
}

/// Every `*_lock.rs` in this directory whose source names a server construct.
fn server_surface_locks() -> Vec<(String, bool)> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut out = Vec::new();
    for e in std::fs::read_dir(&dir).expect("tests/ exists").flatten() {
        let p = e.path();
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        if !name.ends_with("_lock.rs") || name == "server_surface_deploys_lock.rs" {
            continue;
        }
        let src = std::fs::read_to_string(&p).unwrap_or_default();
        // Match CODE, not prose. A `///` paragraph that cites `## Route` as a precedent compiles
        // no server program, and treating the mention as a subject made this ratchet demand a
        // deploy-tier assertion from a lock about something else entirely — `wasm_aot_lock.rs`
        // went red for one word in a doc comment enumerating compiler-written references. Dropping
        // whole comment LINES (never a `//` inside a string, which is how a URL is spelled) keeps
        // the heuristic pointed at programs, so the next such paragraph costs nobody an afternoon.
        let code: String = src
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        let names_server = [
            "## Route",
            "Serve on port",
            "html of",
            "toJson_",
            "fromJson_",
        ]
        .iter()
        .any(|k| code.contains(k));
        if names_server {
            out.push((
                name,
                src.contains("compile_to_rust") || src.contains("compile_project"),
            ));
        }
    }
    out.sort();
    out
}

#[test]
fn every_server_lock_either_builds_on_the_deploy_tier_or_says_why_not() {
    let known: Vec<&str> = EXEMPT.iter().map(|(n, _)| *n).collect();
    let unaccounted: Vec<&String> = server_surface_locks()
        .iter()
        .filter(|(_, asserts_itself)| !asserts_itself)
        .map(|(n, _)| n)
        .filter(|n| !known.contains(&n.as_str()))
        .cloned()
        .collect::<Vec<String>>()
        .leak()
        .iter()
        .collect();

    assert!(
        unaccounted.is_empty(),
        "these locks name a server construct, assert nothing on the deploy tier, and are not \
         accounted for: {unaccounted:?}\n\
         AOT-Rust is the ONLY tier that runs the static type checker, so a subject proved on the \
         tree-walker and the VM alone can be green here and unbuildable in production — that is \
         what `route_aot_lock` found the first time anybody looked. Either call `compile_to_rust` \
         (`compile_project` for a project) in the lock itself (and scan the result for \
         `compile_error!` — `Ok` proves nothing), or \
         add a representative program to this file's `corpus()` and list it as `CoveredByCorpus`."
    );
}

#[test]
fn the_catalogue_names_only_locks_that_exist() {
    let present: Vec<String> = server_surface_locks().into_iter().map(|(n, _)| n).collect();
    let stale: Vec<&&str> = EXEMPT
        .iter()
        .map(|(n, _)| n)
        .filter(|n| !present.contains(&n.to_string()))
        .collect();
    assert!(
        stale.is_empty(),
        "the catalogue exempts locks that no longer exist: {stale:?}"
    );
}

#[test]
fn every_corpus_entry_stands_for_a_lock_that_is_exempt_for_that_reason() {
    // The corpus and the catalogue have to agree, or a subject can be listed as covered while
    // nothing compiles it — the same "written down and not kept" failure the emitter catalogue
    // guards against.
    let corpus_subjects: Vec<&str> = corpus().iter().map(|(s, _)| *s).collect();
    for (name, why) in EXEMPT {
        if *why == Why::CoveredByCorpus {
            let stem = name.trim_end_matches(".rs");
            assert!(
                corpus_subjects.contains(&stem),
                "`{name}` is listed as covered by the corpus, but the corpus has no program for it"
            );
        }
    }
}

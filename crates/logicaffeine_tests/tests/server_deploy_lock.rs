//! ★ LOCK — the server surface compiles on the tier a server SHIPS as. ★
//!
//! `route_aot_lock` was the first lock in this campaign to compile anything through
//! `compile_to_rust`, and it found a break the moment it looked: a `## Route` answering a record
//! became a handler declared `-> Text` whose body returned the record, so the build failed naming
//! neither the route nor its file. It survived because the tree-walker and the VM infer loosely
//! and never run the static type checker — **AOT-Rust is the only tier that does** — and every
//! other server lock drives those two.
//!
//! A grep confirms the exposure: `compile_to_rust` appears ZERO times in `server_surface_lock`,
//! `ssr_lock`, `typed_fetch_lock`, `derived_json_lock`, `from_json_lock`, `session_lock` and
//! `csrf_lock`. The entire server surface is proven on tiers a test uses and unproven on the tier
//! a server is deployed as — green suite, broken deploy, which is the worst split there is.
//!
//! ★ `Ok` PROVES NOTHING HERE, AND THAT IS THE TRAP THIS FILE IS BUILT AROUND. `compile_to_rust`
//! returns `Ok` for a program it has REFUSED — the refusal is a `compile_error!` embedded in the
//! Rust it hands back. An `is_ok()` ratchet across these programs would be green while proving
//! neither that they build nor that they run, which is worse than no ratchet because it looks
//! like coverage. Every assertion below reads the emitted program.
//!
//! ★ AND IT IS A PARTITION, NOT A WISHLIST. Some of this surface legitimately does NOT reach
//! AOT-Rust yet — the component model has no lowering there, so a page, a `Mount` and a typed
//! fetch are refused by design. Those are asserted as REFUSALS, by name, so the boundary is a
//! written-down fact instead of a surprise a deploy discovers. A lock that only listed what works
//! would let the refusal set grow silently.

use logicaffeine_compile::compile::compile_to_rust;

/// The refusal a program carries, if the deploy tier declined it.
///
/// Both halves: the call may fail outright, or succeed while embedding a `compile_error!` that
/// only rustc would ever see.
fn deploy_refusal(src: &str) -> Option<String> {
    match compile_to_rust(src) {
        Err(e) => Some(format!("{e:?}")),
        Ok(rust) => rust
            .lines()
            .find(|l| l.contains("compile_error!"))
            .map(|l| l.trim().to_string()),
    }
}

const SHAPES: &str = "## A Todo has:\n\
    \x20   an id, which is Int.\n\
    \x20   a text, which is Text.\n\n";

/// Every server construct that a deployed binary has to carry.
fn must_deploy() -> Vec<(&'static str, String)> {
    vec![
        ("a route answering Text", format!("{SHAPES}## Route GET /hello -> Text:\n    Return \"hi\".\n\n## Main\nServe on port 8080.\n")),
        ("a route answering a record", format!("{SHAPES}## Route GET /todo -> Todo:\n    Return a new Todo with id 1 and text \"x\".\n\n## Main\nServe on port 8080.\n")),
        ("a route answering a list of records", format!("{SHAPES}## Route GET /todos -> Seq of Todo:\n    Return a new Seq of Todo.\n\n## Main\nServe on port 8080.\n")),
        ("a route with a typed path parameter", format!("{SHAPES}## Route GET /todo/:id (id: Int) -> Int:\n    Return id.\n\n## Main\nServe on port 8080.\n")),
        ("a redirect", format!("{SHAPES}## Redirect /old -> /new.\n\n## Route GET /new -> Text:\n    Return \"here\".\n\n## Main\nServe on port 8080.\n")),
        ("a hand-written handler", format!("{SHAPES}## To handle (request: Request) -> Response:\n    Return plain(\"ok\").\n\n## Main\nServe on port 8080 with handle.\n")),
    ]
}

/// Constructs that DO NOT reach AOT-Rust yet, asserted so the boundary cannot move in silence.
fn must_refuse() -> Vec<(&'static str, String)> {
    vec![
        (
            "a component",
            format!("{SHAPES}## Component App\nState:\n    Let n be 0.\nView:\n    <p>{{n}}</p>\n\n## Main\nMount App.\n"),
        ),
        (
            "a typed fetch",
            format!(
                "{SHAPES}## Component App\nState:\n    Let n be 0.\nActions:\n    When the page loads:\n        \
                 Get \"/todos\" as Seq of Todo, then loaded.\n    To loaded (answer: Result of Seq of Todo and HttpError):\n        \
                 Set n to 1.\nView:\n    <p>{{n}}</p>\n\n## Main\nMount App.\n"
            ),
        ),
    ]
}

/// ★ THE LOAD-BEARING TEST — every server construct reaches the tier a server ships as.
#[test]
fn the_server_surface_compiles_on_the_deploy_tier() {
    let mut broken = Vec::new();
    for (label, src) in must_deploy() {
        if let Some(why) = deploy_refusal(&src) {
            broken.push(format!("  {label}: {why}"));
        }
    }
    assert!(
        broken.is_empty(),
        "these server constructs run on the tiers the tests drive and DO NOT COMPILE on the tier a \
         server is deployed as. A suite that is green on all of them still ships a binary that \
         will not build:\n{}",
        broken.join("\n")
    );
}

/// The other half of the partition: what the deploy tier declines, it declines BY NAME.
#[test]
fn what_the_deploy_tier_cannot_carry_it_refuses_by_name() {
    for (label, src) in must_refuse() {
        let why = deploy_refusal(&src).unwrap_or_else(|| {
            panic!(
                "{label} compiled cleanly for AOT-Rust, which has no component-model lowering — so \
                 the artifact builds and can only ever render nothing. A missing capability has to \
                 REFUSE; silently succeeding is the failure mode this whole surface is built to \
                 avoid."
            )
        });
        assert!(
            why.contains("LGX") || why.to_lowercase().contains("component"),
            "{label}'s refusal must name the construct, so a reader knows which line to change \
             rather than hunting a rustc error in generated code: {why}"
        );
    }
}

/// ★ COMPILING IS NOT ANSWERING. A route that builds but replies with the wrong bytes is a
/// deploy that "works" until somebody reads the response — so the new scalar path is checked for
/// WHAT IT SENDS, on the tiers that can run it, not merely for reaching the deploy tier.
#[test]
fn a_route_answering_a_number_sends_json() {
    use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
    let src = format!(
        "{SHAPES}## Route GET /count -> Int:\n    Return 42.\n\n\
         ## Main\n\
         Show route(\"GET\", \"/count\").\n\
         Show routeContentType(\"GET\", \"/count\").\n"
    );
    let tw = tw_outcome(&src);
    assert_eq!(tw.error, None, "the reference tier must run it:\n{src}");
    let lines: Vec<&str> = tw.output.trim().lines().collect();
    assert_eq!(
        lines.first().map(|s| s.trim()),
        Some("42"),
        "a route answering `Int` must send the number itself — `42` is valid JSON with no quoting, \
         which is the whole reason a bare scalar needs no encoder:\n{src}"
    );
    assert_eq!(
        lines.get(1).map(|s| s.trim()),
        Some("application/json"),
        "and it must be TYPED as JSON. A number sent as `text/plain` is a number a client has to \
         guess about — the content type is where the author already said what they are sending."
    );
    assert_eq!(
        vm_outcome(&src).output.trim(),
        tw.output.trim(),
        "and the bytecode tier must agree, byte for byte"
    );
}

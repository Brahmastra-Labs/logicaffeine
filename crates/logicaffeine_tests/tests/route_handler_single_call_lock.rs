//! ★ LOCK — `__lgx_route_handler` runs a route, and each guard before it, EXACTLY ONCE. ★
//!
//! The router answers four questions about a request — the body, the status, the content type and
//! the headers — and for a long time three of those were per-route CONSTANTS that never called the
//! route's function. Only the body walk did. That is what made "the handler runs once per request"
//! true, and it was true by accident of the shape rather than by anything checking it.
//!
//! ★ A `Response` ROUTE BREAKS THAT SHAPE, because its status and its content type are FIELDS OF
//! THE ANSWER rather than constants — the value has to be produced before either can be read. So
//! the obvious way to lift the ceiling, wiring each walk through to the route, makes the handler
//! call it once per walk: **a login would execute four times for one request.** Harmless for
//! setting a cookie, and a bug the moment it writes a session row, sends a mail, or charges a card.
//!
//! The shape that keeps the property is a fifth walk — `__lgx_route_response_for` — built in the
//! same loop from the same conditions as the other four, answering a whole `Response`, and it is
//! the ONLY one `__lgx_route_handler` calls. The four remain what `route(…)`, `routeStatus(…)`,
//! `routeContentType(…)` and `routeHeader(…)` are built on, where asking again to read one more
//! field is the point rather than a cost.
//!
//! ★ AND `## Before each request` GOT THE SAME FIX FOR FREE. A guard used to run TWICE per request
//! — once in the body walk and once in the status walk — which nothing measured and no test named.
//! It now runs once, and that is asserted here rather than assumed.
//!
//! ★ THE COUNTER IS PROVEN TO BE ABLE TO COUNT PAST ONE (`the_counter_can_report_more_than_one`).
//! An "exactly once" assertion whose instrument can only ever report one is the purest form of a
//! check that cannot fail, and this campaign has now found six of those.

use logicaffeine_compile::compile::{compile_to_rust, tw_outcome};

/// A `Request` literal for `method` and `path`, with the empty headers and query a bare call has.
fn request(method: &str, path: &str) -> String {
    format!(
        "a new Request with method \"{method}\" and path \"{path}\" and body \"\" \
         and headers (a new Map of Text to Text) and query (a new Map of Text to Text)"
    )
}

/// Runs `src` on the reference tier and counts how many lines are exactly `mark`.
///
/// The route bodies below `Show "<mark>"`, so one line per execution. Counting OUTPUT rather than
/// instrumenting the compiler is deliberate: it measures what actually ran.
fn times_run(subject: &str, src: &str, mark: &str) -> usize {
    let out = tw_outcome(src);
    assert_eq!(out.error, None, "`{subject}` must run:\n{src}");
    out.output.lines().filter(|l| l.trim() == mark).count()
}

#[test]
fn a_response_route_runs_once_per_handled_request() {
    // ★ THE HEADLINE. This is the route shape that would have run four times.
    let src = format!(
        "## Route POST /login -> Response:\n\
        \x20   Show \"ran\".\n\
        \x20   Return withCookie(plain(\"ok\"), \"sid\", \"ada\").\n\n\
         ## Main\nLet r be __lgx_route_handler({}).\nShow r's body.\n",
        request("POST", "/login")
    );
    assert_eq!(
        times_run("a Response route", &src, "ran"),
        1,
        "a Response route must run ONCE for one request — four times means the handler is \
         composing the body, status, content-type and header walks instead of the serving walk"
    );
    // And the answer still arrives intact, so this is not "once" bought by not running at all.
    let out = tw_outcome(&src);
    assert!(
        out.output.lines().any(|l| l.trim() == "ok"),
        "got {:?}",
        out.output
    );
}

#[test]
fn a_value_route_runs_once_per_handled_request() {
    // The floor: the shape that was already correct stays correct.
    let src = format!(
        "## Route GET /hi -> Text:\n\x20   Show \"ran\".\n\x20   Return \"hi\".\n\n\
         ## Main\nLet r be __lgx_route_handler({}).\nShow r's body.\n",
        request("GET", "/hi")
    );
    assert_eq!(times_run("a Text route", &src, "ran"), 1);
}

#[test]
fn a_guard_runs_once_per_handled_request() {
    // ★ IT USED TO RUN TWICE — once in the body walk, once in the status walk. A guard that counts
    // failed attempts, writes an audit line or takes a rate-limit token was doing it twice for
    // every visit, and nothing said so.
    let src = format!(
        "## Before each request (r: Request) -> Option of Response:\n\
        \x20   Show \"guard\".\n\x20   Return none.\n\n\
         ## Route GET /hi -> Text:\n\x20   Return \"hi\".\n\n\
         ## Main\nLet r be __lgx_route_handler({}).\nShow r's body.\n",
        request("GET", "/hi")
    );
    assert_eq!(
        times_run("a guard that declines", &src, "guard"),
        1,
        "`## Before each request` must run once per request"
    );
}

#[test]
fn a_guard_that_answers_runs_once_and_its_answer_is_served_whole() {
    let src = format!(
        "## Before each request (r: Request) -> Option of Response:\n\
        \x20   Show \"guard\".\n\x20   Return some(unauthorized()).\n\n\
         ## Route GET /secret -> Text:\n\x20   Show \"route\".\n\x20   Return \"secret\".\n\n\
         ## Main\nLet r be __lgx_route_handler({}).\nShow r's status.\n",
        request("GET", "/secret")
    );
    assert_eq!(times_run("a guard that answers", &src, "guard"), 1);
    assert_eq!(
        times_run("the route behind a guard that answers", &src, "route"),
        0,
        "a guard that answered must stop the route from running at all"
    );
    let out = tw_outcome(&src);
    assert!(
        out.output.lines().any(|l| l.trim() == "401"),
        "the guard's OWN status must be served, not a 200 — got {:?}",
        out.output
    );
}

#[test]
fn the_counter_can_report_more_than_one() {
    // ★ ANTI-VACUITY. Two requests must show two runs. Without this, every assertion above would
    // pass just as happily if `Show` inside a route printed nothing, or if the handler never ran
    // the route at all.
    let src = format!(
        "## Route GET /hi -> Text:\n\x20   Show \"ran\".\n\x20   Return \"hi\".\n\n\
         ## Main\nLet a be __lgx_route_handler({}).\nLet b be __lgx_route_handler({}).\n\
         Show a's body.\n",
        request("GET", "/hi"),
        request("GET", "/hi")
    );
    assert_eq!(
        times_run("two requests", &src, "ran"),
        2,
        "the instrument must be able to count past one, or `exactly once` proves nothing"
    );
}

#[test]
fn the_conveniences_still_answer_every_question_about_a_response_route() {
    // The other half of the design: the four question-walks keep working for a `Response` route,
    // which is what makes the status, the content type and a `Set-Cookie` assertable from a test
    // without starting a server. Calling the route again is the point there.
    let src = "## Route POST /login -> Response:\n\
              \x20   Return withCookie(plain(\"ok\"), \"sid\", \"ada\").\n\n\
               ## Main\n\
               Show route(\"POST\", \"/login\").\n\
               Show routeStatus(\"POST\", \"/login\").\n\
               Show routeContentType(\"POST\", \"/login\").\n\
               Show routeHeader(\"POST\", \"/login\", \"Set-Cookie\").\n";
    let out = tw_outcome(src);
    assert_eq!(out.error, None, "{src}");
    let lines: Vec<&str> = out.output.lines().map(|l| l.trim()).collect();
    assert_eq!(lines[0], "ok");
    assert_eq!(lines[1], "200");
    assert_eq!(lines[2], "text/plain");
    assert!(lines[3].contains("sid=ada"), "got {:?}", lines);
}

#[test]
fn the_serving_walk_compiles_on_the_tier_a_server_ships_as() {
    // ★ AOT-RUST IS THE ONLY TIER THAT RUNS THE STATIC TYPE CHECKER, and the serving walk is a
    // NEW synthesized function with a declared `-> Response` return type whose arms have to agree
    // about that — a 404, a 405, a moved page, a 400 for a bad segment, a guard's answer and a
    // route's own value all reaching one type. The tree-walker infers loosely and would not
    // notice a disagreement; the deploy tier is where a mismatch shows up, which is exactly the
    // split `route_aot_lock` found the first time anybody compiled a route on it.
    //
    // `Ok` proves nothing — a refused program is returned as `Ok` with a `compile_error!` inside.
    let src = "## A Todo has:\n\x20   an id, which is Int.\n\x20   a text, which is Text.\n\n\
               ## Before each request (r: Request) -> Option of Response:\n\
              \x20   Return none.\n\n\
               ## Route GET /hi -> Text:\n\x20   Return \"hi\".\n\n\
               ## Route GET /todos/:id -> Todo:\n\
              \x20   Return a new Todo with id id and text \"a\".\n\n\
               ## Route POST /login -> Response:\n\
              \x20   Return withCookie(plain(\"ok\"), \"sid\", \"ada\").\n\n\
               ## Redirect /old -> /hi.\n\n\
               ## Main\nShow route(\"GET\", \"/hi\").\n";
    let rust = compile_to_rust(src).unwrap_or_else(|e| panic!("must compile: {e:?}\n{src}"));
    let refusals: Vec<&str> = rust
        .lines()
        .filter(|l| l.contains("compile_error!"))
        .collect();
    assert!(
        refusals.is_empty(),
        "would not build:\n{}\n{src}",
        refusals.join("\n")
    );
    assert!(
        rust.contains("__lgx_route_response_for"),
        "the serving walk must actually be emitted, or this test compiles a program that does not \
         contain the thing it is about"
    );
}

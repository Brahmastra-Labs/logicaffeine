//! ★ LOCK — the server surface is HONEST about what it cannot do yet. ★
//!
//! `## Route`, `## Before each request`, `## Redirect` and `## Error page` are the four headers
//! that make a LOGOS program a web SERVER. None of them runs yet. That is fine; what is not fine
//! is a declaration that is accepted and then quietly does nothing, because the author has been
//! told their intent was understood.
//!
//! Three of the four refused loudly. `## Error page 404` did not: it parses into a component and
//! every component kind was registered with `Ok(Continue)`, so a page declaring a 404 view got
//! one silently ignored. It is the same failure the event vocabulary already refuses by name —
//! "a sentence that sits in a page doing nothing is the worst outcome for whoever wrote it" —
//! and the same shape as `bind` registering no listener and the `on-click` closure that could
//! never be called: a thing that looks wired up and is not.
//!
//! When these land (Phase 6), each test here flips from "refuses by name" to what it actually
//! does. The list only ever shrinks.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

/// Every server header the runtime cannot yet honour, with a program that declares one and
/// nothing else.
///
/// The entries here are deliberately the forms that do NOT lower — a route answering `Seq of
/// Int`, one answering a bare `Int`, and an error page whose template uses a loop the
/// server-side renderer does not cover yet. A route that answers `Text` with no params RUNS
/// (see the tests below), so this list is the remaining frontier rather than the whole surface,
/// and it shrinks as each form lands.
const SERVER_SURFACE: &[(&str, &str)] = &[
    (
        "a route",
        "# Server\n## Route GET /api/todos -> Seq of Int:\n    Return [1, 2].\n\n## Main\nShow \"ok\".\n",
    ),
    (
        // ★ MOVED, for the reason the fixture below already warns about. This stood on
        // `-> Int`, and a route answering a bare number now LOWERS — `42` is valid JSON with
        // no quoting, so a count endpoint or an id lookup needs no record to wrap it in. The
        // fixture was testing a gap rather than a rule, and the gap closed.
        //
        // It now stands on the PARAMETER arm instead of the return arm: the path matcher binds
        // `Text` and `Int` and nothing else, so a `Bool` segment is refused. That is a
        // different refusal in the same function, so the label stays honest — and if it ever
        // closes too, this fixture moves again rather than quietly asserting nothing.
        "a route with a path param the matcher cannot bind",
        "# Server\n## Route GET /api/todos/:flag (flag: Bool) -> Text:\n    Return \"x\".\n\n## Main\nShow \"ok\".\n",
    ),
    (
        // ★ THIS FIXTURE HAS NOW MOVED THREE TIMES, AND THE LAST MOVE IS THE INSTRUCTIVE ONE.
        //
        // It stood on `{for each}`, then on `bind`, and the note here argued `bind` was durable
        // "because it is not a missing feature: it is BROWSER BEHAVIOUR — a two-way binding needs
        // an element to listen to, and a served string has none — so there is nothing here to
        // implement later."
        //
        // That was wrong, and wrong in the way this campaign keeps finding: it conflated the two
        // halves of a binding. The WRITE half is browser behaviour and always was. The READ half —
        // the field's CURRENT VALUE, rendered as the attribute the element reads it from — is
        // exactly what a server-rendered form is FOR: a reader who never loaded the app sees what
        // is in the fields, and a browser that hydrates finds the values it would have written.
        // `bind` renders now (`ssr_subset_lock::a_bound_field_renders_its_current_value`).
        //
        // ★ `{raw}` IS THE DURABLE SUBJECT, and for a reason the old note only thought it had: it
        // is outside the subset by DECISION rather than by absence. Interpolating unescaped markup
        // is a judgement about trust — who is allowed to inject HTML into this page — not a
        // rendering the walk has yet to learn. There is genuinely nothing here to implement later.
        "an error page the server-side renderer cannot draw",
        "# Server\n## Error page 404\nState:\n    Let h be \"<b>hi</b>\".\nView:\n    \
         <div>{raw h}</div>\n\n## Main\nShow \"ok\".\n",
    ),
];

#[test]
fn every_server_header_refuses_by_name_rather_than_doing_nothing() {
    for (label, src) in SERVER_SURFACE {
        let out = tw_outcome(src);
        let err = out.error.unwrap_or_else(|| {
            panic!(
                "{label} was ACCEPTED and did nothing. A declaration the runtime cannot honour has \
                 to say so: whoever wrote it has been told their intent was understood, and will \
                 find out otherwise from a blank page in production. Refuse it by name, or \
                 implement it.\n{src}"
            )
        });
        assert!(
            err.to_lowercase().contains("route")
                || err.to_lowercase().contains("server")
                || err.to_lowercase().contains("error page"),
            "{label}'s refusal must NAME the surface it is refusing, so the reader knows which \
             line to look at: {err}"
        );
        assert_eq!(out.output, "", "a refused program produces no output");
    }
}

/// The VM refuses identically. A server header that ran on one tier and not the other would be
/// worse than one that runs nowhere.
#[test]
fn the_two_tiers_refuse_the_server_surface_the_same_way() {
    for (label, src) in SERVER_SURFACE {
        let tw = tw_outcome(src).error;
        let vm = vm_outcome(src).error;
        assert!(
            tw.is_some() && vm.is_some(),
            "{label} must be refused on BOTH tiers"
        );
        assert_eq!(
            tw, vm,
            "{label} must be refused in the SAME words on both tiers"
        );
    }
}

// ---- Routes that RUN. ----

/// Both tiers, same answer, no error.
fn both(src: &str) -> String {
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "the tree-walker must run it:\n{src}");
    let vm = vm_outcome(src);
    assert_eq!(vm.error, None, "the VM must run it:\n{src}");
    assert_eq!(
        tw.output.lines().collect::<Vec<_>>(),
        vm.output.lines().collect::<Vec<_>>(),
        "the two tiers answered the same request differently:\n{src}"
    );
    tw.output
}

/// A `## Route` is a FUNCTION plus an entry in a table, and `route(method, path)` walks it.
///
/// This is the same move that opened the browser tier: lower the construct into ordinary LOGOS
/// rather than inventing machinery for it. A route body is a function body, dispatch is a
/// comparison, and both tiers get it for free because neither had to learn anything new.
#[test]
fn a_route_answers_its_own_method_and_path() {
    let src = "# Server\n\
        ## Route GET /hello -> Text:\n    Return \"hi\".\n\n\
        ## Main\nShow route(\"GET\", \"/hello\").\n";
    assert_eq!(both(src).trim(), "hi");
}

#[test]
fn routes_are_told_apart_by_both_method_and_path() {
    // Path alone is not identity: `GET /thing` and `POST /thing` are different routes, and a
    // dispatcher that matched on path would silently serve the wrong one.
    let src = "# Server\n\
        ## Route GET /thing -> Text:\n    Return \"read\".\n\n\
        ## Route POST /thing -> Text:\n    Return \"write\".\n\n\
        ## Route GET /other -> Text:\n    Return \"other\".\n\n\
        ## Main\n\
        Show route(\"GET\", \"/thing\").\n\
        Show route(\"POST\", \"/thing\").\n\
        Show route(\"GET\", \"/other\").\n";
    assert_eq!(
        both(src).lines().collect::<Vec<_>>(),
        vec!["read", "write", "other"]
    );
}

#[test]
fn a_route_body_is_a_real_body_and_can_compute() {
    // A route is not a lookup table of constants — its body is ordinary LOGOS, with locals,
    // control flow and calls to the rest of the program.
    let src = "# Server\n\
        ## To shout (t: Text) -> Text:\n    Return t + \"!\".\n\n\
        ## Route GET /greet -> Text:\n\
        \x20   Let mutable parts be a new Seq of Text.\n\
        \x20   Push \"hello\" to parts.\n\
        \x20   Push \"world\" to parts.\n\
        \x20   Let mutable out be \"\".\n\
        \x20   For each p in parts:\n\
        \x20       Set out to out + p.\n\
        \x20   Return shout(out).\n\n\
        ## Main\nShow route(\"GET\", \"/greet\").\n";
    assert_eq!(both(src).trim(), "helloworld!");
}

#[test]
fn a_request_that_matches_no_route_is_a_404_rather_than_a_wrong_answer() {
    // The edge that matters most: an unmatched request must NOT fall through to whichever route
    // happens to be first. Serving the wrong page is worse than serving none.
    let src = "# Server\n\
        ## Route GET /hello -> Text:\n    Return \"hi\".\n\n\
        ## Main\nShow route(\"GET\", \"/nope\").\n";
    assert_eq!(both(src).trim(), "404");
}

#[test]
fn a_known_path_with_the_wrong_method_is_405_and_not_404() {
    // `POST /hello` when only `GET /hello` exists is a METHOD problem, not a missing page, and
    // saying 404 sends the caller looking for a typo in their URL.
    let src = "# Server\n\
        ## Route GET /hello -> Text:\n    Return \"hi\".\n\n\
        ## Main\nShow route(\"POST\", \"/hello\").\n";
    assert_eq!(both(src).trim(), "405");
}

// ---- Typed path parameters. ----

#[test]
fn a_route_binds_a_typed_path_parameter() {
    let src = "# Server\n\
        ## Route GET /todos/:id (id: Int) -> Text:\n    Return \"todo \" + id.\n\n\
        ## Main\nShow route(\"GET\", \"/todos/42\").\n";
    assert_eq!(both(src).trim(), "todo 42");
}

#[test]
fn a_text_parameter_arrives_unconverted() {
    let src = "# Server\n\
        ## Route GET /users/:name (name: Text) -> Text:\n    Return \"hi \" + name.\n\n\
        ## Main\nShow route(\"GET\", \"/users/ada\").\n";
    assert_eq!(both(src).trim(), "hi ada");
}

#[test]
fn several_parameters_bind_in_declaration_order() {
    // Order is the thing to get wrong here: a dispatcher that passed them positionally by
    // POSITION IN THE PATH rather than by declared name would swap them silently whenever the
    // author wrote the parameter list in a different order from the URL.
    let src = "# Server\n\
        ## Route GET /r/:a/:b (b: Text) and (a: Text) -> Text:\n    Return a + \"|\" + b.\n\n\
        ## Main\nShow route(\"GET\", \"/r/first/second\").\n";
    assert_eq!(both(src).trim(), "first|second");
}

#[test]
fn a_parameter_route_does_not_swallow_a_different_shape() {
    // `/todos/:id` must not answer `/todos` or `/todos/1/edit`. A matcher that checked only a
    // prefix would serve the wrong page for both.
    let src = "# Server\n\
        ## Route GET /todos/:id (id: Int) -> Text:\n    Return \"one\".\n\n\
        ## Main\n\
        Show route(\"GET\", \"/todos/1\").\n\
        Show route(\"GET\", \"/todos\").\n\
        Show route(\"GET\", \"/todos/1/edit\").\n";
    assert_eq!(
        both(src).lines().collect::<Vec<_>>(),
        vec!["one", "404", "404"]
    );
}

#[test]
fn a_literal_route_wins_over_a_parameter_route_that_also_matches() {
    // `/todos/new` and `/todos/:id` both match `/todos/new`. The literal is the more specific
    // statement of intent and must win, whichever order they appear in the file — otherwise
    // adding a `:id` route silently steals a page that already worked.
    let src = "# Server\n\
        ## Route GET /todos/:id (id: Text) -> Text:\n    Return \"param\".\n\n\
        ## Route GET /todos/new -> Text:\n    Return \"literal\".\n\n\
        ## Main\nShow route(\"GET\", \"/todos/new\").\n";
    assert_eq!(both(src).trim(), "literal");
}

#[test]
fn a_path_parameter_that_is_not_the_declared_type_is_a_400() {
    // `/todos/abc` against `(id: Int)` is a BAD REQUEST, not a 404 and not a silent 0. Parsing
    // it to zero would serve todo 0 to someone who asked for "abc" — a wrong answer dressed as
    // a right one, which is the worst outcome a router can produce.
    let src = "# Server\n\
        ## Route GET /todos/:id (id: Int) -> Text:\n    Return \"todo \" + id.\n\n\
        ## Main\nShow route(\"GET\", \"/todos/abc\").\n";
    assert_eq!(both(src).trim(), "400");
}

#[test]
fn a_parameter_route_still_answers_405_for_the_wrong_method() {
    let src = "# Server\n\
        ## Route GET /todos/:id (id: Text) -> Text:\n    Return \"got\".\n\n\
        ## Main\nShow route(\"POST\", \"/todos/9\").\n";
    assert_eq!(both(src).trim(), "405");
}

// ---- Middleware: `## Before each request`. ----

#[test]
fn middleware_that_answers_none_lets_the_route_run() {
    let src = "# Server\n\
        ## Before each request (r: Request) -> Option of Response:\n    Return none.\n\n\
        ## Route GET /hello -> Text:\n    Return \"hi\".\n\n\
        ## Main\nShow route(\"GET\", \"/hello\").\n";
    assert_eq!(both(src).trim(), "hi");
}

#[test]
fn middleware_that_answers_a_response_short_circuits_the_route() {
    // The route body must NOT run. A guard that answered and then let the handler run anyway
    // would be an authorisation check that authorises nothing.
    let src = "# Server\n\
        ## Before each request (r: Request) -> Option of Response:\n\
        \x20   Return some(plain(\"denied\")).\n\n\
        ## Route GET /secret -> Text:\n    Return \"secret\".\n\n\
        ## Main\nShow route(\"GET\", \"/secret\").\n";
    assert_eq!(both(src).trim(), "denied");
}

#[test]
fn middleware_sees_the_method_and_path_of_the_request() {
    let src = "# Server\n\
        ## Before each request (r: Request) -> Option of Response:\n\
        \x20   Return some(plain(r's method + \" \" + r's path)).\n\n\
        ## Route GET /a -> Text:\n    Return \"a\".\n\n\
        ## Main\nShow route(\"POST\", \"/a\").\n";
    assert_eq!(both(src).trim(), "POST /a");
}

#[test]
fn middleware_runs_in_file_order_and_the_first_answer_wins() {
    // Order is the contract: an auth guard declared first must be able to stop a logging or
    // rate-limiting guard declared after it from ever seeing the request.
    let src = "# Server\n\
        ## Before each request (r: Request) -> Option of Response:\n\
        \x20   Return some(plain(\"first\")).\n\n\
        ## Before each request (r: Request) -> Option of Response:\n\
        \x20   Return some(plain(\"second\")).\n\n\
        ## Route GET /x -> Text:\n    Return \"route\".\n\n\
        ## Main\nShow route(\"GET\", \"/x\").\n";
    assert_eq!(both(src).trim(), "first");
}

/// ★ EVERY DISPATCHER QUESTION, ASKED WITH A GUARD PRESENT.
///
/// A `## Before each request` used to emit its `Request` binding into the BODY walk while the
/// STATUS walk referenced it, so any program with a guard produced a `routeStatusWith` naming an
/// undeclared variable. **Nothing caught it, because every middleware test above calls `route(…)`
/// and not one calls `routeStatus(…)`.** The guard was covered; the guard's effect on the other
/// three answers was not — a coverage hole shaped exactly like the bug.
///
/// So this asks all four, on both tiers, of one program that has a guard. The failure it guards
/// against is not a wrong answer but a program that will not run at all, which is why `both()`
/// asserting no error is the load-bearing part.
#[test]
fn every_dispatcher_question_works_with_a_guard_present() {
    let src = "# Server\n\
        ## Before each request (r: Request) -> Option of Response:\n    Return none.\n\n\
        ## Route GET /hello -> Text:\n    Return \"hi\".\n\n\
        ## Main\n\
        Show route(\"GET\", \"/hello\").\n\
        Show routeStatus(\"GET\", \"/hello\").\n\
        Show routeContentType(\"GET\", \"/hello\").\n";
    assert_eq!(both(src).trim(), "hi\n200\ntext/plain; charset=utf-8");
}

/// The same, for a guard that ANSWERS — the short-circuit has to carry its own status and content
/// type too, or a refused request is served with the route's.
#[test]
fn a_guard_that_answers_carries_its_own_status_and_content_type() {
    let src = "# Server\n\
        ## Before each request (r: Request) -> Option of Response:\n\
        \x20   Return some(unauthorized()).\n\n\
        ## Route GET /secret -> Text:\n    Return \"secret\".\n\n\
        ## Main\n\
        Show route(\"GET\", \"/secret\").\n\
        Show routeStatus(\"GET\", \"/secret\").\n";
    assert_eq!(both(src).trim(), "Unauthorized\n401");
}

/// And it BUILDS on the tier a server ships as. The original defect was an undeclared variable in
/// EMITTED code, which the tree-walker and the VM infer their way past and rustc does not — the
/// split this campaign keeps finding.
#[test]
fn a_program_with_a_guard_builds_on_the_deploy_tier() {
    let src = "# Server\n\
        ## Before each request (r: Request) -> Option of Response:\n    Return none.\n\n\
        ## Route GET /hello -> Text:\n    Return \"hi\".\n\n\
        ## Main\nShow routeStatus(\"GET\", \"/hello\").\n";
    let rust = logicaffeine_compile::compile::compile_to_rust(src)
        .unwrap_or_else(|e| panic!("a guarded program must reach AOT-Rust: {e:?}\n{src}"));
    let refusals: Vec<&str> = rust
        .lines()
        .filter(|l| l.contains("compile_error!"))
        .collect();
    assert!(
        refusals.is_empty(),
        "it would not build:\n{}\n{src}",
        refusals.join("\n")
    );
}

#[test]
fn a_later_middleware_runs_when_an_earlier_one_declines() {
    let src = "# Server\n\
        ## Before each request (r: Request) -> Option of Response:\n    Return none.\n\n\
        ## Before each request (r: Request) -> Option of Response:\n\
        \x20   Return some(plain(\"second\")).\n\n\
        ## Route GET /x -> Text:\n    Return \"route\".\n\n\
        ## Main\nShow route(\"GET\", \"/x\").\n";
    assert_eq!(both(src).trim(), "second");
}

#[test]
fn middleware_runs_before_matching_so_it_can_answer_an_unrouted_path() {
    // It is `## Before each REQUEST`, not "before each route". A guard that only ran on matched
    // paths could not redirect an unauthenticated visitor away from a URL that does not exist,
    // and would leak which URLs do exist by answering them differently.
    let src = "# Server\n\
        ## Before each request (r: Request) -> Option of Response:\n\
        \x20   Return some(plain(\"guard\")).\n\n\
        ## Route GET /known -> Text:\n    Return \"known\".\n\n\
        ## Main\nShow route(\"GET\", \"/nowhere\").\n";
    assert_eq!(both(src).trim(), "guard");
}

#[test]
fn middleware_declining_leaves_404_and_405_intact() {
    let src = "# Server\n\
        ## Before each request (r: Request) -> Option of Response:\n    Return none.\n\n\
        ## Route GET /known -> Text:\n    Return \"known\".\n\n\
        ## Main\n\
        Show route(\"GET\", \"/nowhere\").\n\
        Show route(\"POST\", \"/known\").\n";
    assert_eq!(both(src).lines().collect::<Vec<_>>(), vec!["404", "405"]);
}

// ---- Routes that answer a VALUE, not a string. ----

const TODO: &str = "## A Todo has:\n    an id, which is Int.\n    a text, which is Text.\n\n";

#[test]
fn a_route_answering_a_record_serves_its_json() {
    // The point of a typed route: the handler returns a VALUE and the wire gets JSON, without the
    // author writing a serializer or remembering to call one.
    let src = format!(
        "# Server\n{TODO}## Route GET /todo -> Todo:\n\
         \x20   Return a new Todo with id 7 and text \"milk\".\n\n\
         ## Main\nShow route(\"GET\", \"/todo\").\n"
    );
    assert_eq!(both(&src).trim(), r#"{"id":7,"text":"milk"}"#);
}

#[test]
fn a_record_answer_is_escaped_on_the_way_out() {
    // The handler's data is not the author's — it came from a database, a form, or a stranger.
    // A quote in it must not be able to end the JSON string it lives in.
    let src = format!(
        "# Server\n{TODO}## Route GET /todo -> Todo:\n\
         \x20   Return a new Todo with id 1 and text \"say \\\"hi\\\"\".\n\n\
         ## Main\nShow route(\"GET\", \"/todo\").\n"
    );
    assert_eq!(both(&src).trim(), r#"{"id":1,"text":"say \"hi\""}"#);
}

#[test]
fn a_typed_route_still_reads_its_path_parameter() {
    let src = format!(
        "# Server\n{TODO}## Route GET /todo/:id (id: Int) -> Todo:\n\
         \x20   Return a new Todo with id id and text \"found\".\n\n\
         ## Main\nShow route(\"GET\", \"/todo/42\").\n"
    );
    assert_eq!(both(&src).trim(), r#"{"id":42,"text":"found"}"#);
}

#[test]
fn a_text_route_is_not_quoted_as_json() {
    // A `-> Text` route answers its text verbatim. Wrapping it in quotes because it happens to be
    // a string would change every existing route's response.
    let src = "# Server\n## Route GET /plain -> Text:\n    Return \"hi\".\n\n\
        ## Main\nShow route(\"GET\", \"/plain\").\n";
    assert_eq!(both(src).trim(), "hi");
}

#[test]
fn a_route_answering_a_type_with_no_encoder_is_still_refused_by_name() {
    // The frontier has to stay honest: a return type the derive does not cover yet must say so,
    // not answer something plausible.
    let src = "# Server\n## Route GET /nums -> Seq of Int:\n    Return [1, 2].\n\n\
        ## Main\nShow route(\"GET\", \"/nums\").\n";
    let err = tw_outcome(src)
        .error
        .expect("a Seq return must still be refused");
    assert!(
        err.contains("Route") && err.to_lowercase().contains("json"),
        "the refusal names the route and what it is waiting on: {err}"
    );
}

// ---- Server-side rendering: a route whose body is a template. ----

#[test]
fn a_view_route_answers_html() {
    let src = "# Server\n## Route GET /report -> View:\n    <h1>Report</h1>\n\n\
        ## Main\nShow route(\"GET\", \"/report\").\n";
    assert_eq!(both(src).trim(), "<h1>Report</h1>");
}

#[test]
fn a_view_route_renders_nested_markup_and_attributes() {
    let src = "# Server\n## Route GET /page -> View:\n\
        \x20   <div class=\"card\"><p id=\"x\">hi</p></div>\n\n\
        ## Main\nShow route(\"GET\", \"/page\").\n";
    assert_eq!(
        both(src).trim(),
        r#"<div class="card"><p id="x">hi</p></div>"#
    );
}

/// A server-rendered boolean attribute is a PRESENCE too — and this is the tier where getting it
/// wrong reaches people who never loaded the app. `disabled="false"` disables the control in
/// every browser there is, so a form served with it is a form nobody can submit. Present, it is
/// written as the document serializes it, `disabled=""`.
#[test]
fn a_view_route_renders_a_boolean_attribute_as_a_presence() {
    let src = "# Server\n## Route GET /f -> View:\n\
        \x20   <button disabled={true}>go</button>\n\n\
        ## Main\nShow route(\"GET\", \"/f\").\n";
    assert_eq!(both(src).trim(), "<button disabled=\"\">go</button>");
}

#[test]
fn a_false_boolean_attribute_is_absent_from_the_served_html() {
    let src = "# Server\n## Route GET /f -> View:\n\
        \x20   <button disabled={false}>go</button>\n\n\
        ## Main\nShow route(\"GET\", \"/f\").\n";
    assert_eq!(both(src).trim(), "<button>go</button>");
}

/// The dynamic case, which is the one a real page writes: the flag comes from data, so presence
/// has to be decided as the page renders rather than when it compiles.
#[test]
fn a_computed_boolean_attribute_decides_presence_at_render_time() {
    let src = "# Server\n## Route GET /f/:v (v: Text) -> View:\n\
        \x20   <button disabled={v is equal to \"yes\"}>go</button>\n\n\
        ## Main\nShow route(\"GET\", \"/f/yes\").\nShow route(\"GET\", \"/f/no\").\n";
    assert_eq!(
        both(src).trim(),
        "<button disabled=\"\">go</button>\n<button>go</button>"
    );
}

#[test]
fn an_ordinary_attribute_holding_a_bool_still_serves_its_value() {
    let src = "# Server\n## Route GET /f -> View:\n\
        \x20   <p data-done={false}>x</p>\n\n\
        ## Main\nShow route(\"GET\", \"/f\").\n";
    assert_eq!(both(src).trim(), r#"<p data-done="false">x</p>"#);
}

#[test]
fn a_view_route_fills_a_hole_from_its_path_parameter() {
    let src = "# Server\n## Route GET /hello/:name (name: Text) -> View:\n\
        \x20   <p>Hello {name}</p>\n\n\
        ## Main\nShow route(\"GET\", \"/hello/ada\").\n";
    assert_eq!(both(src).trim(), "<p>Hello ada</p>");
}

#[test]
fn a_hole_in_a_server_rendered_page_is_escaped() {
    // ★ The security floor of SSR. A hole carries data the SERVER did not write — a path
    // segment, a database row, a form field. If it reaches the page unescaped, a visitor who
    // asks for the right URL executes script in every other visitor's browser. The browser tier
    // already escapes; the server tier must not be the one that forgets, because it is the tier
    // whose output reaches people who never loaded the app.
    let src = "# Server\n## Route GET /hello/:name (name: Text) -> View:\n\
        \x20   <p>Hello {name}</p>\n\n\
        ## Main\nShow route(\"GET\", \"/hello/<img src=x onerror=alert(1)>\").\n";
    // A payload with no SLASH in it: `</script>` would add a path segment and the request would
    // 404 before ever reaching the hole — which is correct routing, and would have made this test
    // pass for a reason that has nothing to do with escaping. The `<img onerror=…>` form is the
    // one a real attacker reaches for when slashes are unavailable.
    let out = both(src);
    assert!(
        !out.contains("<img"),
        "a hole must never emit raw markup — this page would run a stranger's script: {out}"
    );
    assert_eq!(
        out.trim(),
        "<p>Hello &lt;img src=x onerror=alert(1)&gt;</p>"
    );
}

#[test]
fn an_attribute_hole_is_escaped_too() {
    // The harder half: a bare quote in an attribute value ends the attribute and everything
    // after it becomes markup nobody wrote.
    let src = "# Server\n## Route GET /t/:v (v: Text) -> View:\n\
        \x20   <p title={v}>x</p>\n\n\
        ## Main\nShow route(\"GET\", \"/t/a\\\"b\").\n";
    let out = both(src);
    assert!(
        !out.contains(r#"title="a"b""#),
        "an unescaped quote ends the attribute: {out}"
    );
    assert_eq!(out.trim(), r#"<p title="a&quot;b">x</p>"#);
}

#[test]
fn a_view_route_still_answers_404_and_405_for_other_requests() {
    let src = "# Server\n## Route GET /page -> View:\n    <p>p</p>\n\n\
        ## Main\n\
        Show route(\"GET\", \"/other\").\n\
        Show route(\"POST\", \"/page\").\n";
    assert_eq!(both(src).lines().collect::<Vec<_>>(), vec!["404", "405"]);
}

// ---- Redirects. ----

#[test]
fn a_redirect_answers_302_with_its_destination() {
    // The router's other answers are status codes (`404`, `405`, `400`), so a redirect says the
    // same way: the status and where to go.
    let src = "# Server\n## Redirect /old -> /new.\n\n\
        ## Route GET /new -> Text:\n    Return \"here\".\n\n\
        ## Main\nShow route(\"GET\", \"/old\").\n";
    assert_eq!(both(src).trim(), "302 /new");
}

#[test]
fn a_redirect_does_not_shadow_a_real_route_at_the_same_path() {
    // A path that a ROUTE claims is served, not redirected — otherwise adding a redirect could
    // silently take a working page away.
    let src = "# Server\n## Redirect /thing -> /other.\n\n\
        ## Route GET /thing -> Text:\n    Return \"served\".\n\n\
        ## Main\nShow route(\"GET\", \"/thing\").\n";
    assert_eq!(both(src).trim(), "served");
}

#[test]
fn a_redirect_answers_any_method() {
    // A moved URL has moved for everyone. Answering 405 to a POST at a redirected path would
    // tell the caller the path exists under some other method, which is not what happened.
    let src = "# Server\n## Redirect /old -> /new.\n\n\
        ## Route GET /new -> Text:\n    Return \"here\".\n\n\
        ## Main\n\
        Show route(\"GET\", \"/old\").\n\
        Show route(\"POST\", \"/old\").\n";
    assert_eq!(
        both(src).lines().collect::<Vec<_>>(),
        vec!["302 /new", "302 /new"]
    );
}

#[test]
fn middleware_still_runs_before_a_redirect() {
    // `## Before each REQUEST` — a guard must be able to answer a redirected path too, or an
    // auth check could be bypassed by aiming at a URL that happens to have moved.
    let src = "# Server\n\
        ## Before each request (r: Request) -> Option of Response:\n\
        \x20   Return some(plain(\"guard\")).\n\n\
        ## Redirect /old -> /new.\n\n\
        ## Route GET /new -> Text:\n    Return \"here\".\n\n\
        ## Main\nShow route(\"GET\", \"/old\").\n";
    assert_eq!(both(src).trim(), "guard");
}

#[test]
fn an_unredirected_unrouted_path_is_still_404() {
    let src = "# Server\n## Redirect /old -> /new.\n\n\
        ## Route GET /new -> Text:\n    Return \"here\".\n\n\
        ## Main\nShow route(\"GET\", \"/nowhere\").\n";
    assert_eq!(both(src).trim(), "404");
}

// ---- Error pages. ----

#[test]
fn a_declared_404_page_is_served_instead_of_the_bare_status() {
    let src = "# Server\n## Error page 404\nView:\n    <h1>Nothing here</h1>\n\n\
        ## Route GET /known -> Text:\n    Return \"k\".\n\n\
        ## Main\nShow route(\"GET\", \"/nowhere\").\n";
    assert_eq!(both(src).trim(), "<h1>Nothing here</h1>");
}

#[test]
fn a_405_page_is_distinct_from_the_404_page() {
    // Two different mistakes deserve two different pages: "this URL does not exist" and "this URL
    // does not do that". A router that served one for both would tell half its visitors the wrong
    // thing.
    let src = "# Server\n\
        ## Error page 404\nView:\n    <p>missing</p>\n\n\
        ## Error page 405\nView:\n    <p>wrong method</p>\n\n\
        ## Route GET /known -> Text:\n    Return \"k\".\n\n\
        ## Main\n\
        Show route(\"GET\", \"/nowhere\").\n\
        Show route(\"POST\", \"/known\").\n";
    assert_eq!(
        both(src).lines().collect::<Vec<_>>(),
        vec!["<p>missing</p>", "<p>wrong method</p>"]
    );
}

#[test]
fn without_a_declared_page_the_bare_status_is_still_the_answer() {
    let src = "# Server\n## Route GET /known -> Text:\n    Return \"k\".\n\n\
        ## Main\n\
        Show route(\"GET\", \"/nowhere\").\n\
        Show route(\"POST\", \"/known\").\n";
    assert_eq!(both(src).lines().collect::<Vec<_>>(), vec!["404", "405"]);
}

#[test]
fn declaring_only_one_error_page_leaves_the_other_status_bare() {
    // Partial coverage must not silently borrow the page that IS declared.
    let src = "# Server\n## Error page 404\nView:\n    <p>missing</p>\n\n\
        ## Route GET /known -> Text:\n    Return \"k\".\n\n\
        ## Main\n\
        Show route(\"GET\", \"/nowhere\").\n\
        Show route(\"POST\", \"/known\").\n";
    assert_eq!(
        both(src).lines().collect::<Vec<_>>(),
        vec!["<p>missing</p>", "405"]
    );
}

#[test]
fn an_error_page_hole_is_escaped_like_any_other_template() {
    let src = "# Server\n## Error page 404\nState:\n    Let msg be \"<b>x</b>\".\nView:\n    <p>{msg}</p>\n\n\
        ## Route GET /known -> Text:\n    Return \"k\".\n\n\
        ## Main\nShow route(\"GET\", \"/nowhere\").\n";
    assert_eq!(both(src).trim(), "<p>&lt;b&gt;x&lt;/b&gt;</p>");
}

// ---- The status a route answers WITH, alongside the body it answers. ----

/// `route(…)` answers TEXT, so its status and its body were one string: a page fetching a URL
/// nothing serves heard `"404"` as a BODY and could only discover the problem by failing to
/// decode it. `routeStatus(method, path)` answers the code, built by the SAME walk that builds
/// the body — one walk with two outputs, so a route cannot be added to one and forgotten in the
/// other.
#[test]
fn a_matched_route_answers_200() {
    let src = "# Server\n## Route GET /hello -> Text:\n    Return \"hi\".\n\n\
        ## Main\nShow routeStatus(\"GET\", \"/hello\").\n";
    assert_eq!(both(src).trim(), "200");
}

#[test]
fn a_path_nothing_serves_answers_404() {
    let src = "# Server\n## Route GET /hello -> Text:\n    Return \"hi\".\n\n\
        ## Main\nShow routeStatus(\"GET\", \"/nope\").\n";
    assert_eq!(both(src).trim(), "404");
}

#[test]
fn a_known_path_under_the_wrong_method_answers_405() {
    // The distinction the body already makes, now carried as a code: 404 would send the caller
    // looking for a typo in a URL that is correct.
    let src = "# Server\n## Route GET /hello -> Text:\n    Return \"hi\".\n\n\
        ## Main\nShow routeStatus(\"POST\", \"/hello\").\n";
    assert_eq!(both(src).trim(), "405");
}

#[test]
fn a_redirect_answers_302() {
    let src = "# Server\n## Route GET /new -> Text:\n    Return \"hi\".\n\n\
        ## Redirect /old -> /new.\n\n\
        ## Main\nShow routeStatus(\"GET\", \"/old\").\n";
    assert_eq!(both(src).trim(), "302");
}

#[test]
fn a_typed_route_answers_200_like_any_other() {
    let src = "# Server\n## A Todo has:\n    an id, which is Int.\n\n\
        ## Route GET /todo -> Todo:\n    Return a new Todo with id 1.\n\n\
        ## Main\nShow routeStatus(\"GET\", \"/todo\").\n";
    assert_eq!(both(src).trim(), "200");
}

#[test]
fn the_status_and_the_body_agree_about_every_path() {
    // The invariant the shared walk exists to hold: wherever the body says a miss, the status
    // says one too. Two separate walks would drift the first time a route shape was added.
    let src = "# Server\n## Route GET /a -> Text:\n    Return \"A\".\n\n\
        ## Route POST /b -> Text:\n    Return \"B\".\n\n\
        ## Redirect /c -> /a.\n\n\
        ## Main\n\
        Show routeStatus(\"GET\", \"/a\") + length of route(\"GET\", \"/a\").\n\
        Show routeStatus(\"GET\", \"/b\") + length of route(\"GET\", \"/b\").\n\
        Show routeStatus(\"GET\", \"/c\") + length of route(\"GET\", \"/c\").\n\
        Show routeStatus(\"GET\", \"/z\") + length of route(\"GET\", \"/z\").\n";
    // 200+1 (body "A"), 405+3 ("405"), 302+6 ("302 /a"), 404+3 ("404").
    assert_eq!(both(src).trim(), "201\n408\n308\n407");
}

#[test]
fn a_route_can_answer_a_list_of_records() {
    // ★ THE SHAPE A COLLECTION ENDPOINT ALWAYS HAS, and the plan's own headline example
    // (`## Route GET /api/todos -> Seq of Todo:`). It was REFUSED — "a type the derived JSON
    // codecs do not cover" — so the most ordinary route there is could not be written, and a page
    // could not load a list from its own server.
    //
    // It answers a JSON ARRAY through the list encoder emitted beside the record's own, so what
    // the route sends and what a typed `Get … as Seq of Todo` decodes are two ends of ONE
    // contract rather than two hand-written serialisers.
    let out = both(
        "## A Tag has:\n\
        \x20   a name, which is Text.\n\
         \n\
         ## Route GET /tags -> Seq of Tag:\n\
        \x20   Return [a new Tag with name \"a\", a new Tag with name \"b\"].\n\
         \n\
         ## Main\n\
         Show route(\"GET\", \"/tags\").\n",
    );
    assert_eq!(out.trim(), "[{\"name\":\"a\"},{\"name\":\"b\"}]");
}

#[test]
fn a_route_answering_an_empty_list_sends_an_empty_array() {
    // `[]` and "nothing" are different answers, and a client that told them apart would be right
    // to — an empty collection is a value the server HAS.
    let out = both(
        "## A Tag has:\n\
        \x20   a name, which is Text.\n\
         \n\
         ## Route GET /tags -> Seq of Tag:\n\
        \x20   Return a new Seq of Tag.\n\
         \n\
         ## Main\n\
         Show route(\"GET\", \"/tags\").\n",
    );
    assert_eq!(out.trim(), "[]");
}

// ---------------------------------------------------------------------------------------------
// ★ A QUERY STRING IS PART OF THE URL, and the router has to know that. ★
// ---------------------------------------------------------------------------------------------

#[test]
fn a_path_with_a_query_string_still_matches_its_route() {
    // ★ THE ROUTER NEVER STRIPPED THE QUERY. `/search?q=hi` was compared whole against `/search`
    // and 404'd — so every search box on every LOGOS site hit a wall. And it is not hypothetical
    // any more: a `get` form NAVIGATES to `action?field=value` (`progressive_form_lock`), so the
    // framework was shipping a form whose destination its own router refused.
    let out = both(
        "## Route GET /search -> Text:\n\
        \x20   Return \"searched\".\n\
         \n\
         ## Main\n\
         Show route(\"GET\", \"/search?q=hi\").\n",
    );
    assert_eq!(out.trim(), "searched");
}

#[test]
fn a_declared_param_is_bound_from_the_query() {
    // A route that names a parameter the PATH does not carry reads it from the query — which is
    // what a `get` form sends, and the only way a search route can see what was searched for.
    let out = both(
        "## Route GET /search (q: Text) -> Text:\n\
        \x20   Return \"found {q}\".\n\
         \n\
         ## Main\n\
         Show route(\"GET\", \"/search?q=logos\").\n",
    );
    assert_eq!(out.trim(), "found logos");
}

#[test]
fn a_missing_query_param_is_empty_rather_than_a_mismatch() {
    // A query parameter is OPTIONAL by nature — a bare `/search` is a real request, and refusing
    // it would 404 the page a visitor reaches by typing the URL.
    let out = both(
        "## Route GET /search (q: Text) -> Text:\n\
        \x20   Return \"found [{q}]\".\n\
         \n\
         ## Main\n\
         Show route(\"GET\", \"/search\").\n",
    );
    assert_eq!(out.trim(), "found []");
}

#[test]
fn a_query_carries_several_parameters_and_each_finds_its_own() {
    // A form with two fields sends `a=1&b=2`, and a router that only read the first would hand the
    // route someone else's value.
    let out = both(
        "## Route GET /search (q: Text) and (page: Int) -> Text:\n\
        \x20   Return \"{q}/{page}\".\n\
         \n\
         ## Main\n\
         Show route(\"GET\", \"/search?q=logos&page=3\").\n",
    );
    assert_eq!(out.trim(), "logos/3");
}

#[test]
fn a_path_parameter_still_wins_over_the_query() {
    // When a name appears in BOTH, the PATH is the more specific statement — it is part of the
    // route's identity, while a query is decoration a client may add to any URL.
    let out = both(
        "## Route GET /t/:id (id: Int) -> Text:\n\
        \x20   Return \"id={id}\".\n\
         \n\
         ## Main\n\
         Show route(\"GET\", \"/t/7?id=99\").\n",
    );
    assert_eq!(out.trim(), "id=7");
}

#[test]
fn an_empty_query_string_is_not_a_parameter() {
    // `/search?` is a URL a browser really produces from an empty form, and it must behave as
    // `/search` rather than as a parameter whose name is the empty string.
    let out = both(
        "## Route GET /search -> Text:\n\
        \x20   Return \"searched\".\n\
         \n\
         ## Main\n\
         Show route(\"GET\", \"/search?\").\n",
    );
    assert_eq!(out.trim(), "searched");
}

#[test]
fn a_query_value_is_form_decoded_the_way_a_browser_encoded_it() {
    // A `get` form sends `hello world` as `hello+world`, and a slash as `%2F`. A router that
    // handed the route the RAW text would give a search box the literal `hello+world` — wrong in
    // a way that looks almost right, which is how it survives review.
    let out = both(
        "## Route GET /s (q: Text) -> Text:\n\
        \x20   Return \"[{q}]\".\n\
         \n\
         ## Main\n\
         Show route(\"GET\", \"/s?q=hello+world%2Fnow\").\n",
    );
    assert_eq!(out.trim(), "[hello world/now]");
}

#[test]
fn a_multi_byte_character_survives_the_decoding_whole() {
    // ★ Percent-encoding encodes BYTES. A check mark is `%E2%9C%93` — THREE escapes for ONE
    // character — so a decoder working a character at a time produces three replacement
    // characters and hands the page mojibake the sender never wrote.
    let out = both(
        "## Route GET /s (q: Text) -> Text:\n\
        \x20   Return \"[{q}]\".\n\
         \n\
         ## Main\n\
         Show route(\"GET\", \"/s?q=%E2%9C%93\").\n",
    );
    assert_eq!(out.trim(), "[\u{2713}]");
}

#[test]
fn a_percent_that_is_not_an_escape_is_left_alone() {
    // `50%` is text somebody typed. Dropping the sign, or guessing at the two bytes after it,
    // silently changes their data — and a truncated escape at the very END of the query is the
    // shape a hand-edited URL actually has.
    let out = both(
        "## Route GET /s (q: Text) -> Text:\n\
        \x20   Return \"[{q}]\".\n\
         \n\
         ## Main\n\
         Show route(\"GET\", \"/s?q=50%\").\n\
         Show route(\"GET\", \"/s?q=a%zz\").\n",
    );
    assert_eq!(out.trim(), "[50%]\n[a%zz]");
}

#[test]
fn an_equals_sign_inside_a_value_is_part_of_the_value() {
    // Base64 padding ends in `=`, and a token is the most likely thing to be passed this way.
    // Splitting a pair at EVERY `=` would truncate exactly the values that must not be truncated.
    let out = both(
        "## Route GET /s (t: Text) -> Text:\n\
        \x20   Return \"[{t}]\".\n\
         \n\
         ## Main\n\
         Show route(\"GET\", \"/s?t=YWJj==\").\n",
    );
    assert_eq!(out.trim(), "[YWJj==]");
}

#[test]
fn a_question_mark_inside_the_query_is_ordinary_content() {
    // Only the FIRST `?` separates the path from the query; the rest are characters of the query.
    // Truncating at a later one would silently drop the end of a value.
    let out = both(
        "## Route GET /s (q: Text) -> Text:\n\
        \x20   Return \"[{q}]\".\n\
         \n\
         ## Main\n\
         Show route(\"GET\", \"/s?q=why?\").\n",
    );
    assert_eq!(out.trim(), "[why?]");
}

#[test]
fn a_parameter_present_with_no_value_is_empty_and_not_absent() {
    // `?q=` is what an empty search box sends. It must read as the empty text the visitor
    // submitted rather than looking identical to a parameter nobody sent.
    let out = both(
        "## Route GET /s (q: Text) -> Text:\n\
        \x20   Return \"[{q}]\".\n\
         \n\
         ## Main\n\
         Show route(\"GET\", \"/s?q=&page=2\").\n",
    );
    assert_eq!(out.trim(), "[]");
}

#[test]
fn a_query_parameter_of_the_wrong_type_is_a_bad_request() {
    // A typed parameter is checked wherever it comes from. `page=abc` reaching `parseInt` would
    // serve page 0 to someone who asked for "abc" — a wrong answer dressed as a right one — and
    // `parseInt` traps, which would take the server down instead of answering.
    let out = both(
        "## Route GET /s (page: Int) -> Text:\n\
        \x20   Return \"page {page}\".\n\
         \n\
         ## Main\n\
         Show route(\"GET\", \"/s?page=abc\").\n\
         Show routeStatus(\"GET\", \"/s?page=abc\").\n",
    );
    assert_eq!(out.trim(), "400\n400");
}

#[test]
fn the_status_and_the_body_agree_about_a_url_carrying_a_query() {
    // The two walks are built arm for arm from one loop; a query that matched in one and not the
    // other would report 200 for a body that says 404, which is the worst kind of disagreement.
    let out = both(
        "## Route GET /s (q: Text) -> Text:\n\
        \x20   Return \"found {q}\".\n\
         \n\
         ## Main\n\
         Show routeStatus(\"GET\", \"/s?q=hi\").\n\
         Show routeStatus(\"POST\", \"/s?q=hi\").\n\
         Show routeStatus(\"GET\", \"/nope?q=hi\").\n",
    );
    assert_eq!(out.trim(), "200\n405\n404");
}

#[test]
fn a_middleware_guard_sees_the_path_without_the_query() {
    // ★ A guard asks about the PATH — `path of request starts with "/api"`. Handing it the whole
    // URL would still work for a prefix test and break every exact one, and a query is
    // attacker-controlled: a guard comparing `path` against `/admin` must not be satisfied, or
    // fooled, by what somebody appended after a `?`.
    //
    // ★★ DO NOT "SIMPLIFY" THIS BY PASSING THE REQUEST THROUGH UNCHANGED. The dispatcher receives
    // the whole URL and splits it; a real `Request`'s `path` field carries the query. So the guard
    // is handed a request with every field real EXCEPT `path`, which stays the already-split path
    // — and that asymmetry looks like an oversight until you read this. It was reintroduced once,
    // by passing the raw request straight through when the walks were widened to take a `Request`,
    // and this test is what caught it.
    let out = both(
        "## Before each request (r: Request) -> Option of Response:\n\
        \x20   If r's path is equal to \"/admin\": Return some(plain(\"guarded\")).\n\
        \x20   Return none.\n\
         \n\
         ## Route GET /admin -> Text:\n\
        \x20   Return \"open\".\n\
         \n\
         ## Main\n\
         Show route(\"GET\", \"/admin?x=1\").\n",
    );
    assert_eq!(out.trim(), "guarded");
}

#[test]
fn a_redirect_still_answers_when_the_old_url_carries_a_query() {
    // A moved URL is linked to from places that append their own tracking parameters; matching
    // the whole URL would leave those links dead while the bare one worked.
    let out = both(
        "## Redirect /old -> /new.\n\
         \n\
         ## Main\n\
         Show route(\"GET\", \"/old?ref=email\").\n\
         Show routeStatus(\"GET\", \"/old?ref=email\").\n",
    );
    assert_eq!(out.trim(), "302 /new\n302");
}

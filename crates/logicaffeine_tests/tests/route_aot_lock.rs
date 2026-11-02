//! ★ LOCK — a `## Route` compiles on the tier a server is actually DEPLOYED on. ★
//!
//! `largo build` produces an AOT-Rust binary. That tier is the only one that runs the static type
//! checker, and the route lowering handed it a contradiction: a handler's declared return type was
//! hard-coded to `Text` no matter what the route said it answered. So
//! `## Route GET /api/todos -> Seq of Todo:` became a function declared `-> Text` whose body
//! returns a `Seq of Todo`, and the build failed with a mismatch naming neither the route nor the
//! file it was written in.
//!
//! ★ WHY IT SURVIVED: the tree-walker and the VM infer loosely and never run that check, and every
//! route lock in this campaign drives those two tiers. A route answering a record was proven to
//! WORK on the tiers a test uses and to not COMPILE on the tier a server ships as — which is the
//! worst split there is, because the suite is green and the deploy is broken.
//!
//! ★ THE FIX IS THE OBVIOUS ONE ONCE SEEN: a handler carries the type its route declares, and the
//! DISPATCHER — which already wraps the call in the derived encoder — is what turns the answer into
//! the Text that goes on the wire. The encoding was never the handler's job.
//!
//! Every case here compiles to AOT-Rust AND is checked for the answer the other two tiers give, so
//! a fix that made the build pass by changing what a route ANSWERS would fail loudly.

use logicaffeine_compile::compile::{compile_to_rust, tw_outcome, vm_outcome};

/// Compile to AOT-Rust — what `largo build` does — and fail with the route source on refusal.
///
/// ★ AND IT READS THE GENERATED RUST FOR `compile_error!`, which is the only reason this file
/// catches anything. `compile_to_rust` returns `Ok` for a program it has REFUSED: the refusal is
/// emitted INTO the Rust as a `compile_error!` that fires when cargo builds it. So "the compiler
/// returned Ok" and "the program builds" are different facts, and the first one was true of every
/// route this campaign ever wrote while the second was true of none of them. An Ok-only assertion
/// is exactly the "looks wired up" test this campaign keeps finding.
fn builds(src: &str) -> String {
    let rust =
        compile_to_rust(src).unwrap_or_else(|e| panic!("must compile to AOT-Rust: {e:?}\n{src}"));
    let refusals = rust.matches("compile_error!").count();
    assert_eq!(
        refusals,
        0,
        "the generated Rust carries {refusals} `compile_error!` — it would not build:\n{}\n{src}",
        rust.lines()
            .filter(|l| l.contains("compile_error!"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    rust
}

/// The answer the reference tier and the VM give, held equal to each other.
fn answer(src: &str) -> String {
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "tree-walker:\n{src}");
    let vm = vm_outcome(src);
    assert_eq!(vm.error, None, "VM:\n{src}");
    assert_eq!(
        tw.output.trim_end(),
        vm.output.trim_end(),
        "tree-walker vs VM:\n{src}"
    );
    tw.output.trim_end().to_string()
}

/// Compiles on the deployed tier AND answers the same thing on the other two.
fn holds(src: &str, expected: &str) {
    builds(src);
    assert_eq!(answer(src), expected, "{src}");
}

const TODO: &str = "## A Todo has:\n    an id, which is Int.\n    a text, which is Text.\n    a done, which is Bool.\n\n";

#[test]
fn a_route_answering_text_builds() {
    // The shape that always worked — the handler's declared `Text` happened to be right. It is the
    // control: everything below changes what that declaration is.
    let src = "## Route GET /ping -> Text:\n    Return \"pong\".\n\n## Main\nShow route(\"GET\", \"/ping\").\n";
    holds(src, "pong");
}

#[test]
fn a_route_answering_a_record_builds() {
    // ★ The handler answers a `Todo`; the dispatcher encodes it. Declaring the handler `-> Text`
    // made the two disagree about what the function even returns.
    let src = format!(
        "{TODO}## Route GET /api/todo -> Todo:\n\
        \x20   Return a new Todo with id 7 and text \"milk\" and done false.\n\
         \n\
         ## Main\n\
         Show route(\"GET\", \"/api/todo\").\n"
    );
    holds(&src, "{\"id\":7,\"text\":\"milk\",\"done\":false}");
}

#[test]
fn a_route_answering_a_list_of_records_builds() {
    // ★ THE HEADLINE — the plan's own example, and the shape every collection endpoint has.
    let src = format!(
        "{TODO}## Route GET /api/todos -> Seq of Todo:\n\
        \x20   Return [a new Todo with id 1 and text \"milk\" and done false].\n\
         \n\
         ## Main\n\
         Show route(\"GET\", \"/api/todos\").\n"
    );
    holds(&src, "[{\"id\":1,\"text\":\"milk\",\"done\":false}]");
}

#[test]
fn an_empty_list_answer_builds_and_is_an_empty_array() {
    // An empty answer is a VALUE the endpoint has, not a missing one — and its handler's declared
    // type is the only thing that says what it is.
    let src = format!(
        "{TODO}## Route GET /api/todos -> Seq of Todo:\n\
        \x20   Return a new Seq of Todo.\n\
         \n\
         ## Main\n\
         Show route(\"GET\", \"/api/todos\").\n"
    );
    holds(&src, "[]");
}

#[test]
fn a_route_calling_a_helper_that_returns_the_list_builds() {
    // The real shape: the handler delegates, so the declared type has to agree with a SECOND
    // function's return type rather than with a literal sitting in the body.
    let src = format!(
        "{TODO}## To all () -> Seq of Todo:\n\
        \x20   Return [a new Todo with id 2 and text \"eggs\" and done true].\n\
         \n\
         ## Route GET /api/todos -> Seq of Todo:\n\
        \x20   Return all().\n\
         \n\
         ## Main\n\
         Show route(\"GET\", \"/api/todos\").\n"
    );
    holds(&src, "[{\"id\":2,\"text\":\"eggs\",\"done\":true}]");
}

#[test]
fn a_typed_path_parameter_still_binds_with_a_record_answer() {
    // The parameter machinery and the return machinery meet in one function; a fix to one that
    // broke the other would pass every test above.
    let src = format!(
        "{TODO}## Route GET /api/todos/:id (id: Int) -> Todo:\n\
        \x20   Return a new Todo with id id and text \"x\" and done false.\n\
         \n\
         ## Main\n\
         Show route(\"GET\", \"/api/todos/9\").\n"
    );
    holds(&src, "{\"id\":9,\"text\":\"x\",\"done\":false}");
}

#[test]
fn a_view_route_builds_and_still_answers_html() {
    // A `-> View` route's handler really does return Text — it renders the document — so it must
    // keep that declaration rather than inherit `View`.
    let src = "## Route GET /report -> View:\n    <h1>hi</h1>\n\n## Main\nShow route(\"GET\", \"/report\").\n";
    holds(src, "<h1>hi</h1>");
}

#[test]
fn a_miss_and_a_wrong_method_still_answer_on_the_deployed_tier() {
    // The dispatcher's own answers are Text whatever the routes return; a handler carrying a
    // non-Text type must not make the walk that contains it untypeable.
    let src = format!(
        "{TODO}## Route GET /api/todos -> Seq of Todo:\n\
        \x20   Return a new Seq of Todo.\n\
         \n\
         ## Main\n\
         Show routeStatus(\"GET\", \"/nope\").\n\
         Show routeStatus(\"POST\", \"/api/todos\").\n\
         Show routeStatus(\"GET\", \"/api/todos\").\n"
    );
    holds(&src, "404\n405\n200");
}

#[test]
fn several_routes_with_different_answer_types_build_together() {
    // ★ ONE WALK, MANY RETURN TYPES. The dispatcher calls every handler from the same function, so
    // handlers that no longer all return Text is exactly the case that could break its typing.
    let src = format!(
        "{TODO}## Route GET /ping -> Text:\n\
        \x20   Return \"pong\".\n\
         \n\
         ## Route GET /api/todo -> Todo:\n\
        \x20   Return a new Todo with id 1 and text \"a\" and done false.\n\
         \n\
         ## Route GET /api/todos -> Seq of Todo:\n\
        \x20   Return a new Seq of Todo.\n\
         \n\
         ## Main\n\
         Show route(\"GET\", \"/ping\").\n\
         Show route(\"GET\", \"/api/todo\").\n\
         Show route(\"GET\", \"/api/todos\").\n"
    );
    holds(&src, "pong\n{\"id\":1,\"text\":\"a\",\"done\":false}\n[]");
}

#[test]
fn the_handler_is_still_an_ordinary_function_with_a_real_body() {
    // Locals, a loop and a call out — a route body is a function body, and the declared return
    // type must not narrow what may appear in one.
    let src = format!(
        "{TODO}## Route GET /api/todos -> Seq of Todo:\n\
        \x20   Let mutable out be a new Seq of Todo.\n\
        \x20   Let mutable i be 1.\n\
        \x20   While i is at most 2:\n\
        \x20       Push (a new Todo with id i and text \"t\" and done false) to out.\n\
        \x20       Set i to i + 1.\n\
        \x20   Return out.\n\
         \n\
         ## Main\n\
         Show route(\"GET\", \"/api/todos\").\n"
    );
    holds(
        &src,
        "[{\"id\":1,\"text\":\"t\",\"done\":false},{\"id\":2,\"text\":\"t\",\"done\":false}]",
    );
}

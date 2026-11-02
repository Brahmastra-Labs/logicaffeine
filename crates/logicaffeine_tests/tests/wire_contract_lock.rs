//! ★ LOCK — the compiler checks the wire. ★
//!
//! A page says what it expects: `Get "/api/todos" as Seq of Todo, then loaded.`
//! A server says what it answers: `## Route GET /api/todos -> Seq of Todo:`
//!
//! Both sentences are in ONE program, in one language, in one compilation unit. The AST carries
//! everything needed to compare them —
//!
//! ```text
//! Stmt::Fetch    { method, url, as_type, then }
//! Stmt::RouteDef { method, path, params, return_type, body }
//! ```
//!
//! — and until this lock, NOTHING DID. A page fetching `as Seq of User` from a route answering
//! `Seq of Todo` compiled clean on every tier and failed at run time, in the browser, in front of
//! the person using it, as a decode error naming neither the route nor the page.
//!
//! ★ THIS IS THE THING ONE LANGUAGE BUYS. tRPC exists to do this for TypeScript. GraphQL codegen
//! exists to do this. Both are large machines for re-deriving a fact that, here, is a table
//! lookup — because the client and the server were never separate programs to begin with.
//!
//! ★ A URL NO ROUTE ANSWERS IS NOT AN ERROR. Most pages talk to something they do not compile:
//! a third-party API, a CDN, another service. Refusing those would make the check unusable and
//! would teach people to switch it off. The rule is narrow on purpose — **disagreement is an
//! error; absence is not.**
//!
//! ★ THE LOAD-BEARING TEST is `changing_a_routes_type_turns_the_page_red`. Every other case here
//! would pass against a check that ran once and was never wired to anything. The value is not
//! catching today's mismatch — it is that tomorrow's rename of a field, or a route's return type,
//! cannot silently leave a page decoding a shape the server stopped sending.

use logicaffeine_compile::compile::tw_outcome;

/// The shared record both halves of every fixture agree about.
const SHAPES: &str = "## A Todo has:\n\
    \x20   an id, which is Int.\n\
    \x20   a text, which is Text.\n\n\
    ## A User has:\n\
    \x20   a name, which is Text.\n\n";

/// A program whose route answers `answers` and whose page asks for `asks`.
fn program(answers: &str, asks: &str) -> String {
    format!(
        "{SHAPES}\
         ## Route GET /api/todos -> {answers}:\n\
         \x20   Return a new Seq of Todo.\n\n\
         ## Component App\n\
         State:\n\
         \x20   Let loaded be 0.\n\
         Actions:\n\
         \x20   When the page loads:\n\
         \x20       Get \"/api/todos\" as {asks}, then loaded.\n\
         \x20   To loaded (answer: Result of {asks} and HttpError):\n\
         \x20       Set loaded to 1.\n\
         View:\n\
         \x20   <p>hello</p>\n\n\
         ## Main\n\
         Mount App.\n"
    )
}

/// The two halves agreeing is the ordinary case and must stay silent.
#[test]
fn a_page_and_a_route_that_agree_compile() {
    let src = program("Seq of Todo", "Seq of Todo");
    assert_eq!(
        tw_outcome(&src).error,
        None,
        "a page asking for exactly what its route answers must compile:\n{src}"
    );
}

/// ★ The whole point: a disagreement is refused, and the refusal NAMES BOTH SIDES.
#[test]
fn a_page_expecting_a_different_type_is_refused_naming_both_sides() {
    let src = program("Seq of Todo", "Seq of User");
    let err = tw_outcome(&src).error.unwrap_or_else(|| {
        panic!(
            "a page fetching `Seq of User` from a route answering `Seq of Todo` COMPILED. Both \
             declarations are in this one program; nothing compared them, so the mistake ships \
             and surfaces in a browser as a decode failure naming neither side.\n{src}"
        )
    });
    assert!(
        err.contains("Todo") && err.contains("User"),
        "the refusal must name what the route ANSWERS and what the page ASKS FOR — a reader has \
         to know which of the two to change: {err}"
    );
    assert!(
        err.contains("/api/todos"),
        "the refusal must name the URL the two disagree about: {err}"
    );
}

/// A bare record, not just a `Seq`, so the check is about types rather than one container shape.
#[test]
fn a_single_record_mismatch_is_refused_too() {
    let src = program("Todo", "User");
    assert!(
        tw_outcome(&src).error.is_some(),
        "`-> Todo` answered into `as User` must be refused as surely as the list case:\n{src}"
    );
}

/// ★ ABSENCE IS NOT DISAGREEMENT. A page may talk to a service this program does not compile.
#[test]
fn a_url_no_route_answers_is_not_an_error() {
    let src = format!(
        "{SHAPES}\
         ## Component App\n\
         State:\n\
         \x20   Let loaded be 0.\n\
         Actions:\n\
         \x20   When the page loads:\n\
         \x20       Get \"https://api.example.com/v1/todos\" as Seq of Todo, then loaded.\n\
         \x20   To loaded (answer: Result of Seq of Todo and HttpError):\n\
         \x20       Set loaded to 1.\n\
         View:\n\
         \x20   <p>hello</p>\n\n\
         ## Main\n\
         Mount App.\n"
    );
    assert_eq!(
        tw_outcome(&src).error,
        None,
        "a third-party URL has no local route to disagree with. Refusing it would make the check \
         unusable and teach people to switch it off:\n{src}"
    );
}

/// The METHOD is part of the identity: a `Get` must not be checked against a `POST` route.
#[test]
fn the_method_is_part_of_the_match() {
    let src = format!(
        "{SHAPES}\
         ## Route POST /api/todos -> Seq of User:\n\
         \x20   Return a new Seq of User.\n\n\
         ## Component App\n\
         State:\n\
         \x20   Let loaded be 0.\n\
         Actions:\n\
         \x20   When the page loads:\n\
         \x20       Get \"/api/todos\" as Seq of Todo, then loaded.\n\
         \x20   To loaded (answer: Result of Seq of Todo and HttpError):\n\
         \x20       Set loaded to 1.\n\
         View:\n\
         \x20   <p>hello</p>\n\n\
         ## Main\n\
         Mount App.\n"
    );
    assert_eq!(
        tw_outcome(&src).error,
        None,
        "a GET has no POST route to disagree with — matching on the path alone would accuse a \
         page that is perfectly correct:\n{src}"
    );
}

/// ★★ THE LOAD-BEARING TEST — the check is not a one-time audit, it is a ratchet. The same page,
/// unchanged, must go RED the moment the server it talks to starts answering something else.
#[test]
fn changing_a_routes_type_turns_the_page_red() {
    let before = program("Seq of Todo", "Seq of Todo");
    assert_eq!(
        tw_outcome(&before).error,
        None,
        "the starting state must compile"
    );

    let after = program("Seq of User", "Seq of Todo");
    assert!(
        tw_outcome(&after).error.is_some(),
        "the PAGE did not change and the ROUTE did. That is the shape of every real wire break — \
         somebody edits the server and every client that decoded the old shape is now wrong. If \
         this compiles, the check is documentation rather than a guarantee.\n{after}"
    );
}

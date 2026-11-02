//! ★ LOCK — `html of <Component>` that cannot be server-rendered says SO, by name. ★
//!
//! A server draws a subset of the view tree, and it is right to: *"a page rendered short is worse
//! than one refused, since the reader sees a plausible document and nothing anywhere says a piece
//! is missing."* So the server render of a component asks the server gate
//! (`view_html_lowerable_with`) first, and a construct it has no rendering for is refused as a
//! sentence about the author's own program — naming the component, the construct and `html of` —
//! never as a missing function in generated code the author cannot see. A `## Route … -> View:`
//! template asks the same gate and explains itself the same way.
//!
//! ★ THE GATE IS `html of`, NOT THE VIEW. A component whose view the server cannot draw is
//! perfectly legal in the BROWSER — that is most of what a browser is for — so a page that only
//! `Mount`s it must keep working untouched. The refusal fires when somebody asks for its HTML.

use logicaffeine_compile::compile::{compile_to_rust, tw_outcome};
use logicaffeine_compile::semantics::dom;

/// A component whose view uses `body`, and a `## Main` that does `what`.
fn program(body: &str, what: &str) -> String {
    format!(
        "## Component App\nState:\n\x20   Let h be \"<b>hi</b>\".\n\x20   Let n be 1.\n\
         View:\n\x20   {body}\n\n## Main\n{what}\n"
    )
}

fn failure(subject: &str, src: &str) -> String {
    dom::reset();
    tw_outcome(src)
        .error
        .unwrap_or_else(|| panic!("`{subject}` should have been refused:\n{src}"))
}

/// Every refusal here must read like a sentence about the author's own program.
///
/// `names` is the component the refusal is ABOUT, which is not always the one `html of` asked for:
/// a wrapper is refused because of the CHILD whose view the server cannot draw, and naming the
/// child is what sends the author to the line that has to change.
fn assert_explains_about(subject: &str, names: &str, err: &str) {
    assert!(
        !err.contains("Unknown function"),
        "`{subject}` still fails as generated code the author cannot see: {err}"
    );
    assert!(
        err.contains(names),
        "`{subject}` must name the component the author wrote: {err}"
    );
    assert!(
        err.contains("html of"),
        "`{subject}` must name the thing that asked for it, `html of`: {err}"
    );
    assert!(
        err.len() > 60,
        "`{subject}` must say what to do about it, not just that it happened: {err}"
    );
}

/// The common case: the refusal is about the component `html of` named.
fn assert_explains(subject: &str, err: &str) {
    assert_explains_about(subject, "App", err);
}

#[test]
fn a_raw_block_refuses_by_name_rather_than_as_an_unknown_function() {
    let err = failure("raw", &program("<div>{raw h}</div>", "Show html of App."));
    assert_explains("raw", &err);
}

#[test]
fn a_directive_refuses_the_same_way() {
    // ★ NOT A `{raw}` FIX. Any node or attribute the server walk does not cover reaches this, and a
    // fix that only knew about `{raw}` would leave the next one failing as an unknown function
    // again — which is the whole point of this second case.
    //
    // ★ THIS SUBJECT HAS MOVED TWICE, AND THE SECOND TIME TAUGHT THE RULE.
    //
    // It was `{inspect}` — which now renders, because `{inspect}` is `{if}` with a shape to match
    // on and `{if}` always rendered here. Then it was a WRAPPER component, on the reasoning that
    // filling a `{children}` slot is an inlining rather than a call. That closed too: a wrapper is
    // an instance now, its children rendered in the caller's scope and handed down, so
    // `<Card title="…">…</Card>` server-renders and runs in a browser.
    //
    // Both subjects were GAPS — things nobody had built yet — and a fixture built on a gap is a
    // bet against its own project. A directive is not a gap. `use:tooltip` is code that runs
    // against a live element: there is no element on a server and nothing to run it against, so
    // this refusal is a DECISION about what a server is, and no amount of work closes it.
    let err = failure(
        "directive",
        "## Directive tooltip (el: Element) and (text: Text):\n\
         \x20   Call setAttribute with el and \"title\" and text.\n\n\
         ## Component App\nState:\n\x20   Let hint be \"delete this\".\n\
         View:\n\x20   <button tooltip={hint}>x</button>\n\n## Main\nShow html of App.\n",
    );
    assert_explains("directive", &err);
}

#[test]
fn the_refusal_names_the_construct_that_caused_it() {
    // The reason was always computed — this is the half that gets it in front of the author.
    let err = failure("raw", &program("<div>{raw h}</div>", "Show html of App."));
    let lower = err.to_lowercase();
    assert!(
        lower.contains("raw") || lower.contains("server"),
        "the refusal must say WHICH construct the server-side walk does not cover: {err}"
    );
}

#[test]
fn a_browser_only_component_is_untouched() {
    // ★ THE GATE. A view the server cannot draw is ordinary in a browser; only asking for its
    // HTML is the mistake. Mounting it must keep working exactly as it did.
    dom::reset();
    let src = program("<div>{raw h}</div>", "Mount App.");
    let out = tw_outcome(&src);
    assert_eq!(
        out.error, None,
        "a browser-only component must still mount:\n{src}"
    );
}

#[test]
fn a_server_renderable_component_still_renders() {
    // The floor.
    dom::reset();
    let src = program("<div>{n}</div>", "Show html of App.");
    let out = tw_outcome(&src);
    assert_eq!(out.error, None, "{src}");
    assert_eq!(out.output.trim(), "<div>1</div>");
}

#[test]
fn the_deploy_tier_refuses_it_the_same_way() {
    // AOT-Rust is where a server is actually built, so the explanation has to survive to it —
    // and `Ok` proves nothing, a refused program comes back as `Ok` with `compile_error!` inside.
    let src = program("<div>{raw h}</div>", "Show html of App.");
    match compile_to_rust(&src) {
        Ok(rust) => {
            let refused: Vec<&str> = rust
                .lines()
                .filter(|l| l.contains("compile_error!"))
                .collect();
            assert!(
                !refused.is_empty(),
                "the deploy tier must refuse it rather than emit a call to a function that does \
                 not exist:\n{rust}"
            );
            assert!(
                !refused.join("\n").contains("Unknown function"),
                "and must explain itself: {}",
                refused.join("\n")
            );
        }
        Err(e) => {
            let msg = format!("{e:?}");
            assert_explains("the deploy tier", &msg);
        }
    }
}

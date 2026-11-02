//! ★ LOCK — a `-> View` macro either RENDERS on the browser tier, or is REFUSED by name. ★
//!
//! `## To badge (label: Text) -> View:` is a macro whose value is a view, and placing one in a hole
//! draws the template's nodes there, in the scope of the component placing it. Every tier reaches
//! it the same way: the lowering makes the template DATA (a variant of the program's `__View` enum,
//! `parser::lgx_lower::views`), and placing that value draws its markup through the retained tree.
//!
//! ★ THIS IS THE ONE BOUNDARY IN LGX THAT FAILS **OPEN**. Every other unlowered construct is
//! refused by name — `lowerable()` and `nodes_lowerable()` are an allowlist precisely because a
//! denylist once let `.enter` through and then never honoured it. A `-> View` macro slips past
//! both: the compiler says nothing, the page loads, and the visitor sees a stringified value
//! where the author put markup. `view_value_lock` only ever drives the tree-walker, so nothing
//! was standing on this square.
//!
//! A wrong page in a browser is worse than a build that stops, so this lock accepts EITHER a
//! faithful render OR a refusal that names the macro, and fails only on the silent wrong answer.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::semantics::dom;

const MACRO_PAGE: &str = "\
## To badge (label: Text) -> View:
    <span class=\"badge\">{label}</span>

## Component App
View:
    <p>{badge(\"done\")}</p>

## Main
Mount App.
";

fn tw_render(src: &str) -> String {
    dom::reset();
    let out = tw_outcome(src);
    assert_eq!(
        out.error, None,
        "the reference tier must run it cleanly:\n{src}"
    );
    dom::render()
}

#[test]
fn the_reference_tier_places_the_macros_nodes() {
    // The contract this lock holds the other tier to. If this ever changes, the expectation
    // below is wrong rather than the browser tier.
    assert_eq!(
        tw_render(MACRO_PAGE),
        "<body><p><span class=\"badge\">done</span></p></body>"
    );
}

#[test]
fn the_browser_tier_either_renders_the_macro_or_refuses_it_by_name() {
    let reference = tw_render(MACRO_PAGE);

    dom::reset();
    let out = vm_outcome(MACRO_PAGE);

    if let Some(err) = out.error {
        // A refusal is acceptable — but it must NAME the thing, or an author cannot act on it.
        assert!(
            err.contains("badge") || err.contains("View"),
            "a refusal has to name the macro or the construct, not just fail; said: {err:?}"
        );
        return;
    }

    let rendered = dom::render();
    assert_eq!(
        rendered, reference,
        "a `-> View` macro was neither placed nor refused: the compiled tier rendered a \
         STRINGIFIED value where the author wrote markup, and said nothing. That is the one \
         failure mode LGX's allowlist exists to prevent — a page that is wrong in a browser \
         while its tests pass on the tier the author runs."
    );
}

// ─── A plain function call in a hole — distinct from a `-> View` macro. ───

/// `examples/fullstack_todo/README.md` told readers that "WASM string interpolation can show a
/// plain variable but cannot call a function", and shaped the demo around it: its rows print a
/// raw `id|text` instead of splitting the id off for display. That claim is the reason the
/// example looks the way it does, so whether it is still true is worth a test rather than a
/// sentence — a README is where a limitation goes to outlive the code.
#[test]
fn a_view_hole_may_call_an_ordinary_function_on_the_browser_tier() {
    let src = "\
## To shout (t: Text) -> Text:
    Return upper(t).

## Component App
State:
    Let name be \"ada\".
View:
    <p id=\"out\">{shout(name)}</p>

## Main
Mount App.
";
    // The reference tier is the contract.
    assert_eq!(tw_render(src), "<body><p id=\"out\">ADA</p></body>");

    // And the browser tier must agree, or refuse in words — never quietly print something else.
    dom::reset();
    let out = vm_outcome(src);
    assert_eq!(
        out.error, None,
        "a hole calling an ordinary function must reach the browser tier: {:?}",
        out.error
    );
    assert_eq!(
        dom::render(),
        "<body><p id=\"out\">ADA</p></body>",
        "and produce the same document the reference tier does"
    );

    // The whole point of the claim was about WASM specifically, so compile it.
    logicaffeine_compile::compile::compile_to_wasm(src)
        .expect("a view hole calling a function compiles to wasm");
}

//! ★ LOCK — the renderers of one view produce one document. ★
//!
//! A view reaches a reader three ways: served (`html of <C>`, what a crawler and a reader with no
//! script receive), painted (`Mount`, what the page becomes) and pre-rendered (what the static build
//! ships). All three are the one view walk writing tape records, and the served and pre-rendered
//! markup is the retained tree those records build, serialized by `vdSerializeChildren` — the
//! serializer `vdom_lg_lock` holds to the document. So the served bytes and the painted bytes are
//! compared here RAW, over a corpus chosen to hit every character an escaper could treat differently
//! and every element shape a serializer can get wrong: a quote in text, a void element, a boolean
//! attribute, control flow.

use logicaffeine_compile::compile::tw_outcome;
use logicaffeine_compile::semantics::dom;

/// The markup the SERVER walk produces for `view` — what ships before any script runs.
fn served(subject: &str, state: &str, view: &str) -> String {
    dom::reset();
    let src =
        format!("## Component App\nState:\n{state}View:\n{view}\n\n## Main\nShow html of App.\n");
    let out = tw_outcome(&src);
    assert_eq!(
        out.error, None,
        "`{subject}` must render server-side:\n{src}"
    );
    out.output.trim().to_string()
}

/// The markup the DOM recorder produces for the same view — what the page becomes.
fn painted(subject: &str, state: &str, view: &str) -> String {
    dom::reset();
    let src = format!("## Component App\nState:\n{state}View:\n{view}\n\n## Main\nMount App.\n");
    let out = tw_outcome(&src);
    assert_eq!(
        out.error, None,
        "`{subject}` must render into a document:\n{src}"
    );
    let html = dom::render();
    html.strip_prefix("<body>")
        .and_then(|h| h.strip_suffix("</body>"))
        .unwrap_or(&html)
        .to_string()
}

/// Both renderers, compared byte for byte.
fn agree(subject: &str, state: &str, view: &str) {
    let s = served(subject, state, view);
    let p = painted(subject, state, view);
    assert_eq!(
        s, p,
        "`{subject}` renders differently on the two tiers.\n  served:  {s}\n  painted: {p}\n\
         One view has one document; a difference here is a hydration mismatch and a cross-tier \
         comparison that can no longer be trusted."
    );
}

/// Every character an escaper could treat differently, in one value.
const AWKWARD: &str = "a\\\"b&c<d>e'f`g";

#[test]
fn a_text_hole_escapes_the_same_on_both_tiers() {
    // A quote in text is the character a text escaper and an attribute escaper treat differently.
    agree(
        "a text hole",
        &format!("\x20   Let s be \"{AWKWARD}\".\n"),
        "    <p>{s}</p>",
    );
}

#[test]
fn an_attribute_hole_escapes_the_same_on_both_tiers() {
    agree(
        "an attribute hole",
        &format!("\x20   Let s be \"{AWKWARD}\".\n"),
        "    <p title={s}>x</p>",
    );
}

#[test]
fn a_hole_in_both_positions_at_once_agrees() {
    agree(
        "text and attribute together",
        &format!("\x20   Let s be \"{AWKWARD}\".\n"),
        "    <p title={s}>{s}</p>",
    );
}

#[test]
fn each_awkward_character_agrees_on_its_own() {
    // ★ ONE AT A TIME, so a failure names the character rather than the soup. A combined fixture
    // that passes can still hide a pair of bugs that cancel.
    for (name, ch) in [
        ("ampersand", "&"),
        ("less-than", "<"),
        ("greater-than", ">"),
        ("quote", "\\\""),
        ("apostrophe", "'"),
        ("backtick", "`"),
        ("equals", "="),
        ("slash", "/"),
        ("non-ascii", "café → 世界 🎉"),
        ("an entity that is already one", "&amp;"),
        ("a tag that is not one", "<b>hi</b>"),
    ] {
        let state = format!("\x20   Let s be \"x{ch}y\".\n");
        agree(
            &format!("a text hole holding {name}"),
            &state,
            "    <p>{s}</p>",
        );
        agree(
            &format!("an attribute hole holding {name}"),
            &state,
            "    <p title={s}>z</p>",
        );
    }
}

#[test]
fn a_literal_in_the_template_agrees_too() {
    // The other escaper: markup the AUTHOR typed, which passes through a different path from a
    // hole's value on both tiers.
    agree(
        "a literal ampersand",
        "\x20   Let n be 1.\n",
        "    <p>Tom &amp; Jerry {n}</p>",
    );
}

#[test]
fn nested_elements_and_void_elements_agree() {
    // The shape `void_element_serialize_lock` was written about — held here as part of the whole
    // document rather than as one substring check.
    agree(
        "a nested tree with a void element",
        "\x20   Let n be 2.\n",
        "    <div class=\"card\"><ul><li>{n}</li></ul><input type=\"text\"><br></div>",
    );
}

#[test]
fn a_boolean_attribute_agrees() {
    agree(
        "a boolean attribute",
        "\x20   Let on be true.\n",
        "    <button disabled={on}>go</button>",
    );
    agree(
        "a boolean attribute that is off",
        "\x20   Let on be false.\n",
        "    <button disabled={on}>go</button>",
    );
}

#[test]
fn control_flow_agrees() {
    agree(
        "an if and a for each",
        "\x20   Let xs be [\"a\", \"b\"].\n\x20   Let show be true.\n",
        "    <ul>{if show:}{for each x in xs:}<li>{x}</li>{end}{end}</ul>",
    );
}

#[test]
fn the_comparison_can_fail() {
    // ★ ANTI-VACUITY. If `served` and `painted` both returned the same constant — an empty string,
    // say, because neither program actually rendered — every assertion above would pass. Prove the
    // two functions produce real, DIFFERENT documents for different views.
    let a = served("a", "\x20   Let n be 1.\n", "    <p>{n}</p>");
    let b = served("b", "\x20   Let n be 2.\n", "    <p>{n}</p>");
    assert_ne!(a, b, "the server renderer must actually render");
    assert!(a.contains("<p>1</p>"), "and produce the document: {a}");
    let c = painted("c", "\x20   Let n be 1.\n", "    <p>{n}</p>");
    assert!(c.contains("<p>1</p>"), "and so must the DOM recorder: {c}");
}

#[test]
fn both_renderers_build_on_the_tier_a_server_ships_as() {
    // AOT-Rust is the only tier that runs the static type checker, and the serializer is what a
    // built binary serves through — so the escaping this file pins has to survive to it. `Ok`
    // proves nothing: a refused program comes back as `Ok` with `compile_error!` inside.
    use logicaffeine_compile::compile::compile_to_rust;
    let src = format!(
        "## Component App\nState:\n\x20   Let s be \"{AWKWARD}\".\n\
         View:\n\x20   <p title={{s}}>{{s}}</p>\n\n## Main\nShow html of App.\n"
    );
    let rust = compile_to_rust(&src).unwrap_or_else(|e| panic!("must compile: {e:?}\n{src}"));
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
        rust.contains("vdSerializeChildren") && rust.contains("fn vdEscape("),
        "and the serializer and its escaper reach the emitted program — the served bytes are the \
         document's"
    );
}

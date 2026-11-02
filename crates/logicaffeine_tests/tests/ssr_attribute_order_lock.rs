//! LOCK — the server walk (`html of <C>`) serializes an element's attributes in the order the
//! reference document does: by name, whatever mix of literal, interpolated, boolean, bound and
//! merged-class attributes the element carries.
//!
//! A DOM node keeps its attributes in a `BTreeMap` (`semantics::dom`), so `dom::render()` is
//! alphabetical. The server walk folds a LITERAL attribute into the markup at compile time and emits
//! a run-time one where it is decided, and it merges `class="…"`, every `class:x={…}` and a scope
//! class into ONE `class` attribute — every row below mixes those kinds so that an attribute of one
//! kind has to land between attributes of another. Each row compares the served markup against the
//! painted document byte for byte (`renderer_parity_lock`'s method, including its one boolean
//! normalization), with toggles both on and off.

use logicaffeine_compile::compile::tw_outcome;
use logicaffeine_compile::semantics::dom;

fn served(state: &str, view: &str, style: &str) -> String {
    dom::reset();
    let src = format!(
        "## Component App\nState:\n{state}View:\n{view}\n{style}\n## Main\nShow html of App.\n"
    );
    let out = tw_outcome(&src);
    assert_eq!(out.error, None, "must render server-side:\n{src}");
    out.output.trim().to_string()
}

fn painted(state: &str, view: &str, style: &str) -> String {
    dom::reset();
    let src =
        format!("## Component App\nState:\n{state}View:\n{view}\n{style}\n## Main\nMount App.\n");
    let out = tw_outcome(&src);
    assert_eq!(out.error, None, "must render into a document:\n{src}");
    let html = dom::render();
    html.strip_prefix("<body>")
        .and_then(|h| h.strip_suffix("</body>"))
        .unwrap_or(&html)
        .to_string()
}

/// A bare boolean attribute (`disabled`) and `disabled=""` are the same document.
fn normalize(html: &str) -> String {
    use logicaffeine_language::lgx::html::is_boolean_attribute;
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(at) = rest.find("=\"\"") {
        let (head, tail) = rest.split_at(at);
        let name: String = head
            .chars()
            .rev()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect();
        let name: String = name.chars().rev().collect();
        out.push_str(head);
        if name.is_empty() || !is_boolean_attribute(&name) {
            out.push_str("=\"\"");
        }
        rest = &tail["=\"\"".len()..];
    }
    out.push_str(rest);
    out
}

fn agree(subject: &str, state: &str, view: &str, style: &str) {
    let s = normalize(&served(state, view, style));
    let p = normalize(&painted(state, view, style));
    assert_eq!(s, p, "`{subject}`: the served element's attributes are not in the document's order.\n  served:  {s}\n  painted: {p}");
}

const STATE: &str = "\x20   Let t be \"tip\".\n\x20   Let u be \"/next\".\n\x20   Let k be \"row\".\n\x20   Let on be true.\n\x20   Let off be false.\n";

#[test]
fn a_literal_class_between_two_interpolated_attributes() {
    agree(
        "alt < class < title",
        STATE,
        "    <img alt={t} class=\"card\" title={t}>",
        "",
    );
    agree(
        "authored backwards",
        STATE,
        "    <img title={t} class=\"card\" alt={t}>",
        "",
    );
}

#[test]
fn a_merged_class_between_literal_and_interpolated_attributes() {
    for flag in ["on", "off"] {
        agree(
            &format!("toggle {flag}"),
            STATE,
            &format!("    <a href={{u}} class=\"row\" class:done={{{flag}}} data-x=\"1\" aria-label={{t}}>x</a>"),
            "",
        );
    }
}

#[test]
fn toggles_alone_merge_at_the_class_position() {
    for flag in ["on", "off"] {
        agree(
            &format!("toggles only, {flag}"),
            STATE,
            &format!("    <p id={{t}} class:a={{{flag}}} class:b={{on}} title=\"z\" aria-hidden=\"true\">x</p>"),
            "",
        );
    }
}

#[test]
fn an_interpolated_class_merges_with_its_toggles_in_order() {
    agree(
        "interpolated base",
        STATE,
        "    <li title={t} class=\"{k} item\" class:hot={on} data-k={k}>x</li>",
        "",
    );
}

#[test]
fn a_boolean_attribute_keeps_its_place_on_and_off() {
    for flag in ["on", "off"] {
        agree(
            &format!("disabled {flag}"),
            STATE,
            &format!("    <button type=\"button\" disabled={{{flag}}} class=\"b\" aria-label={{t}}>go</button>"),
            "",
        );
    }
}

#[test]
fn a_bound_value_keeps_its_place_among_the_others() {
    agree(
        "bind",
        STATE,
        "    <input type=\"text\" bind={t} class=\"f\" aria-label={t} name=\"q\">",
        "",
    );
    agree(
        "bind checkbox",
        STATE,
        "    <input type=\"checkbox\" bind={on} aria-label={t} class:x={on} id=\"c\">",
        "",
    );
}

/// `checked` and a bound `value` are PROPERTIES on the reference, serialized after every attribute
/// (`checked` bare when on, `value` only when it holds something) — so an attribute whose name sorts
/// after `value` still comes before it.
#[test]
fn the_checked_and_value_properties_follow_every_attribute() {
    agree(
        "a bound value before a later-sorting attribute",
        STATE,
        "    <input bind={t} width=\"3\" type=\"text\" aria-label=\"q\">",
        "",
    );
    agree(
        "an empty bound value serves no value attribute",
        "\x20   Let t be \"\".\n",
        "    <input bind={t} width=\"3\" type=\"text\" aria-label=\"q\">",
        "",
    );
    agree(
        "a bound checkbox before a later-sorting attribute",
        STATE,
        "    <input type=\"checkbox\" bind={on} width=\"3\" aria-label=\"c\">",
        "",
    );
    agree(
        "a bound checkbox that is off",
        STATE,
        "    <input type=\"checkbox\" bind={off} width=\"3\" aria-label=\"c\">",
        "",
    );
    agree(
        "a written checked text",
        STATE,
        "    <input type=\"checkbox\" checked=\"yes\" width=\"3\" aria-label=\"c\">",
        "",
    );
    agree(
        "a written checked flag",
        STATE,
        "    <input type=\"checkbox\" checked={on} width=\"3\" aria-label=\"c\">",
        "",
    );
    agree(
        "a plain value attribute is an attribute",
        STATE,
        "    <input type=\"text\" value={t} width=\"3\" aria-label=\"v\">",
        "",
    );
}

#[test]
fn a_scoped_component_stamps_its_class_at_the_class_position() {
    agree(
        "scope class with dynamic neighbours",
        STATE,
        "    <p title={t} data-z={k} aria-label=\"a\">x</p>",
        "Style:\n    p { color: red; }\n",
    );
    agree(
        "scope class with a toggle",
        STATE,
        "    <p title={t} class:on={on} aria-label={t}>x</p>",
        "Style:\n    p { color: red; }\n",
    );
}

#[test]
fn the_comparison_sees_an_attribute_order_difference() {
    assert_ne!(
        normalize("<img alt=\"a\" class=\"c\" title=\"t\">"),
        normalize("<img class=\"c\" alt=\"a\" title=\"t\">"),
        "two orders of the same attributes are two different byte strings, and this file compares bytes"
    );
    let painted = painted(STATE, "    <img title={t} class=\"card\" alt={t}>", "");
    assert_eq!(
        painted, "<img alt=\"tip\" class=\"card\" title=\"tip\">",
        "the document is alphabetical"
    );
}

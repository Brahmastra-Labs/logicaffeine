//! LOCK — three refusals that used to name their reason correctly and locate it at the wrong
//! place: a derived-state cycle pointed at the component header instead of the field that starts
//! the circle; `html of <Component>` pointed at the component instead of the construct the
//! server-side walk actually refused; a fetch whose type disagrees with the route that answers it
//! reported `Span::default()` (line 1) instead of the action that fetches, and its message had lost
//! its line-continuation backslashes to a run of raw spaces.

#[path = "language_common/mod.rs"]
mod common;

use common::refusal;

#[test]
fn a_use_naming_no_store_is_refused_at_the_sentence() {
    let src = "## Store Session\nState:\n    Let user be \"ada\".\n\n\
               ## Component App\nUse Nowhere.\nView:\n    <p>hi</p>\n\n## Main\nMount App.\n";
    let (message, covered) = refusal(src);
    assert!(
        message.contains("`Use Nowhere.` names no store"),
        "{message}"
    );
    assert!(
        message.contains("did you mean") == false,
        "`Nowhere` is not close to `Session`: {message}"
    );
    assert_eq!(
        covered, "Nowhere",
        "the span must cover the store name in the `Use` sentence: {message}"
    );
}

#[test]
fn a_misspelt_use_suggests_the_declared_store() {
    let src = "## Store Session\nState:\n    Let user be \"ada\".\n\n\
               ## Component App\nUse Sesion.\nView:\n    <p>hi</p>\n\n## Main\nMount App.\n";
    let (message, covered) = refusal(src);
    assert!(message.contains("did you mean `Session`?"), "{message}");
    assert_eq!(covered, "Sesion");
}

#[test]
fn a_derived_cycle_points_at_the_first_field_not_the_header() {
    let src = "## Component App\nState:\n    Let a always be b + 1.\n    Let b always be a + 1.\nView:\n    <p>{a}</p>\n\n## Main\nMount App.\n";
    let (message, covered) = refusal(src);
    assert!(
        message.contains("depend on each other in a circle"),
        "{message}"
    );
    assert!(
        covered.contains("Let a always"),
        "the span must cover the first field's own declaration, not `App`'s header: covered {covered:?}: {message}"
    );
}

#[test]
fn html_of_a_component_points_at_the_refused_construct() {
    let src = "## Component App\nState:\n    Let h be \"<b>hi</b>\".\nView:\n    <div>{raw h}</div>\n\n## Main\nShow html of App.\n";
    let (message, covered) = refusal(src);
    assert!(message.contains("html of App"), "{message}");
    assert_eq!(
        covered, "{raw h}",
        "the span must cover the construct the server walk refused, not `App`: {message}"
    );
}

#[test]
fn a_wire_type_mismatch_points_at_the_fetching_action() {
    let src = "## Route GET /api/todos -> Seq of Text:\n    Return [\"a\"].\n\n\
               ## Component App\nState:\n    Let n be 0.\n\
               Actions:\n    To load:\n        Get \"/api/todos\" as Seq of Int, then loaded.\n    \
               To loaded (answer: Result of Seq of Int and HttpError):\n        Set n to 1.\n\
               View:\n    <button id=\"load\" on-click={load}>Load</button>\n\n## Main\nMount App.\n";
    let (message, covered) = refusal(src);
    assert!(
        message.contains("this page fetches `/api/todos` as `Seq of Int`"),
        "{message}"
    );
    assert!(message.contains("answers `Seq of Text`"), "{message}");
    assert!(
        !message.contains("  "),
        "no run of raw spaces should remain where a line continued: {message:?}"
    );
    assert_eq!(
        covered, "To",
        "the span must cover the fetching action's own header, not line 1: {message}"
    );
}

//! ★ LOCK — a test asks the rendered page questions in ORDINARY LOGOS. ★
//!
//! `the text of "#draft"`, `the count of ".row"`, `the value of "#draft"`, `the title`,
//! `the URL` are expressions, not statements, so every assertion form LOGOS already has works
//! on them and a page test needs no vocabulary of its own. The selector engine behind them is
//! the CSS subset a page test needs — id, class, tag, attribute, and descendant — and it walks
//! the live document, so an element a re-render detached can never answer.

use logicaffeine_compile::compile::tw_outcome;
use logicaffeine_compile::semantics::dom;

fn output(src: &str) -> Vec<String> {
    dom::reset();
    let out = tw_outcome(src);
    assert_eq!(out.error, None, "the program must run cleanly:\n{src}");
    out.output.lines().map(str::to_string).collect()
}

fn failure(src: &str) -> String {
    dom::reset();
    tw_outcome(src)
        .error
        .unwrap_or_else(|| panic!("expected a failure:\n{src}"))
}

const PAGE: &str = "\
## Component App
State:
    Let name be \"ada\".
View:
    <div class=\"card\">
      <h1 id=\"who\">Hi {name}</h1>
      <ul>
        <li class=\"row done\" data-id=\"1\">first</li>
        <li class=\"row\" data-id=\"2\">second</li>
      </ul>
      <input id=\"draft\" bind={name}>
    </div>

## Main
Mount App.
";

#[test]
fn the_text_of_a_selector_is_its_whole_text_content() {
    assert_eq!(
        output(&format!("{PAGE}Show the text of \"#who\".\n")),
        ["Hi ada"]
    );
}

#[test]
fn the_count_of_a_selector_counts_every_match() {
    assert_eq!(
        output(&format!("{PAGE}Show the count of \".row\".\n")),
        ["2"]
    );
}

#[test]
fn the_count_of_a_collection_is_its_length() {
    // The same English, and the VALUE settles the reading: only a Text is a selector.
    let src = "## Main\nLet xs be [1, 2, 3].\nShow the count of xs.\n";
    assert_eq!(output(src), ["3"]);
}

#[test]
fn the_value_of_an_input_is_what_it_shows() {
    assert_eq!(
        output(&format!("{PAGE}Show the value of \"#draft\".\n")),
        ["ada"]
    );
}

#[test]
fn a_class_selector_narrows_to_the_elements_that_carry_it() {
    assert_eq!(
        output(&format!("{PAGE}Show the count of \".done\".\n")),
        ["1"]
    );
    assert_eq!(
        output(&format!("{PAGE}Show the text of \".done\".\n")),
        ["first"]
    );
}

#[test]
fn a_compound_selector_needs_every_part_to_hold() {
    assert_eq!(
        output(&format!("{PAGE}Show the count of \"li.row.done\".\n")),
        ["1"]
    );
    assert_eq!(
        output(&format!("{PAGE}Show the count of \"h1.row\".\n")),
        ["0"]
    );
}

#[test]
fn an_attribute_selector_matches_presence_and_equality() {
    assert_eq!(
        output(&format!("{PAGE}Show the count of \"[data-id]\".\n")),
        ["2"]
    );
    assert_eq!(
        output(&format!("{PAGE}Show the text of \"[data-id=2]\".\n")),
        ["second"]
    );
}

#[test]
fn a_descendant_selector_reads_left_to_right() {
    assert_eq!(
        output(&format!("{PAGE}Show the count of \".card li\".\n")),
        ["2"]
    );
    assert_eq!(
        output(&format!("{PAGE}Show the count of \"ul .card\".\n")),
        ["0"]
    );
}

#[test]
fn a_tag_selector_matches_every_element_with_that_tag() {
    assert_eq!(output(&format!("{PAGE}Show the count of \"li\".\n")), ["2"]);
}

#[test]
fn asking_a_selector_that_matches_nothing_says_so() {
    let msg = failure(&format!("{PAGE}Show the text of \"#missing\".\n"));
    assert!(
        msg.contains("#missing"),
        "the message names the selector: {msg}"
    );
    assert!(msg.contains("Mount"), "and says what to check: {msg}");
}

#[test]
fn counting_a_selector_that_matches_nothing_is_zero_not_an_error() {
    assert_eq!(
        output(&format!("{PAGE}Show the count of \"#missing\".\n")),
        ["0"]
    );
}

#[test]
fn the_title_is_what_the_page_was_told_to_show() {
    let src = "\
## Component App
State:
    Let n be 0.
Actions:
    When \"#go\" is clicked:
        Set the title to \"Todos\".
View:
    <button id=\"go\">x</button>

## Main
Mount App.
Show the title.
Click \"#go\".
Show the title.
";
    assert_eq!(output(src), ["", "Todos"]);
}

#[test]
fn a_page_query_composes_like_any_other_expression() {
    let src =
        format!("{PAGE}Show the count of \".row\" + 1.\nShow length of the text of \"#who\".\n");
    assert_eq!(output(&src), ["3", "6"]);
}

#[test]
fn a_re_render_detaches_the_old_element_so_only_the_live_one_answers() {
    let src = "\
## Component App
State:
    Let n be 0.
Actions:
    When \"#go\" is clicked:
        Set n to n + 1.
View:
    <p id=\"n\">{n}</p>
    <button id=\"go\">+</button>

## Main
Mount App.
Click \"#go\".
Show the text of \"#n\".
Show the count of \"#n\".
";
    assert_eq!(output(src), ["1", "1"]);
}

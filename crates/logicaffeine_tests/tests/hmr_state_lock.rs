//! ★ LOCK — an edit does not throw away what you typed. ★
//!
//! `largo serve` rebuilds a page when its file changes and the browser reloads. Without state
//! preservation every reload is a cold start: the form you were filling in is empty, the list you
//! had scrolled to is back at the top, the tab you had opened is closed. That turns the edit loop
//! into "change one line, set the page up again", which is the difference between a dev server
//! people use and one they work around.
//!
//! WHAT A RELOAD RESETS AND WHAT IT KEEPS is the whole design. It resets the DOCUMENT and the
//! page's own state — the page really is rebuilt, its program run again. It keeps the browser's
//! STORAGE, which is where preserved state lives. These tests simulate exactly that split rather
//! than a full reload, because the split IS the mechanism.

use logicaffeine_compile::compile::tw_outcome;
use logicaffeine_compile::semantics::{dom, world};

/// Run a program from cold: nothing preserved, nothing rendered before.
fn cold(src: &str) -> String {
    dom::reset();
    world::reset();
    let out = tw_outcome(src);
    assert_eq!(out.error, None, "the page must run:\n{src}");
    dom::render()
}

/// Run a program the way a HOT RELOAD does: the document and the page are rebuilt, the browser's
/// storage survives.
fn after_reload(src: &str) -> String {
    dom::reset();
    world::arm_hot_reload();
    let out = tw_outcome(src);
    assert_eq!(out.error, None, "the page must run after a reload:\n{src}");
    dom::render()
}

const PAGE: &str = "\
## Store Draft preserved across reloads
State:
    Let text be \"\".
Actions:
    To type (t: Text):
        Set text to t.

## Component App
Use Draft.
Actions:
    To fill:
        Call Draft's type with \"half-written\".
View:
    <p id=\"t\">{Draft's text}</p>
    <button id=\"go\" on-click={fill}>x</button>

## Main
Mount App.
";

#[test]
fn state_survives_a_reload() {
    let typed = cold(&format!("{PAGE}Click \"#go\".\n"));
    assert!(
        typed.contains("<p id=\"t\">half-written</p>"),
        "the page took the input: {typed}"
    );

    // The file changed; the page is rebuilt. What was typed must still be there.
    let reloaded = after_reload(PAGE);
    assert!(
        reloaded.contains("<p id=\"t\">half-written</p>"),
        "a reload threw away what was typed — this is what makes an edit loop unusable: {reloaded}"
    );
}

#[test]
fn a_cold_start_does_not_inherit_a_previous_sessions_state() {
    // The other half: preservation is for the EDIT LOOP, not a hidden global. A fresh run must
    // start from the page's initializers, or a test's state leaks into the next one and a
    // developer's stale draft outlives the session that made it.
    cold(&format!("{PAGE}Click \"#go\".\n"));
    let fresh = cold(PAGE);
    assert!(
        fresh.contains("<p id=\"t\"></p>"),
        "a cold start begins from the page's own initializers: {fresh}"
    );
}

#[test]
fn a_field_the_edited_page_no_longer_declares_is_dropped() {
    // The realistic edit: a field is REMOVED from the source. Restoring it would resurrect state
    // the program can no longer describe, and the page would carry a value nothing reads.
    cold(
        "\
## Store Draft preserved across reloads
State:
    Let text be \"\".
    Let gone be \"stale\".
Actions:
    To fill:
        Set text to \"kept\".

## Component App
Use Draft.
Actions:
    To go:
        Call Draft's fill.
View:
    <p id=\"t\">{Draft's text}</p>
    <button id=\"go\" on-click={go}>x</button>

## Main
Mount App.
Click \"#go\".
",
    );
    // The edited page has dropped `gone`.
    let reloaded = after_reload(
        "\
## Store Draft preserved across reloads
State:
    Let text be \"\".

## Component App
Use Draft.
View:
    <p id=\"t\">{Draft's text}</p>

## Main
Mount App.
",
    );
    assert!(
        reloaded.contains("<p id=\"t\">kept</p>"),
        "the field the page still declares survived: {reloaded}"
    );
}

#[test]
fn a_field_the_edited_page_adds_starts_from_its_initializer() {
    // The other realistic edit: a field is ADDED. There is nothing preserved for it, so it must
    // begin at what the source says rather than at nothing.
    cold(
        "\
## Store Draft preserved across reloads
State:
    Let text be \"\".

## Component App
Use Draft.
View:
    <p id=\"t\">{Draft's text}</p>

## Main
Mount App.
",
    );
    let reloaded = after_reload(
        "\
## Store Draft preserved across reloads
State:
    Let text be \"\".
    Let added be \"fresh\".

## Component App
Use Draft.
View:
    <p id=\"t\">{Draft's text}</p>
    <p id=\"a\">{Draft's added}</p>

## Main
Mount App.
",
    );
    assert!(
        reloaded.contains("<p id=\"a\">fresh</p>"),
        "a newly declared field starts from its initializer: {reloaded}"
    );
}

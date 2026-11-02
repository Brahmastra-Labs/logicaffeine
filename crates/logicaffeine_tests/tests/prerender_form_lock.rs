//! ★ LOCK — a page with a FORM carries its content, like every other page. ★
//!
//! `bind` and `class:` are on nearly every page worth serving: a form binds its fields, and a
//! toggle names its state in a class. Both were refused by the server-side walk with *"browser
//! behaviour and have no server-rendered form"*, which is not true of either — a `bind` has an
//! exact server rendering (the initial `value=`, or `checked` on a checkbox) and a `class:`
//! composes into the element's `class=` list the same way it does in the browser.
//!
//! The cost of the refusal was not a missing attribute. It was the WHOLE PAGE: a single `bind`
//! made `render_page_markup` refuse, so `largo build` shipped a `<body>` holding one `<script>`
//! — no heading, no form, no content — for most real pages. A reader with a slow connection saw
//! a blank document, and a crawler saw nothing at all.

use logicaffeine_cli::prerender_page_html;

/// The shape of a real form page: a text `bind`, a checkbox `bind`, a `class:` toggle composed
/// with a plain `class=`, and ordinary content around them.
const FORM_PAGE: &str = r#"## Component App

State:
    Let draft be "milk".
    Let done be true.
    Let urgent be false.

Actions:
    To add:
        Set draft to "".

View:
    <main>
      <h1>Groceries</h1>
      <form on-submit.prevent={add}>
        <input id="draft" type="text" bind={draft} placeholder="What next?">
        <input id="done" type="checkbox" bind={done}>
        <button type="submit">Add</button>
      </form>
      <p class="row" class:done={done} class:urgent={urgent}>one item</p>
    </main>

## Main

Mount App.
"#;

fn form_page() -> String {
    prerender_page_html(FORM_PAGE, "app")
}

#[test]
fn a_page_with_a_form_is_prerendered_at_all() {
    let html = form_page();
    assert!(
        html.contains("Groceries"),
        "a `bind` must not cost the page its content:\n{html}"
    );
    assert!(html.contains("<form"), "the form itself renders:\n{html}");
    assert!(
        html.contains("one item"),
        "and everything after it:\n{html}"
    );
}

#[test]
fn a_text_bind_renders_its_current_value() {
    // The point of pre-rendering a form is that a reader sees what is in it. A bound field whose
    // state says "milk" and whose markup says nothing is a form that loses its contents on every
    // slow load.
    let html = form_page();
    assert!(
        html.contains(r#"value="milk""#),
        "the bound field carries its state:\n{html}"
    );
}

#[test]
fn a_checkbox_bind_renders_presence_not_a_value() {
    // `checked` is a PRESENCE. `value="true"` draws an UNCHECKED box while the page believes it
    // is checked — the exact silent wrong answer `BoundInput` exists to prevent.
    let html = form_page();
    let form = &html[html.find("<form").unwrap()..html.find("</form>").unwrap()];
    assert!(form.contains("checked"), "the checkbox is checked:\n{form}");
    assert!(
        !form.contains(r#"checked="true""#) && !form.contains(r#"value="true""#),
        "and not as a string value:\n{form}"
    );
}

#[test]
fn a_class_toggle_composes_into_one_class_list() {
    // Two toggles and a literal share one attribute. Emitting three `class=` attributes would let
    // the last win and silently drop the others.
    let html = form_page();
    let p = &html[html.find("<p").unwrap()..];
    let p = &p[..p.find("</p>").unwrap()];
    assert_eq!(
        p.matches("class=").count(),
        1,
        "exactly one class attribute:\n{p}"
    );
    assert!(p.contains("row"), "the literal class survives:\n{p}");
    assert!(p.contains("done"), "a true toggle is present:\n{p}");
    assert!(!p.contains("urgent"), "a false toggle is absent:\n{p}");
}

#[test]
fn an_unbound_checkbox_is_not_checked() {
    let page = FORM_PAGE.replace("Let done be true.", "Let done be false.");
    let html = prerender_page_html(&page, "app");
    let form = &html[html.find("<form").unwrap()..html.find("</form>").unwrap()];
    assert!(
        !form.contains("checked"),
        "a false binding renders nothing:\n{form}"
    );
}

#[test]
fn the_page_still_loads_its_module() {
    // Pre-rendering precedes the app; it never replaces it. A form that renders and then never
    // becomes interactive is worse than one that arrives late.
    let html = form_page();
    assert!(html.contains("app.js"), "the module still loads:\n{html}");
}

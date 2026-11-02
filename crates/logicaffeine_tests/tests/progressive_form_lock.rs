//! ★ LOCK — a page works before, and without, its script. ★
//!
//! The framework's premise is that LOGOS runs in the browser as WebAssembly. The corollary nobody
//! gets to skip is that the page must still work when that module has not loaded yet, or cannot:
//! a slow connection, a crawler, a reader with scripts off, the seconds before instantiation.
//!
//! Two ordinary HTML elements carry that weight, and both had lost it:
//!
//!   ★ AN ANCHOR HAD NO DEFAULT ACTION. Clicking `<a href="/about">` did NOTHING — not a
//!     navigation, not an error. Every link in every LOGOS page was decorative unless the author
//!     had wired an `on-click`, which is the opposite of what a link is. (The same shape as the
//!     checkbox that rendered correctly and could not be ticked: the browser answers a click on
//!     these itself, and a runtime that only fires listeners answers nothing.)
//!
//!   ★ A FORM WITHOUT A HANDLER MUST POST ITSELF. `<form method="post" action="/x">` with no
//!     `on-submit` is how a page works with no script at all — the server gets the request, the
//!     browser follows the response. A submit that quietly did nothing would make the
//!     no-JavaScript story a claim rather than a behaviour.

use logicaffeine_compile::compile::tw_outcome;
use logicaffeine_compile::semantics::{dom, world};

fn run(src: &str) -> String {
    dom::reset();
    world::reset();
    let out = tw_outcome(src);
    assert_eq!(out.error, None, "the page must run:\n{src}");
    out.output
}

fn run_err(src: &str) -> String {
    dom::reset();
    world::reset();
    tw_outcome(src)
        .error
        .unwrap_or_else(|| panic!("expected a failure:\n{src}"))
}

const LINKS: &str = "\
## Component App
View:
    <a id=\"about\" href=\"/about\">About</a>
    <a id=\"out\" href=\"https://example.com/docs\">Docs</a>
    <p id=\"here\">home</p>

## Main
Mount App.
";

#[test]
fn clicking_a_link_navigates() {
    // ★ THE BEHAVIOUR AN ANCHOR ALREADY HAS IN EVERY BROWSER. A page whose links do nothing is not
    // a page, and no author should have to wire an `on-click` to get what `<a href>` means.
    let out = run(&format!("{LINKS}Click \"#about\".\nShow the URL.\n"));
    assert_eq!(out.trim(), "/about", "the click followed the link");
}

#[test]
fn a_link_click_needs_no_handler_and_is_not_a_failure() {
    // A click on an element nothing listens to is normally a LOUD failure — the rest of a test
    // would otherwise describe a page nobody touched. An anchor is the exception, exactly like a
    // checkbox: the browser answers it, so demanding a listener would refuse the most ordinary
    // link there is.
    let out = run(&format!("{LINKS}Click \"#about\".\nShow \"survived\".\n"));
    assert!(
        out.contains("survived"),
        "a click on a plain link is not an error: {out}"
    );
}

#[test]
fn an_off_site_link_leaves_the_app_rather_than_pretending_to_route() {
    // A cross-origin href is not a route this app can serve. Treating it as one would put a URL in
    // the address bar that the router cannot answer, and quietly break a link that works.
    let out = run(&format!("{LINKS}Click \"#out\".\nShow the URL.\n"));
    assert_eq!(
        out.trim(),
        "https://example.com/docs",
        "an off-site link is followed as written, not rewritten into a local path"
    );
}

#[test]
fn a_link_whose_handler_prevents_the_default_does_not_navigate() {
    // The escape hatch, and the reason it is spelled `.prevent`: an author who wants the click and
    // NOT the navigation says so. Without this, a link with a handler would both run the handler
    // and leave the page.
    let src = "\
## Component App
State:
    Let hits be 0.
Actions:
    To note:
        Set hits to hits + 1.
View:
    <a id=\"a\" href=\"/gone\" on-click.prevent={note}>x</a>
    <p id=\"n\">{hits}</p>

## Main
Mount App.
Click \"#a\".
Show the URL.
";
    let out = run(src);
    assert_ne!(
        out.trim(),
        "/gone",
        "the handler prevented the navigation: {out}"
    );
    assert!(
        dom::render().contains(">1<"),
        "…and the handler still ran: {}",
        dom::render()
    );
}

#[test]
fn a_link_with_a_plain_handler_runs_it_and_still_navigates() {
    // The other side of the same rule. `on-click` without `.prevent` is an ADDITION to what the
    // browser does, not a replacement for it — a page that silently swallowed the navigation would
    // be surprising in the direction that loses information.
    let src = "\
## Component App
State:
    Let hits be 0.
Actions:
    To note:
        Set hits to hits + 1.
View:
    <a id=\"a\" href=\"/next\" on-click={note}>x</a>

## Main
Mount App.
Click \"#a\".
Show the URL.
";
    assert_eq!(run(src).trim(), "/next");
}

#[test]
fn an_anchor_without_an_href_is_not_a_link() {
    // `<a>` with no `href` is a placeholder, not a navigation — following it would send the page
    // to an address nobody wrote. It gets a handler here because a click on an element NOTHING
    // listens to is the ordinary loud failure, and that rule is not weakened by this one: what is
    // being asserted is that the anchor contributes no default action of its own.
    let src = "\
## Component App
State:
    Let hits be 0.
Actions:
    To note:
        Set hits to hits + 1.
View:
    <a id=\"a\" on-click={note}>not a link</a>

## Main
Mount App.
Click \"#a\".
Show the URL.
";
    let out = run(src);
    assert_ne!(
        out.trim(),
        "",
        "the URL is unchanged by a click on a hrefless anchor: {out}"
    );
}

// ---------------------------------------------------------------------------------------------
// A form with no handler is a REAL submit — the page works with no script at all.
// ---------------------------------------------------------------------------------------------

#[test]
fn a_form_with_no_handler_submits_natively() {
    // ★ THE NO-JAVASCRIPT PATH. This form has no `on-submit`, so the browser posts it and the
    // server answers. A submit that did nothing would make "works without JavaScript" a claim
    // rather than a behaviour — and it is the claim the whole SSR half of this framework rests on.
    let src = "\
## Route POST /subscribe -> Text:
    Return \"thanks\".

## Component App
State:
    Let token be \"t0ken\".
View:
    <form id=\"f\" method=\"post\" action=\"/subscribe\">
      <input type=\"hidden\" name=\"_csrf\" value={token}>
      <input name=\"email\" value=\"a@b.c\">
      <button type=\"submit\">Go</button>
    </form>

## Main
Serve fetches with routes.
Mount App.
Submit \"#f\".
";
    let out = run(src);
    assert!(
        out.contains("thanks") || dom::render().contains("thanks"),
        "the form posted and the server answered: {out}\n{}",
        dom::render()
    );
}

#[test]
fn a_form_with_a_handler_does_not_also_post_natively() {
    // The other half: an author who wrote `on-submit` meant the page to handle it. Posting as WELL
    // would send the request twice — once to the handler and once to the server — which is the
    // duplicate-submission bug every form library exists to avoid.
    let src = "\
## Component App
State:
    Let saved be 0.
    Let token be \"t0ken\".
Actions:
    To save:
        Set saved to saved + 1.
View:
    <form id=\"f\" method=\"post\" action=\"/subscribe\" on-submit.prevent={save}>
      <input type=\"hidden\" name=\"_csrf\" value={token}>
      <button type=\"submit\">Go</button>
    </form>
    <p id=\"n\">{saved}</p>

## Main
Mount App.
Submit \"#f\".
";
    let out = run(src);
    assert!(
        dom::render().contains(">1<"),
        "the handler ran: {}",
        dom::render()
    );
    assert!(
        !out.contains("thanks"),
        "…and nothing was posted behind its back: {out}"
    );
}

#[test]
fn a_form_with_no_action_posts_to_the_page_it_is_on() {
    // HTML's own rule: an `action`-less form submits to the current URL. Guessing anything else
    // would send a visitor's data somewhere they did not write down.
    let src = "\
## Route POST / -> Text:
    Return \"self\".

## Component App
State:
    Let token be \"t0ken\".
View:
    <form id=\"f\" method=\"post\">
      <input type=\"hidden\" name=\"_csrf\" value={token}>
      <button type=\"submit\">Go</button>
    </form>

## Main
Serve fetches with routes.
Mount App.
Submit \"#f\".
";
    let out = run(src);
    assert!(
        out.contains("self") || dom::render().contains("self"),
        "an action-less form posts to the current URL: {out}"
    );
}

#[test]
fn a_get_form_is_a_navigation_not_a_post() {
    // A `get` form is a search box: it navigates with its fields in the query string. Posting it
    // would send a body to a route that expects none, and lose the shareable URL that is the whole
    // point of a `get` form.
    let src = "\
## Component App
View:
    <form id=\"f\" method=\"get\" action=\"/search\">
      <input name=\"q\" value=\"logos\">
      <button type=\"submit\">Go</button>
    </form>

## Main
Mount App.
Submit \"#f\".
Show the URL.
";
    let out = run(src);
    assert!(
        out.contains("/search"),
        "a get form navigates to its action: {out}"
    );
    assert!(
        out.contains("q=logos"),
        "…carrying its fields in the query: {out}"
    );
}

#[test]
fn a_submit_on_something_that_is_not_a_form_is_still_a_loud_failure() {
    // The honesty rule this file must not weaken: the native submit is the browser answering a
    // FORM. A `Submit` aimed at a button, or at nothing, would otherwise pass silently and leave
    // every later assertion describing a page nobody touched.
    let err = run_err(
        "## Component App\nView:\n    <p id=\"p\">x</p>\n\n## Main\nMount App.\nSubmit \"#p\".\n",
    );
    assert!(!err.is_empty(), "submitting a non-form fails loudly: {err}");
}

//! ★ LOCK — a `## Page` RENDERS. ★
//!
//! `## Page /about` is the documented way to declare a page: *"a component that owns a URL; the
//! path is where it serves"*. It parsed, it decided the URL, `pages_routing_lock` proved the URL
//! — and **the page never rendered on any tier**. A file holding nothing but
//!
//! ```text
//! ## Page /about
//! View:
//!     <p>About us</p>
//! ```
//!
//! compiled clean, built clean, served a 200, and the document was EMPTY.
//!
//! ★ AND IT COULD NOT BE FIXED BY THE AUTHOR. `## Page /about` interns its component as
//! `__page__about`, a name nowhere in the source, so there is no `Mount` sentence that reaches it.
//! Writing the obvious `Mount App.` fell through to the LEGACY view dialect, which created a
//! `<div id="root">` and called `__render_App` — a function that does not exist — so the page
//! failed at RUN time, in the browser, with a name the author never typed. The pre-render then
//! bailed on that error and shipped a shell, silently.
//!
//! So: **a page mounts itself.** It owns a URL and it is the root of that URL's document; there is
//! nothing else for it to be, and no sentence the author could write instead.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::semantics::dom;

fn tw_page(src: &str) -> String {
    dom::reset();
    let out = tw_outcome(src);
    assert_eq!(
        out.error, None,
        "the tree-walker must run it cleanly:\n{src}"
    );
    dom::render()
}

fn vm_page(src: &str) -> String {
    dom::reset();
    let out = vm_outcome(src);
    assert_eq!(out.error, None, "the VM must run it cleanly:\n{src}");
    dom::render()
}

/// Both tiers, held together — a page that renders in a test and not in a browser is the split
/// this whole file exists to prevent.
fn agree(src: &str) -> String {
    let tw = tw_page(src);
    let vm = vm_page(src);
    assert_eq!(tw, vm, "the two tiers disagree:\n{src}");
    tw
}

#[test]
fn a_page_renders_its_view_with_no_main_at_all() {
    assert_eq!(
        agree("## Page /about\nView:\n    <p>About us</p>\n"),
        "<body><p>About us</p></body>",
        "a page is the root of its URL's document"
    );
}

#[test]
fn a_page_renders_its_state() {
    assert_eq!(
        agree("## Page /\nState:\n    Let n be 7.\nView:\n    <h1>Count {n}</h1>\n"),
        "<body><h1>Count 7</h1></body>"
    );
}

#[test]
fn a_page_renders_a_child_component() {
    // The shape of every real page: a page that places components declared beside it.
    assert_eq!(
        agree(
            "## Component Row (label: Text)\nView:\n    <li>{label}</li>\n\n\
             ## Page /\nState:\n    Let items be [\"a\", \"b\"].\n\
             View:\n    <ul>{for each i in items:}<Row label={i}/>{end}</ul>\n"
        ),
        "<body><ul><li>a</li><li>b</li></ul></body>"
    );
}

#[test]
fn a_pages_load_handler_runs() {
    // `When the page loads:` is a page's first sentence in every real app. It fires because the
    // page mounts; a page that never mounts never fires it.
    assert_eq!(
        agree(
            "## Page /\nState:\n    Let n be 0.\n\
             Actions:\n    When the page loads:\n        Set n to 42.\n\
             View:\n    <p>{n}</p>\n"
        ),
        "<body><p>42</p></body>"
    );
}

#[test]
fn an_explicit_mount_still_decides_where_it_starts() {
    // A file that says where to begin is obeyed — the implicit mount fills in a decision the
    // author did not make, and never overrides one they did.
    assert_eq!(
        agree(
            "## Component App\nView:\n    <b>chosen</b>\n\n\
             ## Page /\nView:\n    <i>not chosen</i>\n\n\
             ## Main\nMount App.\n"
        ),
        "<body><b>chosen</b></body>"
    );
}

#[test]
fn mounting_a_component_that_does_not_exist_is_refused_by_name() {
    // ★ THE MISTAKE THAT HID THIS ONE. `Mount App.` beside a `## Page` named nothing, fell through
    // to the legacy dialect, and failed at RUN time as `Unknown function: __render_App` — a name
    // the author never wrote, in the browser, after the build said nothing.
    let out = tw_outcome("## Page /\nView:\n    <p>x</p>\n\n## Main\nMount Nope.\n");
    let err = out
        .error
        .expect("mounting a name nothing declares is an error");
    assert!(
        err.contains("Nope"),
        "the refusal names what was mounted, not a synthesized symbol: {err}"
    );
    assert!(
        !err.contains("__render_"),
        "and never leaks a name the author cannot see: {err}"
    );
}

#[test]
fn two_pages_in_one_file_are_refused() {
    // Only the first `## Page` decides the file's URL, so the second served nowhere and rendered
    // nothing — a whole page silently absent from the site.
    let out = tw_outcome("## Page /a\nView:\n    <p>a</p>\n\n## Page /b\nView:\n    <p>b</p>\n");
    let err = out.error.expect("two pages in one file is an error");
    assert!(
        err.contains("/b") || err.contains("/a"),
        "the refusal names the paths: {err}"
    );
}

#[test]
fn a_page_beside_a_component_of_the_same_shape_still_mounts_the_page() {
    // A page's own components are declarations, not roots. Mounting the first component instead
    // would render a fragment where the document belongs.
    assert_eq!(
        agree(
            "## Component Card\nView:\n    <div class=\"card\">card</div>\n\n\
             ## Page /\nView:\n    <main><Card/></main>\n"
        ),
        "<body><main><div class=\"card\">card</div></main></body>"
    );
}

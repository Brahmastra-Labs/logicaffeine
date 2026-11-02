//! ★ LOCK — a `Style:` block that is not valid CSS is REFUSED, not silently dropped. ★
//!
//! The reference tier says so plainly: *"The `Style:` of App is not valid CSS. …"* The lowering
//! that serves a browser did this instead:
//!
//! ```text
//! if let Ok(scoped) = crate::lgx::css::scope(css, &class) { …emit the <style> element… }
//! ```
//!
//! An unbalanced brace took the `Err` arm, no `<style>` was emitted, and the page compiled,
//! loaded and rendered — **unstyled, with nothing anywhere saying why.** No error, no warning, no
//! console line. The author sees a page with no styling and no reason for it, and the one tier
//! that would have told them is not the tier a visitor runs.
//!
//! ★ THE TWO TIERS DISAGREED ABOUT WHETHER IT WAS AN ERROR AT ALL, and the SILENT one was the one
//! serving the browser. That is the worst arrangement of the two: a developer testing on the
//! reference tier is told, ships, and the deployed page quietly drops the stylesheet instead.
//!
//! ★ CSS NEVER REPORTS THIS ITSELF. The same class of failure as the scaffolded stylesheet whose
//! four undefined custom properties made every declaration using one invalid at computed-value
//! time — no error, no warning, a page that simply looks wrong. When the language can see the
//! mistake, staying quiet about it is a choice, and the wrong one.

use logicaffeine_compile::compile::{compile_to_wasm, tw_outcome};

/// A component whose `Style:` has an unbalanced brace — the commonest CSS mistake there is.
const BAD_CSS: &str = "\
## Component App\n\
State:\n\
\x20   Let n be 0.\n\
View:\n\
\x20   <p id=\"t\">{n}</p>\n\
Style:\n\
\x20   p { color: red\n\n\
## Main\n\
Mount App.\n";

/// The same component with the brace closed — the control, so a refusal cannot be blamed on the
/// shape of the fixture.
const GOOD_CSS: &str = "\
## Component App\n\
State:\n\
\x20   Let n be 0.\n\
View:\n\
\x20   <p id=\"t\">{n}</p>\n\
Style:\n\
\x20   p { color: red }\n\n\
## Main\n\
Mount App.\n";

/// The reference tier already refuses it, and names the section.
#[test]
fn the_reference_tier_says_the_style_is_not_valid_css() {
    let err = tw_outcome(BAD_CSS)
        .error
        .expect("invalid CSS must be an error on the reference tier");
    assert!(
        err.contains("Style:") || err.to_lowercase().contains("css"),
        "the message must name the section a reader has to go and fix: {err}"
    );
}

/// ★ THE LOAD-BEARING TEST — the tier that serves a browser must not compile it away in silence.
#[test]
fn the_browser_tier_refuses_it_rather_than_dropping_the_stylesheet() {
    let err = compile_to_wasm(BAD_CSS).err().unwrap_or_else(|| {
        panic!(
            "a component whose `Style:` is not valid CSS COMPILED for the browser. The lowering \
             takes the `Err` arm of `css::scope` and emits no `<style>` at all, so the page loads \
             unstyled with nothing saying why — while the reference tier calls the same source an \
             error. A developer is told on the tier they test on and not on the tier they ship."
        )
    });
    let msg = format!("{err:?}");
    assert!(
        msg.contains("Style") || msg.to_lowercase().contains("css"),
        "the refusal must name `Style:`, or the reader goes hunting through their view: {msg}"
    );
}

/// And valid CSS still lowers — the refusal is about the mistake, not about having a `Style:`.
#[test]
fn a_valid_style_block_still_compiles_for_the_browser() {
    assert_eq!(
        tw_outcome(GOOD_CSS).error,
        None,
        "the control must run on the reference tier"
    );
    assert!(
        compile_to_wasm(GOOD_CSS).is_ok(),
        "closing the brace is the only difference between this and the case above"
    );
}

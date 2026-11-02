//! ★ LOCK — `Reload the page.` re-mounts a `## Page`, the same way a browser reload re-runs the
//! script that mounted it. ★
//!
//! A reload drops the whole page — every instance, listener and the mounted root — and runs the
//! page's main again, which mounts the `## Page` again: a `## Page`'s own component has no name a
//! test can `Mount` by hand, so a page that fetches `When the page loads:` or otherwise depends on
//! its own load-time setup is tested across a reload only because the reload mounts it. Browser
//! storage survives (a reload never touches it), which is what a `persisted`/`preserved across
//! reloads` store is for.
//!
//! An explicitly `Mount`ed `## Component` is NOT re-mounted — nothing in such a program says to
//! mount it a second time, and `lgx_vm_parity_lock::reloading_the_page_forgets_everything_on_both_tiers`
//! pins exactly that.

use logicaffeine_compile::compile::tw_outcome;
use logicaffeine_compile::semantics::dom;

fn rendered(src: &str) -> String {
    dom::reset();
    let out = tw_outcome(src);
    assert_eq!(
        out.error, None,
        "the program must run cleanly:\n{src}\n{out:?}"
    );
    let html = dom::render();
    html.strip_prefix("<body>")
        .and_then(|h| h.strip_suffix("</body>"))
        .unwrap_or(&html)
        .to_string()
}

#[test]
fn a_reload_re_mounts_a_page() {
    let src = "\
## Page /
State:
    Let n be 0.
Actions:
    To bump:
        Set n to n + 1.
View:
    <p id=\"n\">{n}</p>
    <button id=\"b\" on-click={bump}>b</button>

## Main
Click \"#b\".
Reload the page.
";
    assert!(
        rendered(src).contains("<p id=\"n\">0</p>"),
        "the page must be back, freshly mounted, after the reload: {}",
        rendered(src)
    );
}

/// A `persisted` store's saved record survives the reload (a reload never touches storage), and
/// the freshly re-mounted page reads it back — the whole point of a page reload re-mounting is
/// that what a `persisted` store kept is there to be shown again.
#[test]
fn a_persisted_store_survives_a_page_reload_and_the_fresh_mount_reads_it() {
    let src = "\
## Store Cart persisted as \"cart\"
State:
    Let count be 0.
Actions:
    To add:
        Set count to count + 1.

## Page /
Use Cart.
Actions:
    To buy:
        Call Cart's add.
View:
    <p id=\"c\">{Cart's count}</p>
    <button id=\"b\" on-click={buy}>b</button>

## Main
Click \"#b\".
Reload the page.
";
    assert!(
        rendered(src).contains("<p id=\"c\">1</p>"),
        "a persisted store's record must survive the reload, and the fresh mount must read it: {}",
        rendered(src)
    );
}

/// The `lgx_vm_parity_lock` pin this fix must not disturb: an EXPLICITLY `Mount`ed `## Component`
/// (not a `## Page`) is not brought back by a reload — nothing in such a program says to mount it
/// again, and the reload takes it with it exactly as before.
#[test]
fn a_reload_does_not_bring_back_an_explicitly_mounted_component() {
    let src = "\
## Component App
State:
    Let n be 0.
Actions:
    To bump:
        Set n to n + 1.
View:
    <p id=\"n\">{n}</p>
    <button id=\"b\" on-click={bump}>b</button>

## Main
Mount App.
Click \"#b\".
Reload the page.
";
    assert_eq!(
        rendered(src),
        "",
        "a reload takes an explicitly-mounted component with it, and nothing remounts it: {}",
        rendered(src)
    );
}

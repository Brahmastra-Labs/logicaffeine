//! ★ LOCK — `When this component appears:` (gauntlet finding G13) never paints its own subtree
//! twice. ★
//!
//! `Trigger::Appears` runs AFTER a component's first commit, so a `Focus`/`Add class`/etc. aimed at
//! the component's OWN just-mounted elements can find them, and nothing follows the handler with a
//! second render of the same instance: its writes reach the document through the page's ordinary
//! repaint. A second render of an already-painted instance draws its markup again instead of
//! patching it — a doubled `<button id="inner">` on exactly this fixture.
//!
//! Fully deterministic: `tw_outcome`/`dom::render()` run on a plain synchronous `Click`, no `##
//! Test`/`run_tests` sealed-world machinery, no timers, no wall clock, no seed to vary — the DOM
//! string this test asserts on is a pure function of the source text.

use logicaffeine_compile::compile::tw_outcome;
use logicaffeine_compile::semantics::dom;

/// The exact `Box`/outer-page shape `gauntlet_findings_lock`'s G13 tests use, reduced to a plain
/// `## Main`/`Click` driver (no `## Test`) so a duplication shows up as a bare string count rather
/// than needing the `Expect`/`Verdict` machinery to notice it.
const SRC: &str = "## Component Box\n\
Actions:\n    When this component appears:\n        Focus \"#outer\".\n\
View:\n    <button id=\"inner\">inner</button>\n\n\
## Page /\n\
State:\n    Let shown be false.\n\
Actions:\n    To toggle:\n        Set shown to true.\n\
View:\n    <button id=\"outer\" on-click={toggle}>outer</button>\n    {if shown:}<Box/>{end}\n\n\
## Main\nClick \"#outer\".\n";

#[test]
fn appearing_paints_its_own_subtree_exactly_once() {
    dom::reset();
    let out = tw_outcome(SRC);
    assert_eq!(out.error, None, "must run cleanly: {out:?}");
    let doc = dom::render();
    let inner_count = doc.matches("id=\"inner\"").count();
    assert_eq!(
        inner_count, 1,
        "the appearing component's own element must be painted exactly once, not {inner_count}: {doc}"
    );
    let outer_count = doc.matches("id=\"outer\"").count();
    assert_eq!(
        outer_count, 1,
        "the page's own element must be painted exactly once, not {outer_count}: {doc}"
    );
}

/// Re-running the identical, fully-deterministic fixture several times in the SAME process must
/// produce byte-identical documents each time — a flake in either direction (sometimes duplicated,
/// sometimes not) would show up as a mismatch here, not just a one-off count.
#[test]
fn ten_identical_runs_produce_byte_identical_documents() {
    let mut docs = Vec::with_capacity(10);
    for _ in 0..10 {
        dom::reset();
        let out = tw_outcome(SRC);
        assert_eq!(out.error, None);
        docs.push(dom::render());
    }
    for (i, doc) in docs.iter().enumerate().skip(1) {
        assert_eq!(
            &docs[0], doc,
            "run 0 and run {i} of the IDENTICAL deterministic fixture disagree"
        );
    }
}

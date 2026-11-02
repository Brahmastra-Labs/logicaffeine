//! ★ LOCK — a driving verb is answered by the page, or by the browser's own default action. ★
//!
//! `Nothing answers a click on "#x"` turns a test's typo into a caught mistake: a click on an
//! element nothing listens to, with no default action of its own, means the rest of the test
//! describes a page nobody touched. Typing into a control and choosing an option are different:
//! the browser's own default action enters the text or selects the option, which the page shows
//! and which moves the control's validity, whether or not anything listens. So they answer by
//! themselves — where the control can take them. A read-only or disabled field, or an element that
//! is no text field, takes no typing, and a `<select>` takes only an option it offers: there the
//! default action does nothing, the browser raises no `input` or `change`, and the verb is refused
//! by name. Every in-process tier holds all of it: the tree-walker running the page's own
//! components (the reference), and the lowered page on the tree-walker and on the VM.

use logicaffeine_compile::page_host::{compare_tests, PageTierKind, PageTrace, TierRun};

/// The ONE `## Test` in `source` on the reference and the two lowered in-process tiers.
fn tiers(source: &str) -> Vec<(&'static str, PageTrace)> {
    let mut all = compare_tests(
        source,
        &[PageTierKind::LoweredHere, PageTierKind::LoweredVm],
        0,
        50_000_000,
        None,
    )
    .unwrap_or_else(|e| panic!("the page must parse: {e}\n{source}"));
    assert_eq!(all.len(), 1, "exactly one page scenario:\n{source}");
    let c = all.remove(0);
    let mut out = vec![("the reference", c.reference.clone())];
    for (kind, run) in c.candidates {
        match run {
            TierRun::Traced(t) => out.push((kind.label(), t)),
            other => panic!(
                "the {} tier must trace the scenario, not {other:?}",
                kind.label()
            ),
        }
    }
    out
}

const CONTROLS: &str = r##"## Page /

View:
    <main>
      <input id="name" required>
      <textarea id="note"></textarea>
      <select id="size"><option value="s">small</option><option value="m">medium</option></select>
      <button id="plain">plain</button>
      <input id="locked" readonly>
      <input id="off" disabled>
    </main>
"##;

fn scenario(test: &str) -> String {
    format!("{CONTROLS}\n## Test: one act\n    {test}\n")
}

#[test]
fn typing_and_choosing_are_answered_by_the_browser_where_nothing_listens() {
    for (act, shows) in [
        (
            "Type \"Ada\" into \"#name\".",
            "<input id=\"name\" required=\"\" value=\"Ada\">",
        ),
        (
            "Type \"hello\" into \"#note\".",
            "<textarea id=\"note\" value=\"hello\">",
        ),
        (
            "Choose \"m\" in \"#size\".",
            "<select id=\"size\" value=\"m\">",
        ),
    ] {
        for (tier, trace) in tiers(&scenario(act)) {
            assert!(
                trace.stopped.is_none(),
                "{tier}: `{act}` is answered by the browser's own default action: {:?}",
                trace.stopped
            );
            let doc = &trace.steps[1].snapshot.document;
            assert!(
                doc.contains(shows),
                "{tier}: `{act}` left its value on the page:\n{doc}"
            );
        }
    }
}

#[test]
fn a_verb_the_element_cannot_take_and_nothing_hears_is_refused_by_name() {
    for (act, refusal) in [
        ("Click \"#plain\".", "Nothing answers a click on \"#plain\""),
        (
            "Type \"x\" into \"#locked\".",
            "Typing into \"#locked\" enters nothing",
        ),
        (
            "Type \"x\" into \"#off\".",
            "Typing into \"#off\" enters nothing",
        ),
        (
            "Type \"x\" into \"#plain\".",
            "Typing into \"#plain\" enters nothing",
        ),
        (
            "Choose \"xl\" in \"#size\".",
            "Choosing \"xl\" in \"#size\" selects nothing",
        ),
    ] {
        for (tier, trace) in tiers(&scenario(act)) {
            let stop = trace
                .stopped
                .unwrap_or_else(|| panic!("{tier}: `{act}` is answered by nothing and must fail"));
            assert!(
                stop.message.contains(refusal),
                "{tier}: `{act}` is refused by name: {}",
                stop.message
            );
        }
    }
}

const READ_ONLY_HEARD: &str = r##"## Page /

State:
    Let heard be 0.
Actions:
    To hear:
        Set heard to heard + 1.
View:
    <main>
      <input id="locked" readonly value="kept" on-input={hear}>
      <p id="heard">{heard}</p>
    </main>

## Test: typing into a read-only field that listens
    Type "x" into "#locked".
"##;

#[test]
fn typing_into_a_field_that_takes_no_typing_raises_nothing_even_where_a_listener_waits() {
    for (tier, trace) in tiers(READ_ONLY_HEARD) {
        let stop = trace
            .stopped
            .unwrap_or_else(|| panic!("{tier}: typing into a read-only field is refused"));
        assert!(
            stop.message
                .contains("Typing into \"#locked\" enters nothing"),
            "{tier}: {}",
            stop.message
        );
        let journal = &trace.steps.last().expect("the arrival").snapshot.journal;
        assert!(
            !journal.iter().any(|t| t == "hear"),
            "{tier}: no `input` reached the listener: {journal:?}"
        );
    }
}

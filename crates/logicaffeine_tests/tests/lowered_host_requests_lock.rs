//! ★ LOCK — what a page asks its host to do from inside an action, an in-process host does. ★
//!
//! A page's own action can ask the browser for things only the browser can do: `Reload the page.`
//! (`location.reload()`) loads the page again — the module's main runs again, the page mounts
//! itself again, browser storage survives. On a lowered tier the action compiles to a host call
//! (`reloadPage()`) that notes the request; the host carries it out once the call returns, exactly
//! as `Reload the page.` written in a test is carried out. Every in-process tier is held to the
//! page a browser shows.

use logicaffeine_compile::page_host::{compare_tests, PageTierKind, TierRun};

const BUDGET: u64 = 50_000_000;

const RELOADING_PAGE: &str = "\
## Store Session persisted as \"session\"
State:
    Let count be 0.
Actions:
    To note:
        Set count to count + 1.

## Page /
Use Session.
State:
    Let taps be 0.
Actions:
    When the page loads:
        Call Session's note.
    To tap:
        Set taps to taps + 1.
    To restart:
        Reload the page.
View:
    <p id=\"taps\">{taps}</p>
    <p id=\"visits\">{Session's count}</p>
    <button id=\"tap\" on-click={tap}>tap</button>
    <button id=\"restart\" on-click={restart}>restart</button>

";

#[test]
fn an_action_reload_loads_the_page_again_on_every_in_process_tier() {
    let src = format!(
        "{RELOADING_PAGE}## Test: the page reloads itself\n\
         Click \"#tap\".\n\
         Click \"#tap\".\n\
         Click \"#restart\".\n\
         Expect that the text of \"#taps\" is equal to \"0\".\n\
         Expect that the text of \"#visits\" is equal to \"2\".\n"
    );
    let all = compare_tests(
        &src,
        &[PageTierKind::LoweredHere, PageTierKind::LoweredVm],
        0,
        BUDGET,
        None,
    )
    .unwrap_or_else(|e| panic!("the page parses: {e}"));
    assert_eq!(all.len(), 1);
    let c = &all[0];
    let mut docs = vec![(
        "reference",
        c.reference
            .steps
            .last()
            .expect("a step")
            .snapshot
            .document
            .clone(),
    )];
    for (kind, run) in &c.candidates {
        match run {
            TierRun::Traced(t) => {
                assert_eq!(t.stopped, None, "{} runs to the end", kind.label());
                docs.push((
                    kind.label(),
                    t.steps.last().expect("a step").snapshot.document.clone(),
                ));
            }
            TierRun::Refused { statement, reason } => {
                panic!("{} refused at {statement}: {reason}", kind.label())
            }
        }
    }
    assert_eq!(docs.len(), 3, "the reference and both lowered tiers ran");
    for (tier, doc) in docs {
        assert_eq!(
            doc.matches("id=\"taps\"").count(),
            1,
            "{tier}: the page is back, once: {doc}"
        );
        assert!(
            doc.contains("<p id=\"taps\">0</p>"),
            "{tier}: its own state started over: {doc}"
        );
        assert!(
            doc.contains("<p id=\"visits\">2</p>"),
            "{tier}: the persisted store survived and the page's load ran again: {doc}"
        );
    }
}

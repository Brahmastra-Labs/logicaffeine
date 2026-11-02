//! LOCK — the behavioural event modifiers `.stop`, `.once`, `.capture` and `.passive` build for the
//! browser and mean the same thing on every tier. RED until Wave 5 lands across the tiers.
//!
//! Today the reference tier accepts each and IGNORES it (it records only a handler's key and
//! `.prevent`), and the browser lowering refuses the whole page by name
//! (`lgx_lower/gate.rs::nodes_lowerable`). Each row below iterates the catalogue
//! (`lgx::html::BEHAVIOUR_MODIFIERS`), so a behaviour added to the language without its tier support
//! fails here by name. The lowering carries a modifier to the host as the `semantics::dom` listen
//! bits (`LISTEN_STOP`, `LISTEN_ONCE`, `LISTEN_CAPTURE`, `LISTEN_PASSIVE`); the host reads them.

use logicaffeine_compile::compile::compile_to_wasm;
use logicaffeine_compile::testrun::{run_tests, TestConfig, Verdict};
use logicaffeine_language::lgx::html::{Behaviour, BEHAVIOUR_MODIFIERS};

fn page(modifier: &str, test: &str) -> String {
    format!(
        "## Page /\n\nState:\n    Let n be 0.\n\nActions:\n    To hit:\n        Set n to n + 1.\n\n\
         View:\n    <div id=\"outer\">\n        <button id=\"b\" on-click.{modifier}={{hit}}>b</button>\n    </div>\n    \
         <p id=\"n\">{{n}}</p>\n\n## Test: {test}\n\n"
    )
}

#[test]
fn every_behaviour_modifier_builds_for_the_browser() {
    for (word, _) in BEHAVIOUR_MODIFIERS {
        let source = page(word, "builds")
            + "Click \"#b\".\nExpect that the text of \"#n\" is equal to \"1\".\n";
        let built = compile_to_wasm(&source);
        assert!(
            built.is_ok(),
            "`.{word}` must build for the browser: {:?}",
            built.err()
        );
    }
}

#[test]
fn a_once_handler_answers_its_first_event_only_on_every_tier() {
    let once = BEHAVIOUR_MODIFIERS
        .iter()
        .find(|(_, b)| matches!(b, Behaviour::Once))
        .map(|(w, _)| *w)
        .expect("`.once` is catalogued");
    let source = page(once, "a second click answers nothing")
        + "Click \"#b\".\nExpect that the text of \"#n\" is equal to \"1\".\nClick \"#b\".\n";
    let report = run_tests(&source, &TestConfig::default()).expect("the fixture parses and runs");
    let outcome = &report.outcomes[0];
    assert_eq!(
        outcome.verdict,
        Verdict::Failed,
        "the second click has nothing left to answer it"
    );
    assert!(
        outcome
            .failures
            .iter()
            .all(|f| f.contains("Nothing answers") && !f.contains("TIER DIVERGENCE")),
        "both tiers refuse the second click in the same words: {:?}",
        outcome.failures
    );
}

#[test]
fn stop_capture_and_passive_leave_a_lone_handler_answering_every_event() {
    for (word, behaviour) in BEHAVIOUR_MODIFIERS {
        if !matches!(
            behaviour,
            Behaviour::Stop | Behaviour::Capture | Behaviour::Passive
        ) {
            continue;
        }
        let source = page(word, "each click answers")
            + "Click \"#b\".\nClick \"#b\".\nExpect that the text of \"#n\" is equal to \"2\".\n";
        let report =
            run_tests(&source, &TestConfig::default()).expect("the fixture parses and runs");
        let outcome = &report.outcomes[0];
        assert_eq!(
            outcome.verdict,
            Verdict::Passed,
            "`.{word}` changes how an event travels, not whether its own handler answers: {:?}",
            outcome.failures
        );
    }
}

/// What the host is told at registration, on the lowered VM: every handler is ONE `listenFlags`
/// (a `DomOp::Listen`) carrying its host bits — `.prevent`, `.stop`, `.capture`, `.passive` — and
/// `.once`, which the dispatcher spends, contributes none.
#[test]
fn every_behaviour_reaches_the_host_as_its_listen_bits_on_the_lowered_vm() {
    use logicaffeine_compile::compile::vm_outcome;
    use logicaffeine_compile::semantics::dom::{self, DomOp};
    let registered = |qualifiers: &str| -> Vec<DomOp> {
        let src = format!(
            "## Component App\nState:\n    Let n be 0.\nActions:\n    To hit:\n        Set n to n + 1.\n\
             View:\n    <button id=\"b\" on-click{qualifiers}={{hit}}>b</button>\n\n## Main\nMount App.\n"
        );
        dom::reset();
        assert_eq!(
            vm_outcome(&src).error,
            None,
            "`on-click{qualifiers}` runs on the lowered VM"
        );
        let b = dom::query_selector("#b") as u32;
        dom::op_log()
            .into_iter()
            .filter(|op| match op {
                DomOp::PreventDefault { node, .. } | DomOp::Listen { node, .. } => *node == b,
                _ => false,
            })
            .collect()
    };
    for (word, b) in BEHAVIOUR_MODIFIERS {
        let ops = registered(&format!(".{word}"));
        let want = match b {
            Behaviour::Once => vec![DomOp::Listen {
                node: 0,
                event: "click".into(),
                flags: 0,
            }],
            Behaviour::Prevent | Behaviour::Stop | Behaviour::Capture | Behaviour::Passive => {
                vec![DomOp::Listen {
                    node: 0,
                    event: "click".into(),
                    flags: b.listen_bit(),
                }]
            }
        };
        let seen: Vec<DomOp> = ops
            .into_iter()
            .map(|op| match op {
                DomOp::PreventDefault { event, .. } => DomOp::PreventDefault { node: 0, event },
                DomOp::Listen { event, flags, .. } => DomOp::Listen {
                    node: 0,
                    event,
                    flags,
                },
                other => other,
            })
            .collect();
        assert_eq!(
            seen, want,
            "`.{word}` reaches the host as exactly this registration"
        );
    }
    let combined = registered(".prevent.stop.once");
    assert_eq!(
        combined.len(),
        1,
        "`.prevent.stop.once` is ONE registration carrying prevent|stop and not once: {combined:?}"
    );
    assert!(
        matches!(&combined[0], DomOp::Listen { flags, .. } if *flags == Behaviour::Prevent.listen_bit() | Behaviour::Stop.listen_bit()),
        "{combined:?}"
    );
}

//! ★ LOCK — a candidate page tier leaves the caller's own page exactly as it found it. ★
//!
//! `page_host::Scenario` compares the reference tier (the lowered page on the tree-walker) against a
//! candidate (the page's lowered LOGOS on the bytecode VM, a browser) by walking the candidate
//! through the SAME native thread-locals (`semantics::dom`, the journal in `semantics::world`,
//! `semantics::drive`'s page clock, `logicaffeine_system::testworld`) the caller's own page just
//! finished painting into —
//! that sharing is what lets a page query mean the same thing whichever tier ran it. Without a
//! save/restore around the reset a candidate's run needs, whatever the candidate leaves behind
//! becomes the CALLER's ambient state the instant the comparison returns: the caller's own
//! document, its journal, its armed timers, quietly replaced by the candidate's.
//!
//! `World::enter`/`WorldGuard` (`page_host/scenario.rs`) close this: entering a candidate's world
//! takes the caller's own world out first (`world::PageWorld::take`: the document, the journal, the
//! page clock, the sealed test world and storage) and the guard puts it back the moment it drops,
//! however the candidate's run ends (a trace, a refusal, an early return).

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::page_host::{PageTierKind, Scenario, World};
use logicaffeine_compile::semantics::{dom, drive, world};
use logicaffeine_language::ast::stmt::Stmt;

const SRC: &str = "\
## Component App
State:
    Let n be 0.
Actions:
    Every 5 seconds:
        Set n to n + 1.
    To bump:
        Set n to n + 1.
View:
    <button id=\"go\" on-click={bump}>x</button>
    <p id=\"n\">{n}</p>

## Test: bumps
    Mount App.
    Advance time by 5 seconds.
    Click \"#go\".
    Expect that the count of \"p\" is 1.
";

/// A snapshot of every thread-local a page tier's paint touches — the ambient state a candidate
/// must leave exactly as it found it.
#[derive(Debug, PartialEq, Eq)]
struct Ambient {
    document: String,
    journal: Vec<String>,
    intervals: usize,
}

fn ambient() -> Ambient {
    Ambient {
        document: dom::render(),
        journal: world::journal().iter().map(|e| format!("{e:?}")).collect(),
        intervals: drive::intervals().len(),
    }
}

/// Build the caller's own ambient state exactly as the real reference tier does — mount the page,
/// arm its timer, click it — through the actual interpreter, not through any page-tier machinery.
fn establish_callers_page() {
    dom::reset();
    world::reset();
    drive::reset_clock();
    logicaffeine_compile::ui_bridge::with_parsed_program(SRC, |parsed, interner| {
        let (stmts, types, policies) = parsed.expect("the fixture parses");
        let defs: Vec<Stmt> = stmts
            .iter()
            .filter(|s| !matches!(s, Stmt::TestDef { .. }))
            .cloned()
            .collect();
        let body: Vec<Stmt> = stmts
            .iter()
            .find_map(|s| match s {
                Stmt::TestDef { body, .. } => Some(body.to_vec()),
                _ => None,
            })
            .expect("the fixture has one test");
        let mut program = defs;
        program.extend(body);
        let mut interp = logicaffeine_compile::interpreter::Interpreter::new(interner)
            .with_type_registry(types)
            .with_policies(policies);
        interp.enable_test_mode(500_000_000);
        interp.run_sync(&program).expect("the fixture runs clean");
    });
}

/// Run a candidate tier the exact way `testrun.rs`'s mounted-test comparison does: build a
/// `Scenario` over the same source and ask it for the reference trace and a candidate trace.
fn run_a_candidate_comparison() {
    logicaffeine_compile::ui_bridge::with_parsed_program(SRC, |parsed, interner| {
        let (stmts, types, policies) = parsed.expect("the fixture parses");
        let defs: Vec<Stmt> = stmts
            .iter()
            .filter(|s| !matches!(s, Stmt::TestDef { .. }))
            .cloned()
            .collect();
        let body: Vec<Stmt> = stmts
            .iter()
            .find_map(|s| match s {
                Stmt::TestDef { body, .. } => Some(body.to_vec()),
                _ => None,
            })
            .expect("the fixture has one test");
        let world = World::for_test("bumps", 0, Default::default(), 500_000_000);
        let scenario = Scenario::new(interner, types, &policies, &defs, &body, world);
        let _reference = scenario.reference_trace();
        let _lowered = scenario.run(PageTierKind::LoweredVm, None, SRC);
    });
}

#[test]
fn a_candidate_tier_leaves_the_callers_ambient_page_untouched() {
    establish_callers_page();
    let before = ambient();
    assert!(
        !before.journal.is_empty(),
        "the fixture must actually leave a journal entry to prove anything: {before:?}"
    );
    // The 5-second timer fires once at the advance (n: 0 -> 1), then the click's own turn runs
    // (n: 1 -> 2) — two journal entries, matching the `!before.journal.is_empty()` check above.
    assert!(
        before.document.contains("<p id=\"n\">2</p>"),
        "{}",
        before.document
    );

    run_a_candidate_comparison();

    let after = ambient();
    assert_eq!(
        before, after,
        "a candidate page tier changed the caller's own ambient page"
    );
}

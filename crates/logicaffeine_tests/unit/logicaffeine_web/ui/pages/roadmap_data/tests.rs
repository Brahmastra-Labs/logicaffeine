use super::*;

/// Every roadmap example must be REAL compiler output. This recompiles each
/// `english` and checks the displayed output still matches — so an example
/// can never silently become fabricated. FOL is exact-matched; Rust must
/// compile (the shown Rust is a faithful excerpt of the full emitted file);
/// SVA is exact-matched against the synthesizer.
#[test]
fn every_example_is_real_compiler_output() {
    for m in get_milestones() {
        for ex in m.examples {
            match ex.output {
                Output::Fol { simple, unicode } => {
                    let got_simple = logicaffeine_language::compile_simple(ex.english)
                        .unwrap_or_else(|e| {
                            panic!("[{} / {}] FOL did not compile: {:?}", m.title, ex.label, e)
                        });
                    let got_unicode =
                        logicaffeine_language::compile(ex.english).unwrap_or_else(|e| {
                            panic!("[{} / {}] FOL did not compile: {:?}", m.title, ex.label, e)
                        });
                    assert_eq!(
                        got_simple, simple,
                        "[{} / {}] SimpleFOL drifted from the compiler",
                        m.title, ex.label
                    );
                    assert_eq!(
                        got_unicode, unicode,
                        "[{} / {}] Unicode FOL drifted from the compiler",
                        m.title, ex.label
                    );
                }
                Output::Rust(_) => {
                    logicaffeine_compile::compile::compile_to_rust(ex.english).unwrap_or_else(|e| {
                        panic!("[{} / {}] Rust example did not compile: {:?}\n--- source ---\n{}\n--------------", m.title, ex.label, e, ex.english)
                    });
                }
                Output::Sva(sva) => {
                    let synth =
                        logicaffeine_compile::codegen_sva::fol_to_sva::synthesize_sva_from_spec(
                            ex.english, "clk",
                        )
                        .unwrap_or_else(|e| {
                            panic!("[{} / {}] SVA did not synthesize: {}", m.title, ex.label, e)
                        });
                    assert_eq!(
                        synth.sva_text, sva,
                        "[{} / {}] SVA drifted from the synthesizer",
                        m.title, ex.label
                    );
                }
            }
        }
    }
}

/// The timeline's nodes in order, each with its status. A node is what logicaffeine.com
/// publishes as a milestone, so adding, removing, renaming or moving one is a decision taken
/// here, on purpose, and never a side effect of editing the data.
#[test]
fn the_timeline_is_the_published_story() {
    let got: Vec<(&str, Status)> = get_milestones()
        .iter()
        .map(|m| (m.title, m.status))
        .collect();
    assert_eq!(
        got,
        vec![
            ("Core Transpiler", Status::Complete),
            ("Web Platform & Studio", Status::Complete),
            ("Imperative Language", Status::Complete),
            ("Type System", Status::Complete),
            ("Concurrency & Actors", Status::Complete),
            ("Distributed Systems", Status::Complete),
            ("Security & Policies", Status::Complete),
            ("Proof Assistant", Status::Complete),
            ("Native Compilation Tier", Status::Complete),
            ("Hardware Verification", Status::Complete),
            ("Full-Stack Web", Status::Complete),
            ("Translation Validation", Status::InProgress),
            ("Silicon", Status::InProgress),
            ("Quantum Backend", Status::Planned),
        ]
    );
}

/// The line down the timeline changes colour once, where finished work meets work in progress,
/// and again where work in progress meets planned work — so the cards must come in that order.
#[test]
fn the_timeline_runs_complete_then_in_progress_then_planned() {
    let rank = |s: Status| match s {
        Status::Complete => 0,
        Status::InProgress => 1,
        Status::Planned => 2,
    };
    for pair in get_milestones().windows(2) {
        assert!(
            rank(pair[0].status) <= rank(pair[1].status),
            "\"{}\" ({}) comes before \"{}\" ({})",
            pair[0].title,
            pair[0].status.label(),
            pair[1].title,
            pair[1].status.label()
        );
    }
}

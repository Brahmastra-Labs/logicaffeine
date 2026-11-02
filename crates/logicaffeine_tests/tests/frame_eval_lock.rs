//! W10 — `frame_eval_lock` / `frame_eval_parity`: the paused-frame evaluator.
//!
//! FrameEval
//! evaluates a LOGOS expression against a frame's bindings using the
//! tree-walker — so its answer is program semantics, by construction. This is
//! the one evaluator behind conditional breakpoints, the debug REPL, and DAP
//! `evaluate`.

use logicaffeine_compile::compile::tw_outcome;
use logicaffeine_compile::frame_eval::{frame_answer, frame_eval, Binding};

fn frame() -> Vec<Binding<'static>> {
    vec![
        Binding {
            name: "balance",
            value: "92",
        },
        Binding {
            name: "amount",
            value: "50",
        },
    ]
}

/// A bare expression evaluates against the frame.
#[test]
fn evaluates_an_expression_against_the_frame() {
    let f = frame();
    assert_eq!(frame_eval(&f, "balance minus amount").unwrap(), "42");
    assert_eq!(frame_eval(&f, "balance").unwrap(), "92");
}

/// A yes/no question answers in English.
#[test]
fn answers_a_yes_no_question() {
    let f = frame();
    assert_eq!(
        frame_answer(&f, "Is balance less than amount?").unwrap(),
        "No."
    );
    assert_eq!(
        frame_answer(&f, "Is balance greater than amount?").unwrap(),
        "Yes."
    );
}

/// PARITY: FrameEval agrees with the same computation written as a program —
/// it IS program semantics, not a second evaluator that can drift.
#[test]
fn frame_eval_parity_with_a_program() {
    let f = frame();
    let via_frame = frame_eval(&f, "balance minus amount").unwrap();
    let via_program = tw_outcome(
        "## Main\n    Let balance be 92.\n    Let amount be 50.\n\
         \x20   Show balance minus amount.\n",
    )
    .output;
    assert_eq!(
        via_frame, via_program,
        "FrameEval == the tree-walker == program semantics"
    );
}

/// A bad expression fails exactly as it would in a program (read-only, honest).
#[test]
fn a_bad_expression_errors() {
    let f = frame();
    assert!(
        frame_eval(&f, "balance divided by 0").is_err(),
        "division by zero errors in the frame just as in a program"
    );
}

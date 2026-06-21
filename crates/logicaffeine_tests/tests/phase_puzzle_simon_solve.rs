#![cfg(feature = "verification")]
//! Logic puzzles must be NATURALLY SOLVABLE: written as an English theorem and
//! proved by the SAME general prover that proves Socrates — no puzzle-specific
//! primitives, no encoded grid. We hand it English `Given:`s and a `Prove:`, and
//! `Auto` must find the proof. These tests pin the GENERAL reasoning steps a
//! puzzle is built from, each stated purely in English.

use logicaffeine_compile::verify_theorem;

fn proves(doc: &str) {
    let r = verify_theorem(doc);
    assert!(r.is_ok(), "expected a proof, got: {:?}", r.err());
}

/// The baseline: the studio's Socrates proof, end to end from English.
#[test]
fn syllogism_baseline() {
    proves(
        "## Theorem: Socrates\n\
         Given: Socrates is a man.\n\
         Given: Every man is mortal.\n\
         Prove: Socrates is mortal.\n\
         Proof: Auto.\n",
    );
}

/// Disjunctive syllogism over a CLOSED domain — the elimination step every
/// logic-grid clue chain relies on, proved naturally from English: the colors
/// are exactly red and blue, so a color that is not red is blue.
#[test]
fn disjunctive_syllogism_over_closed_domain() {
    proves(
        "## Theorem: Color\n\
         Given: Mauve is a color.\n\
         Given: Every color is red or blue.\n\
         Given: Mauve is not red.\n\
         Prove: Mauve is blue.\n\
         Proof: Auto.\n",
    );
}

/// A COMPLETE small logic grid, proved naturally from English: two people, two
/// jobs, a closed domain + a uniqueness (bijection) constraint, and one clue —
/// the same shape (scaled up) as a PuzzleBaron grid.
#[test]
fn small_grid_proves_naturally() {
    proves(
        "## Theorem: Jobs\n\
         Given: Alice is a person.\n\
         Given: Bob is a person.\n\
         Given: Alice is not Bob.\n\
         Given: Every person is a doctor or a lawyer.\n\
         Given: Exactly one person is a doctor.\n\
         Given: Alice is a doctor.\n\
         Prove: Bob is a lawyer.\n\
         Proof: Auto.\n",
    );
}

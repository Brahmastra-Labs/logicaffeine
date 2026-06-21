//! End-to-end logic-grid SOLVE: a grid stated as an English theorem (declarations
//! + domain-closure bijection + a clue), SOLVED by a wh-QUESTION goal answered by
//! the same general kernel-certified prover that proves Socrates. No grid solver,
//! no puzzle primitives — just English in, the answer out.

use logicaffeine_compile::{answer_question, verify_theorem};

fn proves(doc: &str) {
    let r = verify_theorem(doc);
    assert!(r.is_ok(), "expected a proof; got: {:?}", r.err());
}

/// The bijection ELIMINATION over a converged relation, proved naturally: the
/// state domain is closed to {Florida, Maine}, exactly one trip is in Florida and
/// Alpha is, so Beta — the other trip — must be in Maine. Same shape as the small
/// jobs grid, but over `In(x, <state>)` (the form a label/PP clue converges to).
#[test]
fn two_value_state_bijection_proves() {
    proves(
        "## Theorem: Trips\n\
         Given: Alpha is a trip.\n\
         Given: Beta is a trip.\n\
         Given: Alpha is not Beta.\n\
         Given: Every trip is in Florida or in Maine.\n\
         Given: Exactly one trip is in Florida.\n\
         Given: Alpha is in Florida.\n\
         Prove: Beta is in Maine.\n\
         Proof: Auto.\n",
    );
}

/// The same grid, SOLVED by asking a locative QUESTION — "Who is in Maine?" — and
/// the prover answers with the witness it derived (Beta).
#[test]
fn solve_state_cell_by_question() {
    let ans = answer_question(
        "## Theorem: Trips\n\
         Given: Alpha is a trip.\n\
         Given: Beta is a trip.\n\
         Given: Alpha is not Beta.\n\
         Given: Every trip is in Florida or in Maine.\n\
         Given: Exactly one trip is in Florida.\n\
         Given: Alpha is in Florida.\n\
         Prove: Who is in Maine?\n\
         Proof: Auto.\n",
    )
    .expect("a locative question should be answerable");
    assert!(ans.contains(&"Beta".to_string()), "the trip in Maine is Beta; got: {ans:?}");
}

/// TWO categories coexisting (year × state), solved by a question. The answer
/// (Beta in Maine) is entailed by the state bijection ALONE — the year premises are
/// logically irrelevant to it. While the premises were left QUANTIFIED the kernel's
/// backward search blew up on the irrelevant year clauses; GROUNDING the kernel path
/// (in `answer_question`'s `prepare_premises`) makes the grid quantifier-free and
/// decidable — the year clauses ground to inert facts and the kernel CERTIFIES the
/// elimination, no Z3. This is the multi-category solve the kernel could not do before.
#[test]
fn solve_two_category_grid_by_question() {
    let ans = answer_question(
        "## Theorem: Trips\n\
         Given: Alpha is a trip.\n\
         Given: Beta is a trip.\n\
         Given: Alpha is not Beta.\n\
         Given: Every trip is in 2003 or in 2004.\n\
         Given: Every trip is in Florida or in Maine.\n\
         Given: Exactly one trip is in 2003.\n\
         Given: Exactly one trip is in Florida.\n\
         Given: Alpha is in 2003.\n\
         Given: Alpha is in Florida.\n\
         Prove: Who is in Maine?\n\
         Proof: Auto.\n",
    )
    .expect("the two-category grid should be answerable");
    assert!(ans.contains(&"Beta".to_string()), "Beta is in Maine; got: {ans:?}");
}

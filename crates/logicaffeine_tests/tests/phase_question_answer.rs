//! "Ask the goal": a theorem whose `Prove:` is a WH-QUESTION, answered by the
//! SAME general kernel-certified prover that proves Socrates. The wh-goal becomes
//! ∃x.φ(x); the ANSWER is the witness, found by proving each domain individual's
//! candidate goal φ(c). No question-specific machinery — this is how a logic
//! puzzle's "Determine who/where/when" goal is meant to be answered.

use logicaffeine_compile::answer_question;

#[test]
fn answers_who_is_a_lawyer() {
    // The small grid, but the GOAL is a question. The prover must DERIVE that
    // Bob is the lawyer (exactly one doctor = Alice ⇒ Bob is not a doctor ⇒,
    // since every person is a doctor or a lawyer, Bob is a lawyer) and ANSWER
    // with the witness.
    let ans = answer_question(
        "## Theorem: Jobs\n\
         Given: Alice is a person.\n\
         Given: Bob is a person.\n\
         Given: Alice is not Bob.\n\
         Given: Every person is a doctor or a lawyer.\n\
         Given: Exactly one person is a doctor.\n\
         Given: Alice is a doctor.\n\
         Prove: Who is a lawyer?\n\
         Proof: Auto.\n",
    )
    .expect("a question should be answerable");
    assert!(ans.contains(&"Bob".to_string()), "the lawyer is Bob; got: {ans:?}");
}

//! "English in → proof out, SEE the trace." A `## Theorem` block proves through the
//! kernel-certified backward chainer, and the derivation is surfaced as a rendered,
//! step-by-step proof trace — alongside the FOL the English compiled to.

use logicaffeine_compile::prove_theorem_trace;

#[test]
fn socrates_proof_trace_is_visible() {
    let t = prove_theorem_trace(
        "## Theorem: Socrates\n\
         Given: Socrates is a man.\n\
         Given: Every man is mortal.\n\
         Prove: Socrates is mortal.\n\
         Proof: Auto.\n",
    )
    .expect("the theorem should parse");

    assert!(t.verified, "Socrates is mortal should be PROVED; err: {:?}", t.error);

    // English-in → FOL-out is visible (the renderer lowercases predicate symbols).
    assert!(
        t.premises.iter().any(|p| p.contains("man(Socrates)")),
        "premises rendered as FOL; got: {:?}",
        t.premises
    );
    assert!(t.goal.contains("mortal(Socrates)"), "goal rendered as FOL; got: {}", t.goal);

    // The PROOF TRACE itself is visible and is a rendered derivation tree showing the
    // inference steps (modus ponens over a universally-instantiated premise).
    let trace = t.trace.expect("a verified proof carries a derivation trace");
    assert!(trace.contains("└─"), "trace is a rendered proof tree; got:\n{trace}");
    assert!(trace.contains("ModusPonens"), "trace shows the inference rule; got:\n{trace}");
    assert!(
        trace.contains("mortal(Socrates)"),
        "trace shows the proved conclusion; got:\n{trace}"
    );
}

/// A theorem that does NOT follow returns `verified == false` with no claimed proof.
#[test]
fn unprovable_theorem_reports_no_proof() {
    let t = prove_theorem_trace(
        "## Theorem: Bogus\n\
         Given: Socrates is a man.\n\
         Prove: Socrates is mortal.\n\
         Proof: Auto.\n",
    )
    .expect("the theorem should parse");
    assert!(!t.verified, "no premise makes men mortal, so it must not verify");
}

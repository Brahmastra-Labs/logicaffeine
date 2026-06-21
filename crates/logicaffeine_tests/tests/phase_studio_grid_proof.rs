//! The STUDIO proof path (`compile_theorem_for_ui` → `prove_certify_check`, the same
//! call the web app runs in the browser with NO Z3) must:
//!   1. PROVE a finite-domain grid cell, by grounding it first (a grid is decidable
//!      once quantifier-free), returning a kernel-certified derivation tree; and
//!   2. leave an OPEN syllogism (Socrates) untouched, so it keeps its
//!      `UniversalInst` / `ModusPonens` trace.
//! This is what lets a Simon logic-grid example render a real proof tree at the
//! bottom of the studio, exactly like the Socrates example — kernel-certified, no
//! oracle.

use logicaffeine_compile::compile_theorem_for_ui;

/// A two-category Simon slice (year × state) with the bijection and one cross-pin —
/// the shape a grid cell proves by: closure + exactly-one ⇒ elimination. The studio
/// path must ground it and return a certified derivation.
#[test]
fn grid_cell_proves_via_studio_path() {
    let r = compile_theorem_for_ui(
        "## Theorem: Simon\n\
         Given: Alpha is a trip.\n\
         Given: Beta is a trip.\n\
         Given: Alpha is not Beta.\n\
         Given: Every trip is in 2003 or in 2004.\n\
         Given: Every trip is in Florida or in Maine.\n\
         Given: Exactly one trip is in 2003.\n\
         Given: Exactly one trip is in Florida.\n\
         Given: Alpha is in 2003.\n\
         Given: Alpha is in Florida.\n\
         Prove: Beta is in Maine.\n\
         Proof: Auto.\n",
    );
    assert!(r.error.is_none(), "compile error: {:?}", r.error);
    assert!(
        r.verified,
        "the grid cell must prove (kernel-certified), no Z3; err: {:?}",
        r.verification_error
    );
    let tree = r.derivation.expect("a verified grid cell yields a derivation tree");
    let rendered = tree.display_tree();
    // A real multi-step grid derivation — closure elimination reaches a Contradiction
    // (Beta in Florida would force Beta = Alpha) and a disjunctive step.
    assert!(
        rendered.contains("Contradiction") || rendered.contains("DisjunctionElim"),
        "the grid trace must show the elimination reasoning; got:\n{rendered}"
    );
}

/// Socrates is NOT a grid — it must be proved DIRECTLY (ungrounded), keeping the
/// canonical `∀`-instantiation trace the studio is known for.
#[test]
fn socrates_keeps_universal_inst_trace() {
    let r = compile_theorem_for_ui(
        "## Theorem: Socrates\n\
         Given: Every man is mortal.\n\
         Given: Socrates is a man.\n\
         Prove: Socrates is mortal.\n\
         Proof: Auto.\n",
    );
    assert!(r.verified, "Socrates must prove; err: {:?}", r.verification_error);
    let rendered = r.derivation.expect("Socrates yields a tree").display_tree();
    assert!(
        rendered.contains("UniversalInst"),
        "Socrates must keep its UniversalInst trace (not be grounded); got:\n{rendered}"
    );
}

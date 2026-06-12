//! Z3 is outside the trusted base.
//!
//! The oracle can *find* a proof obligation satisfiable, but it produces no
//! checkable proof term. A derivation discharged only by the oracle must
//! therefore never certify — so the trusted door can never present a Z3 verdict
//! as a kernel-checked proof.

use logicaffeine_kernel::Context;
use logicaffeine_proof::certifier::{certify, CertificationContext};
use logicaffeine_proof::{DerivationTree, InferenceRule, ProofExpr};

/// An `OracleVerification` leaf — what `engine.prove` produces when only Z3
/// could discharge the goal — must fail certification with an honest message.
#[test]
fn oracle_results_do_not_certify() {
    let tree = DerivationTree::leaf(
        ProofExpr::Atom("SomeArithmeticFact".to_string()),
        InferenceRule::OracleVerification("Verified by Z3".to_string()),
    );

    let ctx = Context::new();
    let cert_ctx = CertificationContext::new(&ctx);
    let result = certify(&tree, &cert_ctx);

    assert!(
        result.is_err(),
        "a Z3 oracle result must not certify into a kernel proof"
    );
    let msg = format!("{:?}", result.unwrap_err());
    assert!(
        msg.contains("oracle"),
        "the rejection should name the oracle as the reason, got: {}",
        msg
    );
}

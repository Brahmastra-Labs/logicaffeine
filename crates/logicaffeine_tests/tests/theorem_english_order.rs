//! The English theorem surface states integer ORDER, and `largo prove` certifies it.
//!
//! `≤` and `<` live in the proof layer as the Props `le(a, b) = true` and `lt(a, b) = true`
//! (`engine::le_eq`, `omega_solve::lt_eq`), which is what `omega`, the Farkas core and the
//! certifier all read. The English spellings lower onto exactly those two shapes, with the
//! converse words swapping the operands:
//!
//! | English | Prop |
//! | --- | --- |
//! | `a is at most b`, `a is less than or equal to b`, `a <= b` | `le(a, b) = true` |
//! | `a is at least b`, `a is greater than or equal to b`, `a >= b` | `le(b, a) = true` |
//! | `a is less than b`, `a < b` | `lt(a, b) = true` |
//! | `a is greater than b`, `a > b` | `lt(b, a) = true` |
//!
//! Every assertion here goes through `prove_theorem_trace` and `theorem_certificate`, the two
//! calls `largo prove` makes per English block, so a ✓ here is a ✓ at the command line.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::{prove_theorem_trace, theorem_certificate};

fn theorem(name: &str, givens: &[&str], goal: &str, proof: &str) -> String {
    let mut block = format!("## Theorem: {name}\n");
    for given in givens {
        block.push_str(&format!("Given: {given}\n"));
    }
    block.push_str(&format!("Prove: {goal}\nProof: {proof}\n"));
    block
}

/// `Some(verified)` when the block parses, with the error rendered when it does not.
fn verdict(block: &str) -> Result<bool, String> {
    prove_theorem_trace(block)
        .map(|t| t.verified)
        .map_err(|e| format!("{e:?}"))
}

fn assert_proves(block: &str) {
    let trace = prove_theorem_trace(block)
        .unwrap_or_else(|e| panic!("must parse:\n{block}\nerror: {e:?}"));
    assert!(
        trace.verified,
        "must be kernel-certified:\n{block}\ngoal: {}\npremises: {:?}\nerror: {:?}",
        trace.goal, trace.premises, trace.error
    );
}

fn assert_refuses(block: &str) {
    match verdict(block) {
        Ok(verified) => assert!(
            !verified,
            "a FALSE order statement must never certify:\n{block}"
        ),
        Err(e) => panic!("a false order statement must still PARSE, and fail to prove:\n{block}\nerror: {e}"),
    }
}

// ───────────────────────── the acceptance file from the energy hack ─────────────────────────

const SHEDDING_NEVER_RAISES_POWER: &str = "\
## Theorem: Shedding_Never_Raises_Power
Prove: For all w s, if s is at least 0 then w minus s is at most w.
Proof: omega.
";

const DOUBLING_BREAKS_THE_CAP: &str = "\
## Theorem: Doubling_Breaks_The_Cap
Prove: For all w, if w is greater than 0 then w plus w is at most w.
Proof: omega.
";

const CAP_HOLDS_WITH_PREMISES: &str = "\
## Theorem: Cap_Holds_With_Premises
Given (h1): s is at least 0.
Given (h2): s is at most w.
Prove: w minus s is at least 0.
Proof: omega.
";

#[test]
fn shedding_load_never_raises_power_is_certified() {
    assert_proves(SHEDDING_NEVER_RAISES_POWER);
}

#[test]
fn doubling_past_the_cap_parses_and_is_refused() {
    assert_refuses(DOUBLING_BREAKS_THE_CAP);
}

#[test]
fn a_cap_stated_as_named_premises_is_certified() {
    assert_proves(CAP_HOLDS_WITH_PREMISES);
}

#[test]
#[ignore = "DAT-433: `Proof: Auto.` on the English path is the backward chainer, which has no \
            linear-arithmetic step, so order goals need `Proof: omega.`. Whether Auto should fall \
            back to omega is Collin's design call"]
fn the_same_theorems_certify_under_auto() {
    assert_proves(&SHEDDING_NEVER_RAISES_POWER.replace("Proof: omega.", "Proof: Auto."));
    assert_proves(&CAP_HOLDS_WITH_PREMISES.replace("Proof: omega.", "Proof: Auto."));
    assert_refuses(&DOUBLING_BREAKS_THE_CAP.replace("Proof: omega.", "Proof: Auto."));
}

// ───────────────────────── every spelling, both directions ─────────────────────────

/// Each non-strict spelling is pinned by a TRUE statement and the FALSE statement with its
/// operands swapped. A spelling that lowered its operands in the wrong order would certify the
/// false one, and a spelling that did not lower at all would certify neither.
#[test]
fn every_non_strict_spelling_orders_its_operands_the_way_english_reads() {
    for (at_most, at_least) in [
        ("is at most", "is at least"),
        ("is less than or equal to", "is greater than or equal to"),
        ("<=", ">="),
    ] {
        assert_proves(&theorem(
            "Le",
            &[],
            &format!("For all n, n {at_most} n plus 1."),
            "omega.",
        ));
        assert_refuses(&theorem(
            "Le_Swapped",
            &[],
            &format!("For all n, n plus 1 {at_most} n."),
            "omega.",
        ));
        assert_proves(&theorem(
            "Ge",
            &[],
            &format!("For all n, n plus 1 {at_least} n."),
            "omega.",
        ));
        assert_refuses(&theorem(
            "Ge_Swapped",
            &[],
            &format!("For all n, n {at_least} n plus 1."),
            "omega.",
        ));
    }
}

/// A strict fact is usable as a PREMISE: `omega` tightens `a < b` to `a + 1 ≤ b`. (A symbolic
/// strict CONCLUSION has no image in the trusted base, see `omega_solve::omega_prove`, so the
/// strict spellings are pinned where they can be certified: on the hypothesis side.)
#[test]
fn every_strict_spelling_is_a_usable_premise_in_the_right_direction() {
    for (less, greater) in [("is less than", "is greater than"), ("<", ">")] {
        assert_proves(&theorem(
            "Lt",
            &[&format!("x {less} y.")],
            "x plus 1 is at most y.",
            "omega.",
        ));
        assert_refuses(&theorem(
            "Lt_Swapped",
            &[&format!("x {less} y.")],
            "y plus 1 is at most x.",
            "omega.",
        ));
        assert_proves(&theorem(
            "Gt",
            &[&format!("x {greater} y.")],
            "y plus 1 is at most x.",
            "omega.",
        ));
        assert_refuses(&theorem(
            "Gt_Swapped",
            &[&format!("x {greater} y.")],
            "x plus 1 is at most y.",
            "omega.",
        ));
    }
}

/// Ground comparisons decide by computation, in both strict and non-strict form.
#[test]
fn ground_comparisons_decide_by_computation() {
    assert_proves(&theorem("G1", &[], "2 plus 2 is at most 5.", "omega."));
    assert_refuses(&theorem("G2", &[], "2 plus 2 is at most 3.", "omega."));
    assert_proves(&theorem("G3", &[], "2 times 3 is less than 7.", "omega."));
    assert_refuses(&theorem("G4", &[], "2 times 3 is less than 6.", "omega."));
}

/// `or equal to` belongs to the relation, never to a disjunction of two propositions.
#[test]
fn or_equal_to_is_part_of_the_relation_not_a_disjunction() {
    assert_proves(&theorem(
        "Le_Refl",
        &[],
        "For all n, n is less than or equal to n.",
        "omega.",
    ));
    assert_refuses(&theorem(
        "Lt_Refl",
        &[],
        "For all n, n plus 1 is less than or equal to n.",
        "omega.",
    ));
}

/// Order composes with the connectives the math layer already has.
#[test]
fn order_composes_with_and_and_if() {
    assert_proves(&theorem(
        "Between",
        &[],
        "For all a b c, if a is at most b and b is at most c then a is at most c.",
        "omega.",
    ));
    assert_refuses(&theorem(
        "Not_Between",
        &[],
        "For all a b c, if a is at most b and b is at most c then c is at most a.",
        "omega.",
    ));
}

/// A premise and a goal that mention the same lowercase name mean the same variable.
#[test]
fn a_premise_constrains_the_goal_it_shares_names_with() {
    assert_proves(&theorem(
        "Shed_Within_Limit",
        &["s is at least 0.", "s is at most w."],
        "w minus s is at most w.",
        "omega.",
    ));
    assert_refuses(&theorem(
        "Shed_Unbounded",
        &["s is at most w."],
        "w minus s is at most w.",
        "omega.",
    ));
}

// ───────────────────────── the certificate ─────────────────────────

/// The certificate of `block` re-checks in the kernel alone, with no prover in the loop.
#[cfg(feature = "serde")]
fn assert_certificate_rechecks(block: &str) {
    use logicaffeine_compile::kernel::certificate::recheck_theorem;

    let cert = theorem_certificate(block)
        .unwrap_or_else(|e| panic!("must parse: {e:?}"))
        .unwrap_or_else(|| panic!("a proved theorem must mint a certificate:\n{block}"));
    let recheck = recheck_theorem(&cert);
    assert!(
        recheck.rechecks,
        "the certificate must re-check in the kernel alone:\n{block}\nerror: {:?}",
        recheck.error
    );
}

/// A proved order theorem mints a certificate that the kernel re-checks on its own, and an
/// unproved one mints none.
#[cfg(feature = "serde")]
#[test]
fn a_certified_order_theorem_mints_a_certificate_the_kernel_rechecks() {
    assert_certificate_rechecks(SHEDDING_NEVER_RAISES_POWER);
    assert!(
        theorem_certificate(DOUBLING_BREAKS_THE_CAP)
            .expect("parses")
            .is_none(),
        "a refuted theorem has no proof term, so no certificate"
    );
}

/// A theorem stated over free names in its `Given` lines certifies the same way.
#[cfg(feature = "serde")]
#[test]
#[ignore = "DAT-433: a theorem whose `Given` lines name free variables (`s is at most w`) mints a \
            certificate that does not re-check (`UnboundVariable(\"w\")`), because the certificate \
            does not bind the theorem's free variables. Pre-existing for equalities too. Closing \
            the certificate over its free variables is a format decision"]
fn a_theorem_over_free_premise_names_mints_a_certificate_the_kernel_rechecks() {
    assert_certificate_rechecks(CAP_HOLDS_WITH_PREMISES);
}

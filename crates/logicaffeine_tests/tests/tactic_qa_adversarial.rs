//! Adversarial QA battery for the literate tactic surface.
//!
//! The critical property is SOUNDNESS: no false theorem may ever be accepted, by ANY tactic. This
//! throws a large table of tricky/false/nested/boundary goals at each tactic and asserts the exact
//! `is_ok()` verdict. A single wrong verdict fails the whole battery with the offending case, so a
//! soundness regression (a false goal slipping through) or a completeness regression (a valid goal
//! wrongly rejected) is caught loudly.

use logicaffeine_kernel::interface::Repl;

/// (label, statement, tactic, should_prove)
const CASES: &[(&str, &str, &str, bool)] = &[
    // ---- arithmetic equalities: true proven, false rejected, across every equality tactic ------
    ("eq_true_ring", "3 equals 3", "ring", true),
    ("eq_false_ring", "3 equals 4", "ring", false),
    ("eq_true_simp", "(mul 2 3) equals 6", "simp", true),
    ("eq_false_simp", "(mul 2 3) equals 7", "simp", false),
    ("eq_true_auto", "48 equals (mul 6 8)", "auto", true),
    ("eq_false_auto", "48 equals (mul 6 9)", "auto", false),
    ("eq_true_decide", "(add 20 22) equals 42", "decide", true),
    ("eq_false_decide", "(add 20 22) equals 41", "decide", false),
    ("ring_identity", "(add a b) equals (add b a)", "ring", true),
    ("ring_nonidentity", "a equals b", "ring", false),
    // ---- inequalities ---------------------------------------------------------------------------
    ("lt_true", "2 < 5", "lia", true),
    ("lt_false", "5 < 2", "lia", false),
    ("lt_refl_false", "x < x", "lia", false),
    ("lt_succ", "x < x + 1", "omega", true),
    ("le_true", "(le 5 5)", "auto", true),
    ("le_false", "(le 5 4)", "auto", false),
    // ---- negation & disequality (both directions) ----------------------------------------------
    ("diseq_true", "(not (Eq Int 2 3))", "auto", true),
    ("diseq_false", "(not (Eq Int 3 3))", "auto", false),
    ("not_false_lt_true", "(not (Lt 5 2))", "decide", true),
    ("not_true_lt_false", "(not (Lt 2 5))", "decide", false),
    ("double_neg_true", "(not (not (Lt 2 5)))", "auto", true),
    ("double_neg_false", "(not (not (Lt 5 2)))", "auto", false),
    // ---- conjunction ---------------------------------------------------------------------------
    ("and_tt", "(And True True)", "auto", true),
    ("and_tf", "(And True False)", "auto", false),
    ("and_ft", "(And False True)", "auto", false),
    ("and_eq_true", "(And (Eq Int 2 2) (Lt 3 7))", "decide", true),
    (
        "and_eq_false",
        "(And (Eq Int 2 2) (Eq Int 3 4))",
        "decide",
        false,
    ),
    (
        "and_nested_true",
        "(And (And True True) (Or False True))",
        "auto",
        true,
    ),
    (
        "and_nested_false",
        "(And (And True True) (Or False False))",
        "auto",
        false,
    ),
    // ---- disjunction ---------------------------------------------------------------------------
    ("or_tt", "(Or True True)", "auto", true),
    ("or_tf", "(Or True False)", "auto", true),
    ("or_ff", "(Or False False)", "auto", false),
    ("or_one_true", "(Or (Lt 5 2) (Lt 3 7))", "auto", true),
    ("or_both_false", "(Or (Lt 5 2) (Lt 9 2))", "auto", false),
    // ---- implication ---------------------------------------------------------------------------
    ("impl_tt", "(implies True True)", "auto", true),
    ("impl_tf", "(implies True False)", "auto", false),
    ("impl_ft_vacuous", "(implies False True)", "auto", true),
    ("impl_ff_vacuous", "(implies False False)", "auto", true),
    ("impl_self", "(implies (Lt x y) (Lt x y))", "auto", true),
    (
        "impl_nonvalid",
        "(implies (Lt x y) (Lt a b))",
        "auto",
        false,
    ),
    // ---- hypothesis rewrite (equality hyp into any conclusion) ---------------------------------
    (
        "hyp_eq_into_eq",
        "(implies (Eq Int x 0) (Eq Int (add x 1) 1))",
        "auto",
        true,
    ),
    (
        "hyp_eq_into_lt_true",
        "(implies (Eq Int x 5) (Lt x 10))",
        "auto",
        true,
    ),
    (
        "hyp_eq_into_lt_false",
        "(implies (Eq Int x 5) (Lt x 3))",
        "auto",
        false,
    ),
    (
        "hyp_eq_into_not",
        "(implies (Eq Int x 5) (not (Eq Int x 6)))",
        "auto",
        true,
    ),
    (
        "hyp_two_true",
        "(implies (Eq Int x 1) (implies (Eq Int y 2) (Eq Int (add x y) 3)))",
        "auto",
        true,
    ),
    (
        "hyp_two_false",
        "(implies (Eq Int x 1) (implies (Eq Int y 2) (Eq Int (add x y) 4)))",
        "auto",
        false,
    ),
    // ---- contradiction (jointly-unsatisfiable hypotheses ⇒ vacuous) ----------------------------
    (
        "contra_pair",
        "(implies (Eq Int x 5) (implies (Eq Int x 6) (Lt a b)))",
        "auto",
        true,
    ),
    (
        "contra_symbolic",
        "(implies (Eq Int x 1) (implies (Eq Int x 2) (Eq Int p q)))",
        "decide",
        true,
    ),
    (
        "contra_false_hyp",
        "(implies (Lt 5 2) (Eq Int p q))",
        "auto",
        true,
    ),
    (
        "consistent_symbolic",
        "(implies (Eq Int x 5) (implies (Eq Int y 6) (Lt a b)))",
        "auto",
        false,
    ),
    (
        "consistent_same_symbolic",
        "(implies (Eq Int x 5) (implies (Eq Int x 5) (Lt a b)))",
        "auto",
        false,
    ),
    // ---- directed introduction tactics ---------------------------------------------------------
    ("split_true", "(And True True)", "split", true),
    ("split_false", "(And True False)", "split", false),
    ("split_non_and", "(Lt 2 5)", "split", false),
    ("left_true", "(Or (Lt 3 7) (Lt 5 2))", "left", true),
    (
        "left_directed_false",
        "(Or (Lt 5 2) (Lt 3 7))",
        "left",
        false,
    ),
    ("right_true", "(Or (Lt 5 2) (Lt 3 7))", "right", true),
    (
        "right_directed_false",
        "(Or (Lt 3 7) (Lt 5 2))",
        "right",
        false,
    ),
    ("left_non_or", "(And True True)", "left", false),
    // ---- assumption tactic (directed: hypothesis only) -----------------------------------------
    (
        "assumption_self",
        "(implies (Lt x y) (Lt x y))",
        "assumption",
        true,
    ),
    (
        "assumption_weaken",
        "(implies (Lt a b) (implies (Lt x y) (Lt a b)))",
        "assumption",
        true,
    ),
    (
        "assumption_not_hyp",
        "(implies (Lt x y) (Lt a b))",
        "assumption",
        false,
    ),
    (
        "assumption_decidable_not_hyp",
        "(And True True)",
        "assumption",
        false,
    ),
    // ---- boundary / mixed ----------------------------------------------------------------------
    (
        "mixed_true",
        "(And (not (Lt 5 2)) (Or False True))",
        "auto",
        true,
    ),
    (
        "mixed_false",
        "(And (not (Lt 2 5)) (Or True True))",
        "auto",
        false,
    ),
    ("neg_ineq_boundary", "(not (Lt 5 5))", "auto", true),
    ("le_self", "(le 0 0)", "decide", true),
    ("gt_true", "(Gt 5 2)", "auto", true),
    ("gt_false", "(Gt 2 5)", "auto", false),
];

#[test]
fn adversarial_tactic_battery_every_verdict_is_exact() {
    let mut failures: Vec<String> = Vec::new();
    for (label, stmt, tac, want) in CASES {
        let mut repl = Repl::new();
        let src = format!("## Theorem: QA\n    Statement: {stmt}.\n    Proof: {tac}.");
        let got = repl.execute(&src).is_ok();
        if got != *want {
            failures.push(format!(
                "  [{label}] `{stmt}` via `{tac}`: got proved={got}, expected proved={want}"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "tactic QA battery found {} wrong verdict(s) — a SOUNDNESS or completeness bug:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Cross-tactic consistency: `auto` and `decide` must agree on every decidable goal in the table
/// (both are the decision procedure; a disagreement signals a wiring bug).
#[test]
fn auto_and_decide_agree_on_decidable_goals() {
    let mut mismatches: Vec<String> = Vec::new();
    for (label, stmt, tac, _want) in CASES {
        // Only the goals routed through the decision procedure (skip the directed/structural ones,
        // which auto also proves but with different directedness).
        if !["auto", "decide"].contains(tac) {
            continue;
        }
        let a = Repl::new()
            .execute(&format!(
                "## Theorem: QA\n    Statement: {stmt}.\n    Proof: auto."
            ))
            .is_ok();
        let d = Repl::new()
            .execute(&format!(
                "## Theorem: QA\n    Statement: {stmt}.\n    Proof: decide."
            ))
            .is_ok();
        if a != d {
            mismatches.push(format!("  [{label}] `{stmt}`: auto={a}, decide={d}"));
        }
    }
    assert!(
        mismatches.is_empty(),
        "auto/decide disagreed on {} goal(s):\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );
}

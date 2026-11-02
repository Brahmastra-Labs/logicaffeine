//! Multi-step proof scripts: `Proof: intro. <closer>.`
//!
//! `intro` introduces the antecedent of an implication into a hypothesis context and hands the
//! consequent to the rest of the script; the closer discharges it under those hypotheses. Sound:
//! the script's derivation must still `concludes` the original goal, which the theorem soundness
//! gate re-checks. These tests are written RED-first (the feature does not exist yet).

use logicaffeine_kernel::interface::Repl;

fn prove(src: &str) -> bool {
    Repl::new().execute(src).is_ok()
}

// ---- the feature: intro then close ---------------------------------------------------------

#[test]
fn intro_then_assumption_closes_reflexive_implication() {
    assert!(
        prove("## Theorem: T\n    Statement: (implies (Lt x y) (Lt x y)).\n    Proof: intro. assumption."),
        "intro. assumption. should prove P -> P"
    );
}

#[test]
fn intro_then_auto_uses_the_hypothesis() {
    assert!(
        prove("## Theorem: T\n    Statement: (implies (Eq Int x 5) (Lt x 10)).\n    Proof: intro. auto."),
        "intro. auto. should prove x=5 -> x<10"
    );
}

#[test]
fn nested_intros_then_assumption() {
    assert!(
        prove("## Theorem: T\n    Statement: (implies (Lt a b) (implies (Lt x y) (Lt a b))).\n    Proof: intro. intro. assumption."),
        "intro. intro. assumption. should prove P -> Q -> P"
    );
}

// ---- SOUNDNESS: false / mis-structured scripts must be rejected ----------------------------

#[test]
fn intro_then_assumption_rejects_non_hypothesis_conclusion() {
    assert!(
        !prove("## Theorem: T\n    Statement: (implies (Lt x y) (Lt a b)).\n    Proof: intro. assumption."),
        "the conclusion is not a hypothesis — must be rejected"
    );
}

#[test]
fn intro_then_auto_rejects_false_implication() {
    assert!(
        !prove("## Theorem: T\n    Statement: (implies (Eq Int x 5) (Lt x 3)).\n    Proof: intro. auto."),
        "x=5 -> x<3 is false — must be rejected"
    );
}

#[test]
fn intro_on_a_non_implication_is_rejected() {
    assert!(
        !prove("## Theorem: T\n    Statement: (Lt 2 5).\n    Proof: intro. auto."),
        "intro on a non-implication has nothing to introduce — must be rejected"
    );
}

// ---- adversarial soundness battery for scripts ---------------------------------------------

#[test]
fn sequencing_adversarial_battery_every_verdict_is_exact() {
    // (label, statement, proof-script, should_prove)
    let cases: &[(&str, &str, &str, bool)] = &[
        (
            "intro_auto_true",
            "(implies (Eq Int x 5) (Lt x 10))",
            "intro. auto.",
            true,
        ),
        (
            "intro_auto_false",
            "(implies (Eq Int x 5) (Lt x 3))",
            "intro. auto.",
            false,
        ),
        (
            "intro_assumption_true",
            "(implies (Lt x y) (Lt x y))",
            "intro. assumption.",
            true,
        ),
        (
            "intro_assumption_false",
            "(implies (Lt x y) (Lt a b))",
            "intro. assumption.",
            false,
        ),
        (
            "two_intro_assumption_true",
            "(implies (Lt a b) (implies (Lt x y) (Lt a b)))",
            "intro. intro. assumption.",
            true,
        ),
        (
            "two_intro_assumption_false",
            "(implies (Lt a b) (implies (Lt x y) (Lt p q)))",
            "intro. intro. assumption.",
            false,
        ),
        (
            "intro_on_conjunction_rejected",
            "(And True True)",
            "intro. auto.",
            false,
        ),
        (
            "intro_on_atom_rejected",
            "(Eq Int 3 3)",
            "intro. auto.",
            false,
        ),
        (
            "intro_decide_contradiction",
            "(implies (Eq Int x 5) (implies (Eq Int x 6) (Lt a b)))",
            "intro. intro. decide.",
            true,
        ),
        (
            "intro_ring_hypothesis_congruence",
            "(implies (Eq Int a b) (Eq Int a b))",
            "intro. assumption.",
            true,
        ),
        (
            "intro_auto_vacuous_false_hyp",
            "(implies (Lt 5 2) (Eq Int p q))",
            "intro. auto.",
            true,
        ),
        // --- nastiest: probe the "accept pre-reduced derivation" fix for a soundness hole ---
        (
            "true_hyp_false_concl_rejected",
            "(implies (Eq Int 2 2) (Lt 5 3))",
            "intro. auto.",
            false,
        ),
        (
            "intro_decide_false_concl",
            "(implies (Eq Int x 5) (Lt x 3))",
            "intro. decide.",
            false,
        ),
        (
            "intro_left_true",
            "(implies (Lt a b) (Or (Lt 3 7) (Lt 9 2)))",
            "intro. left.",
            true,
        ),
        (
            "intro_left_directed_false",
            "(implies (Lt a b) (Or (Lt 9 2) (Lt 3 7)))",
            "intro. left.",
            false,
        ),
        (
            "intro_split_true",
            "(implies (Lt a b) (And True (Lt 2 5)))",
            "intro. split.",
            true,
        ),
        (
            "intro_split_false",
            "(implies (Lt a b) (And True (Lt 5 2)))",
            "intro. split.",
            false,
        ),
        (
            "intro_ring_needs_hyp_but_ring_cannot",
            "(implies (Eq Int a b) (Lt a b))",
            "intro. ring.",
            false,
        ),
        (
            "three_intros_assumption_deep",
            "(implies (Lt a b) (implies (Lt c d) (implies (Lt e f) (Lt c d))))",
            "intro. intro. intro. assumption.",
            true,
        ),
        (
            "three_intros_wrong_target",
            "(implies (Lt a b) (implies (Lt c d) (implies (Lt e f) (Lt p q))))",
            "intro. intro. intro. assumption.",
            false,
        ),
    ];
    let mut failures: Vec<String> = Vec::new();
    for (label, stmt, script, want) in cases {
        let got = prove(&format!(
            "## Theorem: QA\n    Statement: {stmt}.\n    Proof: {script}"
        ));
        if got != *want {
            failures.push(format!(
                "  [{label}] `{stmt}` via `{script}`: got={got}, want={want}"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "sequencing QA found {} wrong verdict(s) — SOUNDNESS bug:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

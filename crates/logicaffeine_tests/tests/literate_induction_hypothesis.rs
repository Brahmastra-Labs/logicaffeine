//! A literate induction step closes its goal WITH its induction hypothesis — and with nothing
//! else.
//!
//! Lean's `Nat.add` and `Nat.mul` recurse on the second argument, so `n + 0 = n` is `rfl` and
//! `0 + n = n` is a theorem by induction: its step `0 + (k+1) = k+1` computes to
//! `succ (0 + k) = succ k`, and only the hypothesis `0 + k = k` finishes it. `simp` uses an
//! equality hypothesis as a rewrite between normal forms (`reduction::rewrite_by_hypotheses`),
//! which is what these tests lock: every true step closes, every false one is refused, a goal
//! without its hypothesis is refused, and the stored derivation is re-checked by both kernels.
use logicaffeine_kernel::interface::Repl;
use logicaffeine_kernel::{double_check, infer_type, DoubleCheck};

/// Lean's `Nat.zero_add`, `Nat.zero_mul`, `Nat.one_add` and `Nat.one_mul`, each true only by
/// induction.
const BY_INDUCTION: [(&str, &str); 4] = [
    ("ZeroAdd", "(Nat_add Zero n) equals n"),
    ("ZeroMul", "(Nat_mul Zero n) equals Zero"),
    ("OneAdd", "(Nat_add (Succ Zero) n) equals (Succ n)"),
    ("OneMul", "(Nat_mul (Succ Zero) n) equals n"),
];

/// A false neighbour of each goal above, with its base case: every one is true at `0` and false
/// at `1`, so it is the step — and only the step — that must refuse it.
const FALSE_STEPS: [(&str, &str, &str); 4] = [
    (
        "ZeroAddDouble",
        "(Nat_add Zero n) equals (Nat_add n n)",
        "(Nat_add Zero Zero) equals (Nat_add Zero Zero)",
    ),
    (
        "ZeroMulSelf",
        "(Nat_mul Zero n) equals n",
        "(Nat_mul Zero Zero) equals Zero",
    ),
    (
        "OneAddDouble",
        "(Nat_add (Succ Zero) n) equals (Succ (Nat_add n n))",
        "(Nat_add (Succ Zero) Zero) equals (Succ (Nat_add Zero Zero))",
    ),
    (
        "OneMulZero",
        "(Nat_mul (Succ Zero) n) equals (Nat_mul n Zero)",
        "(Nat_mul (Succ Zero) Zero) equals (Nat_mul Zero Zero)",
    ),
];

fn theorem(name: &str, statement: &str, proof: &str) -> String {
    format!("## Theorem: {name}\n    Statement: {statement}.\n    Proof: {proof}")
}

const INDUCTION_BY_SIMP: &str = "induction n.\n    - simp.\n    - simp.";

#[test]
fn every_step_closes_with_its_induction_hypothesis_and_both_kernels_recheck_it() {
    let mut repl = Repl::new();
    for (name, statement) in BY_INDUCTION {
        repl.execute(&theorem(name, statement, INDUCTION_BY_SIMP))
            .unwrap_or_else(|e| panic!("`{statement}` by induction on n: {e:?}"));
        let ctx = repl.context();
        let body = ctx
            .get_definition_body(name)
            .cloned()
            .unwrap_or_else(|| panic!("`{name}` was accepted but nothing was stored"));
        infer_type(ctx, &body)
            .unwrap_or_else(|e| panic!("`{name}`'s derivation does not re-check: {e:?}"));
        assert_eq!(
            double_check(ctx, &body),
            DoubleCheck::Agreed,
            "`{name}`: the second kernel disagrees"
        );
    }
}

#[test]
fn auto_closes_the_same_steps() {
    let mut repl = Repl::new();
    for (name, statement) in BY_INDUCTION {
        let name = format!("{name}ByAuto");
        repl.execute(&theorem(
            &name,
            statement,
            "induction n.\n    - auto.\n    - auto.",
        ))
        .unwrap_or_else(|e| panic!("`{statement}` by induction with auto: {e:?}"));
    }
}

#[test]
fn a_false_step_is_refused_although_its_base_case_holds() {
    let mut repl = Repl::new();
    for (name, statement, base_statement) in FALSE_STEPS {
        let base = format!("{name}Base");
        repl.execute(&theorem(&base, base_statement, "simp."))
            .unwrap_or_else(|e| panic!("the base case `{base_statement}` is true: {e:?}"));
        for proof in [
            INDUCTION_BY_SIMP,
            "induction n.\n    - auto.\n    - auto.",
            "simp.",
            "auto.",
        ] {
            let attempt = repl.execute(&theorem(name, statement, proof));
            assert!(
                attempt.is_err(),
                "`{statement}` is false at n = 1, yet `{proof}` certified it"
            );
            assert!(
                repl.context().get_definition_body(name).is_none(),
                "a refused `{name}` registered a body"
            );
        }
    }
}

#[test]
fn without_induction_there_is_no_hypothesis_to_use() {
    let mut repl = Repl::new();
    for (name, statement) in BY_INDUCTION {
        for tactic in ["simp", "auto", "refl", "decide", "cc"] {
            let name = format!("{name}Direct{tactic}");
            assert!(
                repl.execute(&theorem(&name, statement, &format!("{tactic}.")))
                    .is_err(),
                "`{statement}` needs induction, yet `{tactic}` proved it outright"
            );
        }
    }
}

#[test]
fn a_syntactic_closer_cannot_use_the_hypothesis() {
    // `refl` compares the two sides as written; the step's sides differ until the hypothesis is
    // used, so a proof whose step is `refl` is refused.
    let mut repl = Repl::new();
    for (name, statement) in BY_INDUCTION {
        let name = format!("{name}StepRefl");
        assert!(
            repl.execute(&theorem(
                &name,
                statement,
                "induction n.\n    - simp.\n    - refl."
            ))
            .is_err(),
            "`{statement}`: `refl` closed a step that needs its hypothesis"
        );
    }
}

#[test]
fn a_stated_hypothesis_is_used_exactly_as_an_induction_hypothesis_is() {
    let mut repl = Repl::new();
    repl.execute(&theorem(
        "StepOfZeroAdd",
        "((Nat_add Zero n) equals n) implies ((Nat_add Zero (Succ n)) equals (Succ n))",
        "simp.",
    ))
    .expect("the step of `0 + n = n`, stated with its hypothesis, is closed by rewriting with it");
    repl.execute(&theorem(
        "TwoSteps",
        "((Nat_add Zero n) equals n) implies ((Nat_add Zero (Succ (Succ n))) equals (Succ (Succ n)))",
        "simp.",
    ))
    .expect("the hypothesis folds the inner `0 + n` once reduction exposes it");
    for (name, statement) in [
        (
            "WrongStep",
            "((Nat_add Zero n) equals n) implies ((Nat_add Zero (Succ n)) equals n)",
        ),
        (
            "WrongHypothesisUse",
            "((Nat_add Zero n) equals n) implies ((Nat_add n Zero) equals (Succ n))",
        ),
        (
            "BackwardsStep",
            "((Nat_add Zero (Succ n)) equals (Succ n)) implies ((Nat_add Zero n) equals (Succ n))",
        ),
    ] {
        assert!(
            repl.execute(&theorem(name, statement, "simp.")).is_err(),
            "`{statement}` is false"
        );
        assert!(
            repl.context().get_definition_body(name).is_none(),
            "a refused `{name}` registered a body"
        );
    }
}

#[test]
fn a_hypothesis_that_only_grows_the_goal_gives_up_in_bounded_time() {
    // `0 + n = succ (0 + n)` rewrites its left side into a larger term containing it. Whatever the
    // verdict (the hypothesis is unsatisfiable, so the implication is true), it must arrive, and a
    // certificate must re-check.
    let mut repl = Repl::new();
    let started = std::time::Instant::now();
    let verdict = repl.execute(&theorem(
        "Grows",
        "((Nat_add Zero n) equals (Succ (Nat_add Zero n))) implies ((Nat_add Zero n) equals Zero)",
        "simp.",
    ));
    assert!(
        started.elapsed() < std::time::Duration::from_secs(300),
        "the rewriting did not stop"
    );
    if verdict.is_ok() {
        let ctx = repl.context();
        let body = ctx
            .get_definition_body("Grows")
            .cloned()
            .expect("an accepted theorem is stored");
        assert_eq!(double_check(ctx, &body), DoubleCheck::Agreed);
    }
}

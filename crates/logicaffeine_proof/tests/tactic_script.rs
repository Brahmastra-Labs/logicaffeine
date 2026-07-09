//! Proofs written as TEXT: a script string compiles to a composed tactic and runs
//! through the kernel trust door. The seam toward the English vernacular — every
//! proof here is kernel-certified, driven entirely by a string.

use logicaffeine_proof::tactic::ProofState;
use logicaffeine_proof::tactic_script::{
    parse_script, prove_scripted_library, ScriptedTheorem, TacticEnv,
};
use logicaffeine_proof::{ProofExpr, ProofTerm};

fn k(n: &str) -> ProofTerm {
    ProofTerm::Constant(n.to_string())
}
fn v(n: &str) -> ProofTerm {
    ProofTerm::Variable(n.to_string())
}
fn pr(name: &str, who: ProofTerm) -> ProofExpr {
    ProofExpr::Predicate { name: name.to_string(), args: vec![who], world: None }
}
fn and(l: ProofExpr, r: ProofExpr) -> ProofExpr {
    ProofExpr::And(Box::new(l), Box::new(r))
}
fn or(l: ProofExpr, r: ProofExpr) -> ProofExpr {
    ProofExpr::Or(Box::new(l), Box::new(r))
}
fn implies(l: ProofExpr, r: ProofExpr) -> ProofExpr {
    ProofExpr::Implies(Box::new(l), Box::new(r))
}
fn forall(var: &str, body: ProofExpr) -> ProofExpr {
    ProofExpr::ForAll { variable: var.to_string(), body: Box::new(body) }
}
fn succ(t: ProofTerm) -> ProofTerm {
    ProofTerm::Function("Succ".to_string(), vec![t])
}

fn proves(premises: Vec<ProofExpr>, goal: ProofExpr, script: &str) -> bool {
    let mut st = ProofState::start(premises, goal);
    st.run_script(script).expect("script should run");
    st.qed().expect("qed").verified
}

#[test]
fn script_intro_assumption() {
    let goal = implies(pr("man", k("Socrates")), pr("man", k("Socrates")));
    assert!(proves(vec![], goal, "intro h; assumption"));
}

// --- M: user-defined tactics (metaprogramming) ------------------------------

#[test]
fn user_defined_tactic_is_reusable_and_composes() {
    // Define tactics IN THE LANGUAGE: `close := assumption`, and `both := split <;> close`
    // (built from a primitive, a combinator, AND another user tactic). Then prove with the
    // named `both` — no Rust involved. The whole thing is kernel-certified.
    let mut env = TacticEnv::new();
    // `close := assumption`, and `trivial_impl := intro h; close` — a user tactic built
    // from a primitive AND another user tactic. No Rust involved.
    env.define("close", "assumption");
    env.define("trivial_impl", "intro h; close");

    let goal = implies(pr("man", k("Socrates")), pr("man", k("Socrates")));
    let mut st = ProofState::start(vec![], goal);
    st.run_script_with_env("trivial_impl", &env).expect("user-defined tactic runs");
    assert!(st.qed().expect("qed").verified, "proof via a user-defined tactic is kernel-certified");
}

#[test]
fn user_tactics_nest_through_combinators() {
    // A user tactic used inside a `repeat`/`first` combinator resolves correctly.
    let mut env = TacticEnv::new();
    env.define("finish", "assumption");
    let premises = vec![pr("happy", k("Bob")), pr("tall", k("Bob"))];
    let goal = and(pr("happy", k("Bob")), pr("tall", k("Bob")));
    let mut st = ProofState::start(premises, goal);
    st.run_script_with_env("split; (first [ finish | auto ]); finish", &env)
        .expect("user tactic under combinators runs");
    assert!(st.qed().expect("qed").verified);
}

#[test]
fn user_tactic_shadows_a_builtin_alias() {
    // A user definition is AUTHORITATIVE and may even shadow a built-in alias — `both` is a
    // canonical synonym for `split`, but a user who defines `both := split <;> assumption`
    // gets THEIR tactic. This also exercises `<;>` inside a user tactic.
    let mut env = TacticEnv::new();
    env.define("both", "split <;> assumption");

    let premises = vec![pr("happy", k("Bob")), pr("tall", k("Bob"))];
    let goal = and(pr("happy", k("Bob")), pr("tall", k("Bob")));
    let mut st = ProofState::start(premises, goal);
    st.run_script_with_env("both", &env).expect("user `both` shadows the builtin");
    assert!(st.qed().expect("qed").verified, "the user-defined `both` closes both goals");
}

#[test]
fn recursive_user_tactic_is_bounded_not_infinite() {
    // A self-referential definition must be REFUSED (depth-bounded), never loop forever.
    let mut env = TacticEnv::new();
    env.define("loopy", "try (loopy)");
    let goal = implies(pr("p", k("a")), pr("p", k("a")));
    let mut st = ProofState::start(vec![], goal);
    assert!(
        st.run_script_with_env("loopy", &env).is_err(),
        "a recursive user tactic must be rejected, not hang"
    );
}

#[test]
fn script_semicolon_sequence() {
    let premises = vec![pr("happy", k("Bob")), pr("tall", k("Bob"))];
    let goal = and(pr("happy", k("Bob")), pr("tall", k("Bob")));
    assert!(proves(premises, goal, "split; assumption; assumption"));
}

#[test]
fn script_then_all_combinator() {
    // `split <;> assumption`: split, then assumption on every resulting goal.
    let premises = vec![pr("happy", k("Bob")), pr("tall", k("Bob"))];
    let goal = and(pr("happy", k("Bob")), pr("tall", k("Bob")));
    assert!(proves(premises, goal, "split <;> assumption"));
}

#[test]
fn script_cases_and_disjunction_commutativity() {
    // ⊢ (A ∨ B) → (B ∨ A), driven entirely by the script.
    let goal = implies(
        or(pr("happy", k("Bob")), pr("tall", k("Bob"))),
        or(pr("tall", k("Bob")), pr("happy", k("Bob"))),
    );
    assert!(proves(vec![], goal, "intro h; cases h; right; assumption; left; assumption"));
}

#[test]
fn script_cases_conjunction_with_grouping() {
    // A ∧ B ⊢ B ∧ A, destructing the premise and re-assembling.
    let premises = vec![and(pr("happy", k("Bob")), pr("tall", k("Bob")))];
    let goal = and(pr("tall", k("Bob")), pr("happy", k("Bob")));
    assert!(proves(premises, goal, "cases hp0; (split <;> assumption)"));
}

#[test]
fn script_repeat_and_first() {
    // (A ∧ B) ∧ C ⊢, decomposed by `repeat (first [split | assumption])`.
    let premises = vec![pr("a", k("X")), pr("b", k("X")), pr("c", k("X"))];
    let goal = and(and(pr("a", k("X")), pr("b", k("X"))), pr("c", k("X")));
    assert!(proves(premises, goal, "repeat (first [split | assumption])"));
}

#[test]
fn script_induction_over_nat() {
    let base = pr("P", k("Zero"));
    let step = forall("k", implies(pr("P", v("k")), pr("P", succ(v("k")))));
    let goal = forall("n", pr("P", v("n")));
    assert!(proves(vec![base, step], goal, "induction; auto; auto"));
}

#[test]
fn script_parse_errors_are_reported() {
    assert!(parse_script("intro").is_err(), "intro needs an argument");
    assert!(parse_script("bogustactic").is_err(), "unknown tactic");
    assert!(parse_script("split )").is_err(), "trailing tokens");
    assert!(parse_script("").is_err(), "empty script");
}

// --- English-esque vernacular ---

#[test]
fn english_suppose_then_assumption() {
    // "Suppose h, then by assumption." reads as prose, parses as `intro h; assumption`.
    let goal = implies(pr("man", k("Socrates")), pr("man", k("Socrates")));
    assert!(proves(vec![], goal, "Suppose h, then by assumption."));
}

#[test]
fn english_disjunction_commutativity_as_prose() {
    // The whole ∨-commutativity proof written in near-English.
    let goal = implies(
        or(pr("happy", k("Bob")), pr("tall", k("Bob"))),
        or(pr("tall", k("Bob")), pr("happy", k("Bob"))),
    );
    let script = "Assume h. By cases on h, right, by assumption. Left, by assumption.";
    assert!(proves(vec![], goal, script));
}

#[test]
fn english_conjunction_destructure() {
    // "Destruct hp0, then split and discharge both by assumption."
    let premises = vec![and(pr("happy", k("Bob")), pr("tall", k("Bob")))];
    let goal = and(pr("tall", k("Bob")), pr("happy", k("Bob")));
    let script = "Destruct hp0. Split <;> by assumption.";
    assert!(proves(premises, goal, script));
}

#[test]
fn english_induction_proof() {
    let base = pr("P", k("Zero"));
    let step = forall("k", implies(pr("P", v("k")), pr("P", succ(v("k")))));
    let goal = forall("n", pr("P", v("n")));
    assert!(proves(vec![base, step], goal, "By induction. Automatically. Automatically."));
}

// --- The certified scripted library (R6) ---

fn pred2(name: &str, a: ProofTerm, b: ProofTerm) -> ProofExpr {
    ProofExpr::Predicate { name: name.to_string(), args: vec![a, b], world: None }
}

#[test]
fn scripted_library_cites_earlier_theorem() {
    // mortal_socrates proves mortal(Socrates) by `auto`; socrates_dies cites it to get
    // mortal(Socrates), then proves dies(Socrates) by `auto`. A scripted Euclid-graph.
    let _ = pred2; // (kept for symmetry with multi-arg libraries)
    let mortal_socrates = ScriptedTheorem {
        name: "mortal_socrates".to_string(),
        premises: vec![
            forall("x", implies(pr("man", v("x")), pr("mortal", v("x")))),
            pr("man", k("Socrates")),
        ],
        goal: pr("mortal", k("Socrates")),
        script: "auto".to_string(),
        cites: vec![],
        simp: false,
    };
    let socrates_dies = ScriptedTheorem {
        name: "socrates_dies".to_string(),
        premises: vec![forall("x", implies(pr("mortal", v("x")), pr("dies", v("x"))))],
        goal: pr("dies", k("Socrates")),
        script: "By automation.".to_string(),
        cites: vec!["mortal_socrates".to_string()],
        simp: false,
    };
    let r = prove_scripted_library(&[], &[mortal_socrates, socrates_dies]);
    assert!(r[0].verified, "mortal_socrates: {:?}", r[0].verification_error);
    assert!(r[1].verified, "socrates_dies (cites mortal_socrates): {:?}", r[1].verification_error);
}

#[test]
fn scripted_library_without_citation_fails() {
    // No citation → mortal(Socrates) unavailable → dies(Socrates) unprovable.
    let socrates_dies = ScriptedTheorem {
        name: "socrates_dies".to_string(),
        premises: vec![forall("x", implies(pr("mortal", v("x")), pr("dies", v("x"))))],
        goal: pr("dies", k("Socrates")),
        script: "auto".to_string(),
        cites: vec![],
        simp: false,
    };
    let r = prove_scripted_library(&[], &[socrates_dies]);
    assert!(!r[0].verified, "without the cited lemma the proof must fail");
}

#[test]
fn scripted_library_with_explicit_tactic_scripts() {
    // `both` proves happy ∧ tall from its own premises by an explicit script;
    // `both_swapped` has NO premises of its own — it can only prove tall ∧ happy by
    // CITING `both` (getting the conjunction), then destructing it. Tests that the
    // citation is load-bearing AND that explicit (non-auto) scripts certify.
    let lemma = ScriptedTheorem {
        name: "both".to_string(),
        premises: vec![pr("happy", k("Bob")), pr("tall", k("Bob"))],
        goal: and(pr("happy", k("Bob")), pr("tall", k("Bob"))),
        script: "split <;> assumption".to_string(),
        cites: vec![],
        simp: false,
    };
    let swapped = ScriptedTheorem {
        name: "both_swapped".to_string(),
        premises: vec![],
        goal: and(pr("tall", k("Bob")), pr("happy", k("Bob"))),
        // only premise is the cited `happy ∧ tall` (hp0); destruct it, re-assemble swapped.
        script: "Destruct hp0. Split <;> by assumption.".to_string(),
        cites: vec!["both".to_string()],
        simp: false,
    };
    let r = prove_scripted_library(&[], &[lemma, swapped]);
    assert!(r[0].verified, "lemma both: {:?}", r[0].verification_error);
    assert!(r[1].verified, "both_swapped (cites both): {:?}", r[1].verification_error);
}

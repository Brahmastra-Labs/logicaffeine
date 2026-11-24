//! ★ LOCK — a development refutes a statement with a countermodel a checker re-verifies. ★
//!
//! `Theorem <name>: refute <formula>.` claims the formula does not follow from the development's axioms. The claim
//! holds when the axioms are a Datalog program (ground facts and function-free Horn rules) and the formula is false in
//! their least model: that model is a finite structure in which every axiom is true and the formula false, so no proof
//! of the formula exists. The countermodel is re-verified axiom by axiom over its finite domain by an evaluator that
//! shares nothing with the engine that built it.
//!
//! - the points-to development refutes `VP(B, H1)` beside the four facts it proves; the countermodel is the least model
//!   and re-checks;
//! - refuting a fact that follows (`VP(C, H1)`) is not verified, and says the fact holds in the least model;
//! - axioms that are not a Datalog program (a disjunction) cannot refute, and say so;
//! - a countermodel missing a derived fact, or holding the refuted fact, fails the re-check;
//! - a `refute` clause parses with `given` and `cites`; a theorem with two claims does not;
//! - `largo prove` prints the refutation as a ✓ and exits 0; `--json` marks it refuted with its countermodel's size;
//! - a premise the model breaks is named with the assignment that breaks it;
//! - a statement of any first-order shape is read in the model: an existential, a universal, an equation, a negation;
//! - `given` premises join the least model and cited theorems' statements must hold in it, and a citation the check
//!   cannot see fails the refutation;
//! - claims keep source order across two `## Theory` blocks;
//! - the inventory counts a refutation as its own kind, never a proof, and `--certificate` prints its countermodel;
//! - a refutation that fails makes `largo prove` fail and says why;
//! - the proof guide's refutation example proves and refutes as the guide says.

#[path = "cli_common/mod.rs"]
mod common;

use common::{largo_in, stderr, stdout};
use logicaffeine_compile::ui_bridge::compile_theory_for_ui;
use logicaffeine_proof::counterexample::{check_countermodel, least_countermodel, FiniteModel};
use logicaffeine_proof::development::parse_development;
use logicaffeine_proof::formula::parse_formula;

const RULES: &str = "\
Axiom new_a: VP0(A, H1).
Axiom new_b: VP0(B, H2).
Axiom copy_c: Assign(C, A).
Axiom store_c: Store(C, Next, B).
Axiom load_d: Load(A, Next, D).
Axiom new_e: VP0(E, H3).
Axiom store_a: Store(A, Next, E).
Axiom alloc: for all v h, if VP0(v, h) then VP(v, h).
Axiom assign: for all v1 v2 h, if Assign(v1, v2) and VP(v2, h) then VP(v1, h).
Axiom store: for all v1 f v2 h1 h2, if Store(v1, f, v2) and VP(v1, h1) and VP(v2, h2) then HP(h1, f, h2).
Axiom load: for all v1 f v2 h1 h2, if Load(v1, f, v2) and VP(v1, h1) and HP(h1, f, h2) then VP(v2, h2).
";

const THEOREMS: &str = "\
Theorem c_points_to_h1: prove VP(C, H1).
Theorem h1_next_holds_h2: prove HP(H1, Next, H2).
Theorem d_points_to_h2: prove VP(D, H2).
Theorem d_points_to_h3: prove VP(D, H3).
Theorem b_points_to_h1: refute VP(B, H1).
";

fn development(theorems: &str) -> String {
    format!("## Theory PointsTo\n\n{RULES}\n{theorems}")
}

fn axioms() -> Vec<(String, logicaffeine_proof::ProofExpr)> {
    parse_development(RULES).expect("the rules parse").axioms
}

fn formula(src: &str) -> logicaffeine_proof::ProofExpr {
    parse_formula(src).expect("the formula parses")
}

/// The points-to facts the four rules derive, as the least model holds them.
fn derived(model: &FiniteModel) -> Vec<String> {
    model
        .facts
        .iter()
        .filter(|(p, _)| p == "VP" || p == "HP")
        .map(|(p, args)| format!("{p}({})", args.join(", ")))
        .collect()
}

#[test]
fn the_points_to_development_refutes_b_pointing_to_h1_beside_its_four_proofs() {
    let result = compile_theory_for_ui(&development(THEOREMS));
    assert!(result.parse_error.is_none(), "{:?}", result.parse_error);
    let names: Vec<(&str, bool, bool)> =
        result.theorems.iter().map(|t| (t.name.as_str(), t.verified, t.refuted)).collect();
    assert_eq!(
        names,
        vec![
            ("c_points_to_h1", true, false),
            ("h1_next_holds_h2", true, false),
            ("d_points_to_h2", true, false),
            ("d_points_to_h3", true, false),
            ("b_points_to_h1", true, true),
        ]
    );
    let refutation = &result.theorems[4];
    assert!(refutation.certificate.is_none(), "a refutation has no proof term to certify");
    assert!(refutation.error.is_none(), "{:?}", refutation.error);
    let model = refutation.countermodel.as_ref().expect("the refutation carries its countermodel");
    assert!(!model.facts.contains(&("VP".to_string(), vec!["B".to_string(), "H1".to_string()])));
    let mut facts = derived(model);
    facts.sort();
    assert_eq!(
        facts,
        vec![
            "HP(H1, Next, H2)",
            "HP(H1, Next, H3)",
            "VP(A, H1)",
            "VP(B, H2)",
            "VP(C, H1)",
            "VP(D, H2)",
            "VP(D, H3)",
            "VP(E, H3)",
        ],
        "the countermodel is the least model of the rules"
    );
    check_countermodel(&axioms(), &formula("VP(B, H1)"), model).expect("the countermodel re-checks");
}

#[test]
fn refuting_a_fact_that_follows_is_not_verified_and_says_why() {
    let result = compile_theory_for_ui(&development("Theorem c_wrongly: refute VP(C, H1).\n"));
    let [t] = result.theorems.as_slice() else { panic!("one theorem: {result:?}") };
    assert!(t.refuted, "it is a refutation claim");
    assert!(!t.verified, "VP(C, H1) follows from the rules");
    assert!(t.countermodel.is_none());
    let why = t.error.as_deref().unwrap_or_default();
    assert!(why.contains("holds in the least model"), "{why}");
    let err = least_countermodel(&axioms(), &formula("VP(C, H1)")).expect_err("no countermodel");
    assert!(err.contains("holds in the least model"), "{err}");
}

#[test]
fn axioms_that_are_not_a_datalog_program_cannot_refute() {
    let source = "## Theory Choice\n\nAxiom either: P(A) or Q(A).\nTheorem not_p: refute P(A).\n";
    let result = compile_theory_for_ui(source);
    let [t] = result.theorems.as_slice() else { panic!("one theorem: {result:?}") };
    assert!(!t.verified);
    let why = t.error.as_deref().unwrap_or_default();
    assert!(why.contains("not a Datalog program"), "{why}");
}

#[test]
fn a_countermodel_missing_a_derived_fact_or_holding_the_refuted_one_fails_the_recheck() {
    let named = axioms();
    let goal = formula("VP(B, H1)");
    let model = least_countermodel(&axioms(), &goal).expect("the least model refutes it");

    let mut missing = model.clone();
    missing.facts.remove(&("VP".to_string(), vec!["D".to_string(), "H3".to_string()]));
    let err = check_countermodel(&named, &goal, &missing).expect_err("the load rule fails without VP(D, H3)");
    assert!(err.contains("load"), "the failing axiom is named: {err}");

    let mut holding = model.clone();
    holding.facts.insert(("VP".to_string(), vec!["B".to_string(), "H1".to_string()]));
    let err = check_countermodel(&named, &goal, &holding).expect_err("the refuted fact holds there");
    assert!(err.contains("the refuted statement holds"), "{err}");

    let mut stranger = model.clone();
    stranger.domain.retain(|c| c != "H3");
    let err = check_countermodel(&named, &goal, &stranger).expect_err("a fact names an element outside the domain");
    assert_eq!(err, "the fact HP(H1, Next, H3) names H3, which is not in the domain");

    let mut outsider = model.clone();
    outsider.facts.insert(("VP".to_string(), vec!["Q9".to_string(), "H1".to_string()]));
    let err = check_countermodel(&named, &goal, &outsider).expect_err("a fact names an element no formula names either");
    assert_eq!(err, "the fact VP(Q9, H1) names Q9, which is not in the domain");
}

#[test]
fn a_refute_clause_parses_with_given_and_cites_and_a_second_claim_does_not() {
    let dev = parse_development(
        "Axiom f: P(A).\nTheorem p_a: prove P(A).\nTheorem no_q cites p_a: given P(A); refute Q(A).\n",
    )
    .expect("parses");
    assert_eq!(dev.theorems.len(), 1);
    let [r] = dev.refutations.as_slice() else { panic!("one refutation: {dev:?}") };
    assert_eq!(r.name, "no_q");
    assert_eq!(r.premises, vec![formula("P(A)")]);
    assert_eq!(r.goal, formula("Q(A)"));
    assert_eq!(r.cites, vec!["p_a".to_string()]);

    let err = parse_development("Theorem both: prove P(A); refute Q(A).\n").expect_err("two claims");
    assert!(err.to_string().contains("more than one claim"), "{err}");
    let err = parse_development("Theorem none: given P(A).\n").expect_err("no claim");
    assert!(err.to_string().contains("no 'prove' or 'refute' clause"), "{err}");
}

#[test]
fn largo_prove_reports_the_refutation_as_a_check_mark_and_exits_zero() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("points_to.lg");
    std::fs::write(&file, development(THEOREMS)).expect("write");
    let out = largo_in(dir.path(), &["prove", "points_to.lg"]);
    assert!(out.status.success(), "every claim holds:\n{}\n{}", stdout(&out), stderr(&out));
    let text = stdout(&out);
    let line = text
        .lines()
        .find(|l| l.contains("b_points_to_h1"))
        .unwrap_or_else(|| panic!("a line names the refutation:\n{text}"));
    assert!(line.contains('✓') && line.contains("refuted"), "{line}");
    assert!(line.contains("countermodel"), "{line}");

    let out = largo_in(dir.path(), &["prove", "points_to.lg", "--json"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let report: serde_json::Value = serde_json::from_str(stdout(&out).trim()).expect("JSON");
    let t = report["theorems"]
        .as_array()
        .and_then(|ts| ts.iter().find(|t| t["name"] == "b_points_to_h1"))
        .unwrap_or_else(|| panic!("the refutation is reported: {report}"));
    assert_eq!(t["verified"], true, "{t}");
    assert_eq!(t["refuted"], true, "{t}");
    assert!(t["countermodel"]["facts"].as_u64().is_some_and(|n| n >= 8), "{t}");
    assert_eq!(report["all_verified"], true, "{report}");
}

#[test]
fn a_broken_premise_is_named_with_the_assignment_that_breaks_it() {
    let goal = formula("VP(B, H1)");
    let mut missing = least_countermodel(&axioms(), &goal).expect("the least model refutes it");
    missing.facts.remove(&("VP".to_string(), vec!["D".to_string(), "H3".to_string()]));
    let err = check_countermodel(&axioms(), &goal, &missing).expect_err("the load rule fails");
    assert_eq!(
        err,
        "the premise `load` is false in the model at v1 = A, f = Next, v2 = D, h1 = H1, h2 = H3",
        "the message names the rule and the instance whose conclusion is missing"
    );

    let mut no_fact = least_countermodel(&axioms(), &goal).expect("the least model refutes it");
    no_fact.facts.remove(&("VP0".to_string(), vec!["A".to_string(), "H1".to_string()]));
    let err = check_countermodel(&axioms(), &goal, &no_fact).expect_err("a stated fact is gone");
    assert_eq!(err, "the premise `new_a` is false in the model");
}

#[test]
fn every_first_order_shape_is_read_in_the_least_model() {
    let refutes = |src: &str| least_countermodel(&axioms(), &formula(src));
    for src in [
        "exists h, VP(B, h) and HP(h, Next, H2)",
        "for all v, VP(v, H1)",
        "A = B",
        "VP(B, H1) or VP(E, H1)",
        "if VP(A, H1) then VP(A, H2)",
        "VP(A, H1) iff VP(A, H2)",
        "exists v, VP(v, H1) and VP(v, H2)",
        "for all v h, if VP(v, h) then VP0(v, h)",
        "for all v, not (v = A)",
    ] {
        let model = refutes(src).unwrap_or_else(|e| panic!("`{src}` is false in the least model: {e}"));
        check_countermodel(&axioms(), &formula(src), &model).unwrap_or_else(|e| panic!("`{src}`: {e}"));
    }
    for src in [
        "exists h, VP(D, h) and HP(H1, Next, h)",
        "for all v, if Assign(v, A) then VP(v, H1)",
        "A = A",
        "not VP(B, H1)",
        "VP(A, H1) iff VP(C, H1)",
        "if VP(B, H1) then VP(A, H2)",
    ] {
        let err = refutes(src).expect_err(src);
        assert!(err.contains("holds in the least model"), "`{src}`: {err}");
    }
    let follows = refutes("exists h, VP(D, h) and HP(H1, Next, h)").expect_err("positive and true");
    assert!(follows.ends_with("it follows"), "a positive statement true in the least model follows: {follows}");
    let unsettled = refutes("not VP(B, H1)").expect_err("true in the least model only");
    assert!(!unsettled.contains("it follows"), "a negation true in the least model need not follow: {unsettled}");
}

#[test]
fn a_program_with_no_constants_is_refuted_over_one_element() {
    let rules = parse_development("Axiom rule: for all x, if P(x) then Q(x).\n").expect("parses").axioms;
    let model = least_countermodel(&rules, &formula("exists x, Q(x)")).expect("nothing need be Q");
    assert_eq!(model.domain.len(), 1, "a structure has at least one element: {model:?}");
    assert!(model.facts.is_empty());
    let err = check_countermodel(&rules, &formula("exists x, Q(x)"), &FiniteModel { domain: vec![], facts: model.facts })
        .expect_err("an empty domain is no structure");
    assert!(err.contains("at least one element"), "{err}");
}

#[test]
fn a_stratified_negation_is_kept_true_in_the_countermodel() {
    let rules = parse_development(
        "Axiom e_ab: Edge(A, B).\nAxiom n_a: Node(A).\nAxiom n_b: Node(B).\nAxiom n_c: Node(C).\n\
         Axiom reach: for all x y, if Edge(x, y) then Reach(y).\n\
         Axiom lone: for all x, if Node(x) and not Reach(x) then Lone(x).\n",
    )
    .expect("parses")
    .axioms;
    let model = least_countermodel(&rules, &formula("Lone(B)")).expect("B is reached");
    assert!(model.facts.contains(&("Lone".to_string(), vec!["C".to_string()])), "{model:?}");
    let mut wrong = model.clone();
    wrong.facts.remove(&("Lone".to_string(), vec!["C".to_string()]));
    let err = check_countermodel(&rules, &formula("Lone(B)"), &wrong).expect_err("C is lone");
    assert_eq!(err, "the premise `lone` is false in the model at x = C");
}

#[test]
fn a_hand_built_model_bigger_than_the_least_one_refutes_as_well() {
    let goal = formula("VP(B, H1)");
    let mut bigger = least_countermodel(&axioms(), &goal).expect("the least model refutes it");
    bigger.domain.push("H9".to_string());
    bigger.facts.insert(("VP".to_string(), vec!["E".to_string(), "H9".to_string()]));
    bigger.facts.insert(("HP".to_string(), vec!["H1".to_string(), "Next".to_string(), "H9".to_string()]));
    bigger.facts.insert(("VP".to_string(), vec!["D".to_string(), "H9".to_string()]));
    check_countermodel(&axioms(), &goal, &bigger).expect("any model of the premises refutes, not only the least");
}

#[test]
fn a_term_the_model_cannot_interpret_is_refused_not_guessed() {
    let err = check_countermodel(
        &[("f".to_string(), formula("P(A)"))],
        &formula("Q(succ(A))"),
        &FiniteModel { domain: vec!["A".to_string()], facts: [("P".to_string(), vec!["A".to_string()])].into() },
    )
    .expect_err("a function symbol has no reading");
    assert!(err.contains("function symbol `succ`"), "{err}");
    let err = check_countermodel(
        &[("f".to_string(), formula("P(X)"))],
        &formula("Q(A)"),
        &FiniteModel { domain: vec!["A".to_string()], facts: [("P".to_string(), vec!["A".to_string()])].into() },
    )
    .expect_err("a constant names an element");
    assert_eq!(err, "the premise `f` names X, which is not in the domain");
    let err = least_countermodel(&[("r".to_string(), formula("for all x, Q(x)"))], &formula("Q(A)"))
        .expect_err("an unsafe rule is no Datalog program");
    assert!(err.contains("not a Datalog program"), "{err}");
}

#[test]
fn givens_join_the_least_model_and_cited_statements_must_hold_in_it() {
    let refuted = |theorems: &str| {
        let result = compile_theory_for_ui(&development(theorems));
        assert!(result.parse_error.is_none(), "{:?}", result.parse_error);
        result.theorems.into_iter().last().expect("a claim")
    };
    let t = refuted("Theorem with_given: given Assign(B, A); refute VP(B, H2).\n");
    assert!(!t.verified, "B also points where A does, but VP(B, H2) holds already: {t:?}");
    let t = refuted("Theorem with_given: given Assign(E, B); refute VP(E, H1).\n");
    assert!(t.verified, "{t:?}");
    let model = t.countermodel.expect("a countermodel");
    assert!(model.facts.contains(&("VP".to_string(), vec!["E".to_string(), "H2".to_string()])), "the given fired");

    let t = refuted("Theorem c_h1: prove VP(C, H1).\nTheorem cited cites c_h1: refute VP(B, H1).\n");
    assert!(t.verified && t.error.is_none(), "a proved citation holds in the least model: {t:?}");
    let t = refuted("Theorem wrong: prove VP(B, H3).\nTheorem cited cites wrong: refute VP(B, H1).\n");
    assert!(!t.verified, "an unproved citation false in the model fails the refutation: {t:?}");
    let why = t.error.unwrap_or_default();
    assert!(why.contains("the premise `wrong` is false in the model"), "{why}");
    let t = refuted("Theorem lemma: given VP(B, H1); prove VP(B, H3).\nTheorem cited cites lemma: refute VP(B, H1).\n");
    assert!(t.verified, "a cited theorem is its givens implying its goal, true where a given is false: {t:?}");
    let t = refuted("Theorem cited cites nowhere: refute VP(B, H1).\n");
    assert!(t.error.as_deref().is_some_and(|e| e.contains("cites `nowhere`, no theorem")), "{t:?}");
    let t = refuted("Theorem r1: refute VP(B, H3).\nTheorem cited cites r1: refute VP(B, H1).\n");
    assert!(t.error.as_deref().is_some_and(|e| e.contains("a refutation, which states no fact")), "{t:?}");
}

#[test]
fn claims_keep_source_order_across_two_theory_blocks() {
    let source = format!(
        "## Theory PointsTo\n\n{RULES}\nTheorem r1: refute VP(B, H1).\nTheorem p1: prove VP(C, H1).\n\n\
         ## Theory More\n\nTheorem p2: prove VP(D, H2).\nTheorem r2: refute VP(E, H1).\nTheorem p3: prove VP(D, H3).\n"
    );
    let result = compile_theory_for_ui(&source);
    let order: Vec<(&str, bool, bool)> =
        result.theorems.iter().map(|t| (t.name.as_str(), t.verified, t.refuted)).collect();
    assert_eq!(
        order,
        vec![("r1", true, true), ("p1", true, false), ("p2", true, false), ("r2", true, true), ("p3", true, false)]
    );
}

#[test]
fn the_inventory_counts_a_refutation_as_its_own_kind_and_never_a_proof() {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("points_to.lg"), development(THEOREMS)).expect("write");
    let out = largo_in(dir.path(), &["prove", "points_to.lg", "--json"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let report: serde_json::Value = serde_json::from_str(stdout(&out).trim()).expect("JSON");
    let item = report["inventory"]
        .as_array()
        .and_then(|items| items.iter().find(|i| i["name"] == "b_points_to_h1"))
        .unwrap_or_else(|| panic!("the refutation is inventoried: {report}"));
    assert_eq!(item["kind"], "refutation", "{item}");
    assert_eq!(item["status"], "refuted", "{item}");
    assert_eq!(item["proof"], false, "a countermodel is not a kernel proof: {item}");
    assert!(item.get("certificate").is_none(), "{item}");
    assert!(item["counterexample"].as_str().is_some_and(|c| c.ends_with("over 9 elements")), "{item}");
    let summary = &report["summary"];
    assert_eq!(summary["theorems"], 4, "{summary}");
    assert_eq!(summary["refutations"], 1, "{summary}");
    assert_eq!(summary["proven"], 4, "{summary}");
    assert_eq!(summary["failing"], 0, "{summary}");
    let proved = report["theorems"].as_array().expect("theorems").iter().filter(|t| t["refuted"] == false).count();
    assert_eq!(proved, 4);

    let out = largo_in(dir.path(), &["prove", "points_to.lg", "--certificate"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let certs: serde_json::Value = serde_json::from_str(stdout(&out).trim()).expect("JSON");
    let entry = certs["certificates"]
        .as_array()
        .and_then(|cs| cs.iter().find(|c| c["name"] == "b_points_to_h1"))
        .unwrap_or_else(|| panic!("the refutation has an entry: {certs}"));
    assert!(entry["certificate"].is_null(), "{entry}");
    assert_eq!(entry["refuted"], true, "{entry}");
    let facts: Vec<&str> =
        entry["countermodel"]["facts"].as_array().expect("facts").iter().filter_map(|f| f.as_str()).collect();
    assert!(facts.contains(&"VP(D, H3)") && !facts.contains(&"VP(B, H1)"), "{facts:?}");
    assert_eq!(entry["countermodel"]["domain"].as_array().map(Vec::len), Some(9), "{entry}");
}

#[test]
fn a_refutation_that_fails_makes_largo_prove_fail_and_says_why() {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("wrong.lg"), development("Theorem c_wrongly: refute VP(C, H1).\n")).expect("write");
    let out = largo_in(dir.path(), &["prove", "wrong.lg"]);
    assert!(!out.status.success(), "a claim failed:\n{}", stdout(&out));
    let text = stdout(&out);
    let line = text.lines().find(|l| l.contains("c_wrongly")).unwrap_or_else(|| panic!("{text}"));
    assert!(line.contains('✗') && line.contains("holds in the least model") && line.contains("it follows"), "{line}");

    let out = largo_in(dir.path(), &["prove", "wrong.lg", "--json"]);
    let report: serde_json::Value = serde_json::from_str(stdout(&out).trim()).expect("JSON");
    let item = report["inventory"].as_array().and_then(|items| items.first()).expect("an item");
    assert_eq!((item["kind"].as_str(), item["status"].as_str()), (Some("refutation"), Some("unrefuted")), "{item}");
    assert_eq!(report["summary"]["failing"], 1, "{report}");
    assert_eq!(report["all_verified"], false, "{report}");
}

#[test]
fn the_proof_guides_refutation_example_holds_as_the_guide_says() {
    let guide = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/proof-and-verification.md"))
        .expect("read docs/proof-and-verification.md");
    let example = guide
        .split("```logos\n")
        .skip(1)
        .filter_map(|block| block.split("\n```").next())
        .find(|block| block.contains(": refute "))
        .expect("the guide shows a refutation");
    let result = compile_theory_for_ui(example);
    assert!(result.all_verified(), "{result:?}");
    let claims: Vec<(&str, bool)> = result.theorems.iter().map(|t| (t.name.as_str(), t.refuted)).collect();
    assert_eq!(claims, vec![("b_points_to_h1", false), ("c_stays_apart", true)]);
}

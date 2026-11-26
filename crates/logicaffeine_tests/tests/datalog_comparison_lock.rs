//! ★ LOCK — Datalog rules compare numbers and tell constants apart. ★
//!
//! An engineering rule is a Horn rule with a limit in it: a string fits a controller when its cold voltage is at most
//! the controller's limit. A rule's condition may hold comparisons beside its atoms (`x ≤ y`, `x < y`, `x ≥ y`,
//! `x > y`, `x = y`, `x ≠ y`, and the ASCII spellings `<=`, `>=`, `!=`), each a guard over values its positive atoms
//! bind:
//!
//! - numbers compare as exact rationals, decimals and negatives included (`107.1 ≤ 145`, `-10 < 0`);
//! - `=` and `≠` tell any two constants apart, numbers by value (`10 = 10.0`);
//! - an ordering between constants that are not numbers is refused by name, never read as false;
//! - a guard over a variable no positive atom binds is unsafe, and refused;
//! - naive and semi-naive evaluation reach the same model, and a derivation's premises are its atoms;
//! - a fact that rests on a comparison or a negation reads the closed world, which the kernel does not certify from
//!   the axioms alone, and the model says which facts those are.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test datalog_comparison_lock`

use logicaffeine_proof::datalog::{evaluate, evaluate_bdd, evaluate_naive, DatalogError, Model, Program};
use logicaffeine_proof::development::parse_development;
use std::collections::BTreeSet;

fn program(theory: &str) -> Program {
    let development = parse_development(theory).expect("the theory parses");
    Program::from_axioms(&development.axioms).expect("a Datalog program")
}

fn model(theory: &str) -> Model {
    let p = program(theory);
    let semi = evaluate(&p).expect("stratifiable");
    let naive = evaluate_naive(&p).expect("stratifiable");
    for (predicate, args) in semi.facts() {
        assert!(
            naive.holds(&predicate, &args.iter().map(String::as_str).collect::<Vec<_>>()),
            "naive evaluation misses {predicate}{args:?}"
        );
    }
    assert_eq!(semi.facts().len(), naive.facts().len(), "naive and semi-naive models differ");
    semi
}

fn tuples(rows: &[&[&str]]) -> BTreeSet<Vec<String>> {
    rows.iter().map(|r| r.iter().map(|s| s.to_string()).collect()).collect()
}

const STRINGS: &str = "Axiom c1: VocMax(V150_35, 145).
Axiom c2: VocMax(V100_30, 100).
Axiom c3: VocMax(V250_60, 245).
Axiom s1: ColdVoc(F4, 107.1).
Axiom s2: ColdVoc(F6, 160.6).
Axiom s3: ColdVoc(F2, 53.55).
Axiom fits: for all a c x y, if ColdVoc(a, x) and VocMax(c, y) and x ≤ y then Fits(a, c).
";

#[test]
fn a_limit_is_a_comparison_of_exact_numbers() {
    let m = model(STRINGS);
    assert_eq!(
        m.relation("Fits"),
        tuples(&[
            &["F2", "V100_30"],
            &["F2", "V150_35"],
            &["F2", "V250_60"],
            &["F4", "V150_35"],
            &["F4", "V250_60"],
            &["F6", "V250_60"],
        ])
    );
}

#[test]
fn every_ordering_and_its_ascii_spelling() {
    let theory = |op: &str| {
        format!(
            "Axiom a: Value(One, 1).\nAxiom b: Value(Two, 2).\nAxiom c: Value(OtherTwo, 2.0).\n\
             Axiom r: for all a b x y, if Value(a, x) and Value(b, y) and x {op} y then R(a, b).\n"
        )
    };
    let pairs = |op: &str| m_pairs(&model(&theory(op)));
    let lt = tuples(&[&["One", "Two"], &["One", "OtherTwo"]]);
    let le = tuples(&[
        &["One", "One"],
        &["One", "Two"],
        &["One", "OtherTwo"],
        &["Two", "Two"],
        &["Two", "OtherTwo"],
        &["OtherTwo", "Two"],
        &["OtherTwo", "OtherTwo"],
    ]);
    assert_eq!(pairs("<"), lt);
    assert_eq!(pairs("≤"), le);
    assert_eq!(pairs("<="), le);
    assert_eq!(pairs(">"), lt.iter().map(|p| vec![p[1].clone(), p[0].clone()]).collect());
    assert_eq!(pairs("≥"), le.iter().map(|p| vec![p[1].clone(), p[0].clone()]).collect());
    assert_eq!(pairs(">="), pairs("≥"));
    let eq = tuples(&[
        &["One", "One"],
        &["Two", "Two"],
        &["Two", "OtherTwo"],
        &["OtherTwo", "Two"],
        &["OtherTwo", "OtherTwo"],
    ]);
    assert_eq!(pairs("="), eq);
    let all: BTreeSet<Vec<String>> = ["One", "Two", "OtherTwo"]
        .iter()
        .flat_map(|a| ["One", "Two", "OtherTwo"].iter().map(move |b| vec![a.to_string(), b.to_string()]))
        .collect();
    let neq: BTreeSet<Vec<String>> = all.difference(&eq).cloned().collect();
    assert_eq!(pairs("≠"), neq);
    assert_eq!(pairs("!="), neq);
}

fn m_pairs(m: &Model) -> BTreeSet<Vec<String>> {
    m.relation("R")
}

#[test]
fn constants_are_told_apart_without_listing_every_pair() {
    let m = model(
        "Axiom k1: Kit(PC1).\nAxiom k2: Kit(PC2).\nAxiom k3: Kit(PC3).\n\
         Axiom pair: for all a b, if Kit(a) and Kit(b) and a ≠ b then Pair(a, b).\n\
         Axiom same: for all a b, if Kit(a) and Kit(b) and not a = b then Other(a, b).\n",
    );
    assert_eq!(m.relation("Pair").len(), 6);
    assert_eq!(m.relation("Pair"), m.relation("Other"));
    assert!(!m.holds("Pair", &["PC1", "PC1"]));
}

#[test]
fn negative_numbers_compare_by_sign() {
    let m = model(
        "Axiom a: Temp(Morning, -10).\nAxiom b: Temp(Noon, 18).\nAxiom c: Temp(Midnight, -0.5).\n\
         Axiom freeze: for all p t, if Temp(p, t) and t < 0 then Freezing(p).\n",
    );
    assert_eq!(m.relation("Freezing"), tuples(&[&["Midnight"], &["Morning"]]));
}

#[test]
fn a_comparison_derives_from_its_atoms_alone() {
    let m = model(STRINGS);
    let d = m.derivation("Fits", &["F4".into(), "V150_35".into()]).expect("derived");
    assert_eq!(d.rule.as_deref(), Some("fits"));
    let premises: BTreeSet<String> = d.premises.iter().map(|(p, _)| p.clone()).collect();
    assert_eq!(premises, ["ColdVoc", "VocMax"].iter().map(|s| s.to_string()).collect());
}

#[test]
fn an_ordering_between_names_is_refused() {
    let p = program(
        "Axiom k1: Kit(PC1).\nAxiom k2: Kit(PC2).\n\
         Axiom r: for all a b, if Kit(a) and Kit(b) and a < b then Before(a, b).\n",
    );
    match evaluate(&p) {
        Err(DatalogError::NotANumber { rule, value }) => {
            assert_eq!(rule, "r");
            assert!(value == "PC1" || value == "PC2", "names the constant: {value}");
        }
        other => panic!("an ordering of names is refused, got {other:?}"),
    }
    let shown = evaluate(&p).unwrap_err().to_string();
    assert!(shown.contains("not a number"), "{shown}");
}

#[test]
fn a_guard_over_an_unbound_variable_is_unsafe() {
    let development = parse_development(
        "Axiom k1: Kit(PC1).\nAxiom r: for all a b, if Kit(a) and a ≠ b then X(a).\n",
    )
    .expect("parses");
    match Program::from_axioms(&development.axioms) {
        Err(DatalogError::Unsafe { rule, variable }) => {
            assert_eq!(rule, "r");
            assert_eq!(variable, "b");
        }
        other => panic!("an unbound guard variable is unsafe, got {other:?}"),
    }
}

#[test]
fn diagrams_refuse_a_comparison_by_name() {
    match evaluate_bdd(&program(STRINGS)) {
        Err(DatalogError::Guarded { rule }) => assert_eq!(rule, "fits"),
        Err(other) => panic!("the diagram evaluation names the guarded rule, got {other:?}"),
        Ok(_) => panic!("the diagram evaluation of a guarded rule is refused"),
    }
}

#[test]
fn the_model_names_the_facts_that_read_the_closed_world() {
    let m = model(
        "Axiom p1: Provides(Ctrl, Dc).\nAxiom p2: Requires(Radio, Dc).\nAxiom p3: Requires(Camera, Poe).\n\
         Axiom p4: Watts(Radio, 9).\nAxiom r1: for all a b c, if Requires(a, b) and Provides(c, b) then Met(a, b).\n\
         Axiom r2: for all a b, if Requires(a, b) and not Met(a, b) then Unmet(a, b).\n\
         Axiom r3: for all a w, if Watts(a, w) and w ≤ 10 then Small(a).\n\
         Axiom r4: for all a b, if Unmet(a, b) then Flagged(a).\n",
    );
    assert!(!m.closed_world("Met", &["Radio".into(), "Dc".into()]), "Met rests on atoms alone");
    assert!(m.closed_world("Unmet", &["Camera".into(), "Poe".into()]), "Unmet rests on a negation");
    assert!(m.closed_world("Small", &["Radio".into()]), "Small rests on a comparison");
    assert!(m.closed_world("Flagged", &["Camera".into()]), "Flagged rests on Unmet, so on a negation");
}

//! ★ LOCK — Datalog: Horn rules evaluated bottom-up to their least model. ★
//!
//! A `## Theory` whose axioms are function-free Horn rules and ground facts is a Datalog program. Evaluated bottom-up,
//! semi-naively and stratum by stratum under stratified negation, it yields its least model: every fact the rules
//! derive, each with the rule and premises that derived it. The model is held to three independent answers:
//!
//! - the naive evaluation (every rule re-fired over the whole model until nothing changes);
//! - the program grounded over its constants and solved as propositional Horn-SAT (`hornsat`, its own least model);
//! - the kernel: every derived fact is a theorem the certified prover proves from the axioms, and a fact outside the
//!   model is not.
//!
//! The same programs evaluated over relations held as binary decision diagrams (bddbddb's representation: a relation
//! is the characteristic function of its tuples' bit encodings, a join is a relational product) derive the same
//! facts, and a long chain's transitive closure fits in a diagram a hundred times smaller than its tuple count.
//!
//! Semi-naive evaluation fires each rule only on what the last round derived, so on a long chain it tries far fewer
//! rule instances than naive evaluation. A program that is not Datalog, a rule whose head names a variable its
//! body does not bind, and a negation through recursion are refused by name.
//!
//! The first program is the context-insensitive points-to analysis of Whaley and Lam (PLDI 2004) in their four rules.

use logicaffeine_proof::datalog::{
    evaluate, evaluate_bdd, evaluate_naive, Atom, DatalogError, Literal, Model, Program, Rule, Term,
};
use logicaffeine_proof::development::parse_development;
use logicaffeine_proof::formula::parse_formula;
use logicaffeine_proof::hornsat::{solve, HornClause, HornOutcome};
use logicaffeine_proof::verify::prove_certify_check;
use std::collections::{BTreeMap, BTreeSet};

const POINTS_TO: &str = "\
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

fn program(theory: &str) -> Program {
    let development = parse_development(theory).expect("the theory parses");
    Program::from_axioms(&development.axioms).expect("the theory is a Datalog program")
}

fn tuples(rows: &[&[&str]]) -> BTreeSet<Vec<String>> {
    rows.iter().map(|r| r.iter().map(|s| s.to_string()).collect()).collect()
}

#[test]
fn the_four_rules_derive_every_points_to_fact_and_no_other() {
    let model = evaluate(&program(POINTS_TO)).expect("stratifiable");
    assert_eq!(
        model.relation("VP"),
        tuples(&[&["A", "H1"], &["B", "H2"], &["C", "H1"], &["D", "H2"], &["D", "H3"], &["E", "H3"]])
    );
    assert_eq!(model.relation("HP"), tuples(&[&["H1", "Next", "H2"], &["H1", "Next", "H3"]]));
    assert!(model.holds("VP", &["D", "H3"]), "d = a.next after a.next = e");
    assert!(!model.holds("VP", &["B", "H1"]), "b never points to h1");
}

#[test]
fn every_derived_fact_has_the_derivation_that_made_it() {
    let model = evaluate(&program(POINTS_TO)).expect("stratifiable");
    for (predicate, args) in model.facts() {
        let derivation = model.derivation(&predicate, &args).expect("every fact in the model was derived or given");
        for (p, a) in &derivation.premises {
            assert!(model.holds(p, &a.iter().map(String::as_str).collect::<Vec<_>>()), "a premise of {predicate}{args:?}");
        }
        match &derivation.rule {
            None => assert!(derivation.premises.is_empty(), "a given fact rests on nothing: {predicate}{args:?}"),
            Some(rule) => assert!(!derivation.premises.is_empty(), "rule {rule} derived {predicate}{args:?} from nothing"),
        }
    }
    let d = model.derivation("VP", &["D".into(), "H3".into()]).expect("derived");
    assert_eq!(d.rule.as_deref(), Some("load"));
}

#[test]
fn every_derived_fact_is_a_kernel_certified_theorem_and_a_missing_one_is_not() {
    let development = parse_development(POINTS_TO).expect("parses");
    let axioms = development.axiom_exprs();
    let model = evaluate(&Program::from_axioms(&development.axioms).expect("Datalog")).expect("stratifiable");
    for (predicate, args) in model.facts() {
        let goal = parse_formula(&format!("{predicate}({})", args.join(", "))).expect("a ground atom");
        let proof = prove_certify_check(&axioms, &goal);
        assert!(proof.verified, "{predicate}({}) is derived, so the kernel proves it: {:?}", args.join(", "), proof.verification_error);
    }
    for missing in ["VP(B, H1)", "VP(A, H2)", "HP(H2, Next, H1)"] {
        let goal = parse_formula(missing).expect("a ground atom");
        assert!(!prove_certify_check(&axioms, &goal).verified, "{missing} is not in the least model");
    }
}

// ── a random program, three ways ─────────────────────────────────────────────────────────────────────────────────

struct SplitMix(u64);
impl SplitMix {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

const VARS: [&str; 3] = ["X", "Y", "Z"];

/// A random program over two binary base relations `e0`, `e1` and three derived ones `p0`, `p1`, `p2`, every rule
/// safe; with `negation`, a body may also deny a base fact.
fn random_program(rng: &mut SplitMix, negation: bool) -> Program {
    let constants = ["c0", "c1", "c2", "c3", "c4", "c5"];
    let mut facts = Vec::new();
    for base in ["e0", "e1"] {
        for _ in 0..rng.below(9) + 2 {
            let (a, b) = (constants[rng.below(6)], constants[rng.below(6)]);
            facts.push(Atom::new(base, vec![Term::constant(a), Term::constant(b)]));
        }
    }
    let preds = ["e0", "e1", "p0", "p1", "p2"];
    let mut rules = Vec::new();
    for r in 0..rng.below(5) + 2 {
        let head_pred = ["p0", "p1", "p2"][rng.below(3)];
        let mut body = Vec::new();
        for _ in 0..rng.below(3) + 1 {
            let pred = preds[rng.below(5)];
            let args = vec![Term::var(VARS[rng.below(3)]), Term::var(VARS[rng.below(3)])];
            body.push(Literal::Pos(Atom::new(pred, args)));
        }
        let (h0, h1) = (VARS[rng.below(3)], VARS[rng.below(3)]);
        // Safety: bind both head variables with a base literal.
        body.push(Literal::Pos(Atom::new("e0", vec![Term::var(h0), Term::var(h1)])));
        if negation && rng.below(3) == 0 {
            body.push(Literal::Neg(Atom::new("e1", vec![Term::var(h1), Term::var(h0)])));
        }
        rules.push(Rule::new(format!("r{r}"), Atom::new(head_pred, vec![Term::var(h0), Term::var(h1)]), body));
    }
    Program::new(facts, rules)
}

/// The program grounded over its constants and solved as propositional Horn-SAT: the least model by another road.
fn grounded_least_model(p: &Program) -> BTreeMap<String, BTreeSet<Vec<String>>> {
    let constants: Vec<String> = p.constants().into_iter().collect();
    let mut ids: BTreeMap<(String, Vec<String>), usize> = BTreeMap::new();
    let id = |pred: &str, args: Vec<String>, ids: &mut BTreeMap<(String, Vec<String>), usize>| {
        let n = ids.len();
        *ids.entry((pred.to_string(), args)).or_insert(n)
    };
    let mut clauses = Vec::new();
    for f in p.facts() {
        clauses.push(HornClause::fact(id(&f.predicate, f.ground().expect("a fact is ground"), &mut ids)));
    }
    for rule in p.rules() {
        let vars: Vec<String> = rule.variables().into_iter().collect();
        let mut assignment = vec![0usize; vars.len()];
        loop {
            let value = |t: &Term| match t {
                Term::Const(c) => c.clone(),
                Term::Var(v) => constants[assignment[vars.iter().position(|x| x == v).unwrap()]].clone(),
            };
            let body: Vec<usize> = rule
                .body
                .iter()
                .map(|l| match l {
                    Literal::Pos(a) => id(&a.predicate, a.terms.iter().map(value).collect(), &mut ids),
                    Literal::Neg(_) => unreachable!("the Horn oracle is for positive programs"),
                })
                .collect();
            let head = id(&rule.head.predicate, rule.head.terms.iter().map(value).collect(), &mut ids);
            clauses.push(HornClause::rule(body, head));
            let mut k = 0;
            while k < assignment.len() {
                assignment[k] += 1;
                if assignment[k] < constants.len() {
                    break;
                }
                assignment[k] = 0;
                k += 1;
            }
            if k == assignment.len() {
                break;
            }
        }
    }
    let HornOutcome::Sat(model) = solve(&clauses, ids.len()) else {
        panic!("a program with no goal clause is satisfiable");
    };
    let mut out: BTreeMap<String, BTreeSet<Vec<String>>> = BTreeMap::new();
    for ((pred, args), i) in ids {
        if model[i] {
            out.entry(pred).or_default().insert(args);
        }
    }
    out
}

fn as_map(model: &Model) -> BTreeMap<String, BTreeSet<Vec<String>>> {
    let mut out: BTreeMap<String, BTreeSet<Vec<String>>> = BTreeMap::new();
    for (pred, args) in model.facts() {
        out.entry(pred).or_default().insert(args);
    }
    out
}

#[test]
fn semi_naive_naive_and_grounded_horn_sat_agree_on_random_programs() {
    let mut rng = SplitMix(0x5EED_DA7A_1060_0001);
    for case in 0..300 {
        let p = random_program(&mut rng, false);
        let semi = evaluate(&p).expect("a positive program is stratifiable");
        let naive = evaluate_naive(&p).expect("a positive program is stratifiable");
        assert_eq!(as_map(&semi), as_map(&naive), "case {case}: semi-naive and naive differ on\n{p}");
        assert_eq!(as_map(&semi), grounded_least_model(&p), "case {case}: the least model differs from Horn-SAT's on\n{p}");
    }
}

#[test]
fn semi_naive_and_naive_agree_under_stratified_negation() {
    let mut rng = SplitMix(0x5EED_DA7A_1060_0002);
    for case in 0..300 {
        let p = random_program(&mut rng, true);
        let semi = evaluate(&p).expect("negation of a base relation is stratified");
        let naive = evaluate_naive(&p).expect("negation of a base relation is stratified");
        assert_eq!(as_map(&semi), as_map(&naive), "case {case}: semi-naive and naive differ on\n{p}");
    }
}

#[test]
fn a_negation_reads_the_finished_lower_stratum() {
    let p = program(
        "Axiom n1: Node(N1).\nAxiom n2: Node(N2).\nAxiom n3: Node(N3).\nAxiom n4: Node(N4).\n\
         Axiom e12: Edge(N1, N2).\nAxiom e23: Edge(N2, N3).\nAxiom root: Reach(N1).\n\
         Axiom step: for all x y, if Reach(x) and Edge(x, y) then Reach(y).\n\
         Axiom cut: for all x, if Node(x) and not Reach(x) then Unreached(x).\n",
    );
    let model = evaluate(&p).expect("Unreached depends on Reach only through negation, one stratum down");
    assert_eq!(model.relation("Reach"), tuples(&[&["N1"], &["N2"], &["N3"]]));
    assert_eq!(model.relation("Unreached"), tuples(&[&["N4"]]));
}

#[test]
fn semi_naive_tries_far_fewer_rule_instances_than_naive_on_a_chain() {
    let mut facts = Vec::new();
    for i in 0..120 {
        facts.push(Atom::new("edge", vec![Term::constant(&format!("n{i}")), Term::constant(&format!("n{}", i + 1))]));
    }
    let x = || Term::var("X");
    let y = || Term::var("Y");
    let z = || Term::var("Z");
    let rules = vec![
        Rule::new("base", Atom::new("path", vec![x(), y()]), vec![Literal::Pos(Atom::new("edge", vec![x(), y()]))]),
        Rule::new(
            "step",
            Atom::new("path", vec![x(), z()]),
            vec![Literal::Pos(Atom::new("path", vec![x(), y()])), Literal::Pos(Atom::new("edge", vec![y(), z()]))],
        ),
    ];
    let p = Program::new(facts, rules);
    let semi = evaluate(&p).expect("positive");
    let naive = evaluate_naive(&p).expect("positive");
    assert_eq!(semi.relation("path").len(), 120 * 121 / 2, "every pair along the chain");
    assert_eq!(as_map(&semi), as_map(&naive));
    assert!(
        semi.work() * 10 < naive.work(),
        "semi-naive tried {} rule instances, naive {}: each round should join only the last round's facts",
        semi.work(),
        naive.work()
    );
}

#[test]
fn a_negation_through_recursion_is_refused_by_name() {
    let p = program(
        "Axiom a: Base(K).\n\
         Axiom win: for all x, if Base(x) and not Lose(x) then Win(x).\n\
         Axiom lose: for all x, if Base(x) and not Win(x) then Lose(x).\n",
    );
    match evaluate(&p) {
        Err(DatalogError::Unstratifiable { predicates }) => {
            assert!(predicates.contains(&"Win".to_string()) && predicates.contains(&"Lose".to_string()), "{predicates:?}");
        }
        other => panic!("Win and Lose deny each other: no stratification exists, got {other:?}"),
    }
}

#[test]
fn a_rule_whose_head_variable_is_unbound_is_refused() {
    let development = parse_development("Axiom loose: for all x y, if Base(x) then Pair(x, y).\n").expect("parses");
    match Program::from_axioms(&development.axioms) {
        Err(DatalogError::Unsafe { rule, variable }) => {
            assert_eq!(rule, "loose");
            assert_eq!(variable, "y");
        }
        other => panic!("y appears in no body literal: the rule ranges over an unbound domain, got {other:?}"),
    }
}

#[test]
fn a_theory_that_is_not_datalog_is_refused_with_its_axiom() {
    for (axiom, theory) in [
        ("some", "Axiom some: there exists x, Base(x).\n"),
        ("either", "Axiom either: for all x, if Base(x) then Red(x) or Blue(x).\n"),
    ] {
        let development = parse_development(theory).expect("parses");
        match Program::from_axioms(&development.axioms) {
            Err(DatalogError::NotDatalog { axiom: named, .. }) => assert_eq!(named, axiom),
            other => panic!("`{axiom}` is not a Horn rule or a ground fact, got {other:?}"),
        }
    }
}

// ── relations as binary decision diagrams ──────────────────────────────────────────────────────────────────────────

#[test]
fn the_bdd_engine_derives_the_same_points_to_facts() {
    let p = program(POINTS_TO);
    let hashed = evaluate(&p).expect("stratifiable");
    let diagrams = evaluate_bdd(&p).expect("stratifiable");
    for relation in ["VP0", "Assign", "Store", "Load", "VP", "HP"] {
        assert_eq!(diagrams.relation(relation), hashed.relation(relation), "{relation}");
    }
    assert!(diagrams.holds("VP", &["D", "H3"]));
    assert!(!diagrams.holds("VP", &["B", "H1"]));
}

#[test]
fn bdd_relations_equal_the_hash_engines_on_random_programs() {
    for (seed, negation) in [(0x5EED_DA7A_1060_0003u64, false), (0x5EED_DA7A_1060_0004, true)] {
        let mut rng = SplitMix(seed);
        for case in 0..200 {
            let p = random_program(&mut rng, negation);
            let hashed = evaluate(&p).expect("stratified");
            let diagrams = evaluate_bdd(&p).expect("stratified");
            for relation in ["e0", "e1", "p0", "p1", "p2"] {
                assert_eq!(
                    diagrams.relation(relation),
                    hashed.relation(relation),
                    "case {case} (negation {negation}), relation {relation}, on\n{p}"
                );
            }
        }
    }
}

#[test]
fn a_chains_closure_is_a_diagram_far_smaller_than_its_tuples() {
    let mut facts = Vec::new();
    for i in 0..255 {
        facts.push(Atom::new("edge", vec![Term::constant(&format!("n{i}")), Term::constant(&format!("n{}", i + 1))]));
    }
    let x = || Term::var("X");
    let y = || Term::var("Y");
    let z = || Term::var("Z");
    let rules = vec![
        Rule::new("base", Atom::new("path", vec![x(), y()]), vec![Literal::Pos(Atom::new("edge", vec![x(), y()]))]),
        Rule::new(
            "step",
            Atom::new("path", vec![x(), z()]),
            vec![Literal::Pos(Atom::new("path", vec![x(), y()])), Literal::Pos(Atom::new("edge", vec![y(), z()]))],
        ),
    ];
    let diagrams = evaluate_bdd(&Program::new(facts, rules)).expect("positive");
    let tuples = diagrams.relation("path").len();
    assert_eq!(tuples, 255 * 256 / 2, "every pair along the chain");
    assert!(
        diagrams.nodes("path") * 100 < tuples,
        "{tuples} tuples in {} diagram nodes: the order-interleaved encoding keeps `x < y` linear in the bits",
        diagrams.nodes("path")
    );
}

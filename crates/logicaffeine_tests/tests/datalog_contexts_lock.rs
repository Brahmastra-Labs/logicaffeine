//! ★ LOCK — context-sensitive Datalog: every call path its own context, numbered and held as diagrams. ★
//!
//! Whaley and Lam (PLDI 2004) make a points-to analysis context-sensitive by cloning every method once per call path
//! that reaches it, and make the clones affordable by numbering the paths and keeping the relations over those numbers
//! as binary decision diagrams. A call graph whose methods each call the next twice has 2^k paths to its k-th method; the
//! paths' numbers are consecutive ranges, and "callee context = caller context + the call edge's offset" is a diagram
//! linear in the bits, so the relations stay small however many contexts there are.
//!
//! `Contexts::number` numbers the call paths of a program's `Calls(caller, site, callee)` facts: a method's contexts are
//! `0..n`, n the number of call paths reaching it; a method no other calls is a root, with one context; the methods of a
//! recursive cycle share one set of contexts, a call inside the cycle keeping its context. Evaluating over diagrams with
//! the numbering gives the rules two relations computed from it, `Context(method, c)` and `CallContext(c, site, d, callee)`
//! (site, in context c of its caller, calls the callee in context d). The lock holds the numbering and the evaluation
//! to independent answers:
//!
//! - the numbering is a bijection: every call path to a method has its own number below the method's count, and the
//!   numbers `CallContext` composes along a path are exactly those;
//! - the diagrams' relations equal the hash engine's on the same program with the numbering written out as facts;
//! - the context-sensitive points-to rules agree with the context-insensitive ones run on the program cloned once per
//!   call path, the definition of context sensitivity;
//! - on the identity function called twice, the context-sensitive answer keeps the two calls apart and the
//!   context-insensitive one merges them;
//! - a call chain of depth 40 has 2^40 contexts at its bottom and 2^42 − 3 points-to tuples, held in a diagram whose
//!   size grows polynomially in the depth.
//!
//! The call graph may be one the rules discover (Whaley and Lam's Algorithm 6: a virtual call's targets from its
//! receiver's context-insensitive points-to set): it is evaluated to completion first and numbered then. A program
//! that states a computed relation itself, derives `Calls` from the contexts numbered over it, uses one variable as a
//! context and as a constant, or has more call paths to a method than 64 bits number is refused by name.

use logicaffeine_proof::datalog::{
    evaluate, evaluate_bdd_with_contexts, Atom, BddModel, Contexts, DatalogError, Literal, Model, Program, Rule, Term,
};
use logicaffeine_proof::development::parse_development;
use std::collections::{BTreeMap, BTreeSet};

/// Whaley and Lam's context-sensitive points-to analysis over a precomputed call graph, in LOGOS's theory notation.
const CONTEXT_SENSITIVE: &str = "\
Axiom alloc: for all v h m c, if VP0(v, h) and Declares(m, v) and Context(m, c) then VPC(c, v, h).
Axiom assign: for all c v1 v2 h, if Assign(v1, v2) and VPC(c, v2, h) then VPC(c, v1, h).
Axiom pass: for all c i d n z a f h, if CallContext(c, i, d, n) and Actual(i, z, a) and Formal(n, z, f) and VPC(c, a, h) then VPC(d, f, h).
Axiom result: for all c i d n v r h, if CallContext(c, i, d, n) and Returns(n, v) and Receives(i, r) and VPC(d, v, h) then VPC(c, r, h).
Axiom store: for all c v1 f v2 h1 h2, if Store(v1, f, v2) and VPC(c, v1, h1) and VPC(c, v2, h2) then HP(h1, f, h2).
Axiom load: for all c v1 f v2 h1 h2, if Load(v1, f, v2) and VPC(c, v1, h1) and HP(h1, f, h2) then VPC(c, v2, h2).
Axiom project: for all c v h, if VPC(c, v, h) then VP(v, h).
";

/// The same analysis with every context merged: a call passes its arguments to the one copy of its callee.
const CONTEXT_INSENSITIVE: &str = "\
Axiom alloc: for all v h, if VP0(v, h) then VP(v, h).
Axiom assign: for all v1 v2 h, if Assign(v1, v2) and VP(v2, h) then VP(v1, h).
Axiom pass: for all m i n z a f h, if Calls(m, i, n) and Actual(i, z, a) and Formal(n, z, f) and VP(a, h) then VP(f, h).
Axiom result: for all m i n v r h, if Calls(m, i, n) and Returns(n, v) and Receives(i, r) and VP(v, h) then VP(r, h).
Axiom store: for all v1 f v2 h1 h2, if Store(v1, f, v2) and VP(v1, h1) and VP(v2, h2) then HP(h1, f, h2).
Axiom load: for all v1 f v2 h1 h2, if Load(v1, f, v2) and VP(v1, h1) and HP(h1, f, h2) then VP(v2, h2).
";

/// `x = id(a); y = id(b)` with `id(p) = p`.
const IDENTITY: &str = "\
Axiom new_a: VP0(A, H1).
Axiom new_b: VP0(B, H2).
Axiom main_a: Declares(Main, A).
Axiom main_b: Declares(Main, B).
Axiom main_x: Declares(Main, X).
Axiom main_y: Declares(Main, Y).
Axiom id_p: Declares(Id, P).
Axiom call_1: Calls(Main, S1, Id).
Axiom call_2: Calls(Main, S2, Id).
Axiom pass_1: Actual(S1, Z0, A).
Axiom pass_2: Actual(S2, Z0, B).
Axiom formal: Formal(Id, Z0, P).
Axiom returns: Returns(Id, P).
Axiom result_1: Receives(S1, X).
Axiom result_2: Receives(S2, Y).
";

fn program(theory: &str) -> Program {
    let development = parse_development(theory).expect("the theory parses");
    Program::from_axioms(&development.axioms).expect("the theory is a Datalog program")
}

fn rules_of(theory: &str) -> Vec<Rule> {
    program(theory).rules().to_vec()
}

fn tuples(rows: &[&[&str]]) -> BTreeSet<Vec<String>> {
    rows.iter().map(|r| r.iter().map(|s| s.to_string()).collect()).collect()
}

fn atom(predicate: &str, args: &[&str]) -> Atom {
    Atom::new(predicate, args.iter().map(|a| Term::constant(a)).collect())
}

fn with_numbering(p: &Program, contexts: &Contexts) -> Program {
    Program::new(p.facts().iter().cloned().chain(contexts.facts()).collect(), p.rules().to_vec())
}

fn number(p: &Program) -> Contexts {
    Contexts::number(p).unwrap_or_else(|e| panic!("the call graph numbers: {e}\n{p}"))
}

fn in_contexts(p: &Program) -> (Contexts, BddModel) {
    let contexts = number(p);
    let model = evaluate_bdd_with_contexts(p, &contexts).unwrap_or_else(|e| panic!("evaluates: {e}\n{p}"));
    (contexts, model)
}

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

/// A random program for the points-to rules: two to five methods of three variables each, allocations, copies,
/// field stores and loads inside each method, and call sites passing one argument and maybe receiving a result.
/// Without `recursion` every call goes to a later method, so the call graph is acyclic; with it a call may go to any
/// method, the caller included.
fn random_facts(rng: &mut SplitMix, recursion: bool) -> Vec<Atom> {
    let methods = rng.below(4) + 2;
    let var = |m: usize, t: usize| format!("V{m}_{t}");
    let mut facts = Vec::new();
    let mut site = 0;
    for m in 0..methods {
        let method = format!("M{m}");
        for t in 0..3 {
            facts.push(atom("Declares", &[&method, &var(m, t)]));
        }
        for k in 0..rng.below(2) + 1 {
            facts.push(atom("VP0", &[&var(m, rng.below(3)), &format!("H{m}_{k}")]));
        }
        for _ in 0..rng.below(3) {
            facts.push(atom("Assign", &[&var(m, rng.below(3)), &var(m, rng.below(3))]));
        }
        for _ in 0..rng.below(2) {
            let field = ["F0", "F1"][rng.below(2)];
            facts.push(atom("Store", &[&var(m, rng.below(3)), field, &var(m, rng.below(3))]));
        }
        for _ in 0..rng.below(2) {
            let field = ["F0", "F1"][rng.below(2)];
            facts.push(atom("Load", &[&var(m, rng.below(3)), field, &var(m, rng.below(3))]));
        }
        if m > 0 {
            facts.push(atom("Formal", &[&method, "Z0", &var(m, 0)]));
            if rng.below(2) == 0 {
                facts.push(atom("Returns", &[&method, &var(m, 1)]));
            }
        }
        let callees: Vec<usize> = if recursion { (1..methods).collect() } else { (m + 1..methods).collect() };
        if callees.is_empty() {
            continue;
        }
        for _ in 0..rng.below(3) {
            let callee = callees[rng.below(callees.len())];
            let s = format!("S{site}");
            site += 1;
            facts.push(atom("Calls", &[&method, &s, &format!("M{callee}")]));
            facts.push(atom("Actual", &[&s, "Z0", &var(m, rng.below(3))]));
            if rng.below(2) == 0 {
                facts.push(atom("Receives", &[&s, &var(m, rng.below(3))]));
            }
        }
    }
    facts
}

fn as_map(model: &Model) -> BTreeMap<String, BTreeSet<Vec<String>>> {
    let mut out: BTreeMap<String, BTreeSet<Vec<String>>> = BTreeMap::new();
    for (p, args) in model.facts() {
        out.entry(p).or_default().insert(args);
    }
    out
}

fn calls_of(p: &Program) -> Vec<(String, String, String)> {
    p.facts()
        .iter()
        .filter(|f| f.predicate == "Calls")
        .map(|f| {
            let g = f.ground().expect("a fact is ground");
            (g[0].clone(), g[1].clone(), g[2].clone())
        })
        .collect()
}

/// `CallContext`'s tuples as a map from (caller context, site) to (callee context, callee).
fn call_context(contexts: &Contexts) -> BTreeMap<(u64, String), (u64, String)> {
    contexts
        .facts()
        .into_iter()
        .filter(|f| f.predicate == "CallContext")
        .map(|f| {
            let g = f.ground().expect("a fact is ground");
            let c: u64 = g[0].parse().expect("a context is a number");
            let d: u64 = g[2].parse().expect("a context is a number");
            ((c, g[1].clone()), (d, g[3].clone()))
        })
        .collect()
}

#[test]
fn the_identity_called_twice_has_one_context_for_main_and_two_for_the_identity() {
    let p = with_rules(IDENTITY, CONTEXT_SENSITIVE);
    let contexts = number(&p);
    assert_eq!(contexts.of("Main"), 1, "a method no other calls is a root, with one context");
    assert_eq!(contexts.of("Id"), 2, "one context per call path");
    assert_eq!(contexts.total(), 3);
    assert_eq!(contexts.bits(), 1, "contexts 0 and 1 take one bit");
    let facts: BTreeSet<Vec<String>> = contexts
        .facts()
        .into_iter()
        .map(|f| std::iter::once(f.predicate.clone()).chain(f.ground().expect("ground")).collect())
        .collect();
    assert_eq!(
        facts,
        tuples(&[
            &["Context", "Main", "0"],
            &["Context", "Id", "0"],
            &["Context", "Id", "1"],
            &["CallContext", "0", "S1", "0", "Id"],
            &["CallContext", "0", "S2", "1", "Id"],
        ]),
        "the calls into a method take consecutive ranges of its contexts, in the order the program states them"
    );
}

fn with_rules(facts: &str, rules: &str) -> Program {
    let base = program(facts);
    Program::new(base.facts().to_vec(), rules_of(rules))
}

#[test]
fn context_sensitivity_keeps_the_two_calls_apart() {
    let (_, model) = in_contexts(&with_rules(IDENTITY, CONTEXT_SENSITIVE));
    assert_eq!(
        model.relation("VP"),
        tuples(&[&["A", "H1"], &["B", "H2"], &["P", "H1"], &["P", "H2"], &["X", "H1"], &["Y", "H2"]]),
        "x gets only what the first call passed and y only what the second passed"
    );
    assert_eq!(
        model.relation("VPC"),
        tuples(&[
            &["0", "A", "H1"],
            &["0", "B", "H2"],
            &["0", "P", "H1"],
            &["1", "P", "H2"],
            &["0", "X", "H1"],
            &["0", "Y", "H2"],
        ]),
        "the identity's parameter holds h1 in its first context and h2 in its second"
    );
    assert!(model.holds("VPC", &["1", "P", "H2"]));
    assert!(!model.holds("VPC", &["1", "P", "H1"]));
    assert!(!model.holds("VPC", &["7", "P", "H2"]), "no context 7");
    assert_eq!(model.count("VPC"), 6);

    let merged = evaluate(&with_rules(IDENTITY, CONTEXT_INSENSITIVE)).expect("positive");
    assert!(
        merged.holds("VP", &["X", "H2"]) && merged.holds("VP", &["Y", "H1"]),
        "without contexts both results may hold both objects: {:?}",
        merged.relation("VP")
    );
}

#[test]
fn the_computed_relations_are_the_numbering_written_out() {
    let mut rng = SplitMix(0xC0_7E47_0001);
    for case in 0..150 {
        let recursion = case % 2 == 1;
        let p = Program::new(random_facts(&mut rng, recursion), rules_of(CONTEXT_SENSITIVE));
        let (contexts, model) = in_contexts(&p);
        let written: Model = evaluate(&with_numbering(&p, &contexts)).expect("positive");
        for relation in ["Context", "CallContext"] {
            assert_eq!(model.relation(relation), written.relation(relation), "case {case}, {relation}, on\n{p}");
        }
    }
}

#[test]
fn the_diagrams_equal_the_hash_engine_over_the_numbering_written_out() {
    let mut rng = SplitMix(0xC0_7E47_0002);
    for case in 0..150 {
        let recursion = case % 2 == 1;
        let p = Program::new(random_facts(&mut rng, recursion), rules_of(CONTEXT_SENSITIVE));
        let (contexts, model) = in_contexts(&p);
        let written = as_map(&evaluate(&with_numbering(&p, &contexts)).expect("positive"));
        let relations: BTreeSet<String> =
            written.keys().cloned().chain(["Context", "CallContext", "VPC", "HP", "VP"].map(String::from)).collect();
        for relation in &relations {
            let expected = written.get(relation).cloned().unwrap_or_default();
            assert_eq!(model.relation(relation), expected, "case {case}, relation {relation}, on\n{p}");
            assert_eq!(model.count(relation), expected.len() as u128, "case {case}, {relation}'s count");
        }
    }
}

/// Every call path from a root to each method, as the sites it goes through.
fn paths(p: &Program) -> BTreeMap<String, Vec<Vec<String>>> {
    let calls = calls_of(p);
    let callees: BTreeSet<&str> = calls.iter().map(|(_, _, n)| n.as_str()).collect();
    let mut methods: BTreeSet<&str> = calls.iter().map(|(m, _, _)| m.as_str()).collect();
    methods.extend(callees.iter().copied());
    let mut out: BTreeMap<String, Vec<Vec<String>>> = BTreeMap::new();
    let mut stack: Vec<(String, Vec<String>)> =
        methods.iter().filter(|m| !callees.contains(*m)).map(|m| (m.to_string(), Vec::new())).collect();
    while let Some((m, path)) = stack.pop() {
        for (caller, site, callee) in &calls {
            if *caller == m {
                let mut longer = path.clone();
                longer.push(site.clone());
                stack.push((callee.clone(), longer));
            }
        }
        out.entry(m).or_default().push(path);
    }
    out
}

#[test]
fn every_call_path_to_a_method_has_its_own_number() {
    let mut rng = SplitMix(0xC0_7E47_0003);
    for case in 0..200 {
        let p = Program::new(random_facts(&mut rng, false), rules_of(CONTEXT_SENSITIVE));
        let contexts = number(&p);
        let step = call_context(&contexts);
        let calls = calls_of(&p);
        let roots: BTreeSet<String> = paths(&p).iter().filter(|(_, ps)| ps.contains(&Vec::new())).map(|(m, _)| m.clone()).collect();
        for (method, method_paths) in paths(&p) {
            assert_eq!(contexts.of(&method), method_paths.len() as u64, "case {case}: {method}'s count is its paths'\n{p}");
            let mut numbers = BTreeSet::new();
            for path in &method_paths {
                let first_caller = match path.first() {
                    None => method.clone(),
                    Some(site) => calls.iter().find(|(_, s, _)| s == site).expect("a site of a call").0.clone(),
                };
                assert!(roots.contains(&first_caller), "case {case}: a path starts at a root");
                let mut context = 0u64;
                let mut at = first_caller;
                for site in path {
                    let (d, callee) = step.get(&(context, site.clone())).unwrap_or_else(|| {
                        panic!("case {case}: no CallContext for site {site} in context {context} of {at}\n{p}")
                    });
                    context = *d;
                    at = callee.clone();
                }
                assert_eq!(at, method);
                assert!(context < contexts.of(&method), "case {case}: {method}'s context {context} is in range");
                assert!(numbers.insert(context), "case {case}: two paths to {method} share context {context}\n{p}");
            }
            assert_eq!(numbers.len() as u64, contexts.of(&method), "case {case}: every number of {method} is a path's");
        }
        assert_eq!(contexts.total(), paths(&p).values().map(|ps| ps.len() as u128).sum::<u128>());
    }
}

#[test]
fn a_recursive_cycle_shares_its_contexts_and_a_call_inside_it_keeps_its_context() {
    let p = program(
        "\
Axiom c1: Calls(M0, S1, M1).
Axiom c2: Calls(M0, S2, M1).
Axiom c3: Calls(M1, S3, M2).
Axiom c4: Calls(M2, S4, M1).
Axiom c5: Calls(M2, S5, M3).
Axiom c6: Calls(M3, S6, M3).
",
    );
    let contexts = number(&p);
    assert_eq!(contexts.of("M0"), 1);
    assert_eq!(contexts.of("M1"), 2, "two paths enter the cycle");
    assert_eq!(contexts.of("M2"), 2, "the cycle's methods share its contexts");
    assert_eq!(contexts.of("M3"), 2, "one call from the cycle, in each of its contexts");
    let step = call_context(&contexts);
    assert_eq!(step.get(&(0, "S1".into())), Some(&(0, "M1".into())));
    assert_eq!(step.get(&(0, "S2".into())), Some(&(1, "M1".into())));
    for c in 0..2 {
        assert_eq!(step.get(&(c, "S3".into())), Some(&(c, "M2".into())), "inside the cycle a call keeps its context");
        assert_eq!(step.get(&(c, "S4".into())), Some(&(c, "M1".into())));
        assert_eq!(step.get(&(c, "S5".into())), Some(&(c, "M3".into())));
        assert_eq!(step.get(&(c, "S6".into())), Some(&(c, "M3".into())), "a method calling itself keeps its context");
    }
    assert_eq!(step.len(), 2 + 4 * 2, "no other call contexts");
}

/// The program cloned once per call path: each method's variables renamed per context, each call's argument copied
/// into the callee clone's parameter and its result back, by the numbering's `CallContext`.
fn cloned(p: &Program, contexts: &Contexts) -> Program {
    let mut declared: BTreeMap<String, String> = BTreeMap::new();
    for f in p.facts().iter().filter(|f| f.predicate == "Declares") {
        let g = f.ground().expect("ground");
        declared.insert(g[1].clone(), g[0].clone());
    }
    let clone = |v: &str, c: u64| format!("{v}@{c}");
    let mut facts = Vec::new();
    for f in p.facts() {
        let g = f.ground().expect("ground");
        let method = |v: &str| declared[v].clone();
        match f.predicate.as_str() {
            "VP0" => {
                for c in 0..contexts.of(&method(&g[0])) {
                    facts.push(atom("VP0", &[&clone(&g[0], c), &g[1]]));
                }
            }
            "Assign" => {
                for c in 0..contexts.of(&method(&g[0])) {
                    facts.push(atom("Assign", &[&clone(&g[0], c), &clone(&g[1], c)]));
                }
            }
            "Store" | "Load" => {
                for c in 0..contexts.of(&method(&g[0])) {
                    facts.push(atom(&f.predicate, &[&clone(&g[0], c), &g[1], &clone(&g[2], c)]));
                }
            }
            _ => {}
        }
    }
    let of = |pred: &str| -> Vec<Vec<String>> {
        p.facts().iter().filter(|f| f.predicate == pred).map(|f| f.ground().expect("ground")).collect()
    };
    for ((c, site), (d, callee)) in call_context(contexts) {
        for actual in of("Actual").iter().filter(|a| a[0] == site) {
            for formal in of("Formal").iter().filter(|f| f[0] == callee && f[1] == actual[1]) {
                facts.push(atom("Assign", &[&clone(&formal[2], d), &clone(&actual[2], c)]));
            }
        }
        for receives in of("Receives").iter().filter(|r| r[0] == site) {
            for returns in of("Returns").iter().filter(|r| r[0] == callee) {
                facts.push(atom("Assign", &[&clone(&receives[1], c), &clone(&returns[1], d)]));
            }
        }
    }
    let intraprocedural: Vec<Rule> =
        rules_of(CONTEXT_INSENSITIVE).into_iter().filter(|r| r.name != "pass" && r.name != "result").collect();
    Program::new(facts, intraprocedural)
}

#[test]
fn the_context_sensitive_rules_are_the_insensitive_rules_on_the_program_cloned_per_call_path() {
    let mut rng = SplitMix(0xC0_7E47_0004);
    for case in 0..150 {
        let p = Program::new(random_facts(&mut rng, false), rules_of(CONTEXT_SENSITIVE));
        let (contexts, model) = in_contexts(&p);
        let clones = evaluate(&cloned(&p, &contexts)).expect("positive");
        let by_clone: BTreeSet<Vec<String>> = clones
            .relation("VP")
            .into_iter()
            .map(|t| {
                let (v, c) = t[0].split_once('@').expect("a cloned variable");
                vec![c.to_string(), v.to_string(), t[1].clone()]
            })
            .collect();
        assert_eq!(model.relation("VPC"), by_clone, "case {case}: VPC is the clones' VP, on\n{p}");
        assert_eq!(model.relation("HP"), clones.relation("HP"), "case {case}: the heap is the clones' heap");
    }
}

/// A call chain of depth `k`: method j allocates `Gj` into `Lj` and calls method j+1 twice, at `Aj` passing what its
/// own parameter holds (method 0 passes `L0`) and at `Bj` passing `Lj`. Method k is reached by 2^k paths, and in each
/// its parameter holds the object of the deepest method whose `B` call the path took.
fn chain(k: usize) -> Program {
    let mut facts = Vec::new();
    for j in 0..=k {
        let (m, l, g) = (format!("M{j}"), format!("L{j}"), format!("G{j}"));
        facts.push(atom("Declares", &[&m, &l]));
        facts.push(atom("VP0", &[&l, &g]));
        if j > 0 {
            facts.push(atom("Declares", &[&m, &format!("P{j}")]));
            facts.push(atom("Formal", &[&m, "Z0", &format!("P{j}")]));
        }
        if j < k {
            let next = format!("M{}", j + 1);
            let own = if j == 0 { l.clone() } else { format!("P{j}") };
            for (site, passed) in [(format!("A{j}"), own), (format!("B{j}"), l.clone())] {
                facts.push(atom("Calls", &[&m, &site, &next]));
                facts.push(atom("Actual", &[&site, "Z0", &passed]));
            }
        }
    }
    Program::new(facts, rules_of(CONTEXT_SENSITIVE))
}

#[test]
fn a_chain_forty_deep_has_two_to_the_forty_contexts_in_a_small_diagram() {
    let mut nodes = BTreeMap::new();
    for k in [10usize, 20, 40] {
        let (contexts, model) = in_contexts(&chain(k));
        assert_eq!(contexts.of(&format!("M{k}")), 1u64 << k, "2^{k} paths reach the bottom");
        assert_eq!(contexts.total(), (1u128 << (k + 1)) - 1);
        let tuples = model.count("VPC");
        assert_eq!(tuples, (1u128 << (k + 2)) - 3, "each Lj and Pj holds one object in each of its 2^j contexts");
        let all = (1u64 << k) - 1;
        let bottom = format!("P{k}");
        let last = format!("G{}", k - 1);
        assert!(model.holds("VPC", &[&all.to_string(), &bottom, &last]), "every B taken: the deepest object");
        assert!(!model.holds("VPC", &[&all.to_string(), &bottom, "G0"]));
        assert!(model.holds("VPC", &["5", &bottom, "G2"]), "B0 and B2 taken: G2");
        assert!(!model.holds("VPC", &["5", &bottom, "G0"]));
        assert!(model.holds("VPC", &["1", &bottom, "G0"]), "only B0 taken: G0");
        let projected = model.relation("VP");
        let expected: BTreeSet<Vec<String>> = (0..k).map(|j| vec![bottom.clone(), format!("G{j}")]).collect();
        let at_bottom: BTreeSet<Vec<String>> = projected.into_iter().filter(|t| t[0] == bottom).collect();
        assert_eq!(at_bottom, expected, "merged over its contexts the bottom parameter holds every Gj above it");
        assert!(
            (model.nodes("VPC") as u128) * 1_000_000 < tuples || k < 40,
            "{tuples} tuples in {} nodes",
            model.nodes("VPC")
        );
        nodes.insert(k, model.nodes("VPC"));
    }
    assert!(
        nodes[&40] < nodes[&20] * 8,
        "doubling the depth multiplies the contexts by 2^20 and the diagram by less than 8: {nodes:?}"
    );
}

#[test]
fn a_program_that_states_a_computed_relation_is_refused_by_name() {
    for stated in ["Axiom given: Context(Id, Zero).", "Axiom given: CallContext(Zero, S1, Zero, Id)."] {
        let p = with_rules(&format!("{IDENTITY}{stated}\n"), CONTEXT_SENSITIVE);
        let refusal = Contexts::number(&p).and_then(|c| evaluate_bdd_with_contexts(&p, &c).map(|_| ()));
        let err = refusal.expect_err("a computed relation is the numbering's");
        assert!(matches!(err, DatalogError::Computed { .. }), "{err:?}");
        let predicate = stated.split(": ").nth(1).and_then(|s| s.split('(').next()).expect("a predicate");
        assert!(err.to_string().contains(predicate), "the refusal names {predicate}: {err}");
    }
}

#[test]
fn a_call_graph_derived_from_its_own_contexts_is_refused() {
    let derived = format!(
        "{IDENTITY}Axiom indirect: for all c i d n m, if CallContext(c, i, d, n) and Declares(m, i) then Calls(n, i, m).\n"
    );
    let p = program(&derived);
    let err = Contexts::number(&p).map(|_| ()).expect_err("the contexts are numbered over the call graph, so it cannot depend on them");
    assert!(matches!(err, DatalogError::NotGiven { .. }), "{err:?}");
    assert!(err.to_string().contains("Calls") && err.to_string().contains("indirect"), "names the relation and the rule: {err}");
}

/// Whaley and Lam's Algorithm 6: the call graph discovered by the analysis itself. A virtual call's targets are the
/// methods its receiver's objects dispatch to, found by context-insensitive points-to (`VPI`); the contexts are
/// numbered over the call graph that discovers, and the context-sensitive rules run over them, a receiver passing to
/// each target only the objects that dispatch to it (`this`). `go` is invoked on a receiver that may hold an `HA` or an
/// `HB` object, so it reaches both `RunA` and `RunB`.
const DISCOVERED: &str = "\
Axiom new_a: VP0(A, HA).
Axiom new_b: VP0(B, HB).
Axiom copy_r1: Assign(R, A).
Axiom copy_r2: Assign(R, B).
Axiom main_a: Declares(Main, A).
Axiom main_b: Declares(Main, B).
Axiom main_r: Declares(Main, R).
Axiom main_s: Declares(Main, S1).
Axiom main_t: Declares(Main, S2).
Axiom invoke_1: Invoke(S1, R, Go).
Axiom invoke_2: Invoke(S2, A, Go).
Axiom dispatch_a: Dispatch(HA, Go, RunA).
Axiom dispatch_b: Dispatch(HB, Go, RunB).
Axiom this_a: Formal(RunA, Z0, TA).
Axiom this_b: Formal(RunB, Z0, TB).
Axiom run_a: Declares(RunA, TA).
Axiom run_b: Declares(RunB, TB).
Axiom vpi_alloc: for all v h, if VP0(v, h) then VPI(v, h).
Axiom vpi_assign: for all v1 v2 h, if Assign(v1, v2) and VPI(v2, h) then VPI(v1, h).
Axiom calls: for all m i r name h n, if Declares(m, i) and Invoke(i, r, name) and VPI(r, h) and Dispatch(h, name, n) then Calls(m, i, n).
Axiom this: for all c i d n r name h t, if CallContext(c, i, d, n) and Invoke(i, r, name) and VPC(c, r, h) and Dispatch(h, name, n) and Formal(n, Z0, t) then VPC(d, t, h).
";

#[test]
fn a_call_graph_the_rules_discover_is_numbered_once_it_is_complete() {
    let p = with_rules(DISCOVERED, &format!("{DISCOVERED}{CONTEXT_SENSITIVE}"));
    let contexts = Contexts::number(&p).unwrap_or_else(|e| panic!("the discovered call graph numbers: {e}"));
    assert_eq!(contexts.of("RunA"), 2, "S1 (r may hold HA) and S2 (a holds HA) both reach RunA");
    assert_eq!(contexts.of("RunB"), 1, "only S1's receiver may hold HB");
    let model = evaluate_bdd_with_contexts(&p, &contexts).unwrap_or_else(|e| panic!("evaluates: {e}"));
    assert_eq!(
        model.relation("Calls"),
        tuples(&[&["Main", "S1", "RunA"], &["Main", "S1", "RunB"], &["Main", "S2", "RunA"]]),
        "the call graph the rules discovered"
    );
    assert!(model.holds("VPC", &["1", "TA", "HA"]) && model.holds("VPC", &["0", "TA", "HA"]), "{:?}", model.relation("VPC"));
    assert!(model.holds("VPC", &["0", "TB", "HB"]), "{:?}", model.relation("VPC"));
    assert!(!model.holds("VPC", &["0", "TB", "HA"]), "RunB's receiver never holds an HA: {:?}", model.relation("VPC"));
    let written = evaluate(&with_numbering(&p, &contexts)).expect("positive");
    for relation in ["Calls", "Context", "CallContext", "VPC", "VP"] {
        assert_eq!(model.relation(relation), written.relation(relation), "{relation}: the diagrams and the hash engine agree");
    }
}

#[test]
fn a_variable_used_as_a_context_and_as_a_constant_is_refused() {
    let p = Program::new(
        program(IDENTITY).facts().to_vec(),
        vec![Rule::new(
            "mixed",
            Atom::new("Bad", vec![Term::var("m")]),
            vec![
                Literal::Pos(Atom::new("Context", vec![Term::var("m"), Term::var("c")])),
                Literal::Pos(Atom::new("VP0", vec![Term::var("c"), Term::var("m")])),
            ],
        )],
    );
    let contexts = number(&p);
    let err = evaluate_bdd_with_contexts(&p, &contexts).map(|_| ()).expect_err("c is a context and a variable name");
    assert!(matches!(err, DatalogError::Sort { .. }), "{err:?}");
    assert!(err.to_string().contains("VP0"), "the refusal names the argument that holds constants: {err}");
}

#[test]
fn more_call_paths_than_sixty_four_bits_number_are_refused() {
    let p = chain(64);
    let err = Contexts::number(&p).map(|_| ()).expect_err("2^64 paths reach M64");
    assert!(matches!(err, DatalogError::ContextOverflow { .. }), "{err:?}");
    assert!(err.to_string().contains("M64"), "the refusal names the method: {err}");
    assert_eq!(number(&chain(63)).of("M63"), 1u64 << 63, "2^63 still fits");
}

/// Heap objects named by the context that allocated them as well as their allocation site, in the rules alone:
/// `VPCH(c, v, hc, h)` says `v`, in context `c`, may point to the object site `h` allocated in context `hc`.
const HEAP_CONTEXTS: &str = "\
Axiom alloc: for all v h m c, if VP0(v, h) and Declares(m, v) and Context(m, c) then VPCH(c, v, c, h).
Axiom assign: for all c v1 v2 hc h, if Assign(v1, v2) and VPCH(c, v2, hc, h) then VPCH(c, v1, hc, h).
Axiom pass: for all c i d n z a f hc h, if CallContext(c, i, d, n) and Actual(i, z, a) and Formal(n, z, f) and VPCH(c, a, hc, h) then VPCH(d, f, hc, h).
Axiom result: for all c i d n v r hc h, if CallContext(c, i, d, n) and Returns(n, v) and Receives(i, r) and VPCH(d, v, hc, h) then VPCH(c, r, hc, h).
Axiom store: for all c v1 f v2 hc1 h1 hc2 h2, if Store(v1, f, v2) and VPCH(c, v1, hc1, h1) and VPCH(c, v2, hc2, h2) then HPC(hc1, h1, f, hc2, h2).
Axiom load: for all c v1 f v2 hc1 h1 hc2 h2, if Load(v1, f, v2) and VPCH(c, v1, hc1, h1) and HPC(hc1, h1, f, hc2, h2) then VPCH(c, v2, hc2, h2).
Axiom project: for all c v hc h, if VPCH(c, v, hc, h) then VP(v, h).
";

/// `x = make(); y = make(); x.f = a; y.f = b; r = x.f` with `make()` returning a fresh object.
const FACTORY: &str = "\
Axiom new_o: VP0(O, H).
Axiom make_o: Declares(Make, O).
Axiom returns: Returns(Make, O).
Axiom new_a: VP0(A, HA).
Axiom new_b: VP0(B, HB).
Axiom main_a: Declares(Main, A).
Axiom main_b: Declares(Main, B).
Axiom main_x: Declares(Main, X).
Axiom main_y: Declares(Main, Y).
Axiom main_r: Declares(Main, R).
Axiom call_1: Calls(Main, S1, Make).
Axiom call_2: Calls(Main, S2, Make).
Axiom result_1: Receives(S1, X).
Axiom result_2: Receives(S2, Y).
Axiom store_x: Store(X, F, A).
Axiom store_y: Store(Y, F, B).
Axiom load_x: Load(X, F, R).
";

#[test]
fn heap_contexts_tell_apart_the_objects_one_site_allocates_in_two_contexts() {
    let (_, model) = in_contexts(&with_rules(FACTORY, HEAP_CONTEXTS));
    assert!(model.holds("VP", &["R", "HA"]), "r = x.f, and x.f was set to a: {:?}", model.relation("VP"));
    assert!(
        !model.holds("VP", &["R", "HB"]),
        "the object make() returned to y is another object, so y.f = b never reaches x.f: {:?}",
        model.relation("VP")
    );
    assert!(model.holds("HPC", &["0", "H", "F", "0", "HA"]), "{:?}", model.relation("HPC"));
    assert!(model.holds("HPC", &["1", "H", "F", "0", "HB"]), "{:?}", model.relation("HPC"));

    let (_, site_named) = in_contexts(&with_rules(FACTORY, CONTEXT_SENSITIVE));
    assert!(
        site_named.holds("VP", &["R", "HB"]),
        "named by site alone, both calls' objects are one object H, so x.f may be b: {:?}",
        site_named.relation("VP")
    );
    let written = evaluate(&with_numbering(&with_rules(FACTORY, HEAP_CONTEXTS), &number(&with_rules(FACTORY, HEAP_CONTEXTS))))
        .expect("positive");
    for relation in ["VPCH", "HPC", "VP"] {
        assert_eq!(model.relation(relation), written.relation(relation), "{relation}: the diagrams and the hash engine agree");
    }
}

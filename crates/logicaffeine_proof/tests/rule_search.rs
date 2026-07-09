//! `aesop`-style best-first rule search — `auto` inverted into one extensible
//! rule among many, with safe/unsafe classification, priorities, a node
//! budget, and search statistics.

use logicaffeine_proof::rule_search::{default_ruleset, Rule, RuleKind, RuleSet, Safety};
use logicaffeine_proof::tactic::{combinators as c, ProofState};
use logicaffeine_proof::{ProofExpr, ProofTerm};

fn k(n: &str) -> ProofTerm {
    ProofTerm::Constant(n.to_string())
}
fn pr(name: &str, args: Vec<ProofTerm>) -> ProofExpr {
    ProofExpr::Predicate { name: name.to_string(), args, world: None }
}
fn implies(l: ProofExpr, r: ProofExpr) -> ProofExpr {
    ProofExpr::Implies(Box::new(l), Box::new(r))
}
fn and(l: ProofExpr, r: ProofExpr) -> ProofExpr {
    ProofExpr::And(Box::new(l), Box::new(r))
}

#[test]
fn default_ruleset_matches_auto() {
    // The default rule set (auto as the sole unsafe fallback) closes what auto
    // closes and certifies. A representative auto goal: P, P→Q ⊢ Q.
    let goal = pr("Q", vec![k("A")]);
    let premises = vec![pr("P", vec![k("A")]), implies(pr("P", vec![k("A")]), pr("Q", vec![k("A")]))];
    let mut st = ProofState::start(premises, goal);
    let stats = default_ruleset().search(&mut st);
    assert!(stats.succeeded, "default ruleset must close an auto-provable goal");
    assert!(st.qed().unwrap().verified, "the search result must certify");
}

#[test]
fn safe_intro_then_close() {
    // ⊢ P → P by a safe `intro` then an unsafe `auto`. The safe rule fires
    // first (free), then auto closes.
    let goal = implies(pr("P", vec![k("A")]), pr("P", vec![k("A")]));
    let mut rs = RuleSet::new();
    rs.register(Rule::new("intro", RuleKind::Intro, Safety::Safe, c::intro("h")));
    rs.register(Rule::new("auto", RuleKind::Elim, Safety::Unsafe(10), c::auto()));
    let mut st = ProofState::start(vec![], goal);
    let stats = rs.search(&mut st);
    assert!(stats.succeeded);
    assert!(st.qed().unwrap().verified, "intro;auto search must certify");
}

#[test]
fn unsafe_dead_end_backtracks() {
    // A high-priority `left` on a non-disjunction goal always fails (dead end);
    // the correct unsafe `auto` still closes the goal. Best-first must recover.
    let goal = pr("Q", vec![k("A")]);
    let premises = vec![pr("P", vec![k("A")]), implies(pr("P", vec![k("A")]), pr("Q", vec![k("A")]))];
    let mut rs = RuleSet::new();
    rs.register(Rule::new("trap", RuleKind::Intro, Safety::Unsafe(200), c::left()));
    rs.register(Rule::new("auto", RuleKind::Elim, Safety::Unsafe(40), c::auto()));
    let mut st = ProofState::start(premises, goal);
    let stats = rs.search(&mut st);
    assert!(stats.succeeded, "search must recover from the dead-end trap rule");
    assert!(st.qed().unwrap().verified);
}

#[test]
fn node_budget_declines_cleanly() {
    // An unreachable goal with a zero-progress rule set must stop at the budget
    // and report failure, never hang.
    let goal = pr("Unreachable", vec![k("A")]);
    let mut rs = RuleSet::new();
    rs.register(Rule::new("auto", RuleKind::Elim, Safety::Unsafe(10), c::auto()));
    let mut st = ProofState::start(vec![], goal);
    let stats = rs.search_bounded(&mut st, 50);
    assert!(!stats.succeeded, "an unreachable goal is not proved");
    assert!(stats.nodes_expanded <= 51, "the budget bounds the search");
}

#[test]
fn safe_rules_explored_before_unsafe() {
    // With both a safe and an unsafe rule available, the safe branch (cost 0)
    // is expanded first — reflected in a small node count when it succeeds.
    let goal = and(
        implies(pr("P", vec![k("A")]), pr("P", vec![k("A")])),
        implies(pr("R", vec![k("B")]), pr("R", vec![k("B")])),
    );
    let mut rs = RuleSet::new();
    rs.register(Rule::new("split", RuleKind::Intro, Safety::Safe, c::split()));
    rs.register(Rule::new("intro", RuleKind::Intro, Safety::Safe, c::intro("h")));
    rs.register(Rule::new("auto", RuleKind::Elim, Safety::Unsafe(10), c::auto()));
    let mut st = ProofState::start(vec![], goal);
    let stats = rs.search(&mut st);
    assert!(stats.succeeded, "safe split+intro then auto closes the conjunction");
    assert!(st.qed().unwrap().verified);
}

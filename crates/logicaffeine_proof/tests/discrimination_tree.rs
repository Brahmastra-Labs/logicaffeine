//! Discrimination-tree retrieval: the fast pattern index under `simp`,
//! `exact?`, and `crush`'s E-matching.
//!
//! The contract has two halves. (1) NEVER MISS: every pattern that one-sided
//! matches a query is among the returned candidates — the tree may
//! over-approximate (non-linear patterns like `f(x,x)` retrieve for `f(a,b)`),
//! the matcher is the arbiter. (2) PRUNE: retrieval filters by head symbols,
//! so a query touches only the rules that share its structure, not the whole
//! rule set.

use logicaffeine_proof::discrimination::DiscTree;
use logicaffeine_proof::unify::{match_expr_pattern, match_term_pattern};
use logicaffeine_proof::{ProofExpr, ProofTerm};

fn v(name: &str) -> ProofTerm {
    ProofTerm::Variable(name.to_string())
}

fn c(name: &str) -> ProofTerm {
    ProofTerm::Constant(name.to_string())
}

fn f(name: &str, args: Vec<ProofTerm>) -> ProofTerm {
    ProofTerm::Function(name.to_string(), args)
}

fn p(name: &str, args: Vec<ProofTerm>) -> ProofExpr {
    ProofExpr::Predicate { name: name.to_string(), args, world: None }
}

#[test]
fn dtree_exact_hit_returns_payload() {
    let mut tree: DiscTree<&str> = DiscTree::new();
    tree.insert_term(&f("add", vec![v("x"), c("Zero")]), "add_zero");
    let hits = tree.candidates_term(&f("add", vec![c("a"), c("Zero")]));
    assert_eq!(hits, vec![&"add_zero"]);
}

#[test]
fn dtree_variable_positions_are_wildcards() {
    let mut tree: DiscTree<&str> = DiscTree::new();
    tree.insert_term(&f("f", vec![v("x"), v("y")]), "rule");
    // A pattern variable spans a whole subterm, however deep.
    let hits = tree.candidates_term(&f("f", vec![c("a"), f("g", vec![c("b")])]));
    assert_eq!(hits, vec![&"rule"]);
}

#[test]
fn dtree_head_filtering_prunes() {
    let mut tree: DiscTree<String> = DiscTree::new();
    for i in 0..100 {
        tree.insert_term(&f(&format!("f{i}"), vec![v("x")]), format!("rule{i}"));
    }
    let hits = tree.candidates_term(&f("f42", vec![c("a")]));
    assert_eq!(hits, vec![&"rule42".to_string()]);
}

#[test]
fn dtree_never_misses_vs_linear_scan() {
    // A corpus with the awkward shapes: non-linear variables, nesting, bare
    // variables under constructors, constants in pattern position.
    let patterns: Vec<(ProofTerm, usize)> = vec![
        (f("add", vec![v("x"), c("Zero")]), 0),
        (f("add", vec![c("Zero"), v("x")]), 1),
        (f("f", vec![v("x"), v("x")]), 2),
        (f("g", vec![f("f", vec![v("x"), c("c")])]), 3),
        (f("h", vec![v("x")]), 4),
        (c("k"), 5),
        (f("f", vec![f("g", vec![v("x")]), v("y")]), 6),
    ];
    let queries = vec![
        f("add", vec![c("a"), c("Zero")]),
        f("add", vec![c("Zero"), f("h", vec![c("a")])]),
        f("f", vec![c("a"), c("a")]),
        f("f", vec![c("a"), c("b")]),
        f("g", vec![f("f", vec![c("q"), c("c")])]),
        f("h", vec![f("h", vec![c("z")])]),
        c("k"),
        f("f", vec![f("g", vec![c("m")]), c("n")]),
        f("unrelated", vec![c("a")]),
    ];

    let mut tree: DiscTree<usize> = DiscTree::new();
    for (pat, id) in &patterns {
        tree.insert_term(pat, *id);
    }

    for q in &queries {
        let expected: Vec<usize> = patterns
            .iter()
            .filter(|(pat, _)| match_term_pattern(pat, q).is_some())
            .map(|(_, id)| *id)
            .collect();
        let got = tree.candidates_term(q);
        for id in &expected {
            assert!(
                got.contains(&id),
                "query {q} must retrieve pattern {id} (linear scan matched it)"
            );
        }
    }
}

#[test]
fn dtree_nonlinear_pattern_is_retrieved_and_matcher_arbitrates() {
    // The tree treats each variable position independently, so f(x,x) is a
    // candidate for f(a,b) — retrieval over-approximates, and the one-sided
    // matcher makes the call.
    let mut tree: DiscTree<&str> = DiscTree::new();
    let pat = f("f", vec![v("x"), v("x")]);
    tree.insert_term(&pat, "nonlinear");
    assert_eq!(tree.candidates_term(&f("f", vec![c("a"), c("b")])), vec![&"nonlinear"]);
    assert!(match_term_pattern(&pat, &f("f", vec![c("a"), c("b")])).is_none());
    let binding = match_term_pattern(&pat, &f("f", vec![c("a"), c("a")]))
        .expect("f(x,x) matches f(a,a)");
    assert_eq!(binding.get("x"), Some(&c("a")));
}

#[test]
fn dtree_expr_patterns_with_connectives() {
    let mut tree: DiscTree<&str> = DiscTree::new();
    let pat = ProofExpr::And(
        Box::new(p("P", vec![v("x")])),
        Box::new(p("Q", vec![v("x")])),
    );
    tree.insert_expr(&pat, "conj_rule");
    let hit = ProofExpr::And(
        Box::new(p("P", vec![c("a")])),
        Box::new(p("Q", vec![c("a")])),
    );
    let miss = ProofExpr::Or(
        Box::new(p("P", vec![c("a")])),
        Box::new(p("Q", vec![c("a")])),
    );
    assert_eq!(tree.candidates_expr(&hit), vec![&"conj_rule"]);
    assert!(tree.candidates_expr(&miss).is_empty());
}

#[test]
fn one_sided_match_binds_only_pattern_vars() {
    // Pattern variables bind; target structure is inspected, never bound.
    let subst = match_expr_pattern(&p("P", vec![v("x")]), &p("P", vec![c("a")]))
        .expect("pattern var binds target constant");
    assert_eq!(subst.get("x"), Some(&c("a")));
    // Reversed roles must FAIL: a pattern constant does not match a target
    // variable (one-sided means the target is not bindable).
    assert!(match_expr_pattern(&p("P", vec![c("a")]), &p("P", vec![v("x")])).is_none());
    // A pattern variable may bind a target variable as an opaque term.
    let s2 = match_expr_pattern(&p("P", vec![v("x")]), &p("P", vec![v("y")]))
        .expect("pattern var binds target var as a term");
    assert_eq!(s2.get("x"), Some(&v("y")));
}

#[test]
fn one_sided_match_requires_consistent_bindings() {
    let pat = p("R", vec![v("x"), v("x")]);
    assert!(match_expr_pattern(&pat, &p("R", vec![c("a"), c("b")])).is_none());
    let subst = match_expr_pattern(&pat, &p("R", vec![c("a"), c("a")]))
        .expect("consistent bindings succeed");
    assert_eq!(subst.get("x"), Some(&c("a")));
}

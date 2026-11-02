use super::*;

#[test]
fn test_union_find_basic() {
    let mut uf = UnionFind::new();
    let a = uf.make_set();
    let b = uf.make_set();
    assert_ne!(uf.find(a), uf.find(b));
    uf.union(a, b);
    assert_eq!(uf.find(a), uf.find(b));
}

#[test]
fn test_union_find_transitivity() {
    let mut uf = UnionFind::new();
    let a = uf.make_set();
    let b = uf.make_set();
    let c = uf.make_set();
    uf.union(a, b);
    uf.union(b, c);
    assert_eq!(uf.find(a), uf.find(c));
}

#[test]
fn test_egraph_reflexive() {
    let mut eg = EGraph::new();
    let x = eg.add(ENode::Var(0));
    assert!(eg.equivalent(x, x));
}

#[test]
fn test_egraph_congruence() {
    let mut eg = EGraph::new();
    let x = eg.add(ENode::Var(0));
    let y = eg.add(ENode::Var(1));
    let f = eg.add(ENode::Name("f".to_string()));
    let fx = eg.add(ENode::App { func: f, arg: x });
    let fy = eg.add(ENode::App { func: f, arg: y });

    // Before merging x=y, f(x) != f(y)
    assert!(!eg.equivalent(fx, fy));

    // After merging x=y, congruence gives f(x) = f(y)
    eg.merge(x, y);
    assert!(eg.equivalent(fx, fy));
}

#[test]
fn test_egraph_nested_congruence() {
    let mut eg = EGraph::new();
    let a = eg.add(ENode::Var(0));
    let b = eg.add(ENode::Var(1));
    let c = eg.add(ENode::Var(2));
    let f = eg.add(ENode::Name("f".to_string()));

    let fa = eg.add(ENode::App { func: f, arg: a });
    let fc = eg.add(ENode::App { func: f, arg: c });
    let ffa = eg.add(ENode::App { func: f, arg: fa });
    let ffc = eg.add(ENode::App { func: f, arg: fc });

    // a = b, b = c should give f(f(a)) = f(f(c))
    eg.merge(a, b);
    eg.merge(b, c);
    assert!(eg.equivalent(ffa, ffc));
}

#[test]
fn test_egraph_binary_congruence() {
    let mut eg = EGraph::new();
    let a = eg.add(ENode::Var(0));
    let b = eg.add(ENode::Var(1));
    let c = eg.add(ENode::Var(2));
    let add = eg.add(ENode::Name("add".to_string()));

    // add(a, c) and add(b, c) as curried: add a c = (add a) c
    let add_a = eg.add(ENode::App { func: add, arg: a });
    let add_b = eg.add(ENode::App { func: add, arg: b });
    let add_a_c = eg.add(ENode::App {
        func: add_a,
        arg: c,
    });
    let add_b_c = eg.add(ENode::App {
        func: add_b,
        arg: c,
    });

    assert!(!eg.equivalent(add_a_c, add_b_c));
    eg.merge(a, b);
    assert!(eg.equivalent(add_a_c, add_b_c));
}

// =========================================================================
// EXTRACTION TESTS
// =========================================================================

/// Helper to build SName "s"
fn make_sname(s: &str) -> Term {
    Term::App(
        Box::new(Term::Global("SName".to_string())),
        Box::new(Term::Lit(Literal::Text(s.to_string()))),
    )
}

/// Helper to build SVar i
fn make_svar(i: i64) -> Term {
    Term::App(
        Box::new(Term::Global("SVar".to_string())),
        Box::new(Term::Lit(Literal::Int(i))),
    )
}

/// Helper to build SApp f a
fn make_sapp(f: Term, a: Term) -> Term {
    Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("SApp".to_string())),
            Box::new(f),
        )),
        Box::new(a),
    )
}

#[test]
fn test_extract_sname() {
    let term = make_sname("f");
    assert_eq!(extract_sname(&term), Some("f".to_string()));
}

#[test]
fn test_extract_svar() {
    let term = make_svar(0);
    assert_eq!(extract_svar(&term), Some(0));
}

#[test]
fn test_extract_sapp() {
    // SApp (SName "f") (SVar 0)
    let term = make_sapp(make_sname("f"), make_svar(0));
    let result = extract_sapp(&term);
    assert!(result.is_some());
    let (func, arg) = result.unwrap();
    assert_eq!(extract_sname(&func), Some("f".to_string()));
    assert_eq!(extract_svar(&arg), Some(0));
}

#[test]
fn test_extract_binary_app() {
    // SApp (SApp (SName "Eq") (SVar 0)) (SVar 1)
    let term = make_sapp(make_sapp(make_sname("Eq"), make_svar(0)), make_svar(1));
    let result = extract_binary_app(&term);
    assert!(result.is_some(), "Should extract binary app");
    let (op, a, b) = result.unwrap();
    assert_eq!(op, "Eq");
    assert_eq!(extract_svar(&a), Some(0));
    assert_eq!(extract_svar(&b), Some(1));
}

#[test]
fn test_extract_equality() {
    // SApp (SApp (SName "Eq") (SVar 0)) (SVar 1)
    let term = make_sapp(make_sapp(make_sname("Eq"), make_svar(0)), make_svar(1));
    let result = extract_equality(&term);
    assert!(result.is_some(), "Should extract equality");
    let (lhs, rhs) = result.unwrap();
    assert_eq!(extract_svar(&lhs), Some(0));
    assert_eq!(extract_svar(&rhs), Some(1));
}

#[test]
fn test_extract_implication() {
    // Build: SApp (SApp (SName "implies") hyp) concl
    // hyp = SApp (SApp (SName "Eq") x) y
    // concl = SApp (SApp (SName "Eq") fx) fy
    let x = make_svar(0);
    let y = make_svar(1);
    let hyp = make_sapp(make_sapp(make_sname("Eq"), x.clone()), y.clone());

    let f = make_sname("f");
    let fx = make_sapp(f.clone(), x);
    let fy = make_sapp(f, y);
    let concl = make_sapp(make_sapp(make_sname("Eq"), fx), fy);

    let goal = make_sapp(make_sapp(make_sname("implies"), hyp.clone()), concl.clone());

    let result = extract_implication(&goal);
    assert!(result.is_some(), "Should extract implication");
    let (hyp_extracted, concl_extracted) = result.unwrap();

    // Verify hypothesis is the equality x = y
    let hyp_eq = extract_equality(&hyp_extracted);
    assert!(hyp_eq.is_some(), "Hypothesis should be equality");
    let (h_lhs, h_rhs) = hyp_eq.unwrap();
    assert_eq!(extract_svar(&h_lhs), Some(0));
    assert_eq!(extract_svar(&h_rhs), Some(1));

    // Verify conclusion is an equality
    let concl_eq = extract_equality(&concl_extracted);
    assert!(concl_eq.is_some(), "Conclusion should be equality");
}

#[test]
fn test_decompose_goal_with_hypothesis() {
    // Build: (implies (Eq x y) (Eq (f x) (f y)))
    let x = make_svar(0);
    let y = make_svar(1);
    let hyp = make_sapp(make_sapp(make_sname("Eq"), x.clone()), y.clone());

    let f = make_sname("f");
    let fx = make_sapp(f.clone(), x);
    let fy = make_sapp(f, y);
    let concl = make_sapp(make_sapp(make_sname("Eq"), fx), fy);

    let goal = make_sapp(make_sapp(make_sname("implies"), hyp), concl);

    let (hypotheses, conclusion) = decompose_goal(&goal);
    assert_eq!(hypotheses.len(), 1, "Should have 1 hypothesis");

    // Verify conclusion is an equality
    let concl_eq = extract_equality(&conclusion);
    assert!(concl_eq.is_some(), "Conclusion should be equality");
}

#[test]
fn test_check_goal_with_hypothesis() {
    // Build: (implies (Eq x y) (Eq (f x) (f y)))
    // This should be provable by CC
    let x = make_svar(0);
    let y = make_svar(1);
    let hyp = make_sapp(make_sapp(make_sname("Eq"), x.clone()), y.clone());

    let f = make_sname("f");
    let fx = make_sapp(f.clone(), x.clone());
    let fy = make_sapp(f.clone(), y.clone());
    let concl = make_sapp(make_sapp(make_sname("Eq"), fx), fy);

    let goal = make_sapp(make_sapp(make_sname("implies"), hyp), concl);

    assert!(check_goal(&goal), "CC should prove x=y → f(x)=f(y)");
}

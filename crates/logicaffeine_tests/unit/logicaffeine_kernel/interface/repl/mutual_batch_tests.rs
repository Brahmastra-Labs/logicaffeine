use super::*;

fn ctor_names(repl: &Repl, ind: &str) -> Vec<String> {
    repl.context()
        .get_constructors(ind)
        .iter()
        .map(|(n, _)| n.to_string())
        .collect()
}

#[test]
fn mutual_forward_reference_registers_both_constructor_sets() {
    // `Tree.Node : Forest -> Tree` forward-references `Forest`, declared next; the two
    // form a cycle and must register together as a mutual block.
    let mut repl = Repl::new();
    let stmts = vec![
        "Inductive Tree := Leaf : Tree | Node : Forest -> Tree.".to_string(),
        "Inductive Forest := Nil2 : Forest | Grow : Tree -> Forest -> Forest.".to_string(),
    ];
    let results = repl.execute_batch(&stmts);
    assert!(
        results.iter().all(|r| r.is_ok()),
        "batch must register cleanly: {results:?}"
    );
    assert!(
        ctor_names(&repl, "Tree").contains(&"Node".to_string()),
        "Tree.Node (forward ref to Forest) must register: {:?}",
        ctor_names(&repl, "Tree")
    );
    assert!(
        ctor_names(&repl, "Forest").contains(&"Grow".to_string()),
        "Forest.Grow must register: {:?}",
        ctor_names(&repl, "Forest")
    );
    assert!(
        repl.context().mutual_block_of("Tree").is_some(),
        "a genuine cycle must be recorded as a mutual block"
    );
}

#[test]
fn acyclic_forward_reference_registers_without_a_mutual_block() {
    // `Leafy.MkLeafy : Bare -> Leafy` references `Bare`, declared next, but `Bare` does
    // NOT reference back — no cycle, so two singletons registered `Bare`-first.
    let mut repl = Repl::new();
    let stmts = vec![
        "Inductive Leafy := MkLeafy : Bare -> Leafy.".to_string(),
        "Inductive Bare := MkBare : Bare.".to_string(),
    ];
    let results = repl.execute_batch(&stmts);
    assert!(
        results.iter().all(|r| r.is_ok()),
        "batch must register cleanly: {results:?}"
    );
    assert!(
        ctor_names(&repl, "Leafy").contains(&"MkLeafy".to_string()),
        "Leafy.MkLeafy (ref to Bare, declared later) must register"
    );
    assert!(ctor_names(&repl, "Bare").contains(&"MkBare".to_string()));
    assert!(
        repl.context().mutual_block_of("Leafy").is_none(),
        "an acyclic reference must NOT be recorded as mutual"
    );
}

#[test]
fn plain_inductive_batch_matches_single_execution() {
    // No forward references: the batch path is byte-identical to running the statement
    // on its own — same constructors registered, in the same order.
    let stmts = vec!["Inductive Color := Red : Color | Green : Color | Blue : Color.".to_string()];
    let mut batched = Repl::new();
    assert!(batched.execute_batch(&stmts).iter().all(|r| r.is_ok()));
    let mut single = Repl::new();
    assert!(single.execute(&stmts[0]).is_ok());
    assert_eq!(ctor_names(&batched, "Color"), ctor_names(&single, "Color"));
}

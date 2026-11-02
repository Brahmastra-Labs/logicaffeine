use super::*;

#[test]
fn a_mark_outside_every_scope_records_nothing() {
    let node = 7u64;
    assert_eq!(open_scopes(), 0);
    mark(Fact::InBounds, &node);
    assert!(!holds(Fact::InBounds, &node));
    let _scope = scope();
    assert!(
        !holds(Fact::InBounds, &node),
        "a scope opened later does not inherit a mark made outside it"
    );
}

#[test]
fn a_fact_ends_with_its_scope() {
    let node = 7u64;
    {
        let _scope = scope();
        mark(Fact::RawIntOp, &node);
        assert!(holds(Fact::RawIntOp, &node));
    }
    assert_eq!(open_scopes(), 0);
    assert!(!holds(Fact::RawIntOp, &node));
    let _next = scope();
    assert!(
        !holds(Fact::RawIntOp, &node),
        "the next compile starts with nothing proven"
    );
}

#[test]
fn an_inner_compile_sees_the_outer_facts_and_keeps_its_own() {
    let outer_node = 1u64;
    let inner_node = 2u64;
    let _outer = scope();
    mark(Fact::TileLegal, &outer_node);
    {
        let _inner = scope();
        assert!(
            holds(Fact::TileLegal, &outer_node),
            "a live outer compile's node keeps its fact"
        );
        mark(Fact::InBounds, &inner_node);
        assert!(holds(Fact::InBounds, &inner_node));
    }
    assert!(
        !holds(Fact::InBounds, &inner_node),
        "the inner compile's facts end with it"
    );
    assert!(
        holds(Fact::TileLegal, &outer_node),
        "the outer compile's facts outlive the inner one"
    );
}

#[test]
fn kinds_are_distinct() {
    const ALL: [Fact; 7] = [
        Fact::RawIntOp,
        Fact::IntStrengthAdvance,
        Fact::BlackBox,
        Fact::InBounds,
        Fact::TileLegal,
        Fact::IntRange,
        Fact::LentIteration,
    ];
    let nodes = [10u64, 11, 12, 13, 14, 15, 16];
    let _scope = scope();
    for (fact, node) in ALL.iter().zip(&nodes) {
        mark(*fact, node);
    }
    for (marked, node) in ALL.iter().zip(&nodes) {
        for asked in ALL {
            assert_eq!(
                holds(asked, node),
                asked == *marked,
                "{asked:?} on the node marked {marked:?}"
            );
        }
    }
}

#[test]
fn rebuilding_an_expression_copies_every_expression_fact_in_its_original_scope() {
    const EXPRESSION_FACTS: [Fact; 5] = [
        Fact::RawIntOp,
        Fact::IntStrengthAdvance,
        Fact::BlackBox,
        Fact::InBounds,
        Fact::TileLegal,
    ];
    let from = Expr::Literal(crate::ast::stmt::Literal::Number(7));
    let to = Expr::Literal(crate::ast::stmt::Literal::Number(8));
    let _scope = scope();
    for fact in EXPRESSION_FACTS {
        mark_expr(fact, &from);
    }

    copy_expr_facts(&from, &to);

    for fact in EXPRESSION_FACTS {
        assert!(holds_expr(fact, &from), "copying removed {fact:?}");
        assert!(holds_expr(fact, &to), "copying omitted {fact:?}");
    }
}

#[test]
fn unmark_takes_the_fact_off_where_it_is() {
    let node = 4u64;
    let _outer = scope();
    mark(Fact::BlackBox, &node);
    let _inner = scope();
    assert!(unmark(Fact::BlackBox, &node), "the outer scope held it");
    assert!(!holds(Fact::BlackBox, &node));
    assert!(!unmark(Fact::BlackBox, &node), "nothing left to take");
}

#[test]
fn expression_api_keys_the_node_not_a_reference_slot() {
    let node = Expr::Literal(crate::ast::stmt::Literal::Number(7));
    let borrowed = &node;
    let _scope = scope();
    mark_expr(Fact::InBounds, borrowed);
    assert!(
        holds_expr(Fact::InBounds, &borrowed),
        "an &&Expr consumer must resolve to the same arena node"
    );
    assert!(unmark_expr(Fact::InBounds, &borrowed));
    assert!(!holds_expr(Fact::InBounds, borrowed));
}

/// Every statement arena a compile makes is followed at once by its facts scope, so nothing proven about a node
/// can outlive the arena that holds it (`mark` outside a scope records nothing, so a missing scope costs speed;
/// this keeps one from going missing).
#[test]
fn every_statement_arena_opens_its_facts_scope() {
    fn rs_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for e in std::fs::read_dir(dir).expect("src is readable").flatten() {
            let p = e.path();
            if p.is_dir() {
                rs_files(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rs_files(&src, &mut files);
    let mut arenas = 0;
    let mut bare = Vec::new();
    for f in files {
        let text = std::fs::read_to_string(&f).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            let t = line.trim();
            let makes_stmt_arena = t.starts_with("let ")
                && t.contains("Arena::new()")
                && (t.contains("Arena<Stmt>") || t.contains("stmt_arena"));
            if makes_stmt_arena {
                arenas += 1;
                let next = lines.get(i + 1).map(|l| l.trim()).unwrap_or("");
                if !(next.starts_with("let _") && next.ends_with("facts::scope();")) {
                    bare.push(format!(
                        "{}:{}",
                        f.strip_prefix(&src).unwrap().display(),
                        i + 1
                    ));
                }
            }
        }
    }
    assert!(
        arenas >= 25,
        "found only {arenas} statement arenas: the scan broke"
    );
    assert!(
        bare.is_empty(),
        "statement arenas with no facts scope after them:\n  {}",
        bare.join("\n  ")
    );
}

/// A compile leaves no facts scope open behind it, and the next compile starts from nothing.
#[test]
fn a_compile_leaves_no_facts_behind() {
    let tiled = "## To native parseInt (s: Text) -> Int\n\n## Main\nLet n be parseInt(\"40\").\nLet mutable a be a new Seq of Int.\nLet mutable c be a new Seq of Int.\nLet mutable i be 0.\nWhile i is less than n * n:\n    Push i % 7 to a.\n    Push 0 to c.\n    Set i to i + 1.\nSet i to 0.\nWhile i is less than n:\n    Let mutable k be 0.\n    While k is less than n:\n        Let mutable j be 0.\n        While j is less than n:\n            Let idx be i * n + j + 1.\n            Set item idx of c to (item idx of c) + (item (i * n + k + 1) of a) * (item (k * n + j + 1) of a).\n            Set j to j + 1.\n        Set k to k + 1.\n    Set i to i + 1.\nShow item 1 of c.\n";
    assert_eq!(open_scopes(), 0);
    crate::compile::compile_to_rust(tiled).expect("compiles");
    assert_eq!(open_scopes(), 0, "a compile left its facts open");
    crate::compile::interpret_program(tiled).expect("runs");
    assert_eq!(open_scopes(), 0, "a run left its facts open");
}

#[test]
fn a_scope_cannot_cross_threads() {
    // A TLS guard must be neither Send nor Sync. Either trait would make its witness ambiguous.
    trait AmbiguousIfSend<A> {
        fn witness() {}
    }
    impl<T: ?Sized> AmbiguousIfSend<()> for T {}
    impl<T: ?Sized + Send> AmbiguousIfSend<u8> for T {}
    let _ = <Scope as AmbiguousIfSend<_>>::witness;

    trait AmbiguousIfSync<A> {
        fn witness() {}
    }
    impl<T: ?Sized> AmbiguousIfSync<()> for T {}
    impl<T: ?Sized + Sync> AmbiguousIfSync<u8> for T {}
    let _ = <Scope as AmbiguousIfSync<_>>::witness;
}

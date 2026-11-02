use super::*;

#[test]
fn test_literal_number() {
    let interner = Interner::new();
    let synced_vars = HashSet::new();
    let expr = Expr::Literal(Literal::Number(42));
    assert_eq!(codegen_expr(&expr, &interner, &synced_vars), "42");
}

#[test]
fn test_literal_boolean() {
    let interner = Interner::new();
    let synced_vars = HashSet::new();
    assert_eq!(
        codegen_expr(
            &Expr::Literal(Literal::Boolean(true)),
            &interner,
            &synced_vars
        ),
        "true"
    );
    assert_eq!(
        codegen_expr(
            &Expr::Literal(Literal::Boolean(false)),
            &interner,
            &synced_vars
        ),
        "false"
    );
}

#[test]
fn test_literal_nothing() {
    let interner = Interner::new();
    let synced_vars = HashSet::new();
    assert_eq!(
        codegen_expr(&Expr::Literal(Literal::Nothing), &interner, &synced_vars),
        "()"
    );
}

/// The range oracle holds facts keyed by the addresses of one program's nodes, including what that program's
/// own guards established while it was emitted. It ends with its compile: after each of two programs compiled
/// back to back no oracle is installed, and the second compiles to exactly what it compiles to alone.
#[test]
fn the_range_oracle_ends_with_its_compile() {
    let tiled = "## To native parseInt (s: Text) -> Int\n\n## Main\nLet n be parseInt(\"40\").\nLet mutable a be a new Seq of Int.\nLet mutable c be a new Seq of Int.\nLet mutable i be 0.\nWhile i is less than n * n:\n    Push i % 7 to a.\n    Push 0 to c.\n    Set i to i + 1.\nSet i to 0.\nWhile i is less than n:\n    Let mutable k be 0.\n    While k is less than n:\n        Let mutable j be 0.\n        While j is less than n:\n            Let idx be i * n + j + 1.\n            Set item idx of c to (item idx of c) + (item (i * n + k + 1) of a) * (item (k * n + j + 1) of a).\n            Set j to j + 1.\n        Set k to k + 1.\n    Set i to i + 1.\nShow item 1 of c.\n";
    let plain = "## Main\nLet xs be [10, 20, 30].\nLet mutable i be 1.\nWhile i is at most 4:\n    Show item i of xs.\n    Set i to i + 1.\n";
    let alone = crate::compile::compile_to_rust(plain).expect("compiles");
    assert!(
        !range_oracle_installed(),
        "the range oracle outlived its compile"
    );
    let first = crate::compile::compile_to_rust(tiled).expect("compiles");
    assert!(
        first.contains("step_by"),
        "the first program tiles, establishing facts as it is emitted:\n{first}"
    );
    assert!(
        !range_oracle_installed(),
        "the range oracle outlived its compile"
    );
    let after = crate::compile::compile_to_rust(plain).expect("compiles");
    assert!(
        !range_oracle_installed(),
        "the range oracle outlived its compile"
    );
    assert_eq!(
        after, alone,
        "a program compiles the same after another as it does alone"
    );
}

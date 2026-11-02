use super::*;
use crate::ast::{LogicExpr, Term};
use crate::registry::SymbolRegistry;
use crate::OutputFormat;
use logicaffeine_base::Interner;

#[test]
fn test_lambda_formatting_unicode() {
    let mut interner = Interner::new();
    let expr_arena: Arena<LogicExpr> = Arena::new();
    let term_arena: Arena<Term> = Arena::new();

    let x = interner.intern("x");
    let sleep = interner.intern("Sleep");

    let body = expr_arena.alloc(LogicExpr::Predicate {
        name: sleep,
        args: term_arena.alloc_slice([Term::Variable(x)]),
        world: None,
    });
    let lambda = expr_arena.alloc(LogicExpr::Lambda { variable: x, body });

    let mut registry = SymbolRegistry::new();
    let output = lambda.transpile(&mut registry, &interner, OutputFormat::Unicode);
    assert!(output.contains("λx"), "Unicode should use λ: {}", output);
}

#[test]
fn test_lambda_formatting_latex() {
    let mut interner = Interner::new();
    let expr_arena: Arena<LogicExpr> = Arena::new();
    let term_arena: Arena<Term> = Arena::new();

    let x = interner.intern("x");
    let sleep = interner.intern("Sleep");

    let body = expr_arena.alloc(LogicExpr::Predicate {
        name: sleep,
        args: term_arena.alloc_slice([Term::Variable(x)]),
        world: None,
    });
    let lambda = expr_arena.alloc(LogicExpr::Lambda { variable: x, body });

    let mut registry = SymbolRegistry::new();
    let output = lambda.transpile(&mut registry, &interner, OutputFormat::LaTeX);
    assert!(
        output.contains("\\lambda"),
        "LaTeX should use \\lambda: {}",
        output
    );
}

#[test]
fn test_application_formatting() {
    let mut interner = Interner::new();
    let expr_arena: Arena<LogicExpr> = Arena::new();

    let p = interner.intern("P");
    let j = interner.intern("j");

    let func = expr_arena.alloc(LogicExpr::Atom(p));
    let arg = expr_arena.alloc(LogicExpr::Atom(j));
    let app = expr_arena.alloc(LogicExpr::App {
        function: func,
        argument: arg,
    });

    let mut registry = SymbolRegistry::new();
    let output = app.transpile(&mut registry, &interner, OutputFormat::Unicode);
    assert!(
        output.contains("(") && output.contains(")"),
        "App should have parens: {}",
        output
    );
}

#[test]
fn test_nested_lambda() {
    let mut interner = Interner::new();
    let expr_arena: Arena<LogicExpr> = Arena::new();

    let x = interner.intern("x");
    let y = interner.intern("y");

    let inner_body = expr_arena.alloc(LogicExpr::Atom(x));
    let inner_lambda = expr_arena.alloc(LogicExpr::Lambda {
        variable: y,
        body: inner_body,
    });
    let outer_lambda = expr_arena.alloc(LogicExpr::Lambda {
        variable: x,
        body: inner_lambda,
    });

    let mut registry = SymbolRegistry::new();
    let output = outer_lambda.transpile(&mut registry, &interner, OutputFormat::Unicode);
    assert!(
        output.contains("λx") && output.contains("λy"),
        "Nested lambdas: {}",
        output
    );
}

#[test]
fn test_lambda_app_helper_functions() {
    let mut interner = Interner::new();
    let expr_arena: Arena<LogicExpr> = Arena::new();
    let _term_arena: Arena<Term> = Arena::new();

    let x = interner.intern("x");
    let p = interner.intern("P");

    let body = expr_arena.alloc(LogicExpr::Atom(x));
    let lambda = LogicExpr::lambda(x, body, &expr_arena);

    let arg = expr_arena.alloc(LogicExpr::Atom(p));
    let app = LogicExpr::app(lambda, arg, &expr_arena);

    assert!(matches!(app, LogicExpr::App { .. }));
}

#[test]
fn lift_proper_name_returns_lambda() {
    let mut interner = Interner::new();
    let arena: Arena<LogicExpr> = Arena::new();

    let john = interner.intern("John");
    let lifted = lift_proper_name(john, &mut interner, &arena);

    assert!(
        matches!(lifted, LogicExpr::Lambda { .. }),
        "Should return Lambda"
    );
}

#[test]
fn lift_proper_name_applies_predicate() {
    let mut interner = Interner::new();
    let arena: Arena<LogicExpr> = Arena::new();

    let john = interner.intern("John");
    let lifted = lift_proper_name(john, &mut interner, &arena);

    if let LogicExpr::Lambda { body, .. } = lifted {
        assert!(matches!(body, LogicExpr::App { .. }), "Body should be App");
    } else {
        panic!("Expected Lambda");
    }
}

#[test]
fn lift_quantifier_universal_returns_lambda() {
    let mut interner = Interner::new();
    let expr_arena: Arena<LogicExpr> = Arena::new();
    let term_arena: Arena<Term> = Arena::new();

    let woman = interner.intern("woman");
    let lifted = lift_quantifier(
        QuantifierKind::Universal,
        woman,
        &mut interner,
        &expr_arena,
        &term_arena,
    );

    assert!(
        matches!(lifted, LogicExpr::Lambda { .. }),
        "Should return Lambda"
    );
}

#[test]
fn lift_quantifier_universal_structure() {
    let mut interner = Interner::new();
    let expr_arena: Arena<LogicExpr> = Arena::new();
    let term_arena: Arena<Term> = Arena::new();

    let woman = interner.intern("woman");
    let lifted = lift_quantifier(
        QuantifierKind::Universal,
        woman,
        &mut interner,
        &expr_arena,
        &term_arena,
    );

    if let LogicExpr::Lambda { body, .. } = lifted {
        assert!(
            matches!(
                body,
                LogicExpr::Quantifier {
                    kind: QuantifierKind::Universal,
                    ..
                }
            ),
            "Body should contain ∀, got {:?}",
            body
        );
    } else {
        panic!("Expected Lambda, got {:?}", lifted);
    }
}

#[test]
fn lift_quantifier_existential_returns_lambda() {
    let mut interner = Interner::new();
    let expr_arena: Arena<LogicExpr> = Arena::new();
    let term_arena: Arena<Term> = Arena::new();

    let man = interner.intern("man");
    let lifted = lift_quantifier(
        QuantifierKind::Existential,
        man,
        &mut interner,
        &expr_arena,
        &term_arena,
    );

    assert!(
        matches!(lifted, LogicExpr::Lambda { .. }),
        "Should return Lambda"
    );
}

#[test]
fn lift_quantifier_existential_structure() {
    let mut interner = Interner::new();
    let expr_arena: Arena<LogicExpr> = Arena::new();
    let term_arena: Arena<Term> = Arena::new();

    let man = interner.intern("man");
    let lifted = lift_quantifier(
        QuantifierKind::Existential,
        man,
        &mut interner,
        &expr_arena,
        &term_arena,
    );

    if let LogicExpr::Lambda { body, .. } = lifted {
        assert!(
            matches!(
                body,
                LogicExpr::Quantifier {
                    kind: QuantifierKind::Existential,
                    ..
                }
            ),
            "Body should contain ∃, got {:?}",
            body
        );
    } else {
        panic!("Expected Lambda, got {:?}", lifted);
    }
}

#[test]
fn beta_reduce_simple_predicate() {
    let mut interner = Interner::new();
    let expr_arena: Arena<LogicExpr> = Arena::new();
    let term_arena: Arena<Term> = Arena::new();

    let x = interner.intern("x");
    let john = interner.intern("John");
    let run = interner.intern("Run");

    let body = expr_arena.alloc(LogicExpr::Predicate {
        name: run,
        args: term_arena.alloc_slice([Term::Variable(x)]),
        world: None,
    });
    let lambda = expr_arena.alloc(LogicExpr::Lambda { variable: x, body });
    let arg = expr_arena.alloc(LogicExpr::Atom(john));
    let app = expr_arena.alloc(LogicExpr::App {
        function: lambda,
        argument: arg,
    });

    let reduced = beta_reduce(app, &expr_arena, &term_arena);

    let mut registry = SymbolRegistry::new();
    let output = reduced.transpile(&mut registry, &interner, OutputFormat::Unicode);
    assert!(
        output.contains("R(J)") || output.contains("Run(John)"),
        "Should substitute: {}",
        output
    );
}

#[test]
fn beta_reduce_with_constant() {
    let mut interner = Interner::new();
    let expr_arena: Arena<LogicExpr> = Arena::new();
    let term_arena: Arena<Term> = Arena::new();

    let x = interner.intern("x");
    let c = interner.intern("c");

    let body = expr_arena.alloc(LogicExpr::Atom(c));
    let lambda = expr_arena.alloc(LogicExpr::Lambda { variable: x, body });
    let arg = expr_arena.alloc(LogicExpr::Atom(interner.intern("anything")));
    let app = expr_arena.alloc(LogicExpr::App {
        function: lambda,
        argument: arg,
    });

    let reduced = beta_reduce(app, &expr_arena, &term_arena);
    assert!(
        matches!(reduced, LogicExpr::Atom(s) if *s == c),
        "Constant should remain"
    );
}

#[test]
fn beta_reduce_nested_lambda() {
    let mut interner = Interner::new();
    let expr_arena: Arena<LogicExpr> = Arena::new();
    let term_arena: Arena<Term> = Arena::new();

    let x = interner.intern("x");
    let y = interner.intern("y");

    let inner_body = expr_arena.alloc(LogicExpr::Atom(x));
    let inner_lambda = expr_arena.alloc(LogicExpr::Lambda {
        variable: y,
        body: inner_body,
    });
    let outer_lambda = expr_arena.alloc(LogicExpr::Lambda {
        variable: x,
        body: inner_lambda,
    });

    let reduced = beta_reduce(outer_lambda, &expr_arena, &term_arena);
    assert!(
        matches!(reduced, LogicExpr::Lambda { .. }),
        "Should still be lambda"
    );
}

#[test]
fn beta_reduce_non_application_unchanged() {
    let mut interner = Interner::new();
    let expr_arena: Arena<LogicExpr> = Arena::new();
    let term_arena: Arena<Term> = Arena::new();

    let p = interner.intern("P");
    let atom = expr_arena.alloc(LogicExpr::Atom(p));

    let reduced = beta_reduce(atom, &expr_arena, &term_arena);
    assert!(
        matches!(reduced, LogicExpr::Atom(s) if *s == p),
        "Atom unchanged"
    );
}

#[test]
fn beta_reduce_preserves_unbound_variables() {
    let mut interner = Interner::new();
    let expr_arena: Arena<LogicExpr> = Arena::new();
    let term_arena: Arena<Term> = Arena::new();

    let x = interner.intern("x");
    let y = interner.intern("y");
    let john = interner.intern("John");
    let loves = interner.intern("Loves");

    let body = expr_arena.alloc(LogicExpr::Predicate {
        name: loves,
        args: term_arena.alloc_slice([Term::Variable(x), Term::Variable(y)]),
        world: None,
    });
    let lambda = expr_arena.alloc(LogicExpr::Lambda { variable: x, body });
    let arg = expr_arena.alloc(LogicExpr::Atom(john));
    let app = expr_arena.alloc(LogicExpr::App {
        function: lambda,
        argument: arg,
    });

    let reduced = beta_reduce(app, &expr_arena, &term_arena);

    let mut registry = SymbolRegistry::new();
    let output = reduced.transpile(&mut registry, &interner, OutputFormat::Unicode);
    assert!(output.contains("y"), "y should remain unbound: {}", output);
}

#[test]
fn enumerate_scopings_single_quantifier() {
    let mut interner = Interner::new();
    let expr_arena: Arena<LogicExpr> = Arena::new();
    let term_arena: Arena<Term> = Arena::new();

    let x = interner.intern("x");
    let dog = interner.intern("Dog");
    let bark = interner.intern("Bark");

    let left = expr_arena.alloc(LogicExpr::Predicate {
        name: dog,
        args: term_arena.alloc_slice([Term::Variable(x)]),
        world: None,
    });
    let right = expr_arena.alloc(LogicExpr::Predicate {
        name: bark,
        args: term_arena.alloc_slice([Term::Variable(x)]),
        world: None,
    });
    let body = expr_arena.alloc(LogicExpr::BinaryOp {
        left,
        op: TokenType::Implies,
        right,
    });
    let expr = expr_arena.alloc(LogicExpr::Quantifier {
        kind: QuantifierKind::Universal,
        variable: x,
        body,
        island_id: 0,
    });

    let scopings = enumerate_scopings(expr, &mut interner, &expr_arena, &term_arena);
    assert_eq!(scopings.len(), 1, "Single quantifier should have 1 reading");
}

#[test]
fn enumerate_scopings_no_quantifier() {
    let mut interner = Interner::new();
    let expr_arena: Arena<LogicExpr> = Arena::new();
    let term_arena: Arena<Term> = Arena::new();

    let run = interner.intern("Run");
    let john = interner.intern("John");

    let expr = expr_arena.alloc(LogicExpr::Predicate {
        name: run,
        args: term_arena.alloc_slice([Term::Constant(john)]),
        world: None,
    });

    let scopings = enumerate_scopings(expr, &mut interner, &expr_arena, &term_arena);
    assert_eq!(scopings.len(), 1, "No quantifiers should have 1 reading");
}

#[test]
fn is_opaque_verb_believes() {
    let mut interner = Interner::new();
    let believes = interner.intern("believes");
    let believes_cap = interner.intern("Believes");
    assert!(
        is_opaque_verb(believes, &interner),
        "believes should be opaque"
    );
    assert!(
        is_opaque_verb(believes_cap, &interner),
        "Believes should be opaque"
    );
}

#[test]
fn is_opaque_verb_seeks() {
    let mut interner = Interner::new();
    let seeks = interner.intern("seeks");
    let wants = interner.intern("wants");
    assert!(is_opaque_verb(seeks, &interner), "seeks should be opaque");
    assert!(is_opaque_verb(wants, &interner), "wants should be opaque");
}

#[test]
fn is_opaque_verb_normal_verbs() {
    let mut interner = Interner::new();
    let runs = interner.intern("runs");
    let loves = interner.intern("loves");
    assert!(
        !is_opaque_verb(runs, &interner),
        "runs should NOT be opaque"
    );
    assert!(
        !is_opaque_verb(loves, &interner),
        "loves should NOT be opaque"
    );
}

#[test]
fn make_intensional_creates_wrapper() {
    let mut interner = Interner::new();
    let expr_arena: Arena<LogicExpr> = Arena::new();
    let term_arena: Arena<Term> = Arena::new();

    let weak = interner.intern("Weak");
    let clark = interner.intern("Clark");
    let believes = interner.intern("believes");

    let content = expr_arena.alloc(LogicExpr::Predicate {
        name: weak,
        args: term_arena.alloc_slice([Term::Constant(clark)]),
        world: None,
    });

    let intensional = make_intensional(believes, content, &expr_arena);

    assert!(
        matches!(intensional, LogicExpr::Intensional { operator, .. } if *operator == believes),
        "Should create Intensional wrapper, got {:?}",
        intensional
    );
}

#[test]
fn intensional_transpiles_with_brackets() {
    let mut interner = Interner::new();
    let expr_arena: Arena<LogicExpr> = Arena::new();
    let term_arena: Arena<Term> = Arena::new();

    let weak = interner.intern("Weak");
    let clark = interner.intern("Clark");
    let believes = interner.intern("Believes");

    let content = expr_arena.alloc(LogicExpr::Predicate {
        name: weak,
        args: term_arena.alloc_slice([Term::Constant(clark)]),
        world: None,
    });

    let intensional = expr_arena.alloc(LogicExpr::Intensional {
        operator: believes,
        content,
    });

    let mut registry = SymbolRegistry::new();
    let output = intensional.transpile(&mut registry, &interner, OutputFormat::Unicode);

    assert!(
        output.contains("[") && output.contains("]"),
        "Intensional should use brackets: got {}",
        output
    );
}

#[test]
fn substitute_respecting_opacity_blocks_inside_intensional() {
    let mut interner = Interner::new();
    let expr_arena: Arena<LogicExpr> = Arena::new();
    let term_arena: Arena<Term> = Arena::new();

    let weak = interner.intern("Weak");
    let clark = interner.intern("Clark");
    let believes = interner.intern("Believes");
    let superman = interner.intern("Superman");

    let inner = expr_arena.alloc(LogicExpr::Predicate {
        name: weak,
        args: term_arena.alloc_slice([Term::Constant(clark)]),
        world: None,
    });
    let expr = expr_arena.alloc(LogicExpr::Intensional {
        operator: believes,
        content: inner,
    });

    let replacement = expr_arena.alloc(LogicExpr::Atom(superman));
    let result = substitute_respecting_opacity(expr, clark, replacement, &expr_arena, &term_arena);

    let mut registry = SymbolRegistry::new();
    let output = result.transpile(&mut registry, &interner, OutputFormat::Unicode);

    assert!(
        output.contains("C") && !output.contains("S"),
        "Should NOT substitute inside intensional context: got {}",
        output
    );
}

#[test]
fn substitute_respecting_opacity_allows_outside() {
    let mut interner = Interner::new();
    let expr_arena: Arena<LogicExpr> = Arena::new();
    let term_arena: Arena<Term> = Arena::new();

    let weak = interner.intern("Weak");
    let clark = interner.intern("Clark");
    let superman = interner.intern("Superman");

    let expr = expr_arena.alloc(LogicExpr::Predicate {
        name: weak,
        args: term_arena.alloc_slice([Term::Constant(clark)]),
        world: None,
    });

    let replacement = expr_arena.alloc(LogicExpr::Atom(superman));
    let result = substitute_respecting_opacity(expr, clark, replacement, &expr_arena, &term_arena);

    let mut registry = SymbolRegistry::new();
    let output = result.transpile(&mut registry, &interner, OutputFormat::Unicode);

    assert!(
        output.contains("S"),
        "Should substitute outside intensional context: got {}",
        output
    );
}

#[test]
fn factorial_basic() {
    assert_eq!(factorial(0), 1);
    assert_eq!(factorial(1), 1);
    assert_eq!(factorial(2), 2);
    assert_eq!(factorial(3), 6);
    assert_eq!(factorial(4), 24);
    assert_eq!(factorial(5), 120);
}

#[test]
fn scope_iterator_two_quantifiers_yields_two() {
    let mut interner = Interner::new();
    let expr_arena: Arena<LogicExpr> = Arena::new();
    let term_arena: Arena<Term> = Arena::new();

    let x = interner.intern("x");
    let y = interner.intern("y");
    let man = interner.intern("Man");
    let woman = interner.intern("Woman");
    let loves = interner.intern("Loves");

    let man_x = expr_arena.alloc(LogicExpr::Predicate {
        name: man,
        args: term_arena.alloc_slice([Term::Variable(x)]),
        world: None,
    });
    let woman_y = expr_arena.alloc(LogicExpr::Predicate {
        name: woman,
        args: term_arena.alloc_slice([Term::Variable(y)]),
        world: None,
    });
    let loves_xy = expr_arena.alloc(LogicExpr::Predicate {
        name: loves,
        args: term_arena.alloc_slice([Term::Variable(x), Term::Variable(y)]),
        world: None,
    });

    let inner = expr_arena.alloc(LogicExpr::BinaryOp {
        left: woman_y,
        op: TokenType::And,
        right: loves_xy,
    });
    let inner_q = expr_arena.alloc(LogicExpr::Quantifier {
        kind: QuantifierKind::Existential,
        variable: y,
        body: inner,
        island_id: 0,
    });

    let outer = expr_arena.alloc(LogicExpr::BinaryOp {
        left: man_x,
        op: TokenType::Implies,
        right: inner_q,
    });
    let expr = expr_arena.alloc(LogicExpr::Quantifier {
        kind: QuantifierKind::Universal,
        variable: x,
        body: outer,
        island_id: 0,
    });

    let scopings: Vec<_> =
        enumerate_scopings(expr, &mut interner, &expr_arena, &term_arena).collect();
    assert_eq!(
        scopings.len(),
        2,
        "Two quantifiers should have 2! = 2 readings"
    );
}

#[test]
fn scope_iterator_three_quantifiers_yields_six() {
    let mut interner = Interner::new();
    let expr_arena: Arena<LogicExpr> = Arena::new();
    let term_arena: Arena<Term> = Arena::new();

    let x = interner.intern("x");
    let y = interner.intern("y");
    let z = interner.intern("z");
    let man = interner.intern("Man");
    let woman = interner.intern("Woman");
    let book = interner.intern("Book");
    let gives = interner.intern("Gives");

    let man_x = expr_arena.alloc(LogicExpr::Predicate {
        name: man,
        args: term_arena.alloc_slice([Term::Variable(x)]),
        world: None,
    });
    let woman_y = expr_arena.alloc(LogicExpr::Predicate {
        name: woman,
        args: term_arena.alloc_slice([Term::Variable(y)]),
        world: None,
    });
    let book_z = expr_arena.alloc(LogicExpr::Predicate {
        name: book,
        args: term_arena.alloc_slice([Term::Variable(z)]),
        world: None,
    });
    let gives_xyz = expr_arena.alloc(LogicExpr::Predicate {
        name: gives,
        args: term_arena.alloc_slice([Term::Variable(x), Term::Variable(y), Term::Variable(z)]),
        world: None,
    });

    let inner_z = expr_arena.alloc(LogicExpr::BinaryOp {
        left: book_z,
        op: TokenType::And,
        right: gives_xyz,
    });
    let q_z = expr_arena.alloc(LogicExpr::Quantifier {
        kind: QuantifierKind::Existential,
        variable: z,
        body: inner_z,
        island_id: 0,
    });

    let inner_y = expr_arena.alloc(LogicExpr::BinaryOp {
        left: woman_y,
        op: TokenType::And,
        right: q_z,
    });
    let q_y = expr_arena.alloc(LogicExpr::Quantifier {
        kind: QuantifierKind::Existential,
        variable: y,
        body: inner_y,
        island_id: 0,
    });

    let outer = expr_arena.alloc(LogicExpr::BinaryOp {
        left: man_x,
        op: TokenType::Implies,
        right: q_y,
    });
    let expr = expr_arena.alloc(LogicExpr::Quantifier {
        kind: QuantifierKind::Universal,
        variable: x,
        body: outer,
        island_id: 0,
    });

    let scopings: Vec<_> =
        enumerate_scopings(expr, &mut interner, &expr_arena, &term_arena).collect();
    assert_eq!(
        scopings.len(),
        6,
        "Three quantifiers should have 3! = 6 readings"
    );
}

#[test]
fn scope_iterator_no_duplicates() {
    use std::collections::HashSet;

    let mut interner = Interner::new();
    let expr_arena: Arena<LogicExpr> = Arena::new();
    let term_arena: Arena<Term> = Arena::new();

    let x = interner.intern("x");
    let y = interner.intern("y");
    let man = interner.intern("Man");
    let woman = interner.intern("Woman");
    let loves = interner.intern("Loves");

    let man_x = expr_arena.alloc(LogicExpr::Predicate {
        name: man,
        args: term_arena.alloc_slice([Term::Variable(x)]),
        world: None,
    });
    let woman_y = expr_arena.alloc(LogicExpr::Predicate {
        name: woman,
        args: term_arena.alloc_slice([Term::Variable(y)]),
        world: None,
    });
    let loves_xy = expr_arena.alloc(LogicExpr::Predicate {
        name: loves,
        args: term_arena.alloc_slice([Term::Variable(x), Term::Variable(y)]),
        world: None,
    });

    let inner = expr_arena.alloc(LogicExpr::BinaryOp {
        left: woman_y,
        op: TokenType::And,
        right: loves_xy,
    });
    let inner_q = expr_arena.alloc(LogicExpr::Quantifier {
        kind: QuantifierKind::Existential,
        variable: y,
        body: inner,
        island_id: 0,
    });

    let outer = expr_arena.alloc(LogicExpr::BinaryOp {
        left: man_x,
        op: TokenType::Implies,
        right: inner_q,
    });
    let expr = expr_arena.alloc(LogicExpr::Quantifier {
        kind: QuantifierKind::Universal,
        variable: x,
        body: outer,
        island_id: 0,
    });

    let mut registry = SymbolRegistry::new();
    let outputs: HashSet<String> =
        enumerate_scopings(expr, &mut interner, &expr_arena, &term_arena)
            .map(|e| e.transpile(&mut registry, &interner, OutputFormat::Unicode))
            .collect();

    assert_eq!(outputs.len(), 2, "All scopings should be unique");
}

#[test]
fn scope_iterator_exact_size() {
    let mut interner = Interner::new();
    let expr_arena: Arena<LogicExpr> = Arena::new();
    let term_arena: Arena<Term> = Arena::new();

    let x = interner.intern("x");
    let y = interner.intern("y");
    let man = interner.intern("Man");
    let woman = interner.intern("Woman");
    let loves = interner.intern("Loves");

    let man_x = expr_arena.alloc(LogicExpr::Predicate {
        name: man,
        args: term_arena.alloc_slice([Term::Variable(x)]),
        world: None,
    });
    let woman_y = expr_arena.alloc(LogicExpr::Predicate {
        name: woman,
        args: term_arena.alloc_slice([Term::Variable(y)]),
        world: None,
    });
    let loves_xy = expr_arena.alloc(LogicExpr::Predicate {
        name: loves,
        args: term_arena.alloc_slice([Term::Variable(x), Term::Variable(y)]),
        world: None,
    });

    let inner = expr_arena.alloc(LogicExpr::BinaryOp {
        left: woman_y,
        op: TokenType::And,
        right: loves_xy,
    });
    let inner_q = expr_arena.alloc(LogicExpr::Quantifier {
        kind: QuantifierKind::Existential,
        variable: y,
        body: inner,
        island_id: 0,
    });

    let outer = expr_arena.alloc(LogicExpr::BinaryOp {
        left: man_x,
        op: TokenType::Implies,
        right: inner_q,
    });
    let expr = expr_arena.alloc(LogicExpr::Quantifier {
        kind: QuantifierKind::Universal,
        variable: x,
        body: outer,
        island_id: 0,
    });

    let mut iter = enumerate_scopings(expr, &mut interner, &expr_arena, &term_arena);
    assert_eq!(iter.len(), 2);
    iter.next();
    assert_eq!(iter.len(), 1);
    iter.next();
    assert_eq!(iter.len(), 0);
}

#[test]
fn island_constraints_reduce_permutations() {
    let mut interner = Interner::new();
    let expr_arena: Arena<LogicExpr> = Arena::new();
    let term_arena: Arena<Term> = Arena::new();

    let x = interner.intern("x");
    let y = interner.intern("y");
    let man = interner.intern("Man");
    let woman = interner.intern("Woman");
    let loves = interner.intern("Loves");

    let man_x = expr_arena.alloc(LogicExpr::Predicate {
        name: man,
        args: term_arena.alloc_slice([Term::Variable(x)]),
        world: None,
    });
    let woman_y = expr_arena.alloc(LogicExpr::Predicate {
        name: woman,
        args: term_arena.alloc_slice([Term::Variable(y)]),
        world: None,
    });
    let loves_xy = expr_arena.alloc(LogicExpr::Predicate {
        name: loves,
        args: term_arena.alloc_slice([Term::Variable(x), Term::Variable(y)]),
        world: None,
    });

    let inner = expr_arena.alloc(LogicExpr::BinaryOp {
        left: woman_y,
        op: TokenType::And,
        right: loves_xy,
    });
    let inner_q = expr_arena.alloc(LogicExpr::Quantifier {
        kind: QuantifierKind::Existential,
        variable: y,
        body: inner,
        island_id: 1,
    });

    let outer = expr_arena.alloc(LogicExpr::BinaryOp {
        left: man_x,
        op: TokenType::Implies,
        right: inner_q,
    });
    let expr = expr_arena.alloc(LogicExpr::Quantifier {
        kind: QuantifierKind::Universal,
        variable: x,
        body: outer,
        island_id: 0,
    });

    let scopings = enumerate_scopings(expr, &mut interner, &expr_arena, &term_arena);
    assert_eq!(
        scopings.len(),
        1,
        "Two quantifiers in different islands: 1! × 1! = 1 reading (no cross-island scoping)"
    );
}

#[test]
fn multiple_quantifiers_per_island() {
    let mut interner = Interner::new();
    let expr_arena: Arena<LogicExpr> = Arena::new();
    let term_arena: Arena<Term> = Arena::new();

    let x = interner.intern("x");
    let y = interner.intern("y");
    let z = interner.intern("z");
    let w = interner.intern("w");
    let pred = interner.intern("P");

    let core = expr_arena.alloc(LogicExpr::Predicate {
        name: pred,
        args: term_arena.alloc_slice([
            Term::Variable(x),
            Term::Variable(y),
            Term::Variable(z),
            Term::Variable(w),
        ]),
        world: None,
    });

    let true_sym = interner.intern("T");
    let t = expr_arena.alloc(LogicExpr::Atom(true_sym));

    let q_w = expr_arena.alloc(LogicExpr::Quantifier {
        kind: QuantifierKind::Existential,
        variable: w,
        body: expr_arena.alloc(LogicExpr::BinaryOp {
            left: t,
            op: TokenType::And,
            right: core,
        }),
        island_id: 1,
    });
    let q_z = expr_arena.alloc(LogicExpr::Quantifier {
        kind: QuantifierKind::Existential,
        variable: z,
        body: expr_arena.alloc(LogicExpr::BinaryOp {
            left: t,
            op: TokenType::And,
            right: q_w,
        }),
        island_id: 1,
    });
    let q_y = expr_arena.alloc(LogicExpr::Quantifier {
        kind: QuantifierKind::Existential,
        variable: y,
        body: expr_arena.alloc(LogicExpr::BinaryOp {
            left: t,
            op: TokenType::And,
            right: q_z,
        }),
        island_id: 0,
    });
    let expr = expr_arena.alloc(LogicExpr::Quantifier {
        kind: QuantifierKind::Universal,
        variable: x,
        body: expr_arena.alloc(LogicExpr::BinaryOp {
            left: t,
            op: TokenType::Implies,
            right: q_y,
        }),
        island_id: 0,
    });

    let scopings = enumerate_scopings(expr, &mut interner, &expr_arena, &term_arena);
    assert_eq!(
        scopings.len(),
        4,
        "4 quantifiers split 2+2 across islands: 2! × 2! = 4 (not 4! = 24)"
    );
}

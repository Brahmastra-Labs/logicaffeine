use super::*;

fn num<'a>(a: &'a Arena<Expr<'a>>, v: i64) -> &'a Expr<'a> {
    a.alloc(Expr::Literal(Literal::Number(v)))
}
fn bin<'a>(
    a: &'a Arena<Expr<'a>>,
    op: BinaryOpKind,
    l: &'a Expr<'a>,
    r: &'a Expr<'a>,
) -> &'a Expr<'a> {
    a.alloc(Expr::BinaryOp {
        op,
        left: l,
        right: r,
    })
}

#[test]
fn const_rational_collapses_a_constant_chain_to_its_exact_value() {
    let a: Arena<Expr> = Arena::new();
    // 1/3 + 1/6  =  1/2.
    let e = bin(
        &a,
        BinaryOpKind::Add,
        bin(&a, BinaryOpKind::Divide, num(&a, 1), num(&a, 3)),
        bin(&a, BinaryOpKind::Divide, num(&a, 1), num(&a, 6)),
    );
    assert_eq!(const_rational(e).unwrap().to_string(), "1/2");

    // 1/3 + 1/3 + 1/3  =  1 (a whole number — distinct from the fractional case).
    let third = bin(&a, BinaryOpKind::ExactDivide, num(&a, 1), num(&a, 3));
    let e = bin(
        &a,
        BinaryOpKind::Add,
        bin(&a, BinaryOpKind::Add, third, third),
        third,
    );
    assert_eq!(const_rational(e).unwrap().to_string(), "1");

    // (2/3 * 3/4)  =  1/2 — multiplication folds too.
    let e = bin(
        &a,
        BinaryOpKind::Multiply,
        bin(&a, BinaryOpKind::ExactDivide, num(&a, 2), num(&a, 3)),
        bin(&a, BinaryOpKind::ExactDivide, num(&a, 3), num(&a, 4)),
    );
    assert_eq!(const_rational(e).unwrap().to_string(), "1/2");
}

#[test]
fn const_rational_refuses_a_non_constant_or_a_zero_divisor() {
    let a: Arena<Expr> = Arena::new();
    // A divide-by-zero subtree does NOT fold — it is left to error at runtime.
    let e = bin(&a, BinaryOpKind::ExactDivide, num(&a, 1), num(&a, 0));
    assert!(const_rational(e).is_none());
    // A variable leaf is not constant — no fold.
    let v = a.alloc(Expr::Identifier(Symbol::from_index(0)));
    let e = bin(&a, BinaryOpKind::Add, num(&a, 1), v);
    assert!(const_rational(e).is_none());
}

#[test]
fn closed_form_is_a_bare_number_when_whole_and_a_reduced_divide_otherwise() {
    let expr_arena: Arena<Expr> = Arena::new();

    // A whole value → a bare Number literal (no division at runtime).
    let whole = closed_form(&expr_arena, &Rational::from_i64(3)).unwrap();
    assert!(matches!(whole, Expr::Literal(Literal::Number(3))));

    // A fraction → a single reduced ExactDivide(num, den).
    let frac = Rational::from_ratio_i64(4, 6).unwrap(); // reduces to 2/3
    match closed_form(&expr_arena, &frac).unwrap() {
        Expr::BinaryOp {
            op: BinaryOpKind::ExactDivide,
            left,
            right,
        } => {
            assert!(matches!(left, Expr::Literal(Literal::Number(2))));
            assert!(matches!(right, Expr::Literal(Literal::Number(3))));
        }
        other => panic!("expected a reduced ExactDivide, got {other:?}"),
    }
}

/// The names of every function the program, as the seam hands it on, calls.
fn calls(source: &str) -> Vec<String> {
    crate::ui_bridge::with_parsed_program(source, |parsed, interner| {
        let (stmts, _, _) = parsed.expect("the program parses");
        crate::analysis::known_calls::harvest_calls(stmts)
            .calls
            .iter()
            .map(|c| interner.resolve(c.function).to_string())
            .collect()
    })
}

#[test]
fn a_power_to_an_exact_exponent_is_the_exact_power() {
    let exact = |source: &str| calls(source).iter().any(|c| c == "exactPower");
    assert!(exact("## Main\nLet half: Rational be 1 / 2.\nShow 2 ** half.\n"));
    assert!(exact("## Main\nLet half: Rational be 1 / 2.\nShow 8 ** (half * 2 / 3).\n"), "an exact operand inside the exponent");
    assert!(exact("## Main\nLet dt be an infinitesimal.\nLet half: Rational be 1 / 2.\nShow dt ** half.\n"));
    assert!(!exact("## Main\nShow 4 ** (3 / 2).\n"), "an exponent with no exact operand floors as an integer");
    assert!(!exact("## Main\nLet half: Rational be 1 / 2.\nShow half ** 2.\n"), "an integer exponent stays `**`");
    assert!(!exact("## Main\nLet half: Rational be 1 / 2.\nShow 2.0 ** half.\n"), "a float written as the base");
    assert!(
        !exact("## To exactPower (a: Int) and (b: Int) -> Int:\n    Return a + b.\n\n## Main\nLet half: Rational be 1 / 2.\nShow 2 ** half.\n"),
        "a program's own function of that name keeps `**` as written"
    );
}

use super::*;

/// `n % 2 = 0` with `n` named by `name`, folded against the facts of `source`'s program.
fn divisibility_test_folds(source: &str, name: &str) -> Option<bool> {
    crate::ui_bridge::with_parsed_program(source, |parsed, interner| {
        let (stmts, _, _) = parsed.as_ref().ok()?;
        let n = interner.lookup(name)?;
        let bools = program_bool_syms(stmts, interner);
        let ea: Arena<Expr> = Arena::new();
        let dividend = ea.alloc(Expr::Identifier(n));
        let two = ea.alloc(Expr::Literal(Literal::Number(2)));
        let zero = ea.alloc(Expr::Literal(Literal::Number(0)));
        let residue = ea.alloc(Expr::BinaryOp {
            op: BinaryOpKind::Modulo,
            left: dividend,
            right: two,
        });
        let folded = try_simplify_algebraic(BinaryOpKind::Eq, residue, zero, &ea, &bools);
        Some(matches!(
            folded,
            Some(Expr::BinaryOp {
                op: BinaryOpKind::Eq,
                left: Expr::BinaryOp {
                    op: BinaryOpKind::BitAnd,
                    ..
                },
                ..
            })
        ))
    })
}

#[test]
fn an_integer_s_divisibility_test_reads_its_low_bits() {
    assert_eq!(
        divisibility_test_folds("## Main\nLet n be 10.\nShow n.\n", "n"),
        Some(true)
    );
    assert_eq!(
        divisibility_test_folds(
            "## Main\nLet dt be an infinitesimal.\nLet n be 10.\nShow n.\nShow dt.\n",
            "n"
        ),
        Some(true),
        "a program that declares the infinitesimal keeps the fold where the infinitesimal does not reach"
    );
}

#[test]
fn an_infinite_integer_s_divisibility_test_keeps_its_remainder() {
    assert_eq!(
        divisibility_test_folds(
            "## Main\nLet dt be an infinitesimal.\nLet n be 1 / dt.\nShow n.\n",
            "n"
        ),
        Some(false),
        "the residue of 1/dt modulo 2 is a fact the mask cannot read"
    );
    assert_eq!(
        divisibility_test_folds(
            "## Main\nLet dt be an infinitesimal.\nLet n be 1 / dt.\nLet m be n + 1.\nShow m.\n",
            "m"
        ),
        Some(false),
        "a value computed from the infinite integer is reached too"
    );
}

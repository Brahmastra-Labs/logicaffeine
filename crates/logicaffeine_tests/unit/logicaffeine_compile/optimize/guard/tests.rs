use super::*;
use crate::intern::Interner;

fn num<'a>(ea: &'a Arena<Expr<'a>>, n: i64) -> &'a Expr<'a> {
    ea.alloc(Expr::Literal(Literal::Number(n)))
}

fn ident<'a>(ea: &'a Arena<Expr<'a>>, name: crate::intern::Symbol) -> &'a Expr<'a> {
    ea.alloc(Expr::Identifier(name))
}

fn bin<'a>(ea: &'a Arena<Expr<'a>>, op: BinaryOpKind, left: &'a Expr<'a>, right: &'a Expr<'a>) -> &'a Expr<'a> {
    ea.alloc(Expr::BinaryOp { op, left, right })
}

fn always(d: Decided) -> bool {
    matches!(d, Decided::Always)
}

fn never(d: Decided) -> bool {
    matches!(d, Decided::Never)
}

#[test]
fn a_comparison_of_two_literals_is_decided() {
    let ea = Arena::new();
    let b = num(&ea, 1 << 62);
    assert!(always(decide(bin(&ea, BinaryOpKind::LtEq, num(&ea, 5), b), &ea)));
    assert!(never(decide(bin(&ea, BinaryOpKind::LtEq, num(&ea, i64::MAX), b), &ea)));
    assert!(always(decide(bin(&ea, BinaryOpKind::GtEq, num(&ea, 3), num(&ea, -4)), &ea)));
    assert!(never(decide(bin(&ea, BinaryOpKind::NotEq, num(&ea, 7), num(&ea, 7)), &ea)));
    let negated = bin(&ea, BinaryOpKind::Subtract, num(&ea, 0), num(&ea, 1 << 30));
    assert!(always(decide(bin(&ea, BinaryOpKind::GtEq, num(&ea, -5), negated), &ea)));
    assert!(always(decide(ea.alloc(Expr::Literal(Literal::Boolean(true))), &ea)));
    assert!(never(decide(ea.alloc(Expr::Literal(Literal::Boolean(false))), &ea)));
}

#[test]
fn a_comparison_that_reads_a_name_is_left_to_run_time() {
    let ea = Arena::new();
    let mut interner = Interner::new();
    let n = ea.alloc(Expr::Identifier(interner.intern("n")));
    let guard = bin(&ea, BinaryOpKind::LtEq, n, num(&ea, 1 << 60));
    match decide(guard, &ea) {
        Decided::AtRunTime(kept) => assert!(std::ptr::eq(kept, guard)),
        _ => panic!("a guard on a name was decided before the program ran"),
    }
}

#[test]
fn a_conjunction_keeps_only_the_parts_that_can_go_either_way() {
    let ea = Arena::new();
    let mut interner = Interner::new();
    let n = ea.alloc(Expr::Identifier(interner.intern("n")));
    let b = num(&ea, 1 << 60);
    let on_n = bin(&ea, BinaryOpKind::LtEq, n, b);
    let literal = bin(&ea, BinaryOpKind::LtEq, num(&ea, 5), b);
    for guard in [
        bin(&ea, BinaryOpKind::And, literal, on_n),
        bin(&ea, BinaryOpKind::And, on_n, literal),
    ] {
        match decide(guard, &ea) {
            Decided::AtRunTime(kept) => assert!(std::ptr::eq(kept, on_n), "kept more than `n <= B`"),
            _ => panic!("the guard on `n` was decided before the program ran"),
        }
    }
    let both = bin(&ea, BinaryOpKind::And, literal, literal);
    assert!(always(decide(both, &ea)));
    let impossible = bin(&ea, BinaryOpKind::LtEq, num(&ea, i64::MAX), b);
    assert!(never(decide(bin(&ea, BinaryOpKind::And, on_n, impossible), &ea)));
}

#[test]
fn a_conjunction_of_names_is_kept_as_it_was_built() {
    let ea = Arena::new();
    let mut interner = Interner::new();
    let (n, m) = (
        ea.alloc(Expr::Identifier(interner.intern("n"))),
        ea.alloc(Expr::Identifier(interner.intern("m"))),
    );
    let b = num(&ea, 1 << 60);
    let guard = bin(
        &ea,
        BinaryOpKind::And,
        bin(&ea, BinaryOpKind::LtEq, n, b),
        bin(&ea, BinaryOpKind::LtEq, m, b),
    );
    match decide(guard, &ea) {
        Decided::AtRunTime(kept) => assert!(std::ptr::eq(kept, guard)),
        _ => panic!("a guard on names was decided before the program ran"),
    }
}

/// The bounds a list of guard parts states, rendered `name op bound` in order.
fn rendered(parts: &[&Expr], interner: &Interner) -> Vec<String> {
    parts
        .iter()
        .map(|p| match p {
            Expr::BinaryOp { op, left, right } => {
                let side = |e: &Expr| match e {
                    Expr::Identifier(s) => interner.resolve(*s).to_string(),
                    other => literal_number(other).map_or("?".to_string(), |n| n.to_string()),
                };
                format!("{} {:?} {}", side(left), op, side(right))
            }
            _ => "?".to_string(),
        })
        .collect()
}

#[test]
fn a_bound_a_tighter_one_implies_is_stated_once_at_its_first_place() {
    let ea = Arena::new();
    let mut interner = Interner::new();
    let n = interner.intern("n");
    let parts = [
        bin(&ea, BinaryOpKind::GtEq, ident(&ea, n), num(&ea, -(1 << 30))),
        bin(&ea, BinaryOpKind::LtEq, ident(&ea, n), num(&ea, 1 << 30)),
        bin(&ea, BinaryOpKind::LtEq, ident(&ea, n), num(&ea, 1 << 28)),
    ];
    let kept = tightened(&parts, &ea);
    assert_eq!(
        rendered(&kept, &interner),
        vec![format!("n GtEq {}", -(1i64 << 30)), format!("n LtEq {}", 1i64 << 28)]
    );
    assert!(std::ptr::eq(kept[0], parts[0]), "an untouched part keeps its node");
}

#[test]
fn a_strict_or_flipped_bound_is_read_as_the_inclusive_one_it_states() {
    let ea = Arena::new();
    let mut interner = Interner::new();
    let n = interner.intern("n");
    let parts = [
        bin(&ea, BinaryOpKind::Lt, ident(&ea, n), num(&ea, 100)),
        bin(&ea, BinaryOpKind::GtEq, num(&ea, 50), ident(&ea, n)),
        bin(&ea, BinaryOpKind::Gt, ident(&ea, n), num(&ea, -1)),
        bin(&ea, BinaryOpKind::LtEq, num(&ea, 3), ident(&ea, n)),
    ];
    let kept = tightened(&parts, &ea);
    assert_eq!(rendered(&kept, &interner), vec!["n LtEq 50".to_string(), "n GtEq 3".to_string()]);
}

#[test]
fn bounds_on_other_names_and_parts_that_compute_are_kept_in_order() {
    let ea = Arena::new();
    let mut interner = Interner::new();
    let (n, m) = (interner.intern("n"), interner.intern("m"));
    let computed = bin(
        &ea,
        BinaryOpKind::LtEq,
        bin(&ea, BinaryOpKind::Multiply, ea.alloc(Expr::Identifier(n)), num(&ea, 2)),
        num(&ea, 1 << 20),
    );
    let parts = [
        bin(&ea, BinaryOpKind::LtEq, ea.alloc(Expr::Identifier(n)), num(&ea, 1 << 30)),
        bin(&ea, BinaryOpKind::LtEq, ea.alloc(Expr::Identifier(m)), num(&ea, 1 << 30)),
        computed,
        bin(&ea, BinaryOpKind::LtEq, ea.alloc(Expr::Identifier(n)), num(&ea, 1 << 10)),
    ];
    let kept = tightened(&parts, &ea);
    assert_eq!(kept.len(), 3, "the looser bound on `n` folds into its first place");
    assert_eq!(rendered(&kept[..1], &interner), vec![format!("n LtEq {}", 1i64 << 10)]);
    assert!(std::ptr::eq(kept[1], parts[1]) && std::ptr::eq(kept[2], computed));
}

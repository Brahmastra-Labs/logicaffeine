use super::*;
use crate::optimization::{Opt, OptimizationConfig};

/// Version the checked program `src` under `cfg`, then hand the result to `inspect` while its facts are open. The
/// parse's place-write splices are fused first, as the optimizer does before codegen versions anything.
fn versioned<R>(
    src: &str,
    cfg: OptimizationConfig,
    inspect: impl FnOnce(&[Stmt], &[Stmt]) -> R,
) -> R {
    crate::ui_bridge::with_parsed_program(src, |parsed, interner| {
        let (stmts, registry, _) = parsed.expect("the program parses");
        let (exprs, stmt_arena, types) = (Arena::new(), Arena::new(), Arena::new());
        let stmts = super::super::splice_fuse::fuse_place_splices(
            stmts.to_vec(),
            &exprs,
            &stmt_arena,
            interner,
        );
        let env = crate::analysis::check_program_located(&stmts, interner, registry)
            .expect("the program checks");
        let _facts = facts::scope();
        let out = version_stmts(
            stmts.clone(),
            Arenas {
                exprs: &exprs,
                stmts: &stmt_arena,
                types: &types,
            },
            &env,
            interner,
            &cfg,
        );
        inspect(&stmts, &out)
    })
}

const PRODUCT: &str = "## To native parseInt (s: Text) -> Int\n\n## Main\nLet n be parseInt(\"40\").\nLet mutable a be a new Seq of Int.\nLet mutable c be a new Seq of Int.\nLet mutable i be 0.\nWhile i is less than n * n:\n    Push i % 7 to a.\n    Push 0 to c.\n    Set i to i + 1.\nSet i to 0.\nWhile i is less than n:\n    Let mutable k be 0.\n    While k is less than n:\n        Let mutable j be 0.\n        While j is less than n:\n            Let idx be i * n + j + 1.\n            Set item idx of c to (item idx of c) + (item (i * n + k + 1) of a) * (item (k * n + j + 1) of a).\n            Set j to j + 1.\n        Set k to k + 1.\n    Set i to i + 1.\nShow item 1 of c.\n";

/// The versioned region's `(guard, fast block, slow block)`.
fn region<'s, 'a>(out: &'s [Stmt<'a>]) -> (&'a Expr<'a>, &'a [Stmt<'a>], &'a [Stmt<'a>]) {
    out.iter()
        .find_map(|s| match s {
            Stmt::If {
                cond,
                then_block,
                else_block: Some(else_block),
            } => Some((*cond, *then_block, *else_block)),
            _ => None,
        })
        .expect("a versioned region")
}

fn every_index<'a>(block: &'a [Stmt<'a>]) -> Vec<&'a Expr<'a>> {
    use logicaffeine_language::ast_depth::{for_each_node, Visited};
    let mut out = Vec::new();
    for_each_node(block, |n| match n {
        Visited::Expr(Expr::Index { index, .. }) | Visited::Stmt(Stmt::SetIndex { index, .. }) => {
            out.push(*index)
        }
        _ => {}
    });
    out
}

fn conjuncts<'a>(e: &'a Expr<'a>, out: &mut Vec<&'a Expr<'a>>) {
    match e {
        Expr::BinaryOp {
            op: BinaryOpKind::And,
            left,
            right,
        } => {
            conjuncts(left, out);
            conjuncts(right, out);
        }
        other => out.push(other),
    }
}

#[test]
fn a_matrix_product_runs_its_clone_under_a_guard_and_the_original_otherwise() {
    versioned(PRODUCT, OptimizationConfig::all_on(), |_, out| {
        let (_, fast, slow) = region(out);
        let fast_loop = fast
            .iter()
            .find(|s| matches!(s, Stmt::While { .. }))
            .expect("the clone's loop");
        let slow_loop = slow
            .iter()
            .find(|s| matches!(s, Stmt::While { .. }))
            .expect("the original loop");
        assert!(
            facts::holds_expr(Fact::TileLegal, loop_cond(fast_loop)),
            "the clone may run in tile order"
        );
        assert!(
            !facts::holds_expr(Fact::TileLegal, loop_cond(slow_loop)),
            "the original runs as written"
        );
        let (fast_idx, slow_idx) = (every_index(fast), every_index(slow));
        assert_eq!(fast_idx.len(), 4);
        assert!(
            fast_idx
                .iter()
                .all(|i| facts::holds_expr(Fact::InBounds, *i)),
            "every access in the clone is proven"
        );
        assert!(
            slow_idx
                .iter()
                .all(|i| !facts::holds_expr(Fact::InBounds, *i)),
            "no access in the original is"
        );
        for i in &fast_idx {
            assert!(
                !slow_idx.iter().any(|s| std::ptr::eq(*s, *i)),
                "the clone shares no index node with the original"
            );
        }
    });
}

#[test]
fn the_guard_bounds_every_name_it_reads_before_any_arithmetic() {
    versioned(PRODUCT, OptimizationConfig::all_on(), |_, out| {
        let (guard, _, _) = region(out);
        let mut parts = Vec::new();
        conjuncts(guard, &mut parts);
        let is_magnitude = |e: &Expr| {
            matches!(e,
            Expr::BinaryOp { op: BinaryOpKind::GtEq | BinaryOpKind::LtEq, left: Expr::Identifier(_), right: Expr::Literal(Literal::Number(b)) }
                if b.abs() == GUARD)
        };
        let first_check = parts
            .iter()
            .position(|e| !is_magnitude(e))
            .expect("the corner checks follow");
        assert!(first_check >= 2, "n is guarded from both sides first");
        assert!(
            parts[first_check..].iter().all(|e| !is_magnitude(e)),
            "every magnitude guard comes before the checks"
        );
        assert!(
            parts[first_check..].iter().all(|e| fits(e)),
            "every check stays inside i64 under the guards"
        );
        for check in &parts[first_check..] {
            let Expr::BinaryOp { left, .. } = check else {
                panic!("a comparison")
            };
            if let Expr::BinaryOp {
                op: BinaryOpKind::Add | BinaryOpKind::Subtract | BinaryOpKind::Multiply,
                ..
            } = left
            {
                assert!(
                    facts::holds_expr(Fact::RawIntOp, *left),
                    "a check's arithmetic lowers raw: the guards before it keep it in i64"
                );
            }
        }
        let checks_least = parts[first_check..].iter().any(|e| {
            matches!(
                e,
                Expr::BinaryOp {
                    op: BinaryOpKind::GtEq,
                    right: Expr::Literal(Literal::Number(1)),
                    ..
                }
            )
        });
        assert!(
            !checks_least,
            "every least corner here is 0·n + 1, at least 1 for every guarded n: no check"
        );
    });
}

#[test]
fn a_corner_that_can_leave_i64_is_not_versioned() {
    let src = PRODUCT.replace(
        "Let idx be i * n + j + 1.",
        "Let idx be i * 100000000000 + j + 1.",
    );
    versioned(&src, OptimizationConfig::all_on(), |before, out| {
        assert_eq!(out.len(), before.len(), "nothing versioned");
        assert!(!out.iter().any(|s| matches!(s, Stmt::If { .. })));
    });
}

#[test]
fn a_nest_whose_order_shows_is_left_as_written() {
    let src = "## To native parseInt (s: Text) -> Int\n\n## Main\nLet n be parseInt(\"3\").\nLet mutable h be 0.\nLet mutable i be 0.\nSet i to 0.\nWhile i is less than n:\n    Let mutable j be 0.\n    While j is less than n:\n        Set h to (h * 31 + i * 7 + j) % 1000000007.\n        Set j to j + 1.\n    Set i to i + 1.\nShow h.\n";
    versioned(src, OptimizationConfig::all_on(), |before, out| {
        assert_eq!(out.len(), before.len());
        let lp = out
            .iter()
            .find(|s| matches!(s, Stmt::While { .. }))
            .unwrap();
        assert!(!facts::holds_expr(Fact::TileLegal, loop_cond(lp)));
    });
}

/// Every loop condition in `block`, nested blocks included.
fn loop_conds<'a>(block: &'a [Stmt<'a>]) -> Vec<&'a Expr<'a>> {
    use logicaffeine_language::ast_depth::{for_each_node, Visited};
    let mut out = Vec::new();
    for_each_node(block, |n| {
        if let Visited::Stmt(Stmt::While { cond, .. }) = n {
            out.push(*cond);
        }
    });
    out
}

#[test]
fn tiling_switched_off_tiles_nothing() {
    versioned(
        PRODUCT,
        OptimizationConfig::all_on().disable(Opt::Tile),
        |_, out| {
            assert!(
                loop_conds(out)
                    .iter()
                    .all(|c| !facts::holds_expr(Fact::TileLegal, *c)),
                "no loop may run in tile order"
            );
        },
    );
}

#[test]
fn bounds_and_tiling_both_switched_off_version_nothing() {
    let cfg = OptimizationConfig::all_on()
        .disable(Opt::Tile)
        .disable(Opt::OracleHints);
    versioned(PRODUCT, cfg, |before, out| {
        assert_eq!(out.len(), before.len());
        assert!(!out.iter().any(|s| matches!(s, Stmt::If { .. })));
    });
}

#[test]
fn a_counter_declared_by_let_stays_in_the_enclosing_scope() {
    let src = "## To native parseInt (s: Text) -> Int\n\n## Main\nLet n be parseInt(\"4\").\nLet mutable c be a new Seq of Int.\nLet mutable q be 0.\nWhile q is less than n * n:\n    Push 0 to c.\n    Set q to q + 1.\nLet mutable i be 0.\nWhile i is less than n:\n    Let mutable j be 0.\n    While j is less than n:\n        Set item (i * n + j + 1) of c to i.\n        Set j to j + 1.\n    Set i to i + 1.\nShow i.\n";
    versioned(src, OptimizationConfig::all_on(), |_, out| {
        let at = out
            .iter()
            .position(|s| matches!(s, Stmt::If { .. }))
            .expect("versioned");
        assert!(
            matches!(&out[at - 1], Stmt::Let { .. }),
            "the `Let` of the counter is hoisted above the guard"
        );
        let (_, fast, slow) = region(out);
        assert!(
            matches!(fast[0], Stmt::Set { .. }) && matches!(slow[0], Stmt::Set { .. }),
            "each branch re-states the counter"
        );
    });
}

fn and_depth(e: &Expr) -> usize {
    match e {
        Expr::BinaryOp {
            op: BinaryOpKind::And,
            left,
            right,
        } => 1 + and_depth(left).max(and_depth(right)),
        _ => 0,
    }
}

#[test]
fn a_guard_with_many_checks_stays_shallow_and_reads_in_order() {
    let reads: String = (0..16)
        .map(|q| format!("    Set h to h + item (i * n + j + {}) of a.\n", q + 1))
        .collect();
    let src = format!("## To native parseInt (s: Text) -> Int\n\n## Main\nLet n be parseInt(\"40\").\nLet i be parseInt(\"3\").\nLet mutable a be a new Seq of Int.\nLet mutable h be 0.\nLet mutable q be 0.\nWhile q is less than n * n:\n    Push q to a.\n    Set q to q + 1.\nLet mutable j be 0.\nWhile j is less than n - 16:\n{reads}    Set j to j + 1.\nShow h.\n");
    versioned(&src, OptimizationConfig::all_on(), |_, out| {
        let (guard, _, _) = region(out);
        let mut parts = Vec::new();
        conjuncts(guard, &mut parts);
        assert!(
            parts.len() >= 30,
            "two guards per name and two checks per access: {}",
            parts.len()
        );
        assert!(
            and_depth(guard) <= 6,
            "a balanced conjunction: depth {} for {} parts",
            and_depth(guard),
            parts.len()
        );
    });
}

#[test]
fn an_item_of_a_row_is_never_recorded_in_range() {
    let src = "## To native parseInt (s: Text) -> Int\n\n## Main\nLet n be parseInt(\"8\").\nLet mutable a be a new Seq of (Seq of Int).\nLet mutable c be a new Seq of (Seq of Int).\nLet mutable i be 0.\nWhile i is less than n * n:\n    Let mutable arow be a new Seq of Int.\n    Push i % 7 to arow.\n    Push arow to a.\n    Let mutable crow be a new Seq of Int.\n    Push 0 to crow.\n    Push crow to c.\n    Set i to i + 1.\nSet i to 0.\nWhile i is less than n:\n    Let mutable k be 0.\n    While k is less than n:\n        Let mutable j be 0.\n        While j is less than n:\n            Let idx be i * n + j + 1.\n            Set item 1 of (item idx of c) to (item 1 of (item idx of c)) + (item 1 of (item (i * n + k + 1) of a)) * (item 1 of (item (k * n + j + 1) of a)).\n            Set j to j + 1.\n        Set k to k + 1.\n    Set i to i + 1.\nShow item 1 of (item 1 of c).\n";
    use logicaffeine_language::ast_depth::{for_each_node, Visited};
    versioned(src, OptimizationConfig::all_on(), |_, out| {
        let (_, fast, _) = region(out);
        let (mut rows, mut items) = (Vec::new(), Vec::new());
        for_each_node(fast, |n| match n {
            Visited::Expr(Expr::Index {
                collection: Expr::Identifier(_),
                index,
            }) => rows.push(*index),
            Visited::Expr(Expr::Index {
                collection: Expr::Index { .. },
                index,
            })
            | Visited::Stmt(Stmt::SetIndex {
                collection: Expr::Index { .. },
                index,
                ..
            }) => items.push(*index),
            _ => {}
        });
        assert!(
            !rows.is_empty() && rows.iter().all(|i| facts::holds_expr(Fact::InBounds, *i)),
            "every row index is in range under the guard"
        );
        assert!(
            !items.is_empty() && items.iter().all(|i| !facts::holds_expr(Fact::InBounds, *i)),
            "no item of a row is: a row may be short"
        );
    });
}

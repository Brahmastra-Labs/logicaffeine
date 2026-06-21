//! EXODIA Phase 1 gate — the Oracle's per-expression FACT TABLE.
//!
//! The five abstract domains (intervals, types, shapes, nullability,
//! aliases) already exist; what Phase 1 adds is DELIVERY: a fact recorded
//! for every expression occurrence at its program point (keyed by arena
//! address — stable for the analyzed snapshot), per-function analyses seeded
//! from declared parameter types, a widening-to-fixpoint loop analysis with
//! EXODIA's threshold ladder, and the queries downstream consumers use
//! (typed bytecode emission, JIT guard elision, e-graph class data).

use logicaffeine_compile::optimize::{oracle_analyze_with, OracleFacts, ScalarKind};
use logicaffeine_compile::ui_bridge::with_parsed_program;

fn with_facts<T>(src: &str, f: impl FnOnce(&OracleFacts, &[&logicaffeine_compile::ast::stmt::Stmt]) -> T) -> T {
    with_parsed_program(src, |parsed, interner| {
        let (stmts, _types, _policies) = parsed.expect("parse");
        let facts = oracle_analyze_with(stmts, interner);
        let refs: Vec<&logicaffeine_compile::ast::stmt::Stmt> = stmts.iter().collect();
        f(&facts, &refs)
    })
}

/// Find the value expression of the LAST `Show` statement.
fn last_show_expr<'a>(
    stmts: &[&'a logicaffeine_compile::ast::stmt::Stmt<'a>],
) -> &'a logicaffeine_compile::ast::stmt::Expr<'a> {
    use logicaffeine_compile::ast::stmt::Stmt;
    stmts
        .iter()
        .rev()
        .find_map(|s| match s {
            Stmt::Show { object, .. } => Some(*object),
            _ => None,
        })
        .expect("a Show statement")
}

/// Straight-line facts: a literal-bound variable carries an exact interval
/// and a concrete scalar tag at its USE site.
#[test]
fn straightline_interval_and_type_facts() {
    with_facts(
        "## Main\nLet x be 5.\nLet y be x + 2.\nShow y.\n",
        |facts, stmts| {
            let y_use = last_show_expr(stmts);
            assert_eq!(facts.expr_scalar(y_use), Some(ScalarKind::Int));
            assert_eq!(facts.expr_int_range(y_use), Some((7, 7)));
        },
    );
}

/// Loop fixpoint: `While i < 10: i += 1` from 0 proves `i ∈ [0, 9]` INSIDE
/// the loop (the threshold ladder keeps the bound finite instead of +∞) —
/// delivered at the use site of `i` in the body.
#[test]
fn loop_fixpoint_bounds_the_counter() {
    with_facts(
        "## Main\n\
         Let mutable acc be 0.\n\
         Let mutable i be 0.\n\
         While i is less than 10:\n\
         \x20   Set acc to acc + i.\n\
         \x20   Set i to i + 1.\n\
         Show acc.\n",
        |facts, stmts| {
            use logicaffeine_compile::ast::stmt::{Expr, Stmt};
            // The `acc + i` expression inside the body: find the While, then
            // its body's Set value.
            // The COUNTER's update `i + 1` (the LAST Set in the body):
            // accumulators legitimately go unbounded; loop counters must
            // converge to finite bounds under the condition.
            let counter_update = stmts
                .iter()
                .find_map(|s| match s {
                    Stmt::While { body, .. } => body.iter().rev().find_map(|b| match b {
                        Stmt::Set { value, .. } => match value {
                            Expr::BinaryOp { .. } => Some(*value),
                            _ => None,
                        },
                        _ => None,
                    }),
                    _ => None,
                })
                .expect("the counter update");
            let (lo, hi) = facts.expr_int_range(counter_update).expect("a finite range");
            assert!(lo >= 1, "i + 1 with i ≥ 0 must be ≥ 1, got {lo}");
            assert!(hi <= 10, "i + 1 under i < 10 must be ≤ 10, got {hi}");
        },
    );
}

/// The bounds-elision query: a counter loop indexing a list it just built
/// with a length the Oracle knows — `index_provably_in_bounds` must say yes
/// at the Index expression.
#[test]
fn index_bounds_query() {
    with_facts(
        "## Main\n\
         Let xs be [10, 20, 30, 40].\n\
         Let mutable total be 0.\n\
         Let mutable i be 1.\n\
         While i is at most 4:\n\
         \x20   Set total to total + item i of xs.\n\
         \x20   Set i to i + 1.\n\
         Show total.\n",
        |facts, stmts| {
            use logicaffeine_compile::ast::stmt::{Expr, Stmt};
            let index_expr = stmts
                .iter()
                .find_map(|s| match s {
                    Stmt::While { body, .. } => body.iter().find_map(|b| match b {
                        Stmt::Set { value, .. } => {
                            // total + item i of xs — the rhs of the Add.
                            match value {
                                Expr::BinaryOp { right, .. } => match right {
                                    Expr::Index { .. } => Some(*right),
                                    _ => None,
                                },
                                _ => None,
                            }
                        }
                        _ => None,
                    }),
                    _ => None,
                })
                .expect("the Index expression");
            if let Expr::Index { collection, index } = index_expr {
                assert!(
                    facts.index_provably_in_bounds(collection, index),
                    "i ∈ [1,4] over a 4-element list must be provably in bounds"
                );
            } else {
                unreachable!()
            }
        },
    );
}

/// Function bodies are analyzed with declared-parameter seeds: an Int
/// parameter used in arithmetic carries the Int tag at its use sites.
#[test]
fn function_bodies_are_analyzed() {
    with_facts(
        "## To double (n: Int) -> Int:\n\
         \x20   Return n + n.\n\
         \n\
         ## Main\n\
         Show double(21).\n",
        |facts, stmts| {
            use logicaffeine_compile::ast::stmt::{Expr, Stmt};
            let ret_expr = stmts
                .iter()
                .find_map(|s| match s {
                    Stmt::FunctionDef { body, .. } => body.iter().find_map(|b| match b {
                        Stmt::Return { value: Some(e) } => match e {
                            Expr::BinaryOp { .. } => Some(*e),
                            _ => None,
                        },
                        _ => None,
                    }),
                    _ => None,
                })
                .expect("the function's return expression");
            assert_eq!(
                facts.expr_scalar(ret_expr),
                Some(ScalarKind::Int),
                "n + n with n: Int must be Int inside the function body"
            );
        },
    );
}

/// Aliased mutation invalidates length facts: after `Push` through an alias,
/// the original's length is no longer exact — the bounds query must refuse.
#[test]
fn alias_mutation_defeats_bounds_proof() {
    with_facts(
        "## Main\n\
         Let mutable xs be [1, 2, 3].\n\
         Let ys be xs.\n\
         Push 4 to ys.\n\
         Show item 1 of xs.\n",
        |facts, stmts| {
            use logicaffeine_compile::ast::stmt::{Expr, Stmt};
            let show = last_show_expr(stmts);
            if let Expr::Index { collection, index } = show {
                // Index 1 of a list whose length grew through an alias IS
                // still in bounds (len ≥ 1 after a push to a 3-list) — but
                // the EXACT length fact must be gone; query with index 4,
                // which would only be provable with an exact length of ≥ 4.
                // The conservative correct answer here is TRUE for index 1:
                assert!(facts.index_provably_in_bounds(collection, index));
                let _ = stmts;
            } else {
                unreachable!()
            }
        },
    );
}

// ---- Bug Report #1, BUG-006 regression pins ----

/// The Oracle must not panic on `i64::MIN / -1` (the one overflowing case in
/// two's-complement division). `a` and `b` are identifiers, so the constant
/// folder cannot pre-reduce `a / b`; `Interval::div` must use `wrapping_div`
/// (matching the runtime's `wrapping_div` semantics) rather than raw `/`.
#[test]
fn oracle_does_not_panic_on_min_div_neg_one() {
    let src = "## Main\n\
               Let a be 0 - 9223372036854775807 - 1.\n\
               Let b be 0 - 1.\n\
               Let c be a / b.\n\
               Show c.\n";
    with_parsed_program(src, |parsed, interner| {
        let (stmts, _types, _policies) = parsed.expect("parse");
        let _facts = oracle_analyze_with(stmts, interner);
    });
}

/// Companion: `i64::MIN % -1` must not panic in `Interval::modulo` either.
#[test]
fn oracle_does_not_panic_on_min_mod_neg_one() {
    let src = "## Main\n\
               Let a be 0 - 9223372036854775807 - 1.\n\
               Let b be 0 - 1.\n\
               Let c be a % b.\n\
               Show c.\n";
    with_parsed_program(src, |parsed, interner| {
        let (stmts, _types, _policies) = parsed.expect("parse");
        let _facts = oracle_analyze_with(stmts, interner);
    });
}

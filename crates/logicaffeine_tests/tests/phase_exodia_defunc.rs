//! EXODIA Sprint 23 (Group 4) — DEFUNCTIONALIZATION (Reynolds 1972).
//!
//! Stage 1: DIRECT closure elimination. A closure bound to an immutable
//! `Let` whose every use is a direct call lifts to a top-level
//! FunctionDef; captures materialize as immutable snapshot bindings at
//! the closure's creation point (value semantics preserved by
//! construction); calls become first-order `Call`s. The closure VALUE —
//! the heap `ClosureValue` box and its env — never exists.
//!
//! Anything outside the shape (escaping closures, mutable rebinding,
//! underivable capture types) DECLINES: the residual keeps the original
//! closure and the MakeClosure path stays exact.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::ast::stmt::{Expr, Stmt};
use logicaffeine_compile::compile::tw_outcome_with_args;
use logicaffeine_compile::ui_bridge::with_v2_optimized_program;

fn norm(s: &str) -> String {
    s.lines()
        .map(|l| l.trim_end())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn expr_closures(e: &Expr, n: &mut usize) {
    match e {
        Expr::Closure { body, .. } => {
            *n += 1;
            match body {
                logicaffeine_compile::ast::stmt::ClosureBody::Expression(b) => {
                    expr_closures(b, n)
                }
                logicaffeine_compile::ast::stmt::ClosureBody::Block(stmts) => {
                    for s in *stmts {
                        stmt_closures(s, n);
                    }
                }
            }
        }
        Expr::CallExpr { callee, args } => {
            *n += 1;
            expr_closures(callee, n);
            for a in args {
                expr_closures(a, n);
            }
        }
        Expr::BinaryOp { left, right, .. } => {
            expr_closures(left, n);
            expr_closures(right, n);
        }
        Expr::Not { operand } => expr_closures(operand, n),
        Expr::Call { args, .. } => {
            for a in args {
                expr_closures(a, n);
            }
        }
        Expr::Index { collection, index } => {
            expr_closures(collection, n);
            expr_closures(index, n);
        }
        Expr::Length { collection } => expr_closures(collection, n),
        Expr::List(items) | Expr::Tuple(items) => {
            for i in items {
                expr_closures(i, n);
            }
        }
        _ => {}
    }
}

fn stmt_closures(s: &Stmt, n: &mut usize) {
    match s {
        Stmt::Let { value, .. } | Stmt::Set { value, .. } => expr_closures(value, n),
        Stmt::Show { object, .. } => expr_closures(object, n),
        Stmt::Return { value } => {
            if let Some(v) = value {
                expr_closures(v, n);
            }
        }
        Stmt::While { cond, body, .. } => {
            expr_closures(cond, n);
            for b in *body {
                stmt_closures(b, n);
            }
        }
        Stmt::If { cond, then_block, else_block } => {
            expr_closures(cond, n);
            for b in *then_block {
                stmt_closures(b, n);
            }
            if let Some(eb) = else_block {
                for b in *eb {
                    stmt_closures(b, n);
                }
            }
        }
        Stmt::FunctionDef { body, .. } => {
            for b in *body {
                stmt_closures(b, n);
            }
        }
        Stmt::Push { value, collection } => {
            expr_closures(value, n);
            expr_closures(collection, n);
        }
        Stmt::Call { args, .. } => {
            for a in args {
                expr_closures(a, n);
            }
        }
        _ => {}
    }
}

/// Runs `src` through v2; returns (output, error, closure+callexpr node
/// count in the residual).
fn v2_closure_census(src: &str, argv: &[String]) -> (String, Option<String>, usize) {
    with_v2_optimized_program(src, |parsed, interner| {
        let (stmts, types, policies) = parsed.expect("v2 parse");
        let mut n = 0usize;
        for s in stmts {
            stmt_closures(s, &mut n);
        }
        let (out, err) = logicaffeine_compile::vm::run_to_outcome_with_args(
            stmts,
            interner,
            Some(types),
            Some(&policies),
            argv,
            None,
        );
        (out, err, n)
    })
}

fn assert_parity_and_census(src: &str, argv_parts: &[&str], expect_out: &str, census: usize) {
    let argv: Vec<String> = argv_parts.iter().map(|s| s.to_string()).collect();
    let (out, err, n) = v2_closure_census(src, &argv);
    let tw = tw_outcome_with_args(src, &argv);
    assert_eq!(
        (norm(&out), &err),
        (norm(&tw.output), &tw.error),
        "v2 diverged from raw tree-walker on:\n{src}"
    );
    assert_eq!(err, None);
    assert_eq!(norm(&out), expect_out);
    assert_eq!(
        n, census,
        "closure/callexpr census mismatch in the residual of:\n{src}"
    );
}

/// A let-bound closure with a DYNAMIC argument (no pass can fold it):
/// the residual must be CLOSURE-FREE and exact.
#[test]
fn direct_closure_eliminated_dynamic_arg() {
    let src = "## To native args () -> Seq of Text\n\
               ## To native parseInt (s: Text) -> Int\n\
               \n\
               ## Main\n\
               Let arguments be args().\n\
               Let d be parseInt(item 2 of arguments).\n\
               Let doubler be (x: Int) -> x * 2.\n\
               Show doubler(d).\n\
               Show doubler(d + 1).\n";
    assert_parity_and_census(src, &["bench", "21"], "42\n44", 0);
}

/// Capture-by-value snapshot semantics: the closure sees the captured
/// variable AS OF its creation; a later `Set` must not leak in. The
/// residual is closure-free and the snapshot is exact.
#[test]
fn capture_snapshot_survives_mutation() {
    let src = "## To native args () -> Seq of Text\n\
               ## To native parseInt (s: Text) -> Int\n\
               \n\
               ## Main\n\
               Let arguments be args().\n\
               Let mutable c be parseInt(item 2 of arguments).\n\
               Let addc be (x: Int) -> x + c.\n\
               Set c to c * 100.\n\
               Show addc(1).\n\
               Show c.\n";
    assert_parity_and_census(src, &["bench", "10"], "11\n1000", 0);
}

/// Two distinct closures, each direct-called: both eliminate.
#[test]
fn two_distinct_closures_eliminate() {
    let src = "## To native args () -> Seq of Text\n\
               ## To native parseInt (s: Text) -> Int\n\
               \n\
               ## Main\n\
               Let arguments be args().\n\
               Let d be parseInt(item 2 of arguments).\n\
               Let inc be (x: Int) -> x + 1.\n\
               Let dec be (x: Int) -> x - 1.\n\
               Show inc(d) + dec(d).\n";
    assert_parity_and_census(src, &["bench", "50"], "100", 0);
}

/// Block-bodied closures lift too.
#[test]
fn block_body_closure_lifts() {
    let src = "## To native args () -> Seq of Text\n\
               ## To native parseInt (s: Text) -> Int\n\
               \n\
               ## Main\n\
               Let arguments be args().\n\
               Let d be parseInt(item 2 of arguments).\n\
               Let process be (n: Int) ->:\n\
               \x20   Let doubled be n * 2.\n\
               \x20   Return doubled + 1.\n\
               Show process(d).\n";
    assert_parity_and_census(src, &["bench", "7"], "15", 0);
}

/// A closure inside a FUNCTION body (not Main) eliminates the same way.
#[test]
fn closure_inside_function_body_eliminates() {
    let src = "## To native args () -> Seq of Text\n\
               ## To native parseInt (s: Text) -> Int\n\
               \n\
               ## To compute (n: Int) -> Int:\n\
               \x20   Let scale be (x: Int) -> x * n.\n\
               \x20   Return scale(3) + scale(4).\n\
               \n\
               ## Main\n\
               Let arguments be args().\n\
               Let d be parseInt(item 2 of arguments).\n\
               Show compute(d).\n";
    assert_parity_and_census(src, &["bench", "5"], "35", 0);
}

/// ESCAPE: a closure passed as an argument is OUT of stage-1 scope —
/// the pass declines, the closure survives, behavior stays exact.
#[test]
fn escaping_closure_declines_exactly() {
    let src = "## To native args () -> Seq of Text\n\
               ## To native parseInt (s: Text) -> Int\n\
               \n\
               ## To apply (f: fn(Int) -> Int) and (x: Int) -> Int:\n\
               \x20   Return f(x).\n\
               \n\
               ## Main\n\
               Let arguments be args().\n\
               Let d be parseInt(item 2 of arguments).\n\
               Let trip be (x: Int) -> x * 3.\n\
               Show apply(trip, d).\n";
    let argv: Vec<String> = ["bench", "9"].iter().map(|s| s.to_string()).collect();
    let (out, err, _) = v2_closure_census(src, &argv);
    let tw = tw_outcome_with_args(src, &argv);
    assert_eq!(
        (norm(&out), &err),
        (norm(&tw.output), &tw.error),
        "escape case diverged"
    );
    assert_eq!(err, None);
    assert_eq!(norm(&out), "27");
}

/// MUTABLE closure binding (`Set f to ...`) is out of stage-1 scope —
/// declines, exact.
#[test]
fn mutable_closure_rebinding_declines_exactly() {
    let src = "## To native args () -> Seq of Text\n\
               ## To native parseInt (s: Text) -> Int\n\
               \n\
               ## Main\n\
               Let arguments be args().\n\
               Let d be parseInt(item 2 of arguments).\n\
               Let mutable f be (x: Int) -> x + 1.\n\
               If d is greater than 5:\n\
               \x20   Set f to (x: Int) -> x * 2.\n\
               Show f(d).\n";
    let argv: Vec<String> = ["bench", "8"].iter().map(|s| s.to_string()).collect();
    let (out, err, _) = v2_closure_census(src, &argv);
    let tw = tw_outcome_with_args(src, &argv);
    assert_eq!(
        (norm(&out), &err),
        (norm(&tw.output), &tw.error),
        "mutable rebinding case diverged"
    );
    assert_eq!(err, None);
    assert_eq!(norm(&out), "16");
}

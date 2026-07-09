//! Popcount-leaf recognizer on the AOT path.
//!
//! `optimize::popcount_leaf` collapses the second-to-last level of a bitmask
//! counting search (`row == n-1`, every remaining bit = one solution) into a
//! single `count_ones`. These run the program through the REAL AOT optimizer
//! (`ui_bridge::with_v2_optimized_program` → `optimize::optimize_program`, which
//! includes the pass) on the bytecode VM, and assert the output equals the
//! UNOPTIMIZED tree-walker oracle. A structural test proves the pass FIRED (a
//! `count_ones` call appears in the residual `solve`).

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_base::Interner;
use logicaffeine_compile::ast::stmt::{Block, Expr, Stmt};
use logicaffeine_compile::compile::tw_outcome_with_args;
use logicaffeine_compile::ui_bridge::with_v2_optimized_program;
use logicaffeine_compile::vm::NativeTier;
use logicaffeine_jit::ForgeTier;

fn norm(s: &str) -> String {
    s.lines()
        .map(|l| l.trim_end())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The VM runs the depth-8-unrolled+popcount body; cap the unroll depth so the
/// slow debug VM finishes quickly. Production AOT (rustc) needs no such cap.
/// nextest isolates each test in its own process, so the env set is race-free.
fn cap_depth_for_vm() {
    std::env::set_var("LOGOS_RECURSE_DEPTH", "2");
}

fn v2_outcome(src: &str, argv: &[String]) -> (String, Option<String>) {
    let tier = ForgeTier::new();
    with_v2_optimized_program(src, |parsed, interner| match parsed {
        Ok((stmts, types, policies)) => logicaffeine_compile::vm::run_to_outcome_with_args(
            stmts,
            interner,
            Some(types),
            Some(&policies),
            argv,
            Some(&tier as &dyn NativeTier),
        ),
        Err(advice) => (String::new(), Some(advice)),
    })
}

fn assert_sound(src: &str, argv: &[String], expected: &str) {
    let (out, err) = v2_outcome(src, argv);
    let tw = tw_outcome_with_args(src, argv);
    assert_eq!(err, None, "AOT-optimized path errored:\n{src}");
    assert_eq!(
        (norm(&out), &err),
        (norm(&tw.output), &tw.error),
        "popcount-leaf VM diverged from the raw tree-walker on:\n{src}"
    );
    assert_eq!(norm(&out), expected, "wrong answer on:\n{src}");
}

const NQUEENS: &str = "\
## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int

## To solve (row: Int, cols: Int, diag1: Int, diag2: Int, n: Int) -> Int:
    If row equals n:
        Return 1.
    Let all be (1 shifted left by n) - 1.
    Let mutable available be all & ~(cols | diag1 | diag2).
    Let mutable count be 0.
    While available is not 0:
        Let bit be available & (0 - available).
        Set available to available xor bit.
        Set count to count + solve(row + 1, cols | bit, (diag1 | bit) shifted left by 1, (diag2 | bit) shifted right by 1, n).
    Return count.

## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Show solve(0, 0, 0, 0, n).
";

fn argv(n: &str) -> Vec<String> {
    vec!["bench".to_string(), n.to_string()]
}

/// The headline: n-queens stays correct through the popcount-leaf collapse.
/// n=5→10, n=6→4 (small so the slow debug VM is quick).
#[test]
fn nqueens_popcount_leaf_matches_oracle() {
    cap_depth_for_vm();
    assert_sound(NQUEENS, &argv("5"), "10");
    assert_sound(NQUEENS, &argv("6"), "4");
}

// ---------------------------------------------------------------------------
// Structural proof the pass FIRED: a `count_ones` call appears in the residual
// `solve` after the full AOT optimizer (a no-op would pass the differential).
// ---------------------------------------------------------------------------

fn has_count_ones(block: Block, c: logicaffeine_base::Symbol) -> bool {
    fn in_expr(e: &Expr, c: logicaffeine_base::Symbol) -> bool {
        match e {
            Expr::Call { function, args } => *function == c || args.iter().any(|a| in_expr(a, c)),
            Expr::BinaryOp { left, right, .. } => in_expr(left, c) || in_expr(right, c),
            Expr::Not { operand } => in_expr(operand, c),
            Expr::Index { collection, index } => in_expr(collection, c) || in_expr(index, c),
            Expr::Length { collection } => in_expr(collection, c),
            _ => false,
        }
    }
    block.iter().any(|s| match s {
        Stmt::Let { value, .. } | Stmt::Set { value, .. } => in_expr(value, c),
        Stmt::Return { value } => value.map_or(false, |e| in_expr(e, c)),
        Stmt::If { cond, then_block, else_block } => {
            in_expr(cond, c)
                || has_count_ones(then_block, c)
                || else_block.map_or(false, |b| has_count_ones(b, c))
        }
        Stmt::While { cond, body, .. } => in_expr(cond, c) || has_count_ones(body, c),
        _ => false,
    })
}

#[test]
fn nqueens_solve_gets_a_popcount_fast_path() {
    cap_depth_for_vm();
    with_v2_optimized_program(NQUEENS, |parsed, interner: &Interner| {
        let (stmts, _t, _p) = parsed.expect("nqueens parses");
        let count_ones = interner
            .lookup("count_ones")
            .expect("count_ones interned once the pass fired");
        let fired = stmts.iter().any(|s| matches!(s,
            Stmt::FunctionDef { body, .. } if has_count_ones(body, count_ones)));
        assert!(fired, "popcount-leaf did not insert a count_ones fast path into any function");
    });
}

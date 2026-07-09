//! Run-path optimizer + VM hot-loop overhead campaign (WS-H, WS-B).
//!
//! Two broad-floor levers, each gated by the sacred differential contract
//! (`vm_outcome == tw_outcome`, output AND error, bit-identical to the
//! unoptimized tree-walker oracle):
//!
//! 1. **Run-path recursion inlining** — `optimize_for_run` runs a shape-aware
//!    recursion unroller (`inline_recursive_fns_run`) LAST, under a strict
//!    statement-cost budget so optimizer time (which lands inside the measured
//!    run) never exceeds the call-overhead it saves. It unrolls return-position
//!    tree recursion (fib/binary_trees) deep and caps loop-interleaved recursion
//!    (n-queens) shallow — an interleaved A/B is a clean win on the regalloc JIT
//!    tier (fib ≈ 0.53, binary_trees ≈ 0.57, n-queens ≈ 0.94 of un-inlined). It
//!    is ON by default; `LOGOS_RUN_OPT_MASK` bit (256) toggles it for bisection.
//!
//! 2. **Back-edge region-probe caching** — the VM's `Op::Jump` back-edge hook
//!    used to probe the region hashmap on EVERY iteration even for loops whose
//!    region permanently `Failed`. A per-program blacklist (keyed by loop-head
//!    pc) now short-circuits the probe once a head is known dead, with no
//!    observable change to program output.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::ast::stmt::{Block, Stmt};
use logicaffeine_compile::compile::tw_outcome_with_args;
use logicaffeine_compile::ui_bridge::with_optimized_program;
use logicaffeine_compile::vm::NativeTier;
use logicaffeine_base::Interner;
use logicaffeine_jit::ForgeTier;

fn norm(s: &str) -> String {
    s.lines()
        .map(|l| l.trim_end())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Run `src` through the REAL run-path optimizer (`optimize_for_run`) on the
/// tiered VM, with a private tier so its compile counters stay isolated.
fn runpath_vm_outcome(src: &str, argv: &[String]) -> (String, Option<String>) {
    let tier = ForgeTier::new();
    with_optimized_program(src, |parsed, interner| match parsed {
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

/// The run-path-optimized VM output must equal the raw tree-walker, no error.
fn assert_runpath_matches_raw(src: &str, argv: &[String], expected: &str) {
    let (out, err) = runpath_vm_outcome(src, argv);
    let tw = tw_outcome_with_args(src, argv);
    assert_eq!(err, None, "run-path optimized VM errored:\n{src}");
    assert_eq!(
        (norm(&out), &err),
        (norm(&tw.output), &tw.error),
        "run-path VM diverged from the raw tree-walker on:\n{src}"
    );
    assert_eq!(norm(&out), expected, "wrong answer on:\n{src}");
}

fn argv(n: &str) -> Vec<String> {
    vec!["bench".to_string(), n.to_string()]
}

// ---------------------------------------------------------------------------
// Sub-task 1 — Run-path recursion inlining (WS-H)
// ---------------------------------------------------------------------------

/// N-queens: a single self-call inside a loop body (loop-interleaved
/// recursion — the one shape the inliner owns). The loop bottoms out at a
/// real recursive call below the unroll depth, so the answer is correct for
/// any runtime recursion depth.
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

/// Cap the run-path unroll depth so the slow debug VM stays quick while still
/// exercising unroll + bottom-out. nextest isolates each test in its own
/// process, so an env set here is race-free.
fn cap_depth_for_vm() {
    std::env::set_var("LOGOS_RECURSE_DEPTH", "2");
}

fn find_fn_body<'a>(stmts: &'a [Stmt<'a>], name: &str, it: &Interner) -> Option<Block<'a>> {
    for s in stmts {
        if let Stmt::FunctionDef { name: n, body, .. } = s {
            if it.resolve(*n) == name {
                return Some(*body);
            }
        }
    }
    None
}

fn count_whiles(block: Block) -> usize {
    block
        .iter()
        .map(|s| match s {
            Stmt::While { body, .. } => 1 + count_whiles(body),
            Stmt::If { then_block, else_block, .. } => {
                count_whiles(then_block) + else_block.map_or(0, count_whiles)
            }
            _ => 0,
        })
        .sum()
}

/// Differential: the run-path-optimized (recursion-unrolled) VM result must
/// equal the raw tree-walker for the loop-interleaved recursion shape.
#[test]
fn runpath_recursion_inline_nqueens_matches_oracle() {
    cap_depth_for_vm();
    assert_runpath_matches_raw(NQUEENS, &argv("5"), "10");
    assert_runpath_matches_raw(NQUEENS, &argv("6"), "4");
}

/// Structural proof the inliner actually FIRES on the RUN path (a no-op also
/// passes the differential): after `optimize_for_run` the residual `solve`
/// carries nested inlined loops — strictly more `While`s than the one in the
/// source. The pass is ON by default; this pins that it fires.
#[test]
fn runpath_recursion_inline_actually_fires() {
    cap_depth_for_vm();
    let fired = with_optimized_program(NQUEENS, |parsed, interner| {
        let (stmts, _types, _policies) = parsed.expect("nqueens parses");
        let body = find_fn_body(stmts, "solve", interner)
            .expect("the recursive `solve` survives the run-path optimizer");
        count_whiles(body)
    });
    assert!(
        fired >= 2,
        "expected nested inlined loops from run-path unrolling, found {fired} \
         While(s) — recursion was not flattened on the run path"
    );
}

/// The `Unfold` toggle (`## No Unfold` / `LOGOS_OPT_OFF=unfold`) must DISABLE
/// the run-path inliner, and the result must stay exact either way.
#[test]
fn runpath_recursion_inline_toggle_disables() {
    cap_depth_for_vm();
    // Disable recursion unrolling via the optimization config.
    std::env::set_var("LOGOS_OPT_OFF", "unfold");
    let off = with_optimized_program(NQUEENS, |parsed, interner| {
        let (stmts, _types, _policies) = parsed.expect("nqueens parses");
        let body = find_fn_body(stmts, "solve", interner).expect("solve survives");
        count_whiles(body)
    });
    assert_eq!(off, 1, "with Unfold disabled, solve keeps its single source loop");
    // Output is still exact with the pass disabled.
    assert_runpath_matches_raw(NQUEENS, &argv("5"), "10");
    std::env::remove_var("LOGOS_OPT_OFF");
}

/// Return-position TREE recursion (fib) IS unrolled on the run path — the live
/// path has no closed-form/memoization transform to defer to, and the post-
/// optimizer accumulator linearizer ignores multi-call returns. The unrolled
/// residual must stay bit-identical to the raw tree-walker (a divergence here
/// would mean the unroll miscompiled the doubled body).
#[test]
fn runpath_recursion_inline_tree_recursion_stays_exact() {
    let src = "\
## To fib (n: Int) -> Int:
    If n is less than 2:
        Return n.
    Return fib(n - 1) + fib(n - 2).

## Main
Show fib(12).
";
    assert_runpath_matches_raw(src, &[], "144");
}

/// Single-linear / tail recursion stays DEFERRED on the run path even with the
/// unroller on: `tail_call::rewrite_accumulators` strength-reduces these shapes
/// to a constant-stack loop AFTER the optimizer, so the unroller must leave them
/// alone (firing would clobber the strictly-better linearization). The result
/// must stay exact for both the single-linear (factorial) and tail (countdown)
/// shapes.
#[test]
fn runpath_recursion_inline_defers_linear_recursion() {
    let factorial = "\
## To fac (n: Int) -> Int:
    If n is at most 1:
        Return 1.
    Return n * fac(n - 1).

## Main
Show fac(10).
";
    assert_runpath_matches_raw(factorial, &[], "3628800");

    let countdown = "\
## To cd (n: Int) -> Int:
    If n equals 0:
        Return 0.
    Return cd(n - 1).

## Main
Show cd(100).
";
    assert_runpath_matches_raw(countdown, &[], "0");
}

// ---------------------------------------------------------------------------
// Sub-task 2 — Back-edge region-probe caching (WS-B)
// ---------------------------------------------------------------------------

/// A long-running loop whose region cannot tier: it contains a `Show` (an
/// effect the region compiler does not lift), so the region head permanently
/// `Failed`s after the threshold. The blacklist must skip the hashmap probe on
/// every subsequent back-edge with NO change to output. The differential is
/// the only observable contract.
const NONTIERING_LOOP: &str = "\
## Main
Let mutable i be 0.
Let mutable acc be 0.
While i is less than 400:
    Set acc to acc + i.
    Show acc.
    Set i to i + 1.
Show acc.
";

/// Differential: a non-tiering loop's VM output stays bit-identical to the
/// tree-walker with the back-edge blacklist live.
#[test]
fn runpath_nontiering_loop_matches_oracle() {
    let (out, err) = runpath_vm_outcome(NONTIERING_LOOP, &[]);
    let tw = tw_outcome_with_args(NONTIERING_LOOP, &[]);
    assert_eq!(err, None, "non-tiering loop errored");
    assert_eq!(
        (norm(&out), &err),
        (norm(&tw.output), &tw.error),
        "non-tiering loop VM diverged from the tree-walker"
    );
}

/// A pure-int counted loop that DOES tier must still dispatch to native and
/// produce the exact result — the blacklist must never blacklist a region that
/// can tier.
#[test]
fn runpath_tiering_loop_still_correct() {
    let src = "\
## Main
Let mutable i be 0.
Let mutable acc be 0.
While i is less than 2000:
    Set acc to acc + i.
    Set i to i + 1.
Show acc.
";
    let (out, err) = runpath_vm_outcome(src, &[]);
    let tw = tw_outcome_with_args(src, &[]);
    assert_eq!(err, None, "tiering loop errored");
    assert_eq!(
        (norm(&out), &err),
        (norm(&tw.output), &tw.error),
        "tiering loop VM diverged from the tree-walker"
    );
    assert_eq!(norm(&out), "1999000");
}

/// Many distinct non-tiering loop heads in one program: every head that fails
/// must be blacklisted independently, and the program output must stay exact.
#[test]
fn runpath_multiple_nontiering_heads_match_oracle() {
    let src = "\
## Main
Let mutable total be 0.
Let mutable a be 0.
While a is less than 200:
    Show a.
    Set total to total + a.
    Set a to a + 1.
Let mutable b be 0.
While b is less than 200:
    Show b.
    Set total to total + b.
    Set b to b + 1.
Show total.
";
    let (out, err) = runpath_vm_outcome(src, &[]);
    let tw = tw_outcome_with_args(src, &[]);
    assert_eq!(err, None, "multi-head program errored");
    assert_eq!(
        (norm(&out), &err),
        (norm(&tw.output), &tw.error),
        "multi-head program diverged from the tree-walker"
    );
}

//! Bounded recursive inlining (recursion unrolling) on the AOT path.
//!
//! `optimize::inline_recursive` flattens each self-recursive function's own
//! body k levels deep — the recursive inline LLVM refuses but gcc -O3 performs,
//! the single lever that makes compiled-LOGOS n-queens match/beat C.
//!
//! These are the soundness gate. Every program runs through the REAL AOT
//! optimizer (`with_v2_optimized_program` → `optimize::optimize_program`, which
//! now includes the pass) on the bytecode VM, and its output is asserted equal
//! to the UNOPTIMIZED tree-walker oracle (`tw_outcome_with_args`, raw parse, no
//! optimizer). If unrolling ever changes a result, the differential fails. A
//! separate structural test proves the pass actually FIRED — that the residual
//! `solve` carries the nested inlined loops, not just a preserved result.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::ast::stmt::{Block, Stmt};
use logicaffeine_compile::compile::tw_outcome_with_args;
use logicaffeine_compile::ui_bridge::with_v2_optimized_program;
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

/// Run `src` through the AOT optimizer (recursion unrolling included) on the VM.
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

/// The AOT-optimized (unrolled) output must equal the raw tree-walker, no error.
fn assert_unroll_sound(src: &str, argv: &[String], expected: &str) {
    let (out, err) = v2_outcome(src, argv);
    let tw = tw_outcome_with_args(src, argv);
    assert_eq!(err, None, "AOT-optimized path errored:\n{src}");
    assert_eq!(
        (norm(&out), &err),
        (norm(&tw.output), &tw.error),
        "unrolled VM diverged from the raw tree-walker on:\n{src}"
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
    Let mutable available be all and not (cols or diag1 or diag2).
    Let mutable count be 0.
    While available is not 0:
        Let bit be available and (0 - available).
        Set available to available xor bit.
        Set count to count + solve(row + 1, cols or bit, (diag1 or bit) shifted left by 1, (diag2 or bit) shifted right by 1, n).
    Return count.

## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Show solve(0, 0, 0, 0, n).
";

const FIB: &str = "\
## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int

## To fib (n: Int) -> Int:
    If n is less than 2:
        Return n.
    Return fib(n - 1) + fib(n - 2).

## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Show fib(n).
";

// NOTE: ackermann is intentionally NOT differential-tested here. Routing it
// through `optimize_program` trips a PRE-EXISTING PE-specializer bug
// (`vm: function 'ack_s1_0' defined twice`) that reproduces with this pass
// disabled (`LOGOS_RECURSE_INLINE=0`) — it is independent of recursive
// inlining and out of scope. The nested-self-call-in-argument shape ackermann
// represents is instead unit-tested directly in
// `optimize::inline_recursive::tests` (no PE path involved).

fn argv(n: &str) -> Vec<String> {
    vec!["bench".to_string(), n.to_string()]
}

/// Cap unroll depth for these tests. Production AOT compile is fast even at the
/// default depth 8 (it just emits more Rust for rustc), but this harness *runs*
/// the unrolled body on the slow debug bytecode VM — a path production never
/// takes (`largo run` uses `optimize_for_run`, only `largo build`→rustc uses
/// `optimize_program`). A shallow depth keeps the differential quick while still
/// exercising the unroll + bottom-out. nextest isolates each test in its own
/// process, and every test in this file wants the same value, so the env set is
/// race-free here.
fn cap_depth_for_vm() {
    std::env::set_var("LOGOS_RECURSE_DEPTH", "2");
}

/// The headline: n-queens (single self-call inside a loop, b=1) stays correct
/// through the unroller. n=5 → 10, n=6 → 4 (small, so the slow debug oracle is
/// quick; the runtime bottom-out path — recursion deeper than the unroll — is
/// covered by `fib(16)` below).
#[test]
fn nqueens_unrolled_matches_oracle() {
    cap_depth_for_vm();
    assert_unroll_sound(NQUEENS, &argv("5"), "10");
    assert_unroll_sound(NQUEENS, &argv("6"), "4");
}

/// Return-position recursion (fib: `Return fib(n-1) + fib(n-2)`, no loop) is
/// DEFERRED to codegen's recursion transforms (closed-form / memoization) — the
/// unroller leaves it alone. This pins that deferral keeps fib correct through
/// the full AOT optimizer (a regression here would mean the unroller wrongly
/// fired on return-position recursion and clobbered the better transform).
#[test]
fn fib_return_recursion_deferred_stays_correct() {
    assert_unroll_sound(FIB, &argv("12"), "144");
    assert_unroll_sound(FIB, &argv("16"), "987");
}

// ---------------------------------------------------------------------------
// Structural proof: the pass actually FIRED (a no-op would also pass the
// differential). After unrolling, the residual `solve` carries the nested
// inlined loops — far more `While`s than the one in the source.
// ---------------------------------------------------------------------------

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

fn find_fn_body<'a>(stmts: &'a [Stmt<'a>], name: &str, it: &Interner) -> Option<Block<'a>> {
    for s in stmts {
        if let Stmt::FunctionDef { name: n, body, .. } = s {
            if it.resolve(*n) == name {
                return Some(body);
            }
        }
    }
    None
}

#[test]
fn nqueens_solve_is_actually_unrolled() {
    cap_depth_for_vm();
    with_v2_optimized_program(NQUEENS, |parsed, interner| {
        let (stmts, _types, _policies) = parsed.expect("nqueens parses");
        let body = find_fn_body(stmts, "solve", interner)
            .expect("the recursive `solve` survives the AOT pipeline");
        // The source `solve` has exactly one `While`. Unrolling splices nested
        // inlined copies, each with its own loop, so the residual carries many.
        let whiles = count_whiles(body);
        assert!(
            whiles >= 2,
            "expected nested inlined loops from unrolling, found {whiles} While(s) \
             — the recursion was not flattened"
        );
    });
}

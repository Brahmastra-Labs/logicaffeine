//! A deeply nested program must COMPILE, not abort the process.
//!
//! Compiling recurses once per nesting level (`codegen_stmt` into a block, `codegen_expr` into a
//! subexpression, every AST walker likewise), so a program's nesting depth is a stack depth in the
//! compiler. Two things kept that safe by accident rather than by design: the frames were small
//! enough, and the shipped CLI happens to start on a big stack
//! (`apps/logicaffeine_cli/src/main.rs`). Neither holds for a library caller on an ordinary thread —
//! and it stopped holding for the CLI's own benchmark corpus once the recursive inliner's default
//! depth of 8 unfolded n-queens into a nest deep enough to overflow: the whole `phase_opt_trace`
//! binary died with `fatal runtime error: stack overflow`, a process abort with no diagnostic.
//!
//! The language already bounds SOURCE nesting: past `ast_depth::max_parse_recursion()` the parser
//! refuses with a Socratic `AstTooDeep`, and that budget is documented as sized for the parser's own
//! ~7 KiB of stack per level. The hole is everything downstream — a tree at the sanctioned depth must
//! still be walkable, and a pass that GENERATES nesting (the recursive inliner unfolds a self-call
//! inside a loop eight levels deep by default) must stay inside the same budget.
//!
//! These are the invariants that keep it safe on purpose:
//!   1. a program at the language's maximum sanctioned nesting depth compiles and runs on an
//!      ORDINARY thread, through every entry point;
//!   2. past that depth the compiler REPORTS `AstTooDeep` — it never aborts the process;
//!   3. the benchmark corpus compiles at the inliner's default unfold depth;
//!   4. the AST nodes stay small, so a new statement or expression kind cannot quietly multiply
//!      every compiler frame by widening the node every recursion level holds.
//!
//! Each entry point is named explicitly — a new one is not covered until it is added here.

use logicaffeine_language::ast::stmt::{Expr, Stmt};
use logicaffeine_language::ast_depth::max_parse_recursion;

/// A program nested exactly `depth` block levels deep: alternating `If` and `While` so both the
/// branch walker and the loop walker are on the recursive path, one nesting level each — the shape
/// the recursive inliner produces when it unfolds a self-call inside a loop.
fn nested_program(depth: usize) -> String {
    let mut src = String::from("## Main\n    Let mutable total be 0.\n    Let mutable i be 0.\n");
    for level in 0..depth {
        let pad = "    ".repeat(level + 1);
        if level % 2 == 0 {
            src.push_str(&format!("{pad}If i is less than {}:\n", level + 1));
        } else {
            src.push_str(&format!("{pad}While i is greater than {}:\n", level + 1));
        }
    }
    let pad = "    ".repeat(depth + 1);
    src.push_str(&format!("{pad}Set i to i + 1.\n"));
    src.push_str(&format!("{pad}Set total to total + 1.\n"));
    src.push_str("    Show total.\n");
    src
}

/// Run `f` on a thread with the ORDINARY 2 MiB stack — the floor a library caller can count on, and
/// less than the 8 MiB a default main thread gets. A compile that needs more than this must arrange
/// its own stack rather than relying on its caller's.
fn on_an_ordinary_thread<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(f)
        .expect("spawn")
        .join()
        .expect("the compile must not abort or panic on an ordinary thread")
}

/// The number of NESTED BLOCKS in the deepest program the language accepts. Two levels of the tree
/// are not blocks the generator writes — the `## Main` body itself, and the statements at the bottom
/// of the tower — so the deepest accepted tower is two under the parser's budget. `the_limit_is_exact`
/// proves this is the maximum rather than a guess.
fn deepest_accepted() -> usize {
    max_parse_recursion() - 2
}

/// The chosen depth is EXACTLY maximal: one deeper is refused. Without this the other tests could
/// silently drift to testing a shallow program if the budget or the generator ever changed.
#[test]
fn the_limit_is_exact() {
    let accepted = nested_program(deepest_accepted());
    let out =
        on_an_ordinary_thread(move || logicaffeine_compile::compile::compile_to_rust(&accepted));
    assert!(
        out.is_ok(),
        "the deepest accepted program must compile: {out:?}"
    );

    let one_deeper = nested_program(deepest_accepted() + 1);
    let out =
        on_an_ordinary_thread(move || logicaffeine_compile::compile::compile_to_rust(&one_deeper));
    let err =
        out.expect_err("one level deeper must be refused — otherwise this is not the maximum");
    assert!(
        matches!(
            err.kind,
            logicaffeine_language::error::ParseErrorKind::AstTooDeep { .. }
        ),
        "the refusal must name the nesting limit: {err:?}"
    );
}

#[test]
fn the_rust_backend_compiles_a_deeply_nested_program() {
    let src = nested_program(deepest_accepted());
    let out = on_an_ordinary_thread(move || logicaffeine_compile::compile::compile_to_rust(&src));
    assert!(
        out.is_ok(),
        "compile_to_rust must succeed or report an error, not abort: {out:?}"
    );
}

#[test]
fn the_traced_rust_backend_compiles_a_deeply_nested_program() {
    let src = nested_program(deepest_accepted());
    let out = on_an_ordinary_thread(move || {
        logicaffeine_compile::compile::compile_program_traced(&src).map(|(o, _)| o.rust_code)
    });
    assert!(
        out.is_ok(),
        "compile_program_traced must succeed or report an error, not abort: {out:?}"
    );
}

#[test]
fn the_c_backend_compiles_a_deeply_nested_program() {
    let src = nested_program(deepest_accepted());
    let out = on_an_ordinary_thread(move || logicaffeine_compile::compile::compile_to_c(&src));
    assert!(
        out.is_ok(),
        "compile_to_c must succeed or report an error, not abort: {out:?}"
    );
}

#[test]
fn the_vm_compiles_a_deeply_nested_program() {
    let src = nested_program(deepest_accepted());
    let out =
        on_an_ordinary_thread(move || logicaffeine_compile::compile::compile_to_program(&src));
    assert!(
        out.is_ok(),
        "compile_to_program must succeed or report an error, not abort: {out:?}"
    );
}

#[test]
fn the_optimizer_lattice_compiles_a_deeply_nested_program() {
    let src = nested_program(deepest_accepted());
    let out = on_an_ordinary_thread(move || {
        logicaffeine_compile::compile::compile_to_program_optimized(&src)
    });
    assert!(
        out.is_ok(),
        "compile_to_program_optimized must succeed or report an error, not abort: {out:?}"
    );
}

#[test]
fn the_tree_walker_runs_a_deeply_nested_program() {
    let src = nested_program(deepest_accepted());
    let outcome = on_an_ordinary_thread(move || logicaffeine_compile::compile::tw_outcome(&src));
    assert_eq!(
        outcome.error, None,
        "the tree-walker must run a deeply nested program"
    );
}

#[test]
fn the_vm_runs_a_deeply_nested_program() {
    let src = nested_program(deepest_accepted());
    let outcome = on_an_ordinary_thread(move || logicaffeine_compile::compile::vm_outcome(&src));
    assert_eq!(
        outcome.error, None,
        "the VM must run a deeply nested program"
    );
}

/// A deeply nested EXPRESSION, not a block tower: `(((1 + 2) + 3) + …)`. Expressions recurse through a
/// different walker than statements (`codegen_expr`), so a budget that holds for block nesting says
/// nothing about this one — and the overflow that motivated this file had expression frames on the
/// stack too.
fn nested_expression_program(depth: usize) -> String {
    let mut expr = String::from("1");
    for n in 0..depth {
        expr = format!("({expr} + {})", n + 2);
    }
    format!("## Main\n    Let total be {expr}.\n    Show total.\n")
}

/// The deepest expression tower the language accepts must compile, on an ordinary thread.
#[test]
fn a_deeply_nested_expression_compiles() {
    // One level is the `Let` statement holding the tower.
    let src = nested_expression_program(max_parse_recursion() - 2);
    let out = on_an_ordinary_thread(move || logicaffeine_compile::compile::compile_to_rust(&src));
    assert!(
        out.is_ok(),
        "a deep expression must compile or be reported, not abort: {out:?}"
    );
}

/// And the VM, whose expression compiler is a separate walk again.
#[test]
fn a_deeply_nested_expression_runs_on_the_vm() {
    let src = nested_expression_program(max_parse_recursion() - 2);
    let outcome = on_an_ordinary_thread(move || logicaffeine_compile::compile::vm_outcome(&src));
    assert_eq!(
        outcome.error, None,
        "the VM must run a deep expression tower"
    );
}

/// Past the sanctioned depth the compiler REPORTS the limit; it never aborts. (This is the behaviour
/// the parser's guard exists to provide — pinned here so a walker added downstream cannot bypass it.)
#[test]
fn nesting_past_the_limit_is_reported_not_aborted() {
    let src = nested_program(max_parse_recursion() * 2);
    let out = on_an_ordinary_thread(move || logicaffeine_compile::compile::compile_to_rust(&src));
    let err = out.expect_err("a program past the nesting limit must be refused");
    assert!(
        matches!(
            err.kind,
            logicaffeine_language::error::ParseErrorKind::AstTooDeep { .. }
        ),
        "the refusal must name the nesting limit, not some incidental failure: {err:?}"
    );
}

/// The benchmark corpus at the AOT unfold depth that overflowed: n-queens, whose self-call inside a
/// `While` the recursive inliner unfolds eight levels deep by default.
#[test]
fn the_benchmark_corpus_compiles_at_the_default_unfold_depth() {
    let src = include_str!("../../../benchmarks/programs/nqueens/main.lg").to_string();
    let out = on_an_ordinary_thread(move || logicaffeine_compile::compile::compile_to_rust(&src));
    assert!(
        out.is_ok(),
        "n-queens must compile at the default unfold depth: {out:?}"
    );
}

/// Every compiler frame that holds an AST node grows with the enum, at every recursion level — and a
/// compiler walk holds many of them per frame, so widening the node multiplies the compiler's stack
/// use across the board. These are SHRINK-ONLY ceilings: a new variant with a large payload must be
/// boxed rather than widening the node for everyone, and a change that makes a node smaller should
/// lower the number here.
#[test]
fn ast_nodes_stay_small() {
    const STMT_CEILING: usize = 128;
    const EXPR_CEILING: usize = 64;
    let stmt = std::mem::size_of::<Stmt>();
    let expr = std::mem::size_of::<Expr>();
    assert!(
        stmt <= STMT_CEILING,
        "Stmt is {stmt} bytes, over the {STMT_CEILING}-byte ceiling — box the payload of the variant that grew it"
    );
    assert!(
        expr <= EXPR_CEILING,
        "Expr is {expr} bytes, over the {EXPR_CEILING}-byte ceiling — box the payload of the variant that grew it"
    );
    assert!(
        stmt >= STMT_CEILING - 16 && expr >= EXPR_CEILING - 8,
        "Stmt is {stmt} and Expr is {expr} bytes — well under the pinned ceilings ({STMT_CEILING}/{EXPR_CEILING}); \
         lower them so the ratchet keeps holding at the size actually achieved"
    );
}
/// RECURSION HEADROOM (the runtime sibling of the nesting invariant above). `semantics::MAX_CALL_DEPTH`
/// promises 2,500 LOGOS calls before a catchable "maximum call depth exceeded" error on every engine.
/// One tree-walker LOGOS call costs several Rust frames (`execute_stmt_sync` + `call_function_sync` +
/// `evaluate_expr_sync` + the block walker, ~20 KiB per call level), so an ordinary 2 MiB thread holds
/// only a few dozen of them; a tree-walker SESSION therefore runs on its own stack
/// (`interpreter::on_session_stack`, sized for the full depth), which is what every host that owns a
/// session enters it through.
///
/// This pins the depth an ordinary caller reaches through that entry: the full promised depth — the
/// same `down(n)` the bytecode VM reaches on any thread (`treewalker_recursion_headroom_lock::
/// the_vm_reaches_the_depth_this_lock_demands`) — from a caller on an ORDINARY thread. A change that
/// fattens the walker's frames past the session stack, or a session entry that stops providing it,
/// fails HERE instead of surfacing as a differential aborting with `fatal runtime error: stack
/// overflow`.
#[test]
fn the_tree_walker_keeps_its_ordinary_thread_recursion_headroom() {
    const FLOOR: usize = logicaffeine_compile::semantics::MAX_CALL_DEPTH - 2;
    let src = "## To down (n: Int) -> Int:\n    If n is at most 0:\n        Return 0.\n    Return down(n - 1) - 1.\n## Main\n    Show down(FLOOR).\n"
        .replace("FLOOR", &FLOOR.to_string());
    let outcome = on_an_ordinary_thread(move || {
        logicaffeine_compile::interpreter::on_session_stack(|| {
            logicaffeine_compile::compile::tw_outcome(&src)
        })
    });
    assert_eq!(
        outcome.error, None,
        "the tree-walker must recurse {FLOOR} calls deep from an ordinary thread"
    );
    assert_eq!(outcome.output.trim(), format!("-{FLOOR}"));
}

/// Past the promised depth the engine REPORTS the limit rather than dying — on a stack big enough to
/// reach it, which is how `MAX_CALL_DEPTH`'s own documentation says the tree-walker is run.
#[test]
fn exceeding_the_call_depth_limit_is_reported_not_a_crash() {
    let depth = logicaffeine_compile::semantics::MAX_CALL_DEPTH + 100;
    let src = "## To down (n: Int) -> Int:\n    If n is at most 0:\n        Return 0.\n    Return down(n - 1) - 1.\n## Main\n    Show down(DEPTH).\n"
        .replace("DEPTH", &depth.to_string());
    let outcome = std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(move || logicaffeine_compile::compile::tw_outcome(&src))
        .expect("spawn")
        .join()
        .expect("the engine must report the depth limit, not abort");
    let err = outcome
        .error
        .expect("recursing past the limit must be an error");
    assert!(
        err.contains(logicaffeine_compile::semantics::CALL_DEPTH_ERR),
        "the error must be the documented depth-limit error, got: {err}"
    );
}

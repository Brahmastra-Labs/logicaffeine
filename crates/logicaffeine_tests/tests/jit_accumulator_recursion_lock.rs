//! ★ LOCK — a function that returns an arithmetic of its own call runs native. ★
//!
//! `Return n % k + digits(k, n / k)` is not a tail call, but its sum is: the run path rewrites the function into a loop
//! that adds into an accumulator (`tail_call::build_accumulator_loop`, a `While true`). That loop leaves only by its
//! returns, so `While true` compiles with no test and the code after it is unreachable: the JIT compiles the function,
//! where a reachable `ReturnNothing` (native code cannot return Nothing) kept it on bytecode.
//!
//! - the bytecode: `While true` tests nothing, and a `Break` still leaves it;
//! - the run path: functions of the accumulator shape print what the tree-walker prints and compile natively, and the
//!   digit sum is specialized on its divisor.

#![cfg(all(target_arch = "x86_64", not(target_os = "windows")))]

use logicaffeine_compile::compile::{tw_outcome_with_args, vm_outcome_with_args};
use logicaffeine_compile::optimize::oracle_analyze_with;
use logicaffeine_compile::ui_bridge::with_parsed_program;
use logicaffeine_compile::vm::{CompiledProgram, Compiler, Constant, Op};
use logicaffeine_jit::{optimization_trace_jit, JitBackendOptimizationTrace, JitOptimization};

const NATIVES: &str = "## To native args () -> Seq of Text\n## To native parseInt (s: Text) -> Int\n\n";

fn compile_program(src: &str) -> CompiledProgram {
    with_parsed_program(src, |parsed, interner| {
        let (stmts, types, _) = parsed.expect("parse");
        let oracle = oracle_analyze_with(stmts, interner);
        Compiler::compile_with_oracle(stmts, interner, Some(types), Some(oracle)).expect("compile")
    })
}

/// Every conditional jump on a register the op before it loaded with the constant `true`.
fn tests_of_true(program: &CompiledProgram) -> Vec<usize> {
    program
        .code
        .windows(2)
        .enumerate()
        .filter_map(|(pc, pair)| match (pair[0], pair[1]) {
            (Op::LoadConst { dst, idx }, Op::JumpIfFalse { cond, .. } | Op::JumpIfTrue { cond, .. })
                if dst == cond && matches!(program.constants.get(idx as usize), Some(Constant::Bool(true))) =>
            {
                Some(pc + 1)
            }
            _ => None,
        })
        .collect()
}

const COUNTS_UNTIL_BREAK: &str = "\
## Main
Let mutable i be 0.
Let mutable total be 0.
While true:
    Set total to total + i.
    Set i to i + 1.
    If i is greater than 100:
        Break.
Show total.
Show i.
";

const NESTED_UNTIL_BREAK: &str = "\
## Main
Let mutable rows be 0.
Let mutable cells be 0.
While true:
    Set rows to rows + 1.
    Let mutable j be 0.
    While true:
        Set j to j + 1.
        Set cells to cells + j.
        If j equals rows:
            Break.
    If rows equals 12:
        Break.
Show rows.
Show cells.
";

#[test]
fn while_true_tests_nothing_and_break_still_leaves_it() {
    for src in [COUNTS_UNTIL_BREAK, NESTED_UNTIL_BREAK] {
        let program = compile_program(src);
        assert_eq!(tests_of_true(&program), Vec::<usize>::new(), "a test of the literal true:\n{src}");
        let args = ["logos".to_string()];
        let vm = vm_outcome_with_args(src, &args, None);
        let tree = tw_outcome_with_args(src, &args);
        assert_eq!(vm.error, tree.error, "{src}");
        assert_eq!(vm.output.trim(), tree.output.trim(), "{src}");
    }
    let shown = vm_outcome_with_args(COUNTS_UNTIL_BREAK, &["logos".to_string()], None);
    assert_eq!(shown.output.trim(), "5050\n101");
}

fn run(functions: &str, body: &str, arg: &str) -> JitBackendOptimizationTrace {
    let source = format!("{NATIVES}{functions}\n## Main\nLet k be parseInt(item 2 of args()).\n{body}");
    let args = ["logos".to_string(), arg.to_string()];
    let traced = optimization_trace_jit(&source, &args);
    let tree = tw_outcome_with_args(&source, &args);
    assert_eq!(traced.outcome.error, tree.error, "the JIT and the tree-walker disagree on the error:\n{source}");
    assert_eq!(
        traced.outcome.output.trim(),
        tree.output.trim(),
        "the JIT and the tree-walker disagree on the output for argument {arg}:\n{source}"
    );
    traced
}

const DIGITS: &str = "\
## To digits (k: Int, n: Int) -> Int:
    If n is less than k:
        Return n.
    Return n % k + digits(k, n / k).
";

const CALLS_DIGITS: &str = "\
Let mutable acc be 0.
Let mutable i be 0.
While i is less than 10000:
    Set acc to acc + digits(k, i * 104729).
    Set i to i + 1.
Show acc.
";

/// A product accumulator: past twenty the factorial leaves the word, the native multiply side-exits and the VM goes
/// on exactly.
const FACTORIAL: &str = "\
## To fact (n: Int) -> Int:
    If n is less than 2:
        Return 1.
    Return n * fact(n - 1).
";

const CALLS_FACTORIAL: &str = "\
Let mutable acc be 0.
Let mutable i be 0.
While i is less than 3000:
    Set acc to acc + fact(i % k).
    Set i to i + 1.
Show acc.
";

#[test]
fn a_digit_sum_returning_the_remainder_plus_its_own_call_compiles_and_is_specialized() {
    for arg in ["2", "10", "97"] {
        let traced = run(DIGITS, CALLS_DIGITS, arg);
        assert!(traced.function_successes >= 1, "`digits` compiles: {traced:?}");
        assert_eq!(traced.native.count(JitOptimization::ValueSpecializedFunction), 1, "k = {arg}: {traced:?}");
    }
}

#[test]
fn a_factorial_returning_the_product_with_its_own_call_compiles_and_stays_exact() {
    for arg in ["13", "21", "40"] {
        let traced = run(FACTORIAL, CALLS_FACTORIAL, arg);
        assert!(traced.function_successes >= 1, "`fact` compiles: {traced:?}");
    }
}

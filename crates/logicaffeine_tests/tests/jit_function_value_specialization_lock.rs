//! ★ LOCK — a hot function is specialized on the argument its callers never change. ★
//!
//! A function called again and again with one value for a parameter computes, for the whole run, the function of the
//! rest. The VM records what each `Int` or `Bool` parameter was passed over the function's warm-up; one that held a
//! single value, and that the function never writes, is folded into the function's native code as that constant (its
//! divisions become reciprocals, the branches it decides are taken), and the code is versioned at its own entry: the
//! specialized body when the argument is that value, the generic body otherwise. The check is in the machine code, so a
//! call from native code (a hot loop calling the function through the dispatch table, or the function calling itself)
//! is held to it as a call from the VM is.
//!
//! - the run path: programs whose invariant argument arrives on the command line print exactly what the tree-walker
//!   prints, and the trace names `value_specialized_function` when a function was specialized, and does not when the
//!   argument changes during the warm-up or nothing folds on it;
//! - the check: an argument that changes after the warm-up, from a native caller or from the function's own recursion,
//!   takes the generic body, and the output is unchanged;
//! - the bound: a function is compiled, and specialized, once;
//! - the report: the trace, and `largo opts --jit`, name each specialized function with what it was specialized on.
//!
//! Each function that loops opens with a guard clause: the run path inlines a pure leaf whose only `Return` is its last
//! statement (`optimize::inline_leaf`), and an inlined function is a loop of its caller, specialized as a region.

#![cfg(all(target_arch = "x86_64", not(target_os = "windows")))]

#[path = "cli_common/mod.rs"]
mod common;

use common::{largo_in, stderr, stdout};
use logicaffeine_compile::compile::tw_outcome_with_args;
use logicaffeine_compile::vm::ScalarValue;
use logicaffeine_jit::{optimization_trace_jit, JitBackendOptimizationTrace, JitOptimization};

const NATIVES: &str = "## To native args () -> Seq of Text\n## To native parseInt (s: Text) -> Int\n\n";

fn program(functions: &str, body: &str) -> String {
    format!("{NATIVES}{functions}\n## Main\nLet k be parseInt(item 2 of args()).\n{body}")
}

fn run(functions: &str, body: &str, arg: &str) -> JitBackendOptimizationTrace {
    let source = program(functions, body);
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

fn specializations(traced: &JitBackendOptimizationTrace) -> u32 {
    traced.native.count(JitOptimization::ValueSpecializedFunction)
}

/// How many calls side-exited precisely: native code that stopped where it was and handed its frame to the VM.
fn precise_deopts(traced: &JitBackendOptimizationTrace) -> u64 {
    traced
        .execution
        .split("precise_deopts=")
        .nth(1)
        .and_then(|rest| rest.split(|c: char| !c.is_ascii_digit()).next())
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("the execution summary counts precise side exits: {}", traced.execution))
}

/// What the one specialized function was specialized on.
fn specialized_on(traced: &JitBackendOptimizationTrace) -> Vec<(u16, ScalarValue)> {
    let [spec] = traced.function_specializations.as_slice() else {
        panic!("one function was specialized: {traced:?}");
    };
    spec.guards.clone()
}

/// The digit sum of `x` in base `k`: `k` reaches a `%` and a `/` in its loop.
const REM: &str = "\
## To rem (k: Int, x: Int) -> Int:
    If x equals 0:
        Return 0.
    Let mutable n be x.
    If n is less than 0:
        Set n to 0 - n.
    Let mutable s be 0.
    While n is greater than 0:
        Set s to s + n % k.
        Set n to n / k.
    Return s.
";

const CALLS_REM: &str = "\
Let mutable acc be 0.
Let mutable i be 0 - 5000.
While i is less than 5000:
    Set acc to acc + rem(k, i * 7919).
    Set i to i + 1.
Show acc.
";

/// Fewer calls, with dividends across the whole range of a word: a warm-up and some calls past it, for many divisors.
const CALLS_REM_WIDE: &str = "\
Let mutable acc be 0.
Let mutable i be 0 - 150.
While i is less than 150:
    Set acc to acc + rem(k, i * 30750992413431527 + i).
    Set i to i + 1.
Show acc.
";

/// A loop whose body branches on a flag the caller never changes.
const PICK: &str = "\
## To pick (flag: Bool, x: Int) -> Int:
    If x is less than 0:
        Return 0.
    Let mutable t be 0.
    Let mutable j be 0.
    While j is less than 8:
        If flag:
            Set t to t + x * 3 + j.
        Otherwise:
            Set t to t + x - j.
        Set j to j + 1.
    Return t.
";

const CALLS_PICK: &str = "\
Let on be k is greater than 0.
Let mutable acc be 0.
Let mutable i be 0.
While i is less than 10000:
    Set acc to acc + pick(on, i).
    Set i to i + 1.
Show acc.
";

/// The argument is the same for the warm-up and then changes for good: the check at the function's own entry sends
/// every later call, from the native loop too, to the generic body.
const CALLS_REM_THEN_CHANGES: &str = "\
Let mutable acc be 0.
Let mutable i be 0.
While i is less than 20000:
    Let d be k + i / 5000.
    Set acc to acc + rem(d, i * 7919 - 3).
    Set i to i + 1.
Show acc.
";

/// The argument changes on every call: nothing to specialize on.
const CALLS_REM_CHANGING: &str = "\
Let mutable acc be 0.
Let mutable i be 1.
While i is less than 10000:
    Set acc to acc + rem(i % 97 + k, i * 7919).
    Set i to i + 1.
Show acc.
";

/// The digit sum again, by recursion: every call the function makes passes `k` on.
const DIGITS: &str = "\
## To digits (k: Int, n: Int) -> Int:
    If n is less than k:
        Return n.
    Let r be n % k.
    Let rest be digits(k, n / k).
    Return r + rest.
";

/// The leading digit of `n` in base `k`, by a tail call the lowering turns into a loop that rewrites both parameters,
/// `k` with itself.
const LEADING: &str = "\
## To leading (k: Int, n: Int) -> Int:
    If n is less than k:
        Return n.
    Return leading(k, n / k).
";

const CALLS_LEADING: &str = "\
Let mutable acc be 0.
Let mutable i be 0.
While i is less than 10000:
    Set acc to acc + leading(k, i * 104729).
    Set i to i + 1.
Show acc.
";

const CALLS_DIGITS: &str = "\
Let mutable acc be 0.
Let mutable i be 0.
While i is less than 10000:
    Set acc to acc + digits(k, i * 104729).
    Set i to i + 1.
Show acc.
";

/// Called with `k` and `n = 0` for the warm-up, then with depth: the function then calls itself with `k + 1`, `k + 2`,
/// …, which its own entry check sends to the generic body.
const SPIN: &str = "\
## To spin (k: Int, n: Int) -> Int:
    If n is less than 1:
        Return 1000003 % k.
    Return (spin(k + 1, n - 1) + n * 7) % k.
";

const CALLS_SPIN: &str = "\
Let mutable acc be 0.
Let mutable i be 0.
While i is less than 4000:
    Set acc to acc + spin(k, i / 1000 * 4) + i.
    Set i to i + 1.
Show acc.
";

/// Two parameters held, both folded on: a mismatch of either takes the generic body.
const MIX: &str = "\
## To mix (flag: Bool, k: Int, x: Int) -> Int:
    If flag:
        Return x % k + x / k.
    Return x / k - x % k.
";

const CALLS_MIX: &str = "\
Let on be k is greater than 0.
Let mutable acc be 0.
Let mutable i be 0.
While i is less than 12000:
    Let d be k + i / 4000.
    Let f be on and i is less than 8000.
    Set acc to acc + mix(f, d, i * 7919 - 50000000).
    Set i to i + 1.
Show acc.
";

/// A function that never reads its held parameter in a branch or a division: nothing folds, so nothing is specialized.
const SCALE: &str = "\
## To scale (k: Int, x: Int) -> Int:
    If x is less than 0:
        Return 0.
    Let mutable t be 0.
    Let mutable j be 0.
    While j is less than 4:
        Set t to t + x * k + j.
        Set j to j + 1.
    Return t.
";

const CALLS_SCALE: &str = "\
Let mutable acc be 0.
Let mutable i be 0.
While i is less than 5000:
    Set acc to acc + scale(k, i).
    Set i to i + 1.
Show acc.
";

/// A list parameter makes the function precise: a side exit hands its frame back to the VM where it stopped. Two
/// elements pushed past the warm-up overflow the sum in the specialized copy, which side-exits there, and the VM
/// finishes the call exactly.
const BUCKET: &str = "\
## To bucket (k: Int, xs: Seq of Int) -> Int:
    If length of xs equals 0:
        Return 0.
    Let mutable s be 0.
    Let mutable j be 1.
    While j is at most length of xs:
        Set s to s + item j of xs % k + item j of xs / k.
        Set j to j + 1.
    Return s.
";

const CALLS_BUCKET: &str = "\
Let mutable xs be [0].
Let mutable j be 0.
While j is less than 64:
    Push j * 7919 - 100000 to xs.
    Set j to j + 1.
Let mutable acc be 0.
Let mutable i be 0.
While i is less than 3000:
    If i equals 2000:
        Push 9223372036854775000 to xs.
        Push 9223372036854775000 to xs.
    Set acc to acc + bucket(k, xs).
    Set i to i + 1.
Show acc.
";

/// The held parameter is reassigned in the body (and handed back to the caller, whose copy is reset before each call):
/// every call passes the same value, but the call does not keep it.
const SHIFTS: &str = "\
## To shifts (k: mutable Int, x: Int) -> Int:
    Let mutable s be 0.
    Let mutable n be x.
    While n is greater than 0:
        Set s to s + n % k.
        Set n to n / k.
        Set k to k + 1.
    Return s.
";

const CALLS_SHIFTS: &str = "\
Let mutable acc be 0.
Let mutable i be 0.
While i is less than 5000:
    Let mutable d be k.
    Set acc to acc + shifts(d, i * 7919) + d.
    Set i to i + 1.
Show acc.
";

#[test]
fn a_function_called_with_an_invariant_divisor_is_specialized_and_exact() {
    for arg in ["2", "3", "7", "10", "641", "1000000007", "9223372036854775807"] {
        let traced = run(REM, CALLS_REM, arg);
        assert_eq!(specializations(&traced), 1, "`rem` with k = {arg} was not specialized once: {traced:?}");
        assert_eq!(specialized_on(&traced), vec![(0, ScalarValue::Int(arg.parse().unwrap()))], "k = {arg}");
    }
}

#[test]
fn every_divisor_of_a_fuzz_set_gives_the_tree_walkers_answer_specialized() {
    let mut divisors: Vec<i64> = (2..=40).collect();
    for e in [7u32, 13, 31, 32, 33, 47, 61, 62] {
        divisors.extend([(1i64 << e) - 1, 1i64 << e, (1i64 << e) + 1]);
    }
    divisors.extend([641, 1_000_000_007, 6_700_417, i64::MAX - 1, i64::MAX]);
    for d in divisors {
        let traced = run(REM, CALLS_REM_WIDE, &d.to_string());
        assert_eq!(specialized_on(&traced), vec![(0, ScalarValue::Int(d))], "k = {d}");
    }
}

#[test]
fn a_function_called_with_an_invariant_flag_is_specialized_on_either_value() {
    for (arg, flag) in [("1", true), ("0", false)] {
        let traced = run(PICK, CALLS_PICK, arg);
        assert_eq!(specializations(&traced), 1, "`pick` with the flag from {arg} was not specialized: {traced:?}");
        assert_eq!(specialized_on(&traced), vec![(0, ScalarValue::Bool(flag))], "flag from {arg}");
    }
}

#[test]
fn an_argument_that_changes_after_the_warm_up_takes_the_generic_body() {
    for arg in ["2", "5", "1000"] {
        let traced = run(REM, CALLS_REM_THEN_CHANGES, arg);
        assert_eq!(specializations(&traced), 1, "specialized once on the warm-up's value, never again: {traced:?}");
        assert_eq!(specialized_on(&traced), vec![(0, ScalarValue::Int(arg.parse().unwrap()))]);
    }
}

#[test]
fn an_argument_that_changes_during_the_warm_up_is_not_specialized() {
    for arg in ["2", "13"] {
        let traced = run(REM, CALLS_REM_CHANGING, arg);
        assert_eq!(specializations(&traced), 0, "no value held for the warm-up: {traced:?}");
        assert!(traced.function_specializations.is_empty(), "{traced:?}");
    }
}

/// Nothing folds on a divisor below two, so nothing is specialized; a zero divisor fails as the tree-walker does. (A
/// base of one is no base: its digit loop never ends.)
#[test]
fn a_divisor_below_two_is_not_specialized_and_a_zero_one_fails_as_the_tree_walker_does() {
    for arg in ["0", "-3"] {
        let traced = run(REM, CALLS_REM, arg);
        assert_eq!(specializations(&traced), 0, "k = {arg} has no reciprocal to specialize on: {traced:?}");
    }
}

#[test]
fn a_function_that_calls_itself_with_the_held_value_is_specialized() {
    for arg in ["2", "10", "97"] {
        let traced = run(DIGITS, CALLS_DIGITS, arg);
        assert_eq!(specialized_on(&traced), vec![(0, ScalarValue::Int(arg.parse().unwrap()))], "k = {arg}");
    }
}

#[test]
fn a_tail_call_passing_the_held_value_on_keeps_it_specialized() {
    for arg in ["2", "10", "97"] {
        let traced = run(LEADING, CALLS_LEADING, arg);
        assert_eq!(specialized_on(&traced), vec![(0, ScalarValue::Int(arg.parse().unwrap()))], "k = {arg}");
    }
}

#[test]
fn a_precise_function_side_exits_from_its_specialized_copy_and_the_vm_finishes_the_call() {
    for arg in ["2", "7", "1000"] {
        let traced = run(BUCKET, CALLS_BUCKET, arg);
        assert_eq!(specialized_on(&traced), vec![(0, ScalarValue::Int(arg.parse().unwrap()))], "k = {arg}");
    }
    let halving = run(BUCKET, CALLS_BUCKET, "2");
    assert!(precise_deopts(&halving) > 0, "halves of the two pushed words overflow the sum: {}", halving.execution);
}

#[test]
fn a_function_calling_itself_with_another_value_takes_the_generic_body() {
    for arg in ["3", "1000"] {
        let traced = run(SPIN, CALLS_SPIN, arg);
        let guards = specialized_on(&traced);
        assert!(guards.contains(&(0, ScalarValue::Int(arg.parse().unwrap()))), "k = {arg}: {guards:?}");
    }
}

#[test]
fn a_function_held_to_two_values_takes_the_generic_body_when_either_changes() {
    for (arg, flag) in [("3", true), ("-9", false), ("100", true)] {
        let traced = run(MIX, CALLS_MIX, arg);
        let k: i64 = arg.parse().unwrap();
        let expected = if k >= 2 {
            vec![(0, ScalarValue::Bool(flag)), (1, ScalarValue::Int(k))]
        } else {
            vec![(0, ScalarValue::Bool(flag))]
        };
        assert_eq!(specialized_on(&traced), expected, "k = {arg}");
    }
}

#[test]
fn a_held_value_nothing_folds_on_is_not_specialized() {
    let traced = run(SCALE, CALLS_SCALE, "7");
    assert!(traced.function_successes >= 1, "`scale` compiles: {traced:?}");
    assert_eq!(specializations(&traced), 0, "a multiply by k decides nothing: {traced:?}");
}

#[test]
fn a_parameter_the_function_writes_is_not_specialized() {
    for arg in ["2", "10"] {
        let traced = run(SHIFTS, CALLS_SHIFTS, arg);
        assert!(traced.function_successes >= 1, "`shifts` compiles: {traced:?}");
        assert_eq!(specializations(&traced), 0, "k changes inside the call: {traced:?}");
    }
}

#[test]
fn largo_opts_reports_each_specialized_function() {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("rem.lg"), program(REM, CALLS_REM)).expect("write source");
    let out = largo_in(dir.path(), &["opts", "rem.lg", "--jit", "--", "7"]);
    assert!(out.status.success(), "`largo opts --jit` succeeds:\n{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("value_specialized_function"), "the lowering is named:\n{text}");
    let line = text
        .lines()
        .find(|l| l.contains("function") && l.contains("specialized on r0 = 7"))
        .unwrap_or_else(|| panic!("a line names the function specialized on 7:\n{text}"));
    assert!(line.contains("generic code"), "{line}");

    let out = largo_in(dir.path(), &["opts", "rem.lg", "--jit", "--json", "--", "7"]);
    assert!(out.status.success(), "`largo opts --jit --json` succeeds:\n{}", stderr(&out));
    let report: serde_json::Value = serde_json::from_str(stdout(&out).trim()).expect("the report is JSON");
    let specs = report["jit_native"]["function_specializations"].as_array().expect("a list of specialized functions");
    assert_eq!(specs.len(), 1, "{report}");
    assert_eq!(specs[0]["guards"], serde_json::json!([{ "slot": 0, "value": 7 }]), "{report}");
    assert!(specs[0]["entry_pc"].is_u64() && specs[0]["function"].is_u64(), "{report}");
}

//! Translation validation of the register VM's bytecode: for every input, each compiled function computes what its
//! source computes — loops proved for every trip count, not unrolled — on both the engine-differential compile
//! (`Target::Vm`) and the program `largo run` runs (`Target::Run`). The lock has three halves: the proofs hold; a
//! seeded miscompile of each kind is refuted with a counterexample, so a proof here can fail; and the two symbolic
//! executors show, run concretely, what the tree-walker and the VM show. Requires the `verification` feature (Z3).

#![cfg(feature = "verification")]

use logicaffeine_compile::vm::{CompiledProgram, Constant, Op};
use logicaffeine_tv::bytecode::{check_executors, validate, validate_edited, ExecutorCheck, Target, Verdict};

const TARGETS: [Target; 2] = [Target::Vm, Target::Run];

fn proven(src: &str, function: &str) -> Vec<Vec<String>> {
    let mut last = Vec::new();
    for target in TARGETS {
        let report = validate(src, target).unwrap_or_else(|e| panic!("{target:?}: {e}\n{src}"));
        match report.verdict(function) {
            Some(Verdict::Proven { invariants, .. }) => last = invariants.clone(),
            other => panic!("{target:?}: `{function}` not proven: {other:?}\n{src}"),
        }
    }
    last
}

fn refuted(src: &str, function: &str, edit: impl Fn(&mut CompiledProgram, &[String])) -> String {
    let report = validate_edited(src, Target::Vm, |p, names| edit(p, names)).expect("compiles");
    match report.verdict(function) {
        Some(Verdict::Refuted { input, source, bytecode, .. }) => {
            assert_ne!(source, bytecode, "a refutation shows the two apart");
            format!("{input}: the source {source}, the bytecode {bytecode}")
        }
        other => panic!("a seeded miscompile of `{function}` was not refuted: {other:?}\n{src}"),
    }
}

/// The pcs of `function`'s body: from its entry to the next function's entry (or the end of the code).
fn body(p: &CompiledProgram, names: &[String], function: &str) -> std::ops::Range<usize> {
    let i = names.iter().position(|n| n == function).expect("the function is compiled");
    let start = p.functions[i].entry_pc;
    let end = p.functions.iter().map(|f| f.entry_pc).filter(|&e| e > start).min().unwrap_or(p.code.len());
    start..end
}

fn first_in(p: &CompiledProgram, names: &[String], function: &str, pick: impl Fn(&Op) -> bool) -> usize {
    body(p, names, function).find(|&pc| pick(&p.code[pc])).expect("the function holds the op")
}

fn agree(src: &str) {
    for target in TARGETS {
        assert_eq!(check_executors(src, target), ExecutorCheck::Agree, "{target:?}\n{src}");
    }
}

const POLY: &str = "\
## To poly (x: Int) -> Int:
    Return x * x + 3 * x - 7.

## Main
Show poly(5).
";

const SIGN: &str = "\
## To sign (x: Int) -> Int:
    If x is greater than 0:
        Return 1.
    If x is less than 0:
        Return 0 - 1.
    Return 0.

## Main
Show sign(0 - 4).
";

const SUM: &str = "\
## To sum (n: Int) -> Int:
    Let mutable total be 0.
    Let mutable i be 1.
    While i is at most n:
        Set total to total + i.
        Set i to i + 1.
    Return total.

## Main
Show sum(10).
";

const GCD_LOOP: &str = "\
## To gcd (a: Int, b: Int) -> Int:
    Let mutable x be a.
    Let mutable y be b.
    While y is not 0:
        Let t be y.
        Set y to x % y.
        Set x to t.
    Return x.

## Main
Show gcd(1071, 462).
";

const GCD_REC: &str = "\
## To gcd (a: Int, b: Int) -> Int:
    If b equals 0:
        Return a.
    Return gcd(b, a % b).

## Main
Show gcd(48, 18).
";

const COLLATZ: &str = "\
## To steps (n: Int) -> Int:
    Let mutable x be n.
    Let mutable count be 0.
    While x is greater than 1:
        If x % 2 equals 0:
            Set x to x / 2.
        Otherwise:
            Set x to 3 * x + 1.
        Set count to count + 1.
    Return count.

## Main
Show steps(27).
";

const TENTH: &str = "\
## To tenth (n: Int) -> Int:
    If n is greater than 0:
        Return n / 10.
    Return 0.

## Main
Show tenth(12345).
";

const DIGITS: &str = "\
## To digits (n: Int) -> Int:
    If n is less than 1:
        Return 1.
    Let mutable x be n.
    Let mutable count be 0.
    While x is greater than 0:
        Set x to x / 10.
        Set count to count + 1.
    Return count.

## Main
Show digits(12345).
";

const NESTED: &str = "\
## To product (a: Int, b: Int) -> Int:
    Let mutable total be 0.
    Let mutable i be 0.
    While i is less than a:
        Let mutable j be 0.
        While j is less than b:
            Set total to total + 1.
            Set j to j + 1.
        Set i to i + 1.
    Return total.

## Main
Show product(3, 4).
";

const BREAK: &str = "\
## To first_square_above (n: Int) -> Int:
    Let mutable i be 0.
    While true:
        If i * i is greater than n:
            Break.
        Set i to i + 1.
    Return i.

## Main
Show first_square_above(50).
";

const INLINE: &str = "\
## To sq (x: Int) -> Int:
    Return x * x.

## To hyp (a: Int, b: Int) -> Int:
    Return sq(a) + sq(b).

## Main
Show hyp(3, 4).
";

const RAISES: &str = "\
## To ratio (n: Int, d: Int) -> Int:
    If d is not 0 and n / d is greater than 2:
        Return n / d.
    Return n % d.

## Main
Show ratio(9, 2).
";

const BOOLS: &str = "\
## To pick (a: Bool, b: Bool, x: Int) -> Int:
    If a and not b:
        Return x.
    If a or b:
        Return 0 - x.
    Return 0.

## Main
Show pick(true, false, 7).
";

const LOW_BITS: &str = "\
## To low (x: Int) -> Int:
    If x is less than 0 - 100:
        Return 0.
    If x is greater than 100:
        Return 0.
    Return x % 8.

## Main
Show low(13).
";

const MASKED: &str = "\
## To low (x: Int) -> Int:
    If x is greater than 0:
        Return x % 8.
    Return 0.

## Main
Show low(13).
";

const SEVENS: &str = "\
## To sev (x: Int) -> Int:
    If x is at least 0:
        Return x % 7 + x / 8.
    Return 0.

## Main
Show sev(13).
";

const FIB: &str = "\
## To fib (n: Int) -> Int:
    If n is less than 2:
        Return n.
    Return fib(n - 1) + fib(n - 2).

## Main
Show fib(10).
";

const SUM_RANGE: &str = "\
## To sum (n: Int) -> Int:
    Let mutable total be 0.
    Repeat for i from 1 to n:
        Set total to total + i.
    Return total.

## Main
Show sum(10).
";

const NESTED_RANGE: &str = "\
## To product (a: Int, b: Int) -> Int:
    Let mutable total be 0.
    Repeat for i from 1 to a:
        Repeat for j from 1 to b:
            Set total to total + 1.
    Return total.

## Main
Show product(3, 4).
";

const RANGE_VALUE: &str = "\
## To f (n: Int) -> Int:
    Return n + 1.

## To g (n: Int) -> Int:
    Let mutable t be 0.
    Repeat for i from 1 to f(n):
        Set t to t + i.
    Return t.

## Main
Show g(4).
";

const COUNT: &str = "\
## To count (a: Int) -> Int:
    Let mutable t be 0.
    Repeat for i from a to a + 2:
        Set t to t + 1.
    Return t.

## Main
Show count(5).
";

const MAIN_LOOP: &str = "\
## Main
Let mutable total be 0.
Let mutable i be 1.
While i is at most 20:
    If i % 3 equals 0:
        Show i.
    Set total to total + i * i.
    Set i to i + 1.
Show total.
";

#[test]
fn straight_line_arithmetic_is_proven_for_every_input() {
    proven(POLY, "poly");
}

#[test]
fn branches_are_proven_for_every_input() {
    proven(SIGN, "sign");
    proven(BOOLS, "pick");
}

#[test]
fn a_loop_is_proven_for_every_trip_count() {
    let report = validate(SUM, Target::Vm).unwrap();
    match report.verdict("sum") {
        Some(Verdict::Proven { invariants, .. }) => {
            assert_eq!(invariants.len(), 1, "one loop, one invariant");
            assert!(invariants[0].iter().any(|c| c.ends_with("= total")), "the accumulator's register: {invariants:?}");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_loop_the_run_path_replaces_with_a_closed_form_is_proven_for_every_n() {
    let (program, names) = logicaffeine_tv::bytecode::compile(SUM, Target::Run).unwrap();
    assert!(
        !body(&program, &names, "sum").any(|pc| matches!(program.code[pc], Op::Jump { target } if target < pc)),
        "the run-path optimizer sums in closed form"
    );
    let report = validate(SUM, Target::Run).unwrap();
    match report.verdict("sum") {
        Some(Verdict::Proven { invariants, .. }) => {
            assert!(invariants.iter().flatten().any(|c| c == "i² = 2·total + i"), "{invariants:?}")
        }
        other => panic!("the VM's closed form is exact, so it is proven outright: {other:?}"),
    }
}

#[test]
fn a_while_loop_with_a_remainder_is_proven() {
    proven(GCD_LOOP, "gcd");
}

#[test]
fn a_tail_call_compiled_to_a_loop_is_proven_as_that_loop_assuming_nothing() {
    for target in TARGETS {
        let (program, names) = logicaffeine_tv::bytecode::compile(GCD_REC, target).unwrap();
        assert!(
            !body(&program, &names, "gcd").any(|pc| matches!(program.code[pc], Op::Call { .. })),
            "{target:?}: the tail call is a jump"
        );
        let report = validate(GCD_REC, target).unwrap();
        match report.verdict("gcd") {
            Some(Verdict::Proven { invariants, assumed }) => {
                assert!(assumed.is_empty(), "{target:?}: {assumed:?}");
                assert_eq!(invariants.len(), 1, "{target:?}: the loop the tail call became");
            }
            other => panic!("{target:?}: {other:?}"),
        }
    }
}

#[test]
fn a_recursive_function_is_proven_assuming_itself() {
    let report = validate(FIB, Target::Vm).unwrap();
    match report.verdict("fib") {
        Some(Verdict::Proven { assumed, .. }) => assert_eq!(assumed, &vec!["fib".to_string()]),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_function_the_run_path_rewrites_outside_the_fragment_is_never_refuted() {
    let report = validate(FIB, Target::Run).unwrap();
    assert!(report.refutes_nothing(), "{report:?}");
    match report.verdict("fib") {
        Some(Verdict::Proven { .. } | Verdict::Unsupported { .. }) => {}
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_counted_range_is_proven_for_every_bound_and_trip_count() {
    for (src, function) in [(COUNT, "count"), (SUM_RANGE, "sum"), (NESTED_RANGE, "product")] {
        let report = validate(src, Target::Vm).unwrap();
        match report.verdict(function) {
            Some(Verdict::Proven { invariants, .. }) => assert!(!invariants.is_empty(), "{function}"),
            other => panic!("`{function}`: {other:?}"),
        }
    }
}

#[test]
fn a_range_value_is_proven_through_the_vms_iterator() {
    let (program, names) = logicaffeine_tv::bytecode::compile(RANGE_VALUE, Target::Vm).unwrap();
    assert!(
        body(&program, &names, "g").any(|pc| matches!(program.code[pc], Op::IterNext { .. })),
        "a bound the optimizer cannot prove an Int keeps the range value"
    );
    proven(RANGE_VALUE, "g");
}

#[test]
fn a_loop_with_a_branch_inside_is_proven() {
    proven(COLLATZ, "steps");
}

#[test]
fn the_magic_reciprocal_division_is_proven_for_every_dividend() {
    for (src, function) in [(TENTH, "tenth"), (DIGITS, "digits")] {
        let (program, names) = logicaffeine_tv::bytecode::compile(src, Target::Vm).unwrap();
        assert!(
            body(&program, &names, function).any(|pc| matches!(program.code[pc], Op::MagicDivU { .. })),
            "`/ 10` on a proven non-negative dividend compiles to the reciprocal in `{function}`"
        );
        proven(src, function);
    }
}

#[test]
fn the_mask_a_proven_non_negative_remainder_lowers_to_is_proven() {
    let (program, names) = logicaffeine_tv::bytecode::compile(MASKED, Target::Vm).unwrap();
    assert!(
        body(&program, &names, "low").any(|pc| matches!(program.code[pc], Op::BitAnd { .. })),
        "`x % 8` on a proven non-negative `x` compiles to `x & 7`"
    );
    proven(MASKED, "low");
    proven(LOW_BITS, "low");
}

#[test]
fn a_reciprocal_remainder_and_a_power_of_two_division_are_proven() {
    let (program, names) = logicaffeine_tv::bytecode::compile(SEVENS, Target::Vm).unwrap();
    let ops: Vec<&Op> = body(&program, &names, "sev").map(|pc| &program.code[pc]).collect();
    assert!(ops.iter().any(|op| matches!(op, Op::MagicDivU { mul_back: 7, .. })), "`x % 7` is the reciprocal remainder");
    assert!(ops.iter().any(|op| matches!(op, Op::DivPow2 { k: 3, .. })), "`x / 8` is a shift");
    proven(SEVENS, "sev");
}

#[test]
fn nested_loops_are_proven() {
    let inv = proven(NESTED, "product");
    assert_eq!(inv.len(), 2);
}

#[test]
fn a_break_out_of_a_loop_is_proven() {
    proven(BREAK, "first_square_above");
}

#[test]
fn a_loop_free_callee_runs_in_place() {
    for target in TARGETS {
        let report = validate(INLINE, target).unwrap();
        match report.verdict("hyp") {
            Some(Verdict::Proven { assumed, .. }) => assert!(assumed.is_empty(), "nothing assumed: {assumed:?}"),
            other => panic!("{target:?}: {other:?}"),
        }
    }
}

#[test]
fn raising_on_the_same_inputs_is_part_of_the_proof() {
    proven(RAISES, "ratio");
}

#[test]
fn main_with_a_loop_and_shows_is_proven() {
    proven(MAIN_LOOP, "## Main");
}

#[test]
fn swapped_subtraction_operands_are_refuted() {
    let why = refuted(POLY, "poly", |p, names| {
        let pc = first_in(p, names, "poly", |op| matches!(op, Op::Sub { .. }));
        if let Op::Sub { dst, lhs, rhs } = p.code[pc] {
            p.code[pc] = Op::Sub { dst, lhs: rhs, rhs: lhs };
        }
    });
    assert!(why.starts_with("x = "), "the refutation names the input: {why}");
}

#[test]
fn a_changed_constant_is_refuted() {
    refuted(POLY, "poly", |p, names| {
        let pc = first_in(p, names, "poly", |op| matches!(op, Op::LoadConst { .. }));
        if let Op::LoadConst { dst, .. } = p.code[pc] {
            p.constants.push(Constant::Int(4));
            p.code[pc] = Op::LoadConst { dst, idx: (p.constants.len() - 1) as u32 };
        }
    });
}

#[test]
fn an_inverted_branch_is_refuted() {
    refuted(SIGN, "sign", |p, names| {
        let pc = first_in(p, names, "sign", |op| matches!(op, Op::JumpIfFalse { .. }));
        if let Op::JumpIfFalse { cond, target } = p.code[pc] {
            p.code[pc] = Op::JumpIfTrue { cond, target };
        }
    });
}

#[test]
fn a_loop_that_adds_the_wrong_register_is_refuted() {
    refuted(SUM, "sum", |p, names| {
        let pc = first_in(p, names, "sum", |op| matches!(op, Op::AddAssign { .. }));
        if let Op::AddAssign { dst, .. } = p.code[pc] {
            p.code[pc] = Op::AddAssign { dst, src: dst };
        }
    });
}

#[test]
fn a_wrong_magic_constant_is_refuted_with_a_dividend_it_gets_wrong() {
    let why = refuted(TENTH, "tenth", |p, names| {
        let pc = first_in(p, names, "tenth", |op| matches!(op, Op::MagicDivU { .. }));
        if let Op::MagicDivU { dst, lhs, magic, more, mul_back, divisor, wrap } = p.code[pc] {
            p.code[pc] = Op::MagicDivU { dst, lhs, magic: magic + (1 << 30), more, mul_back, divisor, wrap };
        }
    });
    let n: i64 = why["n = ".len()..why.find(':').unwrap()].parse().unwrap();
    assert!(
        n >= 13_743_895_349,
        "a reciprocal 2^30 too large divides every dividend below 13743895349 by ten exactly; only a larger one shows it: \
         {why}"
    );
}

#[test]
fn a_remainder_lowered_to_a_mask_where_the_dividend_can_be_negative_is_refuted() {
    let why = refuted(LOW_BITS, "low", |p, names| {
        let pc = first_in(p, names, "low", |op| matches!(op, Op::Mod { .. }));
        let Op::Mod { dst, lhs, rhs } = p.code[pc] else { unreachable!() };
        let load = (0..pc).rev().find(|&q| matches!(p.code[q], Op::LoadConst { dst, .. } if dst == rhs)).expect("the divisor is loaded");
        p.constants.push(Constant::Int(7));
        p.code[load] = Op::LoadConst { dst: rhs, idx: (p.constants.len() - 1) as u32 };
        p.code[pc] = Op::BitAnd { dst, lhs, rhs };
    });
    let x: i64 = why["x = ".len()..why.find(':').unwrap()].parse().unwrap();
    assert!((-100..0).contains(&x) && x % 8 != 0, "only a negative dividend tells `x & 7` from `x % 8`: {why}");
    assert!(why.contains("the bytecode returns"), "a wrong value, not a raise: {why}");
}

#[test]
fn a_raise_where_the_source_returns_is_refuted() {
    let why = refuted(POLY, "poly", |p, names| {
        let ret = first_in(p, names, "poly", |op| matches!(op, Op::Return { .. }));
        p.code[ret] = Op::FailWith { msg: 0 };
    });
    assert!(why.contains("the source returns") && why.ends_with("the bytecode raises"), "{why}");
}

#[test]
fn a_return_of_the_wrong_register_is_refuted() {
    refuted(GCD_LOOP, "gcd", |p, names| {
        let ret = first_in(p, names, "gcd", |op| matches!(op, Op::Return { .. }));
        p.code[ret] = Op::Return { src: 1 };
    });
}

#[test]
fn a_shown_value_that_differs_in_main_is_refuted() {
    refuted(MAIN_LOOP, "## Main", |p, _| {
        let pc = (0..p.code.len()).find(|&pc| matches!(p.code[pc], Op::Mul { .. })).expect("main multiplies");
        if let Op::Mul { dst, lhs, .. } = p.code[pc] {
            p.code[pc] = Op::Add { dst, lhs, rhs: lhs };
        }
    });
}

#[test]
fn every_reciprocal_the_compiler_emits_is_exact_for_every_dividend() {
    use logicaffeine_compile::vm::compiler::magic_u64_gen;
    use logicaffeine_tv::bytecode::value::magic_exact;
    let rims = [i64::MAX, i64::MAX - 1, (1 << 32) + 1, (1 << 32) - 1, 1 << 40, 641, 6_700_417, 1_000_000_007];
    for c in (2..=10_000).chain(rims) {
        let (magic, more) = magic_u64_gen(c as u64);
        assert!(magic_exact(magic, more, c), "the reciprocal of {c} is not exact");
    }
    let (magic, more) = magic_u64_gen(10);
    assert!(!magic_exact(magic - 1, more, 10), "a reciprocal one too small is not exact");
    assert!(!magic_exact(magic + 1, more, 10), "nor one too large");
    assert!(!magic_exact(magic, more, 11), "nor the reciprocal of another divisor");
    assert!(magic_exact(1 << 61, 0, 8), "a power of two by its exact reciprocal on the multiply path");
    assert!(!magic_exact((1 << 61) + 1, 0, 8), "one too large is short at the top of u64: the bound at the last n decides it");
}

#[test]
fn the_executors_show_what_the_engines_show() {
    assert_eq!(check_executors(FIB, Target::Vm), ExecutorCheck::Agree);
    for src in [POLY, SIGN, SUM, SUM_RANGE, NESTED_RANGE, RANGE_VALUE, COUNT, GCD_LOOP, GCD_REC, COLLATZ, TENTH, SEVENS, DIGITS, NESTED, BREAK, INLINE, RAISES, BOOLS, LOW_BITS, MASKED, MAIN_LOOP] {
        agree(src);
    }
}

#[test]
fn the_executors_round_division_toward_zero_as_the_engines_do() {
    agree(
        "## To q (a: Int, b: Int) -> Int:\n    Return a / b * 1000 + a % b.\n\n## Main\n\
         Show q(0 - 7, 2).\nShow q(7, 0 - 2).\nShow q(0 - 7, 0 - 2).\nShow q(7, 2).\n",
    );
}

#[test]
fn the_executors_agree_with_the_engines_when_a_program_raises() {
    agree("## To f (d: Int) -> Int:\n    Return 10 / d.\n\n## Main\nShow 1.\nShow f(0).\n");
}

#[test]
fn a_program_outside_the_fragment_is_unsupported_never_proven() {
    let src = "## To greet (n: Int) -> Int:\n    Show \"hi\".\n    Return n.\n\n## Main\nShow greet(1).\n";
    let report = validate(src, Target::Vm).unwrap();
    assert!(matches!(report.verdict("greet"), Some(Verdict::Unsupported { .. })), "{report:?}");
}

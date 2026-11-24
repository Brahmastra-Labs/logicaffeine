//! ★ LOCK — a hot loop is specialized on the divisor it never changes. ★
//!
//! A `/` or `%` whose divisor is a value the loop reads but never writes divides by one number for a whole entry. The
//! JIT compiles the loop against the value it saw at the hot crossing: the division becomes the signed magic reciprocal
//! (a multiply-high, an add and two shifts in place of `idiv`), and the region carries a value guard. An entry that
//! finds another divisor runs on bytecode, and a region whose guard keeps missing gives way to the generic one.
//!
//! Two layers:
//! - the op: `MagicDivS` built from `SignedReciprocal` is truncating `/` and `%` for dividends of either sign, through
//!   `reference_eval`, the contiguous register allocator and the stencil tier with and without pinned registers;
//! - the run path: programs whose divisor arrives on the command line print exactly what the tree-walker prints, the
//!   trace names `value_specialized_region` when a loop was specialized, and does not when the divisor is written in
//!   the loop, is below two, or is zero;
//! - the report: the trace, and `largo opts --jit`, name each specialized loop with the value it was specialized on,
//!   the entries that ran it, the entries whose value had changed, and whether the generic region replaced it.

#![cfg(all(target_arch = "x86_64", not(target_os = "windows")))]

#[path = "cli_common/mod.rs"]
mod common;

use common::{largo_in, stderr, stdout};
use logicaffeine_base::reciprocal::SignedReciprocal;
use logicaffeine_compile::compile::tw_outcome_with_args;
use logicaffeine_compile::vm::ScalarValue;
use logicaffeine_forge::jit::{
    compile_straightline_pinned_float, compile_straightline_with, reference_eval, ChainOutcome, CompiledChain, MicroOp,
};
use logicaffeine_forge::regalloc::compile_region_regalloc;
use logicaffeine_jit::{optimization_trace_jit, JitBackendOptimizationTrace, JitOptimization};
use std::sync::atomic::AtomicI64;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// The op on every forge backend.
// ---------------------------------------------------------------------------

/// `q = x / d` into slot 1, `r = x % d` into slot 2, return `q`.
fn quotient_and_remainder(d: i64) -> Vec<MicroOp> {
    let r = SignedReciprocal::new(d).expect("a divisor of at least two has a reciprocal");
    let op = |dst, mul_back| MicroOp::MagicDivS {
        dst,
        lhs: 0,
        magic: r.magic(),
        shift: r.shift() as u8,
        add: r.add(),
        mul_back,
    };
    vec![op(1, 0), op(2, d), MicroOp::Return { src: 1 }]
}

fn backends(ops: &[MicroOp]) -> Vec<(&'static str, CompiledChain)> {
    vec![
        (
            "regalloc",
            compile_region_regalloc(ops, Some(Arc::new(AtomicI64::new(0))), &[2]).expect("regalloc lowers MagicDivS"),
        ),
        ("stencil", compile_straightline_with(ops, None).expect("the stencil tier lowers MagicDivS")),
        (
            "stencil, pinned",
            compile_straightline_pinned_float(ops, &[0, 1, 2], &[], None).expect("the pinned tier lowers MagicDivS"),
        ),
    ]
}

fn dividends(d: i64) -> Vec<i64> {
    let mut xs = vec![i64::MIN, i64::MIN + 1, i64::MIN + 2, -(1 << 32), -2, -1, 0, 1, 2, 1 << 32, i64::MAX - 1, i64::MAX];
    for k in [-1000i64, -3, -2, -1, 1, 2, 3, 1000] {
        if let Some(m) = d.checked_mul(k) {
            xs.extend([m.saturating_sub(1), m, m.saturating_add(1)]);
        }
    }
    let top = (i64::MAX / d) * d;
    xs.extend([top - 1, top, -top, -top + 1]);
    xs
}

fn assert_exact(d: i64, xs: &[i64]) {
    let ops = quotient_and_remainder(d);
    let chains = backends(&ops);
    for &x in xs {
        let mut frame = vec![x, 0, 0];
        assert_eq!(
            reference_eval(&ops, &mut frame, 10),
            Some(x / d),
            "reference_eval: {x} / {d}"
        );
        assert_eq!(frame[2], x % d, "reference_eval: {x} % {d}");
        for (name, chain) in &chains {
            let mut frame = vec![x, 0, 0];
            assert_eq!(chain.run_with_frame(&mut frame), ChainOutcome::Return(x / d), "{name}: {x} / {d}");
            assert_eq!(frame[2], x % d, "{name}: {x} % {d}");
        }
    }
}

#[test]
fn the_reciprocal_divides_exactly_on_every_backend_for_small_divisors() {
    for d in 2..=300 {
        assert_exact(d, &dividends(d));
    }
}

#[test]
fn the_reciprocal_divides_exactly_on_every_backend_for_powers_of_two_and_large_divisors() {
    let mut ds: Vec<i64> = (1..63).map(|k| 1i64 << k).collect();
    ds.extend([
        641,
        1_000_000_007,
        (1 << 31) - 1,
        (1 << 61) - 1,
        3 << 60,
        i64::MAX / 3,
        i64::MAX - 1,
        i64::MAX,
    ]);
    for d in ds {
        assert_exact(d, &dividends(d));
    }
}

#[test]
fn the_reciprocal_divides_exactly_on_every_backend_for_random_divisors() {
    // SplitMix64, fixed seed.
    let mut state: u64 = 0x2545_F491_4F6C_DD1D;
    let mut next = || {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    for _ in 0..300 {
        let bits = (next() % 62) as u32 + 2;
        let d = ((next() >> (64 - bits)) as i64).max(2);
        let mut xs = dividends(d);
        xs.extend((0..200).map(|_| next() as i64));
        assert_exact(d, &xs);
    }
}

// ---------------------------------------------------------------------------
// The run path.
// ---------------------------------------------------------------------------

const NATIVES: &str = "## To native args () -> Seq of Text\n## To native parseInt (s: Text) -> Int\n\n";

fn run(body: &str, divisor: &str) -> JitBackendOptimizationTrace {
    let source = format!("{NATIVES}## Main\nLet k be parseInt(item 2 of args()).\n{body}");
    let args = ["logos".to_string(), divisor.to_string()];
    let traced = optimization_trace_jit(&source, &args);
    let tree = tw_outcome_with_args(&source, &args);
    assert_eq!(traced.outcome.error, tree.error, "the JIT and the tree-walker disagree on the error:\n{source}");
    assert_eq!(
        traced.outcome.output.trim(),
        tree.output.trim(),
        "the JIT and the tree-walker disagree on the output for divisor {divisor}:\n{source}"
    );
    traced
}

fn specialized(traced: &JitBackendOptimizationTrace) -> bool {
    traced.native.keywords().contains(&"value_specialized_region")
}

const REMAINDER_LOOP: &str = "\
Let mutable acc be 0.
Let mutable i be 0 - 5000.
While i is less than 5000:
    Set acc to acc + (i * 7919) % k.
    Set i to i + 1.
Show acc.
";

const QUOTIENT_LOOP: &str = "\
Let mutable acc be 0.
Let mutable i be 0 - 5000.
While i is less than 5000:
    Set acc to acc + (i * 7919) / k.
    Set i to i + 1.
Show acc.
";

const EXTREME_DIVIDENDS: &str = "\
Let hi be 9223372036854775807.
Let lo be 0 - hi - 1.
Let mutable acc be 0.
Let mutable last be 0.
Let mutable i be 0.
While i is less than 3000:
    Set acc to acc + (hi - i) % k + (lo + i) % k.
    Set last to (lo + i) / k + (hi - i) / k.
    Set i to i + 1.
Show acc.
Show last.
";

#[test]
fn a_remainder_by_an_invariant_divisor_is_specialized_and_exact() {
    for divisor in ["2", "3", "7", "10", "641", "1000000007", "9223372036854775807"] {
        let traced = run(REMAINDER_LOOP, divisor);
        assert!(specialized(&traced), "`% k` with k = {divisor} was not specialized: {traced:?}");
    }
}

#[test]
fn a_quotient_by_an_invariant_divisor_is_specialized_and_exact() {
    for divisor in ["2", "3", "7", "10", "641", "1000000007", "9223372036854775807"] {
        let traced = run(QUOTIENT_LOOP, divisor);
        assert!(specialized(&traced), "`/ k` with k = {divisor} was not specialized: {traced:?}");
    }
}

#[test]
fn the_extreme_dividends_divide_exactly() {
    for divisor in ["2", "3", "7", "1000", "4611686018427387904"] {
        let traced = run(EXTREME_DIVIDENDS, divisor);
        assert!(specialized(&traced), "k = {divisor} was not specialized: {traced:?}");
    }
}

#[test]
fn a_divisor_that_changes_between_entries_falls_back_to_the_generic_region() {
    for divisor in ["2", "5", "1000"] {
        let traced = run(CHANGING_DIVISOR, divisor);
        assert!(specialized(&traced), "the first entry was not specialized: {traced:?}");
        assert!(
            traced.region_successes >= 2,
            "after the value guard kept missing, the generic region must replace the specialized one: {traced:?}"
        );
        assert_eq!(
            traced.native.count(JitOptimization::ValueSpecializedRegion),
            1,
            "the replacement is generic, not specialized again on the next value: {traced:?}"
        );
    }
}

#[test]
fn a_divisor_written_in_the_loop_is_not_specialized() {
    let body = "\
Let mutable d be k.
Let mutable acc be 0.
Let mutable i be 0 - 3000.
While i is less than 3000:
    Set acc to acc + (i * 13) % d + (i * 11) / d.
    Set d to d + 1.
    Set i to i + 1.
Show acc.
";
    let traced = run(body, "3");
    assert!(!specialized(&traced), "a divisor the loop writes cannot be specialized: {traced:?}");
}

#[test]
fn a_divisor_below_two_is_not_specialized() {
    for divisor in ["1", "-1", "-7", "-9223372036854775807"] {
        let traced = run(REMAINDER_LOOP, divisor);
        assert!(!specialized(&traced), "`% k` with k = {divisor} has no reciprocal: {traced:?}");
        let traced = run(QUOTIENT_LOOP, divisor);
        assert!(!specialized(&traced), "`/ k` with k = {divisor} has no reciprocal: {traced:?}");
    }
}

#[test]
fn a_zero_divisor_fails_as_the_tree_walker_does() {
    let traced = run(REMAINDER_LOOP, "0");
    assert!(traced.outcome.error.is_some(), "a remainder by zero must fail: {traced:?}");
    assert!(!specialized(&traced), "zero has no reciprocal: {traced:?}");
    let traced = run(QUOTIENT_LOOP, "0");
    assert!(traced.outcome.error.is_some(), "a quotient by zero must fail: {traced:?}");
}

const CHANGING_DIVISOR: &str = "\
Let mutable total be 0.
Let mutable d be k.
While d is less than k + 40:
    Let mutable i be 0 - 300.
    While i is less than 300:
        Set total to total + (i * 31) % d + (i * 17) / d.
        Set i to i + 1.
    Set d to d + 1.
Show total.
";

#[test]
fn the_trace_names_the_value_a_loop_was_specialized_on() {
    let traced = run(REMAINDER_LOOP, "7");
    let [spec] = traced.value_specializations.as_slice() else {
        panic!("one loop was specialized: {traced:?}");
    };
    assert_eq!(spec.guards.len(), 1, "one divisor: {spec:?}");
    assert_eq!(spec.guards[0].1, ScalarValue::Int(7), "specialized on k = 7: {spec:?}");
    assert!(spec.entries >= 1, "the specialized region ran: {spec:?}");
    assert_eq!(spec.misses, 0, "k never changed: {spec:?}");
    assert!(!spec.replaced, "the specialized region stayed: {spec:?}");
}

#[test]
fn the_trace_names_the_misses_that_replaced_a_specialized_loop() {
    let traced = run(CHANGING_DIVISOR, "5");
    let [spec] = traced.value_specializations.as_slice() else {
        panic!("one loop was specialized: {traced:?}");
    };
    assert_eq!(
        spec.guards.iter().map(|g| g.1).collect::<Vec<_>>(),
        vec![ScalarValue::Int(5)],
        "the first entry's divisor: {spec:?}"
    );
    assert_eq!(spec.misses, 8, "one miss for each entry with a new divisor, until the region gave way: {spec:?}");
    assert_eq!(spec.generic_entries, 8, "the loop's generic region ran each of those entries: {spec:?}");
    assert!(spec.replaced, "the generic region replaced it: {spec:?}");
}

const ALTERNATING_DIVISOR: &str = "\
Let mutable total be 0.
Let mutable pass be 0.
While pass is less than 40:
    Let d be k + pass % 2.
    Let mutable i be 0 - 300.
    While i is less than 300:
        Set total to total + (i * 31) % d.
        Set i to i + 1.
    Set pass to pass + 1.
Show total.
";

#[test]
fn a_miss_counts_the_entry_not_each_crossing() {
    // The divisor is k on even passes and k + 1 on odd ones. The value cannot change inside a pass, so a pass that
    // finds k + 1 is one miss, not one per iteration: the specialized region keeps running every even pass until the
    // odd passes have missed as many times as it takes to replace it.
    let traced = run(ALTERNATING_DIVISOR, "5");
    let [spec] = traced.value_specializations.as_slice() else {
        panic!("one loop was specialized: {traced:?}");
    };
    assert_eq!(spec.guards.iter().map(|g| g.1).collect::<Vec<_>>(), vec![ScalarValue::Int(5)]);
    assert_eq!(spec.misses, 20, "one miss for each odd pass: {spec:?}");
    assert!(spec.entries >= 19, "the even passes kept running the specialized region: {spec:?}");
    assert!(!spec.replaced, "misses that never outnumber the entries it ran do not retire it: {spec:?}");
    assert_eq!(spec.generic_entries, 20, "the loop's generic region ran every odd pass: {spec:?}");
}

#[test]
fn largo_opts_reports_each_specialized_loop() {
    let dir = tempfile::tempdir().expect("temp dir");
    let source = format!("{NATIVES}## Main\nLet k be parseInt(item 2 of args()).\n{REMAINDER_LOOP}");
    std::fs::write(dir.path().join("divisor.lg"), source).expect("write source");
    let out = largo_in(dir.path(), &["opts", "divisor.lg", "--jit", "--", "7"]);
    assert!(out.status.success(), "`largo opts --jit` succeeds:\n{}", stderr(&out));
    let text = stdout(&out);
    let line = text
        .lines()
        .find(|l| l.contains("specialized on") && l.contains("= 7"))
        .unwrap_or_else(|| panic!("a line names the loop specialized on 7:\n{text}"));
    assert!(line.contains("0 value-guard misses"), "k never changed: {line}");

    let out = largo_in(dir.path(), &["opts", "divisor.lg", "--jit", "--json", "--", "7"]);
    assert!(out.status.success(), "`largo opts --jit --json` succeeds:\n{}", stderr(&out));
    let report: serde_json::Value = serde_json::from_str(stdout(&out).trim()).expect("the report is JSON");
    let specs = report["jit_native"]["value_specializations"].as_array().expect("a list of specialized loops");
    assert_eq!(specs.len(), 1, "{report}");
    assert_eq!(specs[0]["guards"][0]["value"], 7, "{report}");
    assert_eq!(specs[0]["misses"], 0, "{report}");
    assert_eq!(specs[0]["replaced"], false, "{report}");
}

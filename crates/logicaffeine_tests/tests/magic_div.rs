//! W24 RED gate: constant-divisor `%`/`/` → magic-reciprocal multiply.
//!
//! A constant divisor `c > 0` that is NOT a power of two (W5 handles pow2) is
//! lowered, when the Oracle proves the dividend NON-NEGATIVE, to the
//! Granlund-Montgomery / libdivide unsigned magic sequence
//! (`q = mulhi_u(M, x); [add-fixup;] q >>= s`) — a ~3-cycle `mul`+`shr` instead
//! of `idiv`'s ~25-cycle latency. The remainder is derived as `x - q*c`
//! (wrapping), bit-exact with the kernel's `wrapping_rem` for non-negative `x`.
//!
//! Three layers are pinned, all bit-identical to the tree-walker oracle:
//! - the op-level math (`MagicDivU` in the VM, the regalloc backend, and
//!   `reference_eval`) is exhaustively gridded across the full non-negative i64
//!   range for several divisors — esp. boundaries near multiples of `c`, near
//!   powers of two, and near `i64::MAX`;
//! - the run-path rewrite FIRES on the matrix_mult/histogram shapes and the
//!   output matches the raw tree-walker;
//! - a possibly-negative dividend and a non-constant divisor stay `idiv`
//!   (`Op::Mod`/`Op::Div`) — the magic identity is unsound there.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::tw_outcome_with_args;
use logicaffeine_compile::ui_bridge::with_optimized_program;
use logicaffeine_compile::vm::{Compiler, NativeTier, Op};
use logicaffeine_forge::jit::{reference_eval, ChainOutcome, MicroOp};
use logicaffeine_forge::regalloc::compile_region_regalloc;
use logicaffeine_jit::ForgeTier;
use std::sync::atomic::AtomicI64;
use std::sync::Arc;

fn norm(s: &str) -> String {
    s.lines()
        .map(|l| l.trim_end())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------------------
// Layer 1 — the op math: regalloc native == reference_eval == kernel semantics.
//
// The single source of truth for the magic constants is the COMPILER's own
// generator (re-exported); the test never hand-rolls them, so a test green also
// proves the generator the compiler ships produces exact constants.
// ---------------------------------------------------------------------------

use logicaffeine_compile::vm::compiler::magic_u64_gen;

/// `n / c` and `n % c` via the kernel's locked wrapping-i64 spec (for the
/// non-negative `n` this lever targets these equal mathematical floor/rem).
fn kernel_div(n: i64, c: i64) -> i64 {
    n.wrapping_div(c)
}
fn kernel_mod(n: i64, c: i64) -> i64 {
    n.wrapping_rem(c)
}

/// Run a single `MagicDivU` op on the contiguous regalloc backend and return
/// its result, asserting it agrees with `reference_eval` (the differential
/// ground truth the whole JIT is gated against).
fn run_magic_op(n: i64, magic: u64, more: u8, mul_back: i64) -> i64 {
    let ops = vec![
        MicroOp::MagicDivU { dst: 1, lhs: 0, magic, more, mul_back },
        MicroOp::Return { src: 1 },
    ];
    // reference_eval is the portable interpreter the corpus differential trusts.
    let mut rf = vec![n, 0i64];
    let reference = reference_eval(&ops, &mut rf, 1_000).expect("reference evaluates MagicDivU");
    // The native contiguous backend must produce the IDENTICAL value.
    let status = Arc::new(AtomicI64::new(0));
    let chain = compile_region_regalloc(&ops, Some(status)).expect("MagicDivU compiles native");
    let mut f = vec![n, 0i64];
    match chain.run_with_frame(&mut f) {
        ChainOutcome::Return(v) => {
            assert_eq!(v, reference, "native regalloc MagicDivU != reference_eval (n={n})");
            v
        }
        other => panic!("MagicDivU region did not return: {other:?} (n={n})"),
    }
}

/// Exhaustively grid `n / c` and `n % c` for one non-pow2 divisor across every
/// boundary that could expose an off-by-one in the magic constants, plus a
/// dense band around many multiples of `c` and a large pseudo-random sweep.
fn assert_magic_exact_for(c: i64) {
    assert!(c > 0 && (c & (c - 1)) != 0, "test divisor must be a positive non-pow2");
    let (magic, more) = magic_u64_gen(c as u64);

    let check = |n: i64| {
        assert!(n >= 0, "grid must stay non-negative (n={n})");
        // Division: mul_back == 0.
        let q = run_magic_op(n, magic, more, 0);
        assert_eq!(q, kernel_div(n, c), "magic `/` wrong: {n} / {c}");
        // Modulo: mul_back == c.
        let r = run_magic_op(n, magic, more, c);
        assert_eq!(r, kernel_mod(n, c), "magic `%` wrong: {n} % {c}");
    };

    // Boundaries near 0, near c, near 2c.
    for n in [0, 1, c - 1, c, c + 1, 2 * c - 1, 2 * c, 2 * c + 1] {
        check(n);
    }
    // Near every non-negative power of two (and ±1).
    for k in 0..63 {
        let p = 1i64 << k;
        check(p);
        check(p - 1);
        if p < i64::MAX {
            check(p + 1);
        }
    }
    // Near i64::MAX (the top of the proven non-negative range).
    check(i64::MAX);
    check(i64::MAX - 1);
    check(i64::MAX - c);
    // A dense band around several multiples of c (where carries flip).
    for mult in [1i64, 2, 3, 1000, 1_000_000, (i64::MAX) / c] {
        let center = mult.saturating_mul(c);
        for off in 0..1500i64 {
            check(center.saturating_add(off));
            check(center.saturating_sub(off).max(0));
        }
    }
    // Large pseudo-random sweep, masked into the non-negative range.
    let mut s = 0x9E3779B97F4A7C15u64 ^ (c as u64);
    for _ in 0..200_000 {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        check((s & (i64::MAX as u64)) as i64);
    }
}

#[test]
fn magic_op_exact_div_1000000007() {
    assert_magic_exact_for(1_000_000_007);
}
#[test]
fn magic_op_exact_div_1000() {
    assert_magic_exact_for(1000);
}
#[test]
fn magic_op_exact_div_7() {
    assert_magic_exact_for(7);
}
#[test]
fn magic_op_exact_div_100() {
    assert_magic_exact_for(100);
}
#[test]
fn magic_op_exact_div_3_and_11_and_65521() {
    assert_magic_exact_for(3);
    assert_magic_exact_for(11);
    assert_magic_exact_for(65521);
}

// ---------------------------------------------------------------------------
// Layer 2 — run-path integration: the rewrite fires and matches the oracle.
// ---------------------------------------------------------------------------

/// Optimized program → tiered VM (private tier), plus the structural op tally.
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

fn assert_runpath_matches_raw(src: &str, argv: &[String]) {
    let (out, err) = runpath_vm_outcome(src, argv);
    let tw = tw_outcome_with_args(src, argv);
    assert_eq!(err, None, "run-path optimized VM errored:\n{src}");
    assert_eq!(
        (norm(&out), &err),
        (norm(&tw.output), &tw.error),
        "run-path VM diverged from the raw tree-walker on:\n{src}"
    );
}

struct OpTally {
    mods: usize,
    divs: usize,
    magics: usize,
}

fn tally_ops(src: &str) -> OpTally {
    with_optimized_program(src, |parsed, interner| {
        let (stmts, types, _policies) = parsed.expect("program parses");
        let oracle = logicaffeine_compile::optimize::oracle_analyze_with(stmts, interner);
        let program = Compiler::compile_with_oracle(stmts, interner, Some(types), Some(oracle))
            .expect("program compiles");
        let mut t = OpTally { mods: 0, divs: 0, magics: 0 };
        for op in &program.code {
            match op {
                Op::Mod { .. } => t.mods += 1,
                Op::Div { .. } => t.divs += 1,
                Op::MagicDivU { .. } => t.magics += 1,
                _ => {}
            }
        }
        t
    })
}

/// The matrix_mult inner shape: a non-negative accumulator reduced `% c` where
/// `c = 1000000007` is a compile-time constant (and NOT a power of two). The
/// `% c` must become a `MagicDivU`, no `Mod` left, and the answer is exact.
const MATRIX_MOD_SHAPE: &str = "\
## Main
Let mutable acc be 0.
Let mutable i be 0.
While i is less than 5000:
    Set acc to (acc + i * 7) % 1000000007.
    Set i to i + 1.
Show acc.
";

#[test]
fn magic_matrix_mod_matches_oracle() {
    assert_runpath_matches_raw(MATRIX_MOD_SHAPE, &[]);
}

#[test]
fn magic_matrix_mod_rewrite_fires() {
    let t = tally_ops(MATRIX_MOD_SHAPE);
    assert_eq!(
        t.mods, 0,
        "the proven-non-negative `% 1000000007` must NOT stay a Mod (found {} Mod, {} MagicDivU)",
        t.mods, t.magics
    );
    assert!(
        t.magics >= 1,
        "the proven-non-negative constant `% 1000000007` must become a MagicDivU \
         (found {} MagicDivU, {} Mod)",
        t.magics, t.mods
    );
}

/// histogram's `% 1000` (after the LCG `% 2^k` feedback) — a non-negative,
/// constant, non-pow2 modulus. It must lower to MagicDivU and stay exact.
const HISTOGRAM_TAIL: &str = "\
## Main
Let mutable acc be 0.
Let mutable seed be 42.
Let mutable i be 0.
While i is less than 4000:
    Set seed to (seed * 1103515245 + 12345) % 2147483648.
    Let v be ((seed / 65536) % 32768) % 1000.
    Set acc to acc + v.
    Set i to i + 1.
Show acc.
";

#[test]
fn magic_histogram_tail_matches_oracle() {
    assert_runpath_matches_raw(HISTOGRAM_TAIL, &[]);
}

#[test]
fn magic_histogram_tail_rewrite_fires() {
    let t = tally_ops(HISTOGRAM_TAIL);
    assert!(
        t.magics >= 1,
        "histogram's non-negative `% 1000` must become a MagicDivU (found {} MagicDivU)",
        t.magics
    );
    assert_eq!(t.mods, 0, "no Mod should survive (the `% 2^k` become AND, `% 1000` magic)");
}

/// Constant-divisor DIVISION on a non-negative dividend must also lower.
const NONNEG_DIV_SHAPE: &str = "\
## Main
Let mutable acc be 0.
Let mutable i be 0.
While i is less than 5000:
    Let q be (i * 13 + 7) / 1000.
    Set acc to acc + q.
    Set i to i + 1.
Show acc.
";

#[test]
fn magic_nonneg_div_matches_oracle() {
    assert_runpath_matches_raw(NONNEG_DIV_SHAPE, &[]);
}

#[test]
fn magic_nonneg_div_rewrite_fires() {
    let t = tally_ops(NONNEG_DIV_SHAPE);
    assert_eq!(t.divs, 0, "the proven-non-negative `/ 1000` must NOT stay a Div");
    assert!(t.magics >= 1, "the proven-non-negative constant `/ 1000` must become a MagicDivU");
}

// ---------------------------------------------------------------------------
// Layer 3 — soundness gates: what must STAY idiv.
// ---------------------------------------------------------------------------

/// A possibly-negative dividend (`i - 50` spans `[-50, 49]`): `x % c` for
/// negative `x` is negative (truncated remainder), so the unsigned magic is
/// UNSOUND. The `Mod` must survive and the output must match the tree-walker.
const POSSIBLY_NEG: &str = "\
## Main
Let mutable acc be 0.
Let mutable i be 0.
While i is less than 100:
    Let x be i - 50.
    Let r be x % 7.
    Set acc to acc + r.
    Set i to i + 1.
Show acc.
";

#[test]
fn magic_possibly_negative_stays_idiv() {
    let t = tally_ops(POSSIBLY_NEG);
    assert_eq!(
        t.magics, 0,
        "a possibly-negative dividend's `% 7` MUST NOT become MagicDivU (unsound for x<0)"
    );
    assert_eq!(t.mods, 1, "the possibly-negative `% 7` must stay a Mod");
    assert_runpath_matches_raw(POSSIBLY_NEG, &[]);
}

/// A non-constant divisor (`d` is a runtime value): no compile-time magic
/// constant exists, so it must stay `idiv`.
const NON_CONST_DIVISOR: &str = "\
## Main
Let mutable acc be 0.
Let mutable i be 1.
While i is less than 100:
    Let d be i + 3.
    Let r be (i * 100) % d.
    Set acc to acc + r.
    Set i to i + 1.
Show acc.
";

#[test]
fn magic_non_constant_divisor_stays_idiv() {
    let t = tally_ops(NON_CONST_DIVISOR);
    assert_eq!(t.magics, 0, "a non-constant divisor cannot lower to MagicDivU");
    assert!(t.mods >= 1, "the variable-divisor `%` must stay a Mod");
    assert_runpath_matches_raw(NON_CONST_DIVISOR, &[]);
}

/// A power-of-two divisor is W5's territory (AND / shift), NOT magic — magic
/// must not steal it.
const POW2_DIVISOR: &str = "\
## Main
Let mutable acc be 0.
Let mutable i be 0.
While i is less than 100:
    Let r be (i * 7 + 3) % 8.
    Let q be (i * 7 + 3) / 16.
    Set acc to acc + r + q.
    Set i to i + 1.
Show acc.
";

#[test]
fn magic_pow2_divisor_left_to_w5() {
    let t = tally_ops(POW2_DIVISOR);
    assert_eq!(t.magics, 0, "powers of two are W5's AND/shift lever, not magic");
    assert_runpath_matches_raw(POW2_DIVISOR, &[]);
}

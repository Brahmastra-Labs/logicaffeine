//! Run-path `x % 2^k → x & (2^k-1)` strength reduction (WS-H / W5-compile).
//!
//! On a two's-complement i64 the truncated remainder `x % 2^k` equals the
//! masked low bits `x & (2^k-1)` ONLY when `x` is non-negative (for `x < 0`
//! the remainder is negative, the mask is not). The run path therefore lowers
//! `Modulo` by a literal power of two to the register-form `AndEager` (a 1-cycle
//! AND, which the JIT lowers to a `BitAnd` stencil instead of a `Mod` idiv)
//! ONLY when the Oracle proves the dividend non-negative — exactly the gate the
//! AOT e-graph's `mod-pow2-and` rule already uses.
//!
//! histogram's LCG hot loop is the motivating case:
//! `seed = (seed * 1103515245 + 12345) % 2147483648` (`% 2^31`). The `% 2^31`
//! feedback bounds `seed ∈ [0, 2^31)`, and `seed*K + 12345 < i64::MAX`, so the
//! dividend stays non-negative across the loop back-edge — the Oracle proves it,
//! and the idiv becomes an AND.
//!
//! Every test pins the sacred differential contract (`vm_outcome ==
//! tw_outcome`, bit-identical to the unoptimized tree-walker oracle) AND a
//! structural check on the compiled bytecode (the rewrite fired / did not fire).

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::tw_outcome_with_args;
use logicaffeine_compile::ui_bridge::with_optimized_program;
use logicaffeine_compile::vm::{Compiler, NativeTier, Op};
use logicaffeine_jit::ForgeTier;

fn norm(s: &str) -> String {
    s.lines()
        .map(|l| l.trim_end())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Run `src` through the REAL run path (`optimize_for_run` → Oracle → tiered
/// VM), with a private tier so its compile counters stay isolated.
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

/// Counts of `Mod` and `AndEager` ops across the WHOLE compiled program (every
/// function body is emitted into the single `program.code` array, indexed by
/// `entry_pc`), compiled through the exact run path: `optimize_for_run` residual
/// → `oracle_analyze_with` on that residual → `compile_with_oracle`. This is
/// byte-for-byte what `interpret_for_ui_sync` hands the VM/JIT.
struct OpTally {
    mods: usize,
    ands: usize,
    magics: usize,
}

fn tally_ops(src: &str) -> OpTally {
    with_optimized_program(src, |parsed, interner| {
        let (stmts, types, _policies) = parsed.expect("program parses");
        let oracle = logicaffeine_compile::optimize::oracle_analyze_with(stmts, interner);
        let program = Compiler::compile_with_oracle(stmts, interner, Some(types), Some(oracle))
            .expect("program compiles");
        let mut mods = 0usize;
        let mut ands = 0usize;
        let mut magics = 0usize;
        for op in &program.code {
            match op {
                Op::Mod { .. } => mods += 1,
                Op::AndEager { .. } => ands += 1,
                Op::MagicDivU { .. } => magics += 1,
                _ => {}
            }
        }
        OpTally { mods, ands, magics }
    })
}

// ---------------------------------------------------------------------------
// POSITIVE — proven non-negative dividend: rewrite FIRES
// ---------------------------------------------------------------------------

/// The exact histogram LCG: the `% 2^31` (and the inner `% 2^15`) dividends are
/// proven non-negative ACROSS the loop back-edge, so the idiv lowers to an AND.
/// `seed / 65536` is `/ 2^16` (already a DivPow2). The remaining `% 1000` is NOT
/// a power of two, so its `Mod` must survive.
const HISTOGRAM_LCG: &str = "\
## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int

## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Let mutable counts be a new Seq of Int.
Let mutable idx be 0.
While idx is less than 1000:
    Push 0 to counts.
    Set idx to idx + 1.
Let mutable seed be 42.
Let mutable i be 0.
While i is less than n:
    Set seed to (seed * 1103515245 + 12345) % 2147483648.
    Let v be ((seed / 65536) % 32768) % 1000.
    Set item (v + 1) of counts to (item (v + 1) of counts) + 1.
    Set i to i + 1.
Let mutable maxFreq be 0.
Let mutable maxIndex be 0.
Let mutable distinct be 0.
Set i to 0.
While i is less than 1000:
    If item (i + 1) of counts is greater than 0:
        Set distinct to distinct + 1.
    If item (i + 1) of counts is greater than maxFreq:
        Set maxFreq to item (i + 1) of counts.
        Set maxIndex to i.
    Set i to i + 1.
Show \"\" + maxFreq + \" \" + maxIndex + \" \" + distinct.
";

/// Differential: the histogram LCG, run-path-optimized, matches the raw
/// tree-walker exactly (the `% 2^k → &` rewrite preserves every bit because
/// the dividends are non-negative).
#[test]
fn modpow2_histogram_lcg_matches_oracle() {
    let (out, err) = runpath_vm_outcome(HISTOGRAM_LCG, &["bench".into(), "20000".into()]);
    let tw = tw_outcome_with_args(HISTOGRAM_LCG, &["bench".into(), "20000".into()]);
    assert_eq!(err, None, "histogram LCG errored on the run path");
    assert_eq!(
        (norm(&out), &err),
        (norm(&tw.output), &tw.error),
        "histogram LCG VM diverged from the tree-walker"
    );
}

/// Structural: the run path must turn histogram's two power-of-two `%` (`% 2^31`
/// and `% 2^15`) into `AndEager`, while the non-pow2 `% 1000` (proven
/// non-negative) lowers to the W24 magic reciprocal `MagicDivU`. So NO `Mod`
/// survives, at least two `AndEager` appear (the pow2 rewrite), and at least one
/// `MagicDivU` appears (the non-pow2 magic rewrite).
#[test]
fn modpow2_histogram_lcg_rewrite_fires() {
    let t = tally_ops(HISTOGRAM_LCG);
    assert_eq!(
        t.mods, 0,
        "histogram: both `% 2^k` become AndEager and the non-pow2 `% 1000` \
         becomes MagicDivU, so NO Mod should survive (found {} Mod, {} AndEager, \
         {} MagicDivU)",
        t.mods, t.ands, t.magics
    );
    assert!(
        t.ands >= 2,
        "histogram: the two power-of-two `%` should each become an AndEager \
         (found {} AndEager, {} Mod)",
        t.ands, t.mods
    );
    assert!(
        t.magics >= 1,
        "histogram: the non-pow2 `% 1000` should become a MagicDivU \
         (found {} MagicDivU, {} Mod)",
        t.magics, t.mods
    );
}

/// A minimal proven-non-negative dividend: a literal modulus on a value the
/// Oracle bounds non-negative. The single `% 8` must become an AndEager, no Mod
/// left, and the answer is exact.
const SIMPLE_NONNEG: &str = "\
## Main
Let mutable acc be 0.
Let mutable i be 0.
While i is less than 100:
    Let r be (i * 7 + 3) % 8.
    Set acc to acc + r.
    Set i to i + 1.
Show acc.
";

#[test]
fn modpow2_simple_nonneg_matches_oracle() {
    assert_runpath_matches_raw(SIMPLE_NONNEG, &[], &{
        let mut acc = 0i64;
        let mut i = 0i64;
        while i < 100 {
            acc += (i * 7 + 3) % 8;
            i += 1;
        }
        acc.to_string()
    });
}

#[test]
fn modpow2_simple_nonneg_rewrite_fires() {
    let t = tally_ops(SIMPLE_NONNEG);
    assert_eq!(t.mods, 0, "the proven-non-negative `% 8` must NOT stay a Mod");
    assert!(t.ands >= 1, "the proven-non-negative `% 8` must become an AndEager");
}

// ---------------------------------------------------------------------------
// NEGATIVE — possibly-negative dividend: rewrite does NOT fire
// ---------------------------------------------------------------------------

/// The dividend can be negative (`i - 50` spans `[-50, 49]`), so `x % 8 != x & 7`
/// for the negative half. The rewrite MUST NOT fire — the `Mod` must survive,
/// and the output must equal the tree-walker (truncated remainder, signed).
const POSSIBLY_NEG: &str = "\
## Main
Let mutable acc be 0.
Let mutable i be 0.
While i is less than 100:
    Let x be i - 50.
    Let r be x % 8.
    Set acc to acc + r.
    Set i to i + 1.
Show acc.
";

#[test]
fn modpow2_possibly_negative_matches_oracle() {
    assert_runpath_matches_raw(POSSIBLY_NEG, &[], &{
        let mut acc = 0i64;
        let mut i = 0i64;
        while i < 100 {
            let x = i - 50;
            acc += x % 8;
            i += 1;
        }
        acc.to_string()
    });
}

#[test]
fn modpow2_possibly_negative_does_not_fire() {
    let t = tally_ops(POSSIBLY_NEG);
    assert_eq!(
        t.mods, 1,
        "a possibly-negative dividend's `% 8` MUST stay a Mod (rewrite is \
         UNSOUND for negative x): found {} Mod, {} AndEager",
        t.mods, t.ands
    );
}

/// A non-power-of-two modulus is NOT eligible for the pow2 AND identity, but on
/// a proven non-negative dividend the W24 magic-reciprocal lever DOES lower it
/// to `MagicDivU` (the pow2 `AndEager` rewrite must still leave it alone — the
/// AND identity holds for powers of two only). So no `AndEager` and no `Mod`,
/// exactly one `MagicDivU`, and the answer stays exact.
const NONNEG_NON_POW2: &str = "\
## Main
Let mutable acc be 0.
Let mutable i be 0.
While i is less than 100:
    Let r be (i * 7 + 3) % 10.
    Set acc to acc + r.
    Set i to i + 1.
Show acc.
";

#[test]
fn modpow2_nonneg_non_pow2_uses_magic_not_and() {
    let t = tally_ops(NONNEG_NON_POW2);
    assert_eq!(t.ands, 0, "`% 10` is not a power of two — the AND rewrite must not fire");
    assert_eq!(t.mods, 0, "the proven-non-negative `% 10` must NOT stay a Mod (W24 magic fires)");
    assert_eq!(t.magics, 1, "the proven-non-negative non-pow2 `% 10` lowers to MagicDivU");
    assert_runpath_matches_raw(NONNEG_NON_POW2, &[], &{
        let mut acc = 0i64;
        let mut i = 0i64;
        while i < 100 {
            acc += (i * 7 + 3) % 10;
            i += 1;
        }
        acc.to_string()
    });
}

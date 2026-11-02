//! FORGE-JIT CORRECTNESS GAUNTLET — the copy-and-patch JIT must compute EVERY hot program byte-
//! identically to the tree-walker. The suite otherwise never exercises the process-wide forge tier
//! (which is why a list-element-read miscompile slipped through), so this is the systematic net:
//! a battery of RISKY patterns — list/map element reads inside hot loops, early `Return`s and `Break`s
//! out of loops, list parameters forwarded to helpers, recursion over lists, nested loops, text-byte
//! scans — each run under the forge tier and asserted equal to the tree-walker reference.
//!
//! Every case runs its hot function ≥ the tier-up threshold so the native body is what executes. A
//! divergence here is a JIT soundness bug; the fixes live in `logicaffeine_jit` (decline the pattern
//! to the correct VM bytecode until the codegen is corrected).

use logicaffeine_compile::compile::{tw_outcome_with_args, vm_outcome_with_args};
use logicaffeine_compile::vm::NativeTier;
use logicaffeine_jit::ForgeTier;

fn norm(s: &str) -> String {
    s.trim().to_string()
}

/// Assert `src` computes identically on the forge JIT and the tree-walker.
fn forge_eq_tw(label: &str, src: &str) {
    let tw = tw_outcome_with_args(src, &[]);
    let tier = ForgeTier::new();
    let forge = vm_outcome_with_args(src, &[], Some(&tier as &dyn NativeTier));
    assert!(
        tw.error.is_none(),
        "[{label}] tree-walker errored: {:?}\n{src}",
        tw.error
    );
    assert!(
        forge.error.is_none(),
        "[{label}] forge JIT errored: {:?}\n{src}",
        forge.error
    );
    assert_eq!(
        norm(&forge.output),
        norm(&tw.output),
        "[{label}] forge JIT diverged from the tree-walker:\n{src}"
    );
}

/// A hot driver: define `helper`, call it `n` times, `Show` an accumulator over its results.
fn driver(helper: &str, call: &str) -> String {
    format!(
        "{helper}\n## Main\nLet mutable acc be 0.\nLet mutable k be 0.\nWhile k is less than 500:\n\
        \x20   Set acc to acc + ({call}).\n\
        \x20   Set k to k + 1.\nShow acc.\n"
    )
}

#[test]
fn forge_list_read_early_return() {
    forge_eq_tw(
        "list read + early return",
        &driver(
            "## To firstGE (xs: Seq of Int, t: Int) -> Int:\n\
            \x20   Let n be length of xs.\n\
            \x20   Let mutable i be 1.\n\
            \x20   While i is at most n:\n\
            \x20       If (item i of xs) is at least t:\n\
            \x20           Return i.\n\
            \x20       Set i to i + 1.\n\
            \x20   Return 0.\n\
            Let xs be [3, 1, 4, 1, 5, 9, 2, 6].",
            "firstGE(xs, 5)",
        ),
    );
}

#[test]
fn forge_list_read_no_early_return() {
    forge_eq_tw(
        "list read, no early return (must stay JIT-correct)",
        &driver(
            "## To sumList (xs: Seq of Int) -> Int:\n\
            \x20   Let n be length of xs.\n\
            \x20   Let mutable s be 0.\n\
            \x20   Let mutable i be 1.\n\
            \x20   While i is at most n:\n\
            \x20       Set s to s + (item i of xs).\n\
            \x20       Set i to i + 1.\n\
            \x20   Return s.\n\
            Let xs be [3, 1, 4, 1, 5, 9, 2, 6].",
            "sumList(xs)",
        ),
    );
}

#[test]
fn forge_two_list_params_early_return() {
    forge_eq_tw(
        "two list params + early return (matchAt shape)",
        &driver(
            "## To matchAt2 (sb: Seq of Int, pb: Seq of Int, at: Int) -> Int:\n\
            \x20   Let np be length of pb.\n\
            \x20   Let mutable j be 1.\n\
            \x20   While j is at most np:\n\
            \x20       If (item (at + j - 1) of sb) is equal to (item j of pb):\n\
            \x20           Set j to j + 1.\n\
            \x20       Otherwise:\n\
            \x20           Return 0.\n\
            \x20   Return 1.\n\
            Let sb be [7, 7, 6, 5, 7, 7, 6, 6].\nLet pb be [7, 7, 6].",
            "matchAt2(sb, pb, 5)",
        ),
    );
}

#[test]
fn forge_list_read_break() {
    forge_eq_tw(
        "list read + Break out of loop",
        &driver(
            "## To firstGEbreak (xs: Seq of Int, t: Int) -> Int:\n\
            \x20   Let n be length of xs.\n\
            \x20   Let mutable i be 1.\n\
            \x20   Let mutable res be 0.\n\
            \x20   While i is at most n:\n\
            \x20       If (item i of xs) is at least t:\n\
            \x20           Set res to i.\n\
            \x20           Break.\n\
            \x20       Set i to i + 1.\n\
            \x20   Return res.\n\
            Let xs be [3, 1, 4, 1, 5, 9, 2, 6].",
            "firstGEbreak(xs, 5)",
        ),
    );
}

#[test]
fn forge_nested_loop_list_read_early_return() {
    forge_eq_tw(
        "nested loops, list read, early return",
        &driver(
            "## To hasPair (xs: Seq of Int, target: Int) -> Int:\n\
            \x20   Let n be length of xs.\n\
            \x20   Let mutable i be 1.\n\
            \x20   While i is at most n:\n\
            \x20       Let mutable j be i + 1.\n\
            \x20       While j is at most n:\n\
            \x20           If ((item i of xs) + (item j of xs)) is equal to target:\n\
            \x20               Return 1.\n\
            \x20           Set j to j + 1.\n\
            \x20       Set i to i + 1.\n\
            \x20   Return 0.\n\
            Let xs be [3, 1, 4, 1, 5, 9, 2, 6].",
            "hasPair(xs, 8)",
        ),
    );
}

#[test]
fn forge_recursion_over_list() {
    forge_eq_tw(
        "recursion with a list parameter + early return",
        &driver(
            "## To findRec (xs: Seq of Int, i: Int, t: Int) -> Int:\n\
            \x20   If i is greater than (length of xs):\n\
            \x20       Return 0.\n\
            \x20   If (item i of xs) is equal to t:\n\
            \x20       Return i.\n\
            \x20   Return findRec(xs, i + 1, t).\n\
            Let xs be [3, 1, 4, 1, 5, 9, 2, 6].",
            "findRec(xs, 1, 9)",
        ),
    );
}

#[test]
fn forge_map_read_in_hot_loop() {
    forge_eq_tw(
        "map read (guarded) in a hot loop",
        &driver(
            "## To lookupSum (m: Map of Int to Int, keys: Seq of Int) -> Int:\n\
            \x20   Let n be length of keys.\n\
            \x20   Let mutable s be 0.\n\
            \x20   Let mutable i be 1.\n\
            \x20   While i is at most n:\n\
            \x20       Let key be item i of keys.\n\
            \x20       If m contains key:\n\
            \x20           Set s to s + (item key of m).\n\
            \x20       Set i to i + 1.\n\
            \x20   Return s.\n\
            Let mutable m be a new Map of Int to Int.\n\
            Set item 1 of m to 10.\nSet item 3 of m to 30.\n\
            Let keys be [1, 2, 3, 4].",
            "lookupSum(m, keys)",
        ),
    );
}

#[test]
fn forge_text_byte_scan_early_return() {
    forge_eq_tw(
        "text-byte scan with early return (indexOf shape)",
        &driver(
            "## To idxByte (s: Text, b: Int) -> Int:\n\
            \x20   Let sb be text_bytes(s).\n\
            \x20   Let n be length of sb.\n\
            \x20   Let mutable i be 1.\n\
            \x20   While i is at most n:\n\
            \x20       If (item i of sb) is equal to b:\n\
            \x20           Return i.\n\
            \x20       Set i to i + 1.\n\
            \x20   Return 0.\n\
            Let s be \"hello world\".",
            "idxByte(s, 119)",
        ),
    );
}

/// The native bit-count builtins (`count_ones`/`leading_zeros`/`trailing_zeros` → `popcnt`/`lzcnt`/
/// `tzcnt`) over EDGE values the ratchet's `k = 1..600` ground-truth never reaches: zero (where
/// `leading_zeros`/`trailing_zeros` must be 64, not the `bsr`/`bsf`-undefined result), and negatives
/// (high bit set → the full u64 two's-complement pattern). Each runs in a hot loop over the
/// register-pinned induction variable so the native stencil — not a VM fallback — is what executes.
#[test]
fn forge_bit_builtins_edge_values() {
    // k = 0..499: the FIRST iteration feeds 0 (count_ones=0, leading_zeros=64, trailing_zeros=64).
    forge_eq_tw("count_ones over k incl. 0", &driver("", "count_ones(k)"));
    forge_eq_tw(
        "leading_zeros over k incl. 0",
        &driver("", "leading_zeros(k)"),
    );
    forge_eq_tw(
        "trailing_zeros over k incl. 0",
        &driver("", "trailing_zeros(k)"),
    );
    // 0 − k = 0, −1, −2, …: high bit set → leading_zeros collapses toward 0, the u64 pattern is exercised.
    forge_eq_tw(
        "count_ones over negatives",
        &driver("", "count_ones(0 - k)"),
    );
    forge_eq_tw(
        "leading_zeros over negatives",
        &driver("", "leading_zeros(0 - k)"),
    );
    forge_eq_tw(
        "trailing_zeros over negatives",
        &driver("", "trailing_zeros(0 - k)"),
    );
}

/// A hot float driver: `fk` starts at `start`, steps by `-step` each of 500 iterations (so it crosses
/// zero when `start`/`step` are chosen to), accumulating `call` into an Int `acc`.
fn fdriver(call: &str, start: &str, step: &str) -> String {
    format!(
        "## Main\nLet mutable acc be 0.\nLet mutable fk be {start}.\nLet mutable k be 0.\n\
        While k is less than 500:\n\x20   Set acc to acc + ({call}).\n\
        \x20   Set fk to fk - {step}.\n\x20   Set k to k + 1.\nShow acc.\n"
    )
}

/// `floor`/`ceil` (Float → Int via `roundsd`+saturating `cvttsd2si`) over EDGE inputs the ratchet's clean
/// ground-truth doesn't stress: NEGATIVE fractions (where `floor` ≠ truncate-toward-zero), exact integers,
/// and OUT-OF-RANGE magnitudes that must SATURATE to `i64::MAX`/`MIN` byte-identically to the VM's `as
/// i64` (not wrap to `i64::MIN` the way a bare `cvttsd2si` would). Each runs hot so the native stencil
/// executes.
#[test]
fn forge_float_round_edge_values() {
    // 250.5 → −249.5: crosses zero, always a .5 fraction → floor/ceil differ from truncation on negatives.
    forge_eq_tw(
        "floor over sign-crossing halves",
        &fdriver("floor(fk)", "250.5", "1.0"),
    );
    forge_eq_tw(
        "ceil over sign-crossing halves",
        &fdriver("ceil(fk)", "250.5", "1.0"),
    );
    // `round` ties AWAY from zero — every fk is an exact .5 tie crossing zero (2.5→3, 0.5→1, −0.5→−1, …),
    // the case a ties-EVEN roundsd gets wrong. Also the quarter-step variant hits non-tie fractions.
    forge_eq_tw(
        "round over sign-crossing halves",
        &fdriver("round(fk)", "250.5", "1.0"),
    );
    forge_eq_tw(
        "round over sign-crossing quarters",
        &fdriver("round(fk)", "250.25", "0.5"),
    );
    // Exact integers (no fraction): floor/ceil/round are the identity; still exercises round+convert.
    forge_eq_tw(
        "floor over exact integers",
        &fdriver("floor(fk)", "250.0", "1.0"),
    );
    forge_eq_tw(
        "ceil over exact integers",
        &fdriver("ceil(fk)", "250.0", "1.0"),
    );
    forge_eq_tw(
        "round over exact integers",
        &fdriver("round(fk)", "250.0", "1.0"),
    );
    // SATURATION (positive): fk grows ×1.3 per iter past 2^63 → floor/ceil must saturate to i64::MAX byte-
    // identically to the VM's `as i64` (a bare `cvttsd2si` would wrap to i64::MIN). `min(_, 1000)` keeps
    // acc bounded yet READS the native saturated result — a wrong i64::MIN would flip the min and diverge.
    let sat_hi = |call: &str| {
        format!(
            "## Main\nLet mutable acc be 0.\nLet mutable fk be 1.0.\nLet mutable k be 0.\n\
            While k is less than 500:\n\x20   Set acc to acc + min({call}, 1000).\n\
            \x20   Set fk to fk * 1.3.\n\x20   Set k to k + 1.\nShow acc.\n"
        )
    };
    // SATURATION (negative): fk = −1.0 grows ×1.3 toward −∞ → saturate to i64::MIN. `max(_, −1000)` bounds
    // acc; a wrong i64::MAX would flip the max and diverge.
    let sat_lo = |call: &str| {
        format!(
            "## Main\nLet mutable acc be 0.\nLet mutable fk be 0.0 - 1.0.\nLet mutable k be 0.\n\
            While k is less than 500:\n\x20   Set acc to acc + max({call}, 0 - 1000).\n\
            \x20   Set fk to fk * 1.3.\n\x20   Set k to k + 1.\nShow acc.\n"
        )
    };
    forge_eq_tw("floor saturates to i64::MAX", &sat_hi("floor(fk)"));
    forge_eq_tw("ceil saturates to i64::MAX", &sat_hi("ceil(fk)"));
    forge_eq_tw("round saturates to i64::MAX", &sat_hi("round(fk)"));
    forge_eq_tw("floor saturates to i64::MIN", &sat_lo("floor(fk)"));
    forge_eq_tw("ceil saturates to i64::MIN", &sat_lo("ceil(fk)"));
    forge_eq_tw("round saturates to i64::MIN", &sat_lo("round(fk)"));
}

/// `pow(base: Int, const_exp)` unrolls to checked Muls that must DEOPT to a BigInt recompute exactly when
/// the VM's `checked_pow` overflows i64. Covers exp 0/1 (identity paths), an in-range exp, an exp large
/// enough to overflow partway (→ BigInt promotion), and negative bases (sign through the Muls). Each runs
/// hot so the native unroll — not a VM fallback — executes.
#[test]
fn forge_pow_edge_values() {
    forge_eq_tw("pow exp 0 (all → 1)", &driver("", "pow(k, 0)"));
    forge_eq_tw("pow exp 1 (identity)", &driver("", "pow(k, 1)"));
    forge_eq_tw("pow exp 2", &driver("", "pow(k, 2)"));
    forge_eq_tw("pow exp 3", &driver("", "pow(k, 3)"));
    // k reaches 499 → 499⁸ ≈ 3.8e21 overflows i64: the checked-Mul chain must deopt to a BigInt recompute.
    forge_eq_tw("pow exp 8 overflows to BigInt", &driver("", "pow(k, 8)"));
    // Negative base via `0 − k`: (−k)³ negative, (−k)² positive — sign carried through the Muls.
    forge_eq_tw("pow of negatives exp 3", &driver("", "pow(0 - k, 3)"));
    forge_eq_tw("pow of negatives exp 2", &driver("", "pow(0 - k, 2)"));
}

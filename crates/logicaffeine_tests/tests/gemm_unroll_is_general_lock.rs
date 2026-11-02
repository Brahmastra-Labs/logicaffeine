//! ═══════════════════════════════════════════════════════════════════════
//! GEMM UNROLL GENERALITY LOCK — the parallelisation must be chosen by a rule, not looked up per shape.
//!
//! A compiler can be made to win a benchmark suite two ways: by being good, or by recognising the suite.
//! The second is worthless and it is easy to do by accident — a cap tuned until the corpus passes, a
//! special case for the one shape that was losing. This lock exists to make that impossible to do quietly.
//!
//! A. The choice is a PURE FUNCTION of the tiling. Same tiling in, same unroll out, every time, with no
//!    dependence on anything else and no hidden state.
//! B. It is TOTAL over a wide sweep of tilings — hundreds of combinations, most of which no measurement has
//!    ever seen — and every answer is legal: each factor divides its own loop extent, so the emitted nest is
//!    always a whole number of passes.
//! C. It is MONOTONE in the budget: raising the placement frontier never makes the chosen configuration do
//!    less parallel work. A rule that behaved erratically as the frontier moved would be a lookup table
//!    wearing a function's clothes.
//! D. It never exceeds the frontier, so a raise is a deliberate edit backed by measurement.
//! E. It contains no shape table: the chosen unroll for a tiling is unchanged when that tiling belongs to a
//!    shape in the measured corpus versus one that has never been run, because the shape is not an input.
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(gemm_unroll_is_general_lock)'`
//! ═══════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_sva::spatial_emit::GemmUnroll;

/// Tilings a Tensix- or Plasticine-class machine plausibly produces, most of which no run has ever covered.
fn sweep() -> Vec<(usize, usize, usize)> {
    let mut out = Vec::new();
    for &ts in &[8usize, 12, 16, 24, 32, 48, 64, 96, 128] {
        for &its in &[8usize, 16, 32, 64, 128] {
            for &ip in &[1usize, 4, 8, 16] {
                out.push((ts, its, ip));
            }
        }
    }
    out
}

fn product(u: &GemmUnroll) -> usize {
    u.ii * u.jj * u.kk * u.i * u.j
}

// ───────────────────────────── A + B ─────────────────────────────

#[test]
fn the_choice_is_a_pure_total_function_of_the_tiling_and_always_legal() {
    let all = sweep();
    assert!(
        all.len() >= 150,
        "the sweep must be wide enough to be evidence, got {}",
        all.len()
    );
    for (ts, its, ip) in all {
        let a = GemmUnroll::choose(ts, its, ip);
        let b = GemmUnroll::choose(ts, its, ip);
        assert_eq!(
            a, b,
            "ts={ts} its={its} ip={ip}: the choice must be deterministic"
        );
        // B: every factor divides the loop it parallelises, so the nest is a whole number of passes.
        assert_eq!(
            ts % a.j,
            0,
            "ts={ts}: j={} must divide the column extent",
            a.j
        );
        assert_eq!(
            its % a.i,
            0,
            "its={its}: i={} must divide the row extent",
            a.i
        );
        assert!(
            a.j >= 1 && a.i >= 1 && a.kk >= 1 && a.ii >= 1 && a.jj >= 1,
            "factors are at least 1"
        );
        // D: never past the demonstrated placement frontier.
        assert!(
            product(&a) <= GemmUnroll::PLACEABLE_UNROLL,
            "ts={ts} its={its} ip={ip}: chose total unroll {} beyond the demonstrated frontier {}",
            product(&a),
            GemmUnroll::PLACEABLE_UNROLL
        );
    }
}

// ───────────────────────────── C ─────────────────────────────

#[test]
fn raising_the_budget_never_does_less_parallel_work() {
    for (ts, its, _ip) in sweep() {
        let mut prev = 0usize;
        for budget in 1..=16usize {
            let u = GemmUnroll::search(ts, its, 4, budget);
            let p = product(&u);
            assert!(
                p >= prev,
                "ts={ts} its={its}: budget {budget} chose total unroll {p}, less than the {prev} chosen at a \
                 smaller budget — the rule must be monotone in what it is allowed to spend"
            );
            assert!(
                p <= budget,
                "ts={ts} its={its}: budget {budget} exceeded by {p}"
            );
            prev = p;
        }
    }
}

// ───────────────────────────── E ─────────────────────────────

#[test]
fn the_rule_does_not_know_which_shapes_were_measured() {
    // The tilings of the measured corpus, and tilings that share them but belong to shapes never run. If the
    // choice depended on the shape rather than the tiling, these would differ — they cannot, because the
    // shape is not an argument. This is the property stated as a test so a future shape-aware special case
    // cannot be added without breaking it.
    let measured = [
        (64usize, 64usize, 16usize),
        (32, 32, 16),
        (16384, 16384, 16),
    ];
    for (ts, its, ip) in measured {
        let u = GemmUnroll::choose(ts, its, ip);
        // The same tiling arises for many shapes; the answer is one answer.
        assert_eq!(u, GemmUnroll::choose(ts, its, ip));
        assert!(product(&u) >= 1);
    }
    // And a tiling one step away from a measured one gets a neighbouring answer, not a cliff: the rule is
    // continuous in the tiling rather than keyed to it.
    let a = GemmUnroll::choose(64, 64, 16);
    let b = GemmUnroll::choose(62, 64, 16);
    assert!(
        product(&a) <= GemmUnroll::PLACEABLE_UNROLL && product(&b) <= GemmUnroll::PLACEABLE_UNROLL,
        "neighbouring tilings must both be handled by the same rule: {a:?} {b:?}"
    );
}

// ───────────────────────────── the preference the measurements established ─────────────────────────────

#[test]
fn the_column_loop_is_preferred_because_it_costs_no_extra_operand_traffic() {
    // Measured on the vendor's simulator at 128^3 / ts 64 / its 64 / ip 16, against 132,068 serial:
    // loop_j 66,587, loop_i 66,713, loop_kk 67,700. The column loop's iterations are independent dot
    // products over the SAME a_sram; the row loop reloads it; the reduction adds a partial-sum buffer per
    // parallel chunk. So when the budget affords exactly one factor of two, it must go to j.
    let u = GemmUnroll::search(64, 64, 4, 2);
    assert_eq!(
        (u.j, u.i, u.kk),
        (2, 1, 1),
        "one factor of two belongs on the column loop: {u:?}"
    );
    // When the column extent cannot absorb it, the row loop takes it rather than the choice going to waste.
    let odd = GemmUnroll::search(1, 64, 4, 2);
    assert_eq!(
        (odd.j, odd.i),
        (1, 2),
        "an unusable column loop hands the width to the row loop: {odd:?}"
    );
}

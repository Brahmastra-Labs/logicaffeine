//! ★ TRANSLINEAR FUNCTION LOCK ★ — exp / ln / tanh as real tiles: monotone certified boxes,
//! unit-scale pinning (a translinear law is not scale-invariant), Lipschitz-priced budgets,
//! and fail-closed domains.
//!
//! What this file locks:
//!   1. UNIT SCALE OR NOTHING — a translinear netlist schedules at `scale_log2 == 0`
//!      (volt ≡ math); rails and domains gate honestly at that scale.
//!   2. CERTIFIED BOXES — the monotone image rules certify through the independent checker,
//!      which enforces the SAME domain gates fail-closed (a log claim whose operand box
//!      touches the nonpositive axis is REJECTED, not clamped).
//!   3. IDEAL ≡ MATH, PHYSICAL WITHIN ε — the ideal tier reproduces `f64::exp/ln/tanh`
//!      exactly; the quantized volt-domain evaluation sits inside the replayed budget.
//!   4. DOSSIER HONESTY — the physics anchors (`U_T = kT/q`, Mead, Vittoz–Fellrath, Gilbert)
//!      are in the dossier; the tile's ideal math semantics stand on the NORMALIZED
//!      translinear law, with bias realization a characterization-ladder concern.
//!
//! ⚠️ YOU DO NOT GET TO WEAKEN THIS FILE. Domains never widen silently; the unit-scale pin
//! is a soundness line.

use logicaffeine_compile::codegen_ana::{
    error_budget, eval_analog_dc, eval_analog_quantized, scale_pass, to_range_claim, ASrc,
    AnaLowerError, AnalogCell, AnalogNetlist, AnalogTarget, AssumedRange,
};
use logicaffeine_proof::interval_replay::{check_range_claim, RCell, RSrc, RangeClaim};

fn unary_netlist(cell: AnalogCell) -> AnalogNetlist {
    AnalogNetlist {
        n_in: 1,
        cells: vec![cell],
        out: ASrc::Val(0),
    }
}

fn range(lo: f64, hi: f64) -> Vec<AssumedRange> {
    vec![AssumedRange {
        var: "x".to_string(),
        lo,
        hi,
        dim: logicaffeine_base::dimension::Dimension::DIMENSIONLESS,
    }]
}

#[test]
fn the_three_functions_schedule_at_unit_scale_certify_and_match_math() {
    let target = AnalogTarget::ideal();
    let cases: [(&str, AnalogCell, f64, f64, fn(f64) -> f64); 3] = [
        (
            "exp",
            AnalogCell::Exp { a: ASrc::In(0) },
            -0.7,
            -0.4,
            f64::exp,
        ),
        ("ln", AnalogCell::Log { a: ASrc::In(0) }, 0.5, 0.7, f64::ln),
        (
            "tanh",
            AnalogCell::Tanh { a: ASrc::In(0) },
            -0.7,
            0.7,
            f64::tanh,
        ),
    ];
    for (name, cell, lo, hi, f) in cases {
        let netlist = unary_netlist(cell);
        let (schedule, cert) = scale_pass(&netlist, &range(lo, hi), &target)
            .unwrap_or_else(|e| panic!("`{name}` schedules: {e}"));
        assert_eq!(
            schedule.scale_log2, 0,
            "`{name}`: translinear pins the unit scale"
        );
        assert!(
            schedule.per_net_scale_log2.iter().all(|e| *e == 0),
            "`{name}`: every net at unit scale"
        );
        let (cells, claim) = to_range_claim(&schedule, &cert);
        assert!(
            check_range_claim(&cells, &claim),
            "`{name}`: the certificate certifies"
        );

        let budget = error_budget(&netlist, &cert, &schedule, &target);
        assert!(budget.recheck(), "`{name}`: the budget replays");
        for k in 0..8 {
            let x = lo + (hi - lo) * (k as f64 / 7.0);
            let ideal = eval_analog_dc(&netlist, &[x]);
            assert_eq!(ideal, f(x), "`{name}`: the ideal tier IS the math at {x}");
            let physical = eval_analog_quantized(&schedule, &target, &[x]);
            assert!(
                (physical - ideal).abs() <= budget.eps_math,
                "`{name}` at {x}: physical {physical} vs ideal {ideal} exceeds ε {}",
                budget.eps_math
            );
        }
    }
}

#[test]
fn domains_fail_closed_at_the_producer_and_the_checker() {
    let target = AnalogTarget::ideal();
    // Producer: a log over a zero-touching box refuses by name.
    let log = unary_netlist(AnalogCell::Log { a: ASrc::In(0) });
    match scale_pass(&log, &range(-0.5, 0.5), &target) {
        Err(AnaLowerError::UnloweredOp(msg)) => assert!(msg.contains("log domain"), "{msg}"),
        other => panic!("log over [−0.5, 0.5] must refuse, got {other:?}"),
    }
    let sqrt = unary_netlist(AnalogCell::Sqrt { a: ASrc::In(0) });
    match scale_pass(&sqrt, &range(-0.1, 0.5), &target) {
        Err(AnaLowerError::UnloweredOp(msg)) => assert!(msg.contains("sqrt domain"), "{msg}"),
        other => panic!("sqrt over a zero-touching box must refuse, got {other:?}"),
    }

    // Checker: the SAME gates, independently — a hand-built claim over a bad box is rejected.
    let cells = vec![RCell::Log { a: RSrc::In(0) }];
    let claim = RangeClaim {
        input_lo: vec![-0.5],
        input_hi: vec![0.5],
        scale_log2: vec![0],
        node_lo_v: vec![-10.0],
        node_hi_v: vec![10.0],
        rail_half_span_v: 100.0,
        noise_floor_v: 0.0,
        min_full_scale_over_noise: 1.0,
    };
    assert!(
        !check_range_claim(&cells, &claim),
        "the checker rejects a nonpositive log box"
    );
}

#[test]
fn the_physics_anchors_live_in_the_dossier() {
    let dossier = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../benchmarks/silicon/analog/sourced-constants.md"
    ))
    .expect("dossier");
    for needle in ["kT/q", "25.85", "Mead", "Vittoz", "Gilbert", "slope factor"] {
        assert!(
            dossier.contains(needle),
            "the dossier must carry `{needle}`"
        );
    }
}

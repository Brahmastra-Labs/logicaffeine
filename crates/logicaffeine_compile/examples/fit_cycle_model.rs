//! Refit the Wormhole cycle model against the measured ttsim ledger and report where it stands.
//!
//! The cost model is what makes the plan choice emergent rather than tabulated, so its constants are a
//! FITTED artefact of the measurement campaign, not literals anyone should hand-tune. This prints the fit,
//! the residual on every point, and every ledger group where the model would rank a measured loser first —
//! the three things that decide whether the pinned constants are still the fit.

use logicaffeine_compile::spatial::cost::{
    load_calibration_points, quantities_for_point, CalibrationPoint, CycleModel,
};
use logicaffeine_compile::spatial::target::SpatialTarget;
use std::collections::BTreeMap;

fn main() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root");
    let path = root.join(logicaffeine_compile::spatial::cost::CALIBRATION_JSON);
    let text = std::fs::read_to_string(&path).expect("the ttsim GEMM ledger exists");
    let points: Vec<CalibrationPoint> = load_calibration_points(&text).expect("the ledger parses");
    let wh = SpatialTarget::wormhole_ttsim();
    let fit_set: Vec<_> = points
        .iter()
        .filter(|p| p.chip == "wormhole_b0" && p.stack != "ttnn_production")
        .filter_map(|p| quantities_for_point(p, &wh).map(|q| (q, p.cycles)))
        .collect();
    let fit_rows: Vec<&CalibrationPoint> = points
        .iter()
        .filter(|p| p.chip == "wormhole_b0" && p.stack != "ttnn_production")
        .filter(|p| quantities_for_point(p, &wh).is_some())
        .collect();
    println!("ledger rows {}, fit set {}", points.len(), fit_set.len());

    let fitted = CycleModel::fit(&fit_set);
    let pinned = CycleModel::wormhole_ttsim();
    println!("\n── constants ── (pinned vs fitted)");
    for ((n, a), (_, b)) in pinned.constants().iter().zip(fitted.constants().iter()) {
        let off = if a.abs() > 1e-9 {
            (a - b).abs() / a.abs() * 100.0
        } else {
            0.0
        };
        println!(
            "  {n:<16} {a:>12.4}  {b:>12.4}   {off:>6.1}%{}",
            if off > 1.0 { "  DRIFT" } else { "" }
        );
    }

    println!("\n── fitted literals (paste into CycleModel::wormhole_ttsim) ──");
    for (n, v) in fitted.constants() {
        println!("            {n}: {v:.10},");
    }

    for (label, m) in [("pinned", &pinned), ("fitted", &fitted)] {
        let mut worst = 0.0f64;
        println!("\n── residuals ({label}) ──");
        for ((q, measured), pt) in fit_set.iter().zip(fit_rows.iter()) {
            let pred = m.cycles(q);
            let err = (pred - *measured as f64).abs() / *measured as f64;
            worst = worst.max(err);
            if err > 0.15 {
                let e = m.explain(q);
                println!(
                    "  {:>7} measured  {pred:>9.0} predicted  {:>5.1}%  {:?} {:<16} {:<24}",
                    measured,
                    err * 100.0,
                    q.problem,
                    pt.strategy,
                    pt.knobs
                );
                println!("          setup {:>7.0}  compute {:>8.0}  move ({:>7.0},{:>7.0})  sync {:>7.0}  fill {:>6.0}  drain {:>5.0}  II {:>7.1} x{:<4} bound {:?}", e.setup, e.compute, e.move_engine.0, e.move_engine.1, e.sync, e.fill, e.drain, e.ii, q.iterations, e.bound);
            }
        }
        println!("  worst {:.1}%", worst * 100.0);
    }

    // Ranking: within a (shape, chip, knobs) group the model must not put a measured loser first.
    let mut groups: BTreeMap<(String, String), Vec<&CalibrationPoint>> = Default::default();
    for p in points
        .iter()
        .filter(|p| p.chip == "wormhole_b0" && p.stack != "ttnn_production")
    {
        groups
            .entry((format!("{:?}", p.shape), p.knobs.clone()))
            .or_default()
            .push(p);
    }
    for (label, m) in [("pinned", &pinned), ("fitted", &fitted)] {
        println!("\n── ranking ({label}) ──");
        let mut bad = 0;
        for ((shape, knobs), rows) in &groups {
            if rows.len() < 2 {
                continue;
            }
            let Some(best) = rows.iter().min_by_key(|p| p.cycles) else {
                continue;
            };
            let Some(qb) = quantities_for_point(best, &wh) else {
                continue;
            };
            for other in rows.iter().filter(|o| o.cycles > best.cycles) {
                let Some(qo) = quantities_for_point(other, &wh) else {
                    continue;
                };
                if m.cycles(&qb) > m.cycles(&qo) + 1e-6 {
                    bad += 1;
                    println!("  {shape} {knobs} INVERTED");
                    for (who, pt, q) in [("winner", best, &qb), ("loser", other, &qo)] {
                        let e = m.explain(q);
                        println!(
                            "    {who:<6} measured {:>7}  predicted {:>8.0}   setup {:>7.0}  compute {:>8.0}  move ({:>7.0},{:>7.0})  sync {:>7.0}  fill {:>6.0}  drain {:>6.0}  II {:>7.1} x{:<5} bound {:?}",
                            pt.cycles, e.total, e.setup, e.compute, e.move_engine.0, e.move_engine.1, e.sync, e.fill, e.drain, e.ii, q.iterations, e.bound
                        );
                        println!("           strategy {:<16} kernels {:<2} sems {:<2} sites {:<3} dram/engine {:?}", pt.strategy, q.kernel_classes, q.n_sems, q.sites_used, q.dram_tiles_engine);
                    }
                }
            }
        }
        println!("  inversions {bad}");
    }
}

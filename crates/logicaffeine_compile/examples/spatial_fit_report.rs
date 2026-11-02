//! Print the ttsim ledger as calibration points, fit the Wormhole cycle model to them, and report the
//! prediction error and the plan decisions the fitted model makes. Usage:
//!   cargo run -p logicaffeine-compile --features codegen --example spatial_fit_report

use logicaffeine_compile::spatial::cost::{
    load_calibration_points, quantities_for_point, CycleModel, CALIBRATION_JSON,
};
use logicaffeine_compile::spatial::ir::NumericsPolicy;
use logicaffeine_compile::spatial::passes::tile::candidates;
use logicaffeine_compile::spatial::pipeline::{compile_to_spatial_with, template_from_source_with};
use logicaffeine_compile::spatial::target::SpatialTarget;

fn gemm_program(m: usize, n: usize, k: usize) -> String {
    format!(
        r#"## Main
Let rows be {m}.
Let cols be {n}.
Let mid be {k}.
Let mutable a be a new Seq of Int.
Let mutable b be a new Seq of Int.
Let mutable c be a new Seq of Int.
Let mutable p be 0.
While p is less than rows * mid:
    Push (p % 7) - 3 to a.
    Set p to p + 1.
Set p to 0.
While p is less than mid * cols:
    Push (p % 11) - 5 to b.
    Set p to p + 1.
Set p to 0.
While p is less than rows * cols:
    Push 0 to c.
    Set p to p + 1.
Let mutable i be 0.
While i is less than rows:
    Let mutable k be 0.
    While k is less than mid:
        Let mutable j be 0.
        While j is less than cols:
            Let idx be i * cols + j + 1.
            Set item idx of c to item idx of c + item (i * mid + k + 1) of a * item (k * cols + j + 1) of b.
            Set j to j + 1.
        Set k to k + 1.
    Set i to i + 1.
Let mutable checksum be 0.
Let mutable q be 1.
While q is at most rows * cols:
    Set checksum to checksum + item q of c.
    Set q to q + 1.
Show checksum.
"#
    )
}

fn main() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let text = std::fs::read_to_string(root.join(CALIBRATION_JSON)).expect("ledger");
    let pts = load_calibration_points(&text).expect("points");
    let wh = SpatialTarget::wormhole_ttsim();
    println!("{} ledger rows", pts.len());
    let mut fit_set = Vec::new();
    for p in &pts {
        let q = quantities_for_point(p, &wh);
        println!(
            "{:<60} {:>8} cyc  {:?}  plan={:?}  quantified={}",
            p.id,
            p.cycles,
            p.knobs,
            p.plan,
            q.is_some()
        );
        if let (Some(q), true) = (q, p.chip == "wormhole_b0") {
            fit_set.push((q, p.cycles));
        }
    }
    let fitted = CycleModel::fit(&fit_set);
    println!("\nFITTED ({} points):", fit_set.len());
    for (name, v) in fitted.constants() {
        println!("  {name}: {v:.10},");
    }
    println!("\nPINNED vs FITTED:");
    let pinned = CycleModel::wormhole_ttsim();
    for ((name, a), (_, b)) in pinned.constants().into_iter().zip(fitted.constants()) {
        let tol = (a.abs() * 0.01).max(1e-9);
        println!(
            "  {name:<14} pinned {a:>10.4} fitted {b:>10.4} {}",
            if (a - b).abs() <= tol {
                "ok"
            } else {
                "DIFFERS"
            }
        );
    }
    println!("\nPREDICTIONS:");
    for p in &pts {
        if let (Some(q), true) = (quantities_for_point(p, &wh), p.chip == "wormhole_b0") {
            let pred = fitted.cycles(&q);
            println!(
                "  {:<60} measured {:>7} predicted {:>9.0} ({:+.1}%)",
                p.id,
                p.cycles,
                pred,
                (pred - p.cycles as f64) / p.cycles as f64 * 100.0
            );
        }
    }
    println!("\nDECISIONS (calibrated preset):");
    let mut shapes: Vec<(usize, usize, usize)> = pts.iter().map(|p| p.shape).collect();
    shapes.sort();
    shapes.dedup();
    for (m, n, k) in shapes {
        for numerics in [NumericsPolicy::Exact, NumericsPolicy::AllowSpillRounding] {
            let src = gemm_program(m, n, k);
            match compile_to_spatial_with(&src, &wh, numerics) {
                Ok(p) => {
                    let s = p.summary();
                    let cands = candidates(
                        &template_from_source_with(&src, &wh, numerics).unwrap(),
                        &wh,
                    );
                    let mut ranked: Vec<_> = cands.iter().collect();
                    ranked.sort_by(|a, b| a.cycles.partial_cmp(&b.cycles).unwrap());
                    println!("  {m}x{n}x{k} {numerics:?}: streaming={} groups={} grid={:?} per_site={:?} block={:?} nb={} acc={:?}", s.streaming, s.multicast_groups, s.grid, s.per_site_block, s.block, s.num_blocks, s.acc);
                    for c in ranked.iter().take(4) {
                        println!("      {:>9.0} cyc  fused={} per_site=({},{}) block=({},{},{}) grid={:?} nb={}", c.cycles, c.fused, c.plan.per_site_m, c.plan.per_site_n, c.plan.block.osh, c.plan.block.osw, c.plan.block.ibw, c.plan.grid, c.plan.inner.num_blocks);
                    }
                }
                Err(e) => println!("  {m}x{n}x{k} {numerics:?}: {e:?}"),
            }
        }
    }
}

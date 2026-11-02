//! The iso-MAC pairs: identical arithmetic, different shape, so every measured difference is movement.
//!
//! A contention term is only real if it explains BOTH pairs without having been fitted to them. This prints
//! the measured gap beside the model's, and the movement quantities that would have to account for it.

use logicaffeine_compile::spatial::cost::quantities_for_gemm;
use logicaffeine_compile::spatial::ir::NumericsPolicy;
use logicaffeine_compile::spatial::pipeline::compile_to_spatial_with;
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
    let wh = SpatialTarget::wormhole_ttsim();
    let pairs = [
        (
            "0.27G",
            (4096usize, 256usize, 256usize),
            6856u64,
            (256usize, 2048usize, 512usize),
            6074u64,
        ),
        ("1.07G", (1024, 512, 2048), 5678, (2048, 512, 1024), 6792),
    ];
    for (mac, a, ca, b, cb) in pairs {
        println!("── {mac} pair ──");
        let mut preds = Vec::new();
        for (s, measured) in [(a, ca), (b, cb)] {
            let p = compile_to_spatial_with(
                &gemm_program(s.0, s.1, s.2),
                &wh,
                NumericsPolicy::AllowSpillRounding,
            )
            .expect("compiles");
            let sm = p.summary();
            let plan = p.work.plan.expect("plan");
            let out = p.output_tensor().expect("out");
            let q = quantities_for_gemm(
                out.tiles.rows_t,
                out.tiles.cols_t,
                p.work.k_tiles,
                &plan,
                sm.multicast_groups > 0,
                wh.matmul_fidelity().unwrap(),
                &wh,
            );
            let e = wh.cycle.explain(&q);
            preds.push(e.total);
            println!("  {}x{}x{}  measured {measured:>7}  predicted {:>8.0}   sites {:>3}  iters {:>4}  tile_macs {:>7}", s.0, s.1, s.2, e.total, q.sites_used, q.iterations, q.tile_macs);
            println!("      dram/engine {:?}  dram_all {:>7}  cb_handoffs {:>6}  compute {:>8.0}  move ({:>8.0},{:>8.0})  fill {:>7.0}", q.dram_tiles_engine, q.dram_tiles_all, q.cb_handoffs, e.compute, e.move_engine.0, e.move_engine.1, e.fill);
        }
        let meas_gap = ca as f64 - cb as f64;
        let pred_gap = preds[0] - preds[1];
        println!(
            "  measured gap {meas_gap:>8.0}   predicted gap {pred_gap:>8.0}   {}\n",
            if meas_gap.signum() == pred_gap.signum() {
                "SAME SIGN"
            } else {
                "WRONG SIGN"
            }
        );
    }
}

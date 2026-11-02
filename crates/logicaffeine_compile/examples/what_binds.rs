//! What sets the pace of the plan the compiler chooses, shape by shape.
//!
//! "Are we at the floor?" is not answerable from a cycle count. It is answerable from which resource the
//! steady state waits on: a compute-bound plan is at the machine's arithmetic limit, a movement-bound one has
//! headroom that better buffering or fusion can still take, and a sync-bound one is paying for choreography.

use logicaffeine_compile::spatial::cost::{is_streaming, quantities_for_gemm, PaceBound};
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
    println!(
        "{:<18} {:<16} {:>8} {:>9} {:>9} {:>9} {:>8} {:>10}",
        "shape", "arrangement", "II", "compute", "move e0", "move e1", "sync", "bound"
    );
    for (m, n, k) in [
        (640usize, 640usize, 640usize),
        (2048, 1024, 512),
        (2048, 2048, 512),
        (2048, 2048, 1024),
    ] {
        for numerics in [NumericsPolicy::AllowSpillRounding, NumericsPolicy::Exact] {
            let Ok(p) = compile_to_spatial_with(&gemm_program(m, n, k), &wh, numerics) else {
                continue;
            };
            let s = p.summary();
            let Some(plan) = p.work.plan else { continue };
            let out = p.output_tensor().expect("output");
            let q = quantities_for_gemm(
                out.tiles.rows_t,
                out.tiles.cols_t,
                p.work.k_tiles,
                &plan,
                s.multicast_groups > 0,
                wh.matmul_fidelity().unwrap(),
                &wh,
            );
            let e = wh.cycle.explain(&q);
            let head = if numerics == NumericsPolicy::Exact {
                "exact"
            } else {
                "relaxed"
            };
            println!(
                "{:<18} {:<16} {:>8.0} {:>9.0} {:>9.0} {:>9.0} {:>8.0} {:>10}",
                format!("{m}x{n}x{k} {head}"),
                s.arrangement_label(),
                e.ii,
                e.compute,
                e.move_engine.0,
                e.move_engine.1,
                e.sync,
                match e.bound {
                    PaceBound::Compute => "compute".to_string(),
                    PaceBound::Movement(i) => format!("movement e{i}"),
                    PaceBound::Sync => "sync".to_string(),
                }
            );
            let _ = is_streaming(&plan);
        }
    }
}

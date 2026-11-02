//! Which term of the closed form disagrees with the built choreography on a ragged grid.
//!
//! `work_of_plan` predicts; `work_of` walks the built program and is exact. At 640^3 with ragged grids
//! enabled they differ by ~81 KB of NOC bytes, and generalising the multicast send counts and the compute
//! calls moved it by 40 bytes — so the gap is somewhere else. This prints every field of both.
//!
//!   cargo run -q --release -p logicaffeine-compile --features codegen --example probe_closed_form_gap

use logicaffeine_compile::spatial::ir::NumericsPolicy;
use logicaffeine_compile::spatial::passes::tile;
use logicaffeine_compile::spatial::pipeline::{run_passes, template_from_source_with};
use logicaffeine_compile::spatial::target::SpatialTarget;
use logicaffeine_compile::spatial::work::{work_of, Objective};

fn gemm(m: usize, n: usize, k: usize) -> String {
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
    let t = SpatialTarget::wormhole();
    for (m, n, k) in [(640usize, 640usize, 640usize)] {
        let src = gemm(m, n, k);
        let Ok(tm) = template_from_source_with(&src, &t, NumericsPolicy::AllowSpillRounding) else {
            continue;
        };
        let Some(c) = tile::best_candidate_for(&tm, &t, Objective::default()) else {
            continue;
        };
        let Ok(p) = run_passes(tm, &t) else {
            println!("{m}x{n}x{k}: passes refused");
            continue;
        };
        let built = work_of(&p, &t);
        let pred = &c.work;
        println!("=== {m}x{n}x{k} ===");
        println!("  plan {:?}", c.plan);
        println!(
            "  grid {:?}  per_site {}x{}  sub-block {}x{}",
            c.plan.grid, c.plan.per_site_m, c.plan.per_site_n, c.plan.block.osh, c.plan.block.osw
        );
        println!(
            "  site blocks: {:?}",
            c.plan
                .site_blocks(m / 32, n / 32)
                .iter()
                .map(|(g, b)| (*g, *b))
                .take(12)
                .collect::<Vec<_>>()
        );
        println!(
            "\n  {:>20} {:>14} {:>14} {:>12}",
            "field", "closed form", "built", "delta"
        );
        let rows: [(&str, u64, u64); 9] = [
            ("matmul_retired", pred.matmul_retired, built.matmul_retired),
            ("compute_calls", pred.compute_calls, built.compute_calls),
            ("noc_txns", pred.noc_txns, built.noc_txns),
            ("noc_bytes", pred.noc_bytes, built.noc_bytes),
            ("payload_bytes", pred.payload_bytes, built.payload_bytes),
            ("dram_bytes", pred.dram_bytes, built.dram_bytes),
            ("sites_used", pred.sites_used, built.sites_used),
            ("site_noc_bytes", pred.site_noc_bytes, built.site_noc_bytes),
            (
                "engine_noc_bytes",
                pred.engine_noc_bytes,
                built.engine_noc_bytes,
            ),
        ];
        for (name, a, b) in rows {
            println!(
                "  {:>20} {:>14} {:>14} {:>12}",
                name,
                a,
                b,
                b as i64 - a as i64
            );
        }
    }
}

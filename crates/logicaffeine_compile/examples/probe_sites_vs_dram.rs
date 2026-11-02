//! Does the objective leave the machine idle to save DRAM bytes?
//!
//! At 640³ on Blackhole the compiler places onto 100 of 130 sites and reports FEWER cycles than ttnn. At
//! 2048×2048×512 it places onto 64 — half the chip — and reports MORE. `Objective::Balanced` leads with
//! `dram_bytes`, and fewer, bigger blocks read fewer operand bytes: each A row is multicast to fewer
//! column-sites. So the objective has a reason to shrink the grid, and no term that notices the cores it
//! leaves idle.
//!
//! That is the same shape as the CUDA persistent defect fixed earlier today — `intensity_key` cancelled
//! the contraction depth, so depth fell to a tiebreak that reasoned about the benefit of a deeper stage
//! and was silent about its price in occupancy. Here the price is sites.
//!
//! This prints the candidate lattice with sites and DRAM side by side, so the trade is visible rather
//! than inferred.
//!
//!   cargo run -q --release -p logicaffeine-compile --features codegen --example probe_sites_vs_dram

use logicaffeine_compile::spatial::ir::NumericsPolicy;
use logicaffeine_compile::spatial::passes::tile;
use logicaffeine_compile::spatial::pipeline::template_from_source_with;
use logicaffeine_compile::spatial::target::SpatialTarget;
use logicaffeine_compile::spatial::work::Objective;

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
    let bh = SpatialTarget::blackhole();
    let mesh = bh.sites.sites();
    println!("Blackhole mesh: {} sites\n", mesh);

    for (m, n, k) in [
        (640usize, 640usize, 640usize),
        (2048, 1024, 512),
        (2048, 2048, 512),
    ] {
        let src = gemm(m, n, k);
        let Ok(t) = template_from_source_with(&src, &bh, NumericsPolicy::AllowSpillRounding) else {
            continue;
        };
        let mut cands = tile::candidates(&t, &bh);
        let obj = Objective::default();
        cands.sort_by_key(|c| obj.key(&c.work));
        let chosen = cands.first().map(|c| c.plan);

        println!("=== {m}x{n}x{k} ===");
        println!(
            "  {:>5} {:>5} {:>5} {:>9} {:>7} {:>13} {:>9} {:>11}  {}",
            "psm", "psn", "ibw", "grid", "sites", "dram_bytes", "txns", "site_mm", ""
        );
        // The best plan at each distinct site count, so the trade is one row per grid rather than a list.
        let mut best_at: std::collections::BTreeMap<usize, &tile::Candidate> = Default::default();
        for c in &cands {
            let sites = c.plan.grid.0 * c.plan.grid.1;
            let e = best_at.entry(sites).or_insert(c);
            if obj.key(&c.work) < obj.key(&e.work) {
                *e = c;
            }
        }
        for (sites, c) in best_at.iter().filter(|(s, _)| **s <= mesh) {
            println!(
                "  {:>5} {:>5} {:>5} {:>9} {:>7} {:>13} {:>9} {:>11}  {}",
                c.plan.per_site_m,
                c.plan.per_site_n,
                c.plan.block.ibw,
                format!("{}x{}", c.plan.grid.0, c.plan.grid.1),
                sites,
                c.work.dram_bytes,
                c.work.noc_txns,
                c.work.site_matmul_retired,
                if Some(c.plan) == chosen {
                    "★ CHOSEN"
                } else {
                    ""
                }
            );
        }
        // What the chosen plan gives up in parallelism, and what it buys in DRAM.
        if let (Some(ch), Some(widest)) = (
            chosen,
            best_at.keys().filter(|s| **s <= mesh).max().copied(),
        ) {
            let chosen_sites = ch.grid.0 * ch.grid.1;
            let w = best_at[&widest];
            let c = cands.iter().find(|c| c.plan == ch).unwrap();
            println!(
                "  chosen uses {chosen_sites}/{mesh} sites; the widest legal plan uses {widest}"
            );
            if widest > chosen_sites {
                println!("    widest would move {:+.1}% DRAM, {:+.1}% txns, and {:.2}x less per-site arithmetic",
                         100.0 * (w.work.dram_bytes as f64 / c.work.dram_bytes as f64 - 1.0),
                         100.0 * (w.work.noc_txns as f64 / c.work.noc_txns as f64 - 1.0),
                         c.work.site_matmul_retired as f64 / w.work.site_matmul_retired.max(1) as f64);
            }
        }
        println!();
    }
}

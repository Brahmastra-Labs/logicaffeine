//! What the corrected Blackhole L1 buys: the plan the compiler chooses at 1464 KiB (Wormhole's, which
//! Blackhole silently inherited through `..wormhole()`) against 1536 KiB (Blackhole's own, read from the
//! vendor's `blackhole_140_arch.yaml`).
//!
//! RESULT, swept over 2,197 shapes (tile counts 2..64 on each axis, 32x32 tiles):
//!
//!   2197 shapes compiled on both budgets; 20 chose a plan over 1464 KiB; 20 chose a DIFFERENT plan.
//!
//! Not one of the twenty is worse. Median -68% DRAM traffic, best -81.25% (1536x2048x768), and the
//! sharpest single case is 2048x2048x640: the plan Blackhole wants occupies 1,572,864 B — EXACTLY
//! 1536 KiB, the chip's whole L1 — so Wormhole's budget missed it by 72 KiB and the search fell back to
//! a narrower block on a (8,16) grid that re-reads its operands, shipping 71,303,168 DRAM bytes where
//! 13,631,488 would do. 2048x2048x640 is pinned by `spatial_chip_facts_lock`.
//!
//! ★ The first eight shapes probed showed NO difference, because none was L1-bound (`num_blocks: 1`,
//! footprints 16-1280 KiB). A negative result on eight hand-picked shapes is not a negative result.
//!
//!   cargo run -q --release -p logicaffeine-compile --features codegen --example probe_blackhole_l1
//!
use logicaffeine_compile::spatial::passes::tile;
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
    let mut wrong = bh.clone();
    wrong.site.local_mem_bytes = 1464 * 1024;
    wrong.dram.sram_kib = bh.sites.sites() * 1464;

    // A plan is pruned when its footprint exceeds the budget, and a blocked plan that FITS suppresses
    // the wrap fallback entirely — so the shapes where 72 KiB can change a decision are the ones whose
    // best mesh-fitting plan lands in (1464, 1536] KiB. Sweep for them rather than guessing.
    let tiles = [2usize, 4, 5, 8, 10, 12, 16, 20, 24, 32, 40, 48, 64];
    let (mut checked, mut diff, mut window) = (0usize, 0usize, 0usize);
    for &mt in &tiles {
        for &nt in &tiles {
            for &kt in &tiles {
                let (m, n, k) = (mt * 32, nt * 32, kt * 32);
                let src = gemm(m, n, k);
                let one = |t: &SpatialTarget| {
                    let p = logicaffeine_compile::spatial::pipeline::template_from_source(&src, t)
                        .ok()?;
                    tile::best_candidate_for(&p, t, Objective::default())
                };
                let (Some(a), Some(b)) = (one(&wrong), one(&bh)) else {
                    continue;
                };
                checked += 1;
                let win = a.footprint_bytes > 1464 * 1024 || b.footprint_bytes > 1464 * 1024;
                if win {
                    window += 1;
                }
                if a.plan != b.plan {
                    diff += 1;
                    println!("★ {m}x{n}x{k}");
                    println!(
                        "    1464: {:?}\n          footprint {} B, dram {}, txns {}",
                        a.plan, a.footprint_bytes, a.work.dram_bytes, a.work.noc_txns
                    );
                    println!(
                        "    1536: {:?}\n          footprint {} B, dram {}, txns {}",
                        b.plan, b.footprint_bytes, b.work.dram_bytes, b.work.noc_txns
                    );
                    println!(
                        "    dram {:+.2}%, txns {:+.2}%",
                        100.0 * (b.work.dram_bytes as f64 / a.work.dram_bytes as f64 - 1.0),
                        100.0 * (b.work.noc_txns as f64 / a.work.noc_txns as f64 - 1.0)
                    );
                }
            }
        }
    }
    println!("\n{checked} shapes compiled on both budgets; {window} chose a plan over 1464 KiB; {diff} chose a DIFFERENT plan.");
}

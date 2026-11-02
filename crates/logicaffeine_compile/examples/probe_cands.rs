//! Fused vs unfused candidate counts per (shape, numerics) — which combination has no fused family.
use logicaffeine_compile::spatial::ir::NumericsPolicy;
use logicaffeine_compile::spatial::passes::tile::{candidates, tile};
use logicaffeine_compile::spatial::pipeline::template_from_source_with;
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
    let wh = SpatialTarget::wormhole();
    for (m, n, k) in [
        (2048usize, 1024usize, 512usize),
        (2048, 2048, 512),
        (2048, 2048, 1024),
    ] {
        for numerics in [NumericsPolicy::Exact, NumericsPolicy::AllowSpillRounding] {
            let t = template_from_source_with(&gemm_program(m, n, k), &wh, numerics).unwrap();
            let cands = candidates(&t, &wh);
            let f = cands.iter().filter(|c| c.fused).count();
            let u = cands.iter().filter(|c| !c.fused).count();
            let s = match tile(&t, &wh) {
                Ok(p) => p.summary().arrangement_label().to_string(),
                Err(e) => format!("{e:?}"),
            };
            let bf = cands
                .iter()
                .filter(|c| c.fused)
                .min_by(|a, b| a.cycles.partial_cmp(&b.cycles).unwrap());
            let bu = cands
                .iter()
                .filter(|c| !c.fused)
                .min_by(|a, b| a.cycles.partial_cmp(&b.cycles).unwrap());
            let fc = bf.map(|c| c.cycles).unwrap_or(f64::NAN);
            let uc = bu.map(|c| c.cycles).unwrap_or(f64::NAN);
            let fd = bf.map(|c| c.quantities.dram_tiles_all).unwrap_or(0);
            let ud = bu.map(|c| c.quantities.dram_tiles_all).unwrap_or(0);
            println!(
                "{m}x{n}x{k} {:<8} chosen {s}",
                if numerics == NumericsPolicy::Exact {
                    "exact"
                } else {
                    "relaxed"
                }
            );
            let _ = (f, u, fd, ud, fc, uc);
            for (lbl, c) in [("best fused", bf), ("best unfused", bu)] {
                if let Some(c) = c {
                    let e = wh.cycle.explain(&c.quantities);
                    println!("    {lbl:<13} total {:>9.0}  setup {:>7.0}  compute {:>8.0}  move ({:>8.0},{:>8.0})  SYNC {:>8.0}  fill {:>7.0}",
                        e.total, e.setup, e.compute, e.move_engine.0, e.move_engine.1, e.sync, e.fill);
                }
            }
        }
    }
}
